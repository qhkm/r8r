//! Event backend trait and native implementation.
//!
//! Defines the `EventBackend` and `EventReceiver` traits for pub/sub event
//! delivery. The default `NativeEventBackend` uses `tokio::sync::broadcast`
//! for in-process messaging with zero external dependencies.

use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::debug;

use crate::triggers::event::{EventError, EventMessage};

/// Broadcast channel capacity for the native backend.
const NATIVE_CHANNEL_CAPACITY: usize = 4096;

/// Trait for event pub/sub backends.
///
/// Implementations deliver `EventMessage` values from publishers to subscribers.
/// The default backend uses an in-process broadcast channel; an optional Redis
/// backend is available behind the `redis` feature flag.
#[async_trait]
pub trait EventBackend: Send + Sync + 'static {
    /// Publish an event to all subscribers.
    async fn publish(&self, event: EventMessage) -> Result<(), EventError>;

    /// Create a new receiver that will get future published events.
    fn subscribe(&self) -> Box<dyn EventReceiver>;
}

/// Trait for receiving events from a backend.
#[async_trait]
pub trait EventReceiver: Send {
    /// Wait for the next event.
    ///
    /// Returns `Err` if the backend is shut down or the channel is closed.
    async fn recv(&mut self) -> Result<EventMessage, EventError>;
}

// ============================================================================
// Native (in-process) implementation
// ============================================================================

/// In-process event backend using `tokio::sync::broadcast`.
///
/// This is the default backend that requires no external dependencies.
/// All pub/sub happens within a single process.
pub struct NativeEventBackend {
    tx: broadcast::Sender<EventMessage>,
}

impl NativeEventBackend {
    /// Create a new native event backend.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(NATIVE_CHANNEL_CAPACITY);
        debug!(
            "Native event backend created with capacity {}",
            NATIVE_CHANNEL_CAPACITY
        );
        Self { tx }
    }
}

impl Default for NativeEventBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBackend for NativeEventBackend {
    async fn publish(&self, event: EventMessage) -> Result<(), EventError> {
        // If there are no receivers, send returns Err but that's fine —
        // it just means nobody is listening yet.
        match self.tx.send(event) {
            Ok(count) => {
                debug!("Published event to {} receiver(s)", count);
                Ok(())
            }
            Err(_) => {
                // No active receivers — this is not an error for broadcast
                debug!("Published event but no active receivers");
                Ok(())
            }
        }
    }

    fn subscribe(&self) -> Box<dyn EventReceiver> {
        Box::new(NativeEventReceiver {
            rx: self.tx.subscribe(),
        })
    }
}

/// Receiver wrapping `tokio::sync::broadcast::Receiver`.
struct NativeEventReceiver {
    rx: broadcast::Receiver<EventMessage>,
}

#[async_trait]
impl EventReceiver for NativeEventReceiver {
    async fn recv(&mut self) -> Result<EventMessage, EventError> {
        loop {
            match self.rx.recv().await {
                Ok(msg) => return Ok(msg),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Native event receiver lagged by {} messages", n);
                    // Continue receiving — we just missed some messages
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(EventError::Internal(
                        "Event broadcast channel closed".to_string(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_native_backend_publish_subscribe() {
        let backend = NativeEventBackend::new();
        let mut receiver = backend.subscribe();

        let event = EventMessage::new("test.event", json!({"key": "value"}));
        backend.publish(event.clone()).await.unwrap();

        let received = receiver.recv().await.unwrap();
        assert_eq!(received.event, "test.event");
        assert_eq!(received.data["key"], "value");
    }

    #[tokio::test]
    async fn test_native_backend_multiple_subscribers() {
        let backend = NativeEventBackend::new();
        let mut rx1 = backend.subscribe();
        let mut rx2 = backend.subscribe();

        let event = EventMessage::new("multi.test", json!({"n": 1}));
        backend.publish(event).await.unwrap();

        let msg1 = rx1.recv().await.unwrap();
        let msg2 = rx2.recv().await.unwrap();
        assert_eq!(msg1.event, "multi.test");
        assert_eq!(msg2.event, "multi.test");
    }

    #[tokio::test]
    async fn test_native_backend_no_receivers_ok() {
        let backend = NativeEventBackend::new();
        // Publishing with no receivers should not error
        let event = EventMessage::new("no.listeners", json!({}));
        assert!(backend.publish(event).await.is_ok());
    }
}
