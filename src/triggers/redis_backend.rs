//! Redis event backend (optional, behind `redis` feature flag).
//!
//! Provides cross-process event pub/sub via Redis pub/sub channels.
//! Enable with `--features redis` and set the `REDIS_URL` env var.

use async_trait::async_trait;
use redis::Client;
use tracing::{debug, info};

use crate::triggers::event::{EventError, EventMessage};
use crate::triggers::event_backend::{EventBackend, EventReceiver};

/// Redis channel prefix for r8r events.
const CHANNEL_PREFIX: &str = "r8r:events";

/// Redis connection timeout in seconds.
const REDIS_CONNECTION_TIMEOUT_SECS: u64 = 5;

/// Redis event backend using pub/sub channels.
///
/// Publishes events to `r8r:events` and subscribes to the same channel.
/// This enables cross-process event delivery when running multiple r8r instances.
pub struct RedisEventBackend {
    client: Client,
}

impl RedisEventBackend {
    /// Create a new Redis event backend.
    pub fn new(redis_url: &str) -> Result<Self, EventError> {
        let client = Client::open(redis_url).map_err(|e| EventError::Connection(e.to_string()))?;
        info!("Redis event backend created (url: {})", redis_url);
        Ok(Self { client })
    }
}

#[async_trait]
impl EventBackend for RedisEventBackend {
    async fn publish(&self, event: EventMessage) -> Result<(), EventError> {
        use redis::AsyncCommands;

        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let payload =
            serde_json::to_string(&event).map_err(|e| EventError::Serialization(e.to_string()))?;

        conn.publish::<_, _, ()>(CHANNEL_PREFIX, &payload)
            .await
            .map_err(|e| EventError::Publish(e.to_string()))?;

        debug!("Published event '{}' to Redis channel", event.event);
        Ok(())
    }

    fn subscribe(&self) -> Box<dyn EventReceiver> {
        Box::new(RedisEventReceiver {
            client: self.client.clone(),
            state: ReceiverState::NotConnected,
        })
    }
}

/// Internal state for lazy Redis subscription.
enum ReceiverState {
    NotConnected,
    Connected(redis::aio::PubSub),
}

/// Receiver wrapping a Redis pub/sub subscription.
struct RedisEventReceiver {
    client: Client,
    state: ReceiverState,
}

#[async_trait]
impl EventReceiver for RedisEventReceiver {
    async fn recv(&mut self) -> Result<EventMessage, EventError> {
        use futures_util::StreamExt;

        // Lazily connect and subscribe on first recv()
        if matches!(self.state, ReceiverState::NotConnected) {
            let pubsub_future = self.client.get_async_pubsub();
            let mut pubsub = tokio::time::timeout(
                std::time::Duration::from_secs(REDIS_CONNECTION_TIMEOUT_SECS),
                pubsub_future,
            )
            .await
            .map_err(|_| EventError::Connection("Redis connection timeout".to_string()))?
            .map_err(|e| EventError::Connection(e.to_string()))?;

            pubsub
                .subscribe(CHANNEL_PREFIX)
                .await
                .map_err(|e| EventError::Subscription(e.to_string()))?;

            info!("Redis event receiver subscribed to '{}'", CHANNEL_PREFIX);
            self.state = ReceiverState::Connected(pubsub);
        }

        let pubsub = match &mut self.state {
            ReceiverState::Connected(ps) => ps,
            ReceiverState::NotConnected => unreachable!(),
        };

        let mut stream = pubsub.on_message();

        match stream.next().await {
            Some(msg) => {
                let payload: String = msg
                    .get_payload()
                    .map_err(|e| EventError::Internal(e.to_string()))?;

                let event: EventMessage = serde_json::from_str(&payload)
                    .map_err(|e| EventError::Serialization(e.to_string()))?;

                Ok(event)
            }
            None => Err(EventError::Internal(
                "Redis pub/sub stream ended".to_string(),
            )),
        }
    }
}
