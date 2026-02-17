//! Delayed event processing using SQLite storage.
//!
//! This module provides functionality to schedule events for future processing,
//! with support for delayed triggers and scheduled workflow execution.
//! Events are stored in SQLite and polled by a background task.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::api::Monitor;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::triggers::event::{EventError, EventMessage};
use crate::triggers::event_backend::EventBackend;
use crate::workflow::parse_workflow;

/// Poll interval for checking due events (in milliseconds).
const POLL_INTERVAL_MS: u64 = 1000;

/// Delayed event entry stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelayedEvent {
    /// Unique identifier for this delayed event
    pub id: String,
    /// Event name/type
    pub event: String,
    /// Event payload data
    pub data: Value,
    /// Source of the event
    #[serde(default)]
    pub source: Option<String>,
    /// Correlation ID for tracing
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// When the event was originally created
    pub created_at: DateTime<Utc>,
    /// When the event should be processed
    pub scheduled_at: DateTime<Utc>,
    /// Target workflow name (optional, for direct workflow triggers)
    #[serde(default)]
    pub target_workflow: Option<String>,
    /// Number of retry attempts
    #[serde(default)]
    pub retry_count: u32,
    /// Maximum number of retries
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

fn default_max_retries() -> u32 {
    3
}

impl DelayedEvent {
    /// Create a new delayed event.
    pub fn new(event: impl Into<String>, data: Value, delay_seconds: u64) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            event: event.into(),
            data,
            source: None,
            correlation_id: None,
            created_at: now,
            scheduled_at: now + ChronoDuration::seconds(delay_seconds as i64),
            target_workflow: None,
            retry_count: 0,
            max_retries: default_max_retries(),
        }
    }

    /// Set the source of the event.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Set the correlation ID.
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Set the target workflow.
    pub fn with_target_workflow(mut self, workflow: impl Into<String>) -> Self {
        self.target_workflow = Some(workflow.into());
        self
    }

    /// Convert to EventMessage for processing.
    pub fn to_event_message(&self) -> EventMessage {
        EventMessage::new(&self.event, self.data.clone())
            .with_source(self.source.clone().unwrap_or_default())
            .with_correlation_id(self.correlation_id.clone().unwrap_or_default())
    }

    /// Check if this event is due for processing.
    pub fn is_due(&self) -> bool {
        Utc::now() >= self.scheduled_at
    }

    /// Calculate the delay in seconds from now.
    pub fn delay_seconds(&self) -> i64 {
        let now = Utc::now();
        if self.scheduled_at > now {
            (self.scheduled_at - now).num_seconds()
        } else {
            0
        }
    }
}

/// Processor for delayed events using SQLite storage and an EventBackend.
///
/// Polls SQLite for due events on a timer, processes them (either by
/// triggering a target workflow directly or re-publishing via the backend),
/// and handles retries with exponential backoff.
pub struct DelayedEventProcessor {
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    backend: Arc<dyn EventBackend>,
    monitor: Option<Arc<Monitor>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    poll_interval_ms: u64,
}

impl DelayedEventProcessor {
    /// Create a new delayed event processor.
    pub fn new(
        storage: SqliteStorage,
        registry: Arc<NodeRegistry>,
        backend: Arc<dyn EventBackend>,
    ) -> Self {
        Self {
            storage,
            registry,
            backend,
            monitor: None,
            shutdown_tx: None,
            handle: None,
            poll_interval_ms: POLL_INTERVAL_MS,
        }
    }

    /// Set custom poll interval.
    pub fn with_poll_interval(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }

    /// Attach a live execution monitor.
    pub fn with_monitor(mut self, monitor: Arc<Monitor>) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Start the background polling task.
    pub async fn start(&mut self) -> Result<(), EventError> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let backend = self.backend.clone();
        let monitor = self.monitor.clone();
        let poll_interval = self.poll_interval_ms;

        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(poll_interval));

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Delayed event processor received shutdown signal");
                        break;
                    }
                    _ = ticker.tick() => {
                        if let Err(e) = process_due_events(
                            &storage,
                            &registry,
                            &backend,
                            monitor.clone(),
                        ).await {
                            error!("Error processing delayed events: {}", e);
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        info!(
            "Delayed event processor started with {}ms poll interval",
            self.poll_interval_ms
        );
        Ok(())
    }

    /// Stop the background polling task.
    pub async fn stop(&mut self) -> Result<(), EventError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        if let Some(handle) = self.handle.take() {
            handle
                .await
                .map_err(|e| EventError::Internal(e.to_string()))?;
        }

        info!("Delayed event processor stopped");
        Ok(())
    }

    /// Check if the processor is running.
    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }
}

/// Process all due delayed events from SQLite.
async fn process_due_events(
    storage: &SqliteStorage,
    registry: &Arc<NodeRegistry>,
    backend: &Arc<dyn EventBackend>,
    monitor: Option<Arc<Monitor>>,
) -> Result<usize, EventError> {
    let due_events = storage
        .get_due_delayed_events()
        .await
        .map_err(|e| EventError::Storage(e.to_string()))?;

    if due_events.is_empty() {
        return Ok(0);
    }

    let mut processed = 0;

    for (event_id, _event_name, event_json) in due_events {
        // Parse the delayed event from stored JSON
        let event = match serde_json::from_str::<DelayedEvent>(&event_json) {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to parse delayed event '{}': {}", event_id, e);
                // Delete malformed event to prevent infinite retry
                let _ = storage.delete_delayed_event(&event_id).await;
                continue;
            }
        };

        // Delete from queue first (prevents re-processing on crash/restart)
        let _ = storage.delete_delayed_event(&event_id).await;

        match process_single_event(&event, storage, registry, backend, monitor.clone()).await {
            Ok(()) => {
                processed += 1;
            }
            Err(e) => {
                error!("Failed to process delayed event '{}': {}", event.id, e);

                // Retry logic with exponential backoff
                if event.retry_count < event.max_retries {
                    let mut retry_event = event.clone();
                    retry_event.retry_count += 1;
                    retry_event.scheduled_at =
                        Utc::now() + ChronoDuration::seconds(60 * retry_event.retry_count as i64);

                    let retry_json = serde_json::to_string(&retry_event)
                        .map_err(|e| EventError::Serialization(e.to_string()))?;

                    if let Err(store_err) = storage
                        .store_delayed_event(
                            &retry_event.event,
                            &retry_json,
                            retry_event.scheduled_at,
                        )
                        .await
                    {
                        error!("Failed to reschedule event for retry: {}", store_err);
                    } else {
                        info!(
                            "Scheduled retry {} for event '{}' (id: {})",
                            retry_event.retry_count, retry_event.event, retry_event.id
                        );
                    }
                }
            }
        }
    }

    if processed > 0 {
        debug!("Processed {} delayed event(s)", processed);
    }

    Ok(processed)
}

/// Process a single delayed event.
async fn process_single_event(
    event: &DelayedEvent,
    storage: &SqliteStorage,
    registry: &Arc<NodeRegistry>,
    backend: &Arc<dyn EventBackend>,
    monitor: Option<Arc<Monitor>>,
) -> Result<(), EventError> {
    info!(
        "Processing delayed event '{}' (id: {}, waited {}s)",
        event.event,
        event.id,
        (Utc::now() - event.created_at).num_seconds()
    );

    if let Some(ref workflow_name) = event.target_workflow {
        // Direct workflow trigger
        let stored = storage
            .get_workflow(workflow_name)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?
            .ok_or_else(|| EventError::WorkflowNotFound(workflow_name.to_string()))?;

        let workflow = parse_workflow(&stored.definition)
            .map_err(|e| EventError::WorkflowParse(e.to_string()))?;

        let input = serde_json::json!({
            "event": event.event,
            "data": event.data,
            "source": event.source,
            "correlation_id": event.correlation_id,
            "delayed": true,
            "scheduled_at": event.scheduled_at,
            "created_at": event.created_at,
        });

        let mut executor = Executor::new((**registry).clone(), storage.clone());
        if let Some(m) = monitor {
            executor = executor.with_monitor(m);
        }

        let trigger_type = format!("delayed_event:{}", event.event);

        match executor
            .execute(&workflow, &stored.id, &trigger_type, input)
            .await
        {
            Ok(execution) => {
                info!(
                    "Delayed event '{}' triggered workflow '{}' (execution: {})",
                    event.event, workflow_name, execution.id
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to execute workflow '{}' for delayed event '{}': {}",
                    workflow_name, event.event, e
                );
                Err(EventError::Execution(e.to_string()))
            }
        }
    } else {
        // Publish to event bus for normal event handling
        let event_msg = event.to_event_message();
        backend.publish(event_msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_delayed_event_creation() {
        let data = json!({"order_id": "12345"});
        let event = DelayedEvent::new("order.created", data.clone(), 3600);

        assert_eq!(event.event, "order.created");
        assert_eq!(event.data, data);
        assert!(event.delay_seconds() <= 3600);
        assert!(!event.is_due());
    }

    #[test]
    fn test_delayed_event_with_metadata() {
        let event = DelayedEvent::new("test.event", json!({}), 60)
            .with_source("test-service")
            .with_correlation_id("corr-123")
            .with_target_workflow("my-workflow");

        assert_eq!(event.source, Some("test-service".to_string()));
        assert_eq!(event.correlation_id, Some("corr-123".to_string()));
        assert_eq!(event.target_workflow, Some("my-workflow".to_string()));
    }

    #[test]
    fn test_delayed_event_to_event_message() {
        let event = DelayedEvent::new("test.event", json!({"key": "value"}), 60)
            .with_source("source")
            .with_correlation_id("corr");

        let msg = event.to_event_message();
        assert_eq!(msg.event, "test.event");
        assert_eq!(msg.data, json!({"key": "value"}));
        assert_eq!(msg.source, Some("source".to_string()));
        assert_eq!(msg.correlation_id, Some("corr".to_string()));
    }

    #[test]
    fn test_delayed_event_serialization() {
        let event = DelayedEvent::new("test.event", json!({"key": "value"}), 3600);

        let json_str = serde_json::to_string(&event).unwrap();
        let deserialized: DelayedEvent = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.event, event.event);
        assert_eq!(deserialized.data, event.data);
        assert_eq!(deserialized.id, event.id);
    }
}
