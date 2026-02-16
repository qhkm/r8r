//! Delayed event processing using Redis sorted sets.
//!
//! This module provides functionality to schedule events for future processing,
//! with support for delayed triggers and scheduled workflow execution.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use redis::{AsyncCommands, Client};
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
use crate::triggers::event::{EventError, EventMessage, EventPublisher};
use crate::workflow::parse_workflow;

/// Default Redis URL if not configured.
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// Redis key for delayed events sorted set.
const DELAYED_EVENTS_KEY: &str = "r8r:delayed_events";

/// Redis key for delayed events processing lock.
const DELAYED_EVENTS_LOCK: &str = "r8r:delayed_events:lock";

/// Poll interval for checking due events (in milliseconds).
const POLL_INTERVAL_MS: u64 = 1000;

/// Lock timeout to prevent multiple instances processing the same events.
const LOCK_TIMEOUT_SECONDS: u64 = 30;

/// Delayed event entry stored in Redis.
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

/// Queue for managing delayed events using Redis sorted sets.
pub struct DelayedEventQueue {
    client: Client,
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    monitor: Option<Arc<Monitor>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    poll_interval_ms: u64,
}

impl DelayedEventQueue {
    /// Create a new delayed event queue.
    pub fn new(storage: SqliteStorage, registry: Arc<NodeRegistry>) -> Result<Self, EventError> {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());
        let client = Client::open(redis_url).map_err(|e| EventError::Connection(e.to_string()))?;

        Ok(Self {
            client,
            storage,
            registry,
            monitor: None,
            shutdown_tx: None,
            handle: None,
            poll_interval_ms: POLL_INTERVAL_MS,
        })
    }

    /// Create with a custom Redis URL.
    pub fn with_redis_url(
        mut self,
        url: impl Into<String>,
    ) -> Result<Self, EventError> {
        let client = Client::open(url.into()).map_err(|e| EventError::Connection(e.to_string()))?;
        self.client = client;
        Ok(self)
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

    /// Schedule an event for delayed processing.
    pub async fn schedule_event(
        &self,
        event: impl Into<String>,
        data: Value,
        delay_seconds: u64,
    ) -> Result<String, EventError> {
        let delayed_event = DelayedEvent::new(event, data, delay_seconds);
        self.schedule_delayed_event(&delayed_event).await
    }

    /// Schedule a pre-built delayed event.
    pub async fn schedule_delayed_event(
        &self,
        event: &DelayedEvent,
    ) -> Result<String, EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        // Use scheduled timestamp as score for sorted set
        let score = event.scheduled_at.timestamp() as f64;
        let event_json =
            serde_json::to_string(event).map_err(|e| EventError::Serialization(e.to_string()))?;

        // Add to sorted set
        let added: i32 = conn
            .zadd(DELAYED_EVENTS_KEY, &event_json, score)
            .await
            .map_err(|e| EventError::Publish(e.to_string()))?;

        if added > 0 {
            info!(
                "Scheduled event '{}' (id: {}) for {} (delay: {}s)",
                event.event,
                event.id,
                event.scheduled_at,
                event.delay_seconds()
            );
        }

        Ok(event.id.clone())
    }

    /// Cancel a scheduled event by ID.
    ///
    /// TODO(review): This scans the entire sorted set (ZRANGE 0 -1) and deserializes
    /// every entry to find one by ID. O(n) at scale. Consider a secondary Redis hash
    /// mapping event ID to the sorted set member for O(1) lookup.
    pub async fn cancel_event(&self, event_id: &str) -> Result<bool, EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        // Get all events and find the one with matching ID
        let events: Vec<String> = conn
            .zrange(DELAYED_EVENTS_KEY, 0, -1)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        for event_json in events {
            if let Ok(event) = serde_json::from_str::<DelayedEvent>(&event_json) {
                if event.id == event_id {
                    let removed: i32 = conn
                        .zrem(DELAYED_EVENTS_KEY, &event_json)
                        .await
                        .map_err(|e| EventError::Storage(e.to_string()))?;

                    if removed > 0 {
                        info!("Cancelled delayed event '{}' (id: {})", event.event, event_id);
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Get all pending events.
    pub async fn get_pending_events(&self) -> Result<Vec<DelayedEvent>, EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let events: Vec<String> = conn
            .zrange(DELAYED_EVENTS_KEY, 0, -1)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        let mut result = Vec::new();
        for event_json in events {
            if let Ok(event) = serde_json::from_str::<DelayedEvent>(&event_json) {
                result.push(event);
            }
        }

        Ok(result)
    }

    /// Get events that are due for processing.
    pub async fn get_due_events(&self) -> Result<Vec<DelayedEvent>, EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let now = Utc::now().timestamp() as f64;

        // Get events with score <= now (due events)
        let events: Vec<String> = conn
            .zrangebyscore(DELAYED_EVENTS_KEY, 0 as f64, now)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        let mut result = Vec::new();
        for event_json in events {
            if let Ok(event) = serde_json::from_str::<DelayedEvent>(&event_json) {
                result.push(event);
            }
        }

        Ok(result)
    }

    /// Process events that are due.
    ///
    /// Uses an atomic Lua script to fetch and remove due events in one
    /// operation, preventing duplicate processing across instances.
    pub async fn process_due_events(&self) -> Result<usize, EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let now = Utc::now().timestamp() as f64;

        // Atomically pop due events
        let script = redis::Script::new(ATOMIC_POP_DUE_EVENTS_LUA);
        let event_jsons: Vec<String> = script
            .key(DELAYED_EVENTS_KEY)
            .arg(now)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        if event_jsons.is_empty() {
            return Ok(0);
        }

        let mut processed = 0;

        for event_json in event_jsons {
            let event = match serde_json::from_str::<DelayedEvent>(&event_json) {
                Ok(e) => e,
                Err(e) => {
                    warn!("Failed to parse delayed event: {}", e);
                    continue;
                }
            };

            if let Err(e) = self.process_event(&event).await {
                error!("Failed to process delayed event '{}': {}", event.id, e);

                // Retry logic
                if event.retry_count < event.max_retries {
                    let mut retry_event = event.clone();
                    retry_event.retry_count += 1;
                    retry_event.scheduled_at = Utc::now()
                        + ChronoDuration::seconds(60 * retry_event.retry_count as i64);

                    if let Err(e) = self.schedule_delayed_event(&retry_event).await {
                        error!("Failed to reschedule event for retry: {}", e);
                    } else {
                        info!(
                            "Scheduled retry {} for event '{}' (id: {})",
                            retry_event.retry_count, retry_event.event, retry_event.id
                        );
                    }
                }
            } else {
                processed += 1;
            }
            // No need to remove from queue — Lua script already did it
        }

        if processed > 0 {
            info!("Processed {} delayed event(s)", processed);
        }

        Ok(processed)
    }

    /// Remove an event from the queue.
    ///
    /// TODO(review): This re-serializes the event to match the Redis member string.
    /// If JSON field ordering differs between store and remove (e.g., HashMap fields),
    /// the ZREM will silently fail. Store the original JSON string alongside the event
    /// or use the event ID for lookup instead.
    async fn remove_event(&self, event: &DelayedEvent) -> Result<(), EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let event_json =
            serde_json::to_string(event).map_err(|e| EventError::Serialization(e.to_string()))?;

        conn.zrem::<_, _, ()>(DELAYED_EVENTS_KEY, event_json)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Process a single delayed event.
    async fn process_event(&self, event: &DelayedEvent) -> Result<(), EventError> {
        info!(
            "Processing delayed event '{}' (id: {}, waited {}s)",
            event.event,
            event.id,
            (Utc::now() - event.created_at).num_seconds()
        );

        if let Some(ref workflow_name) = event.target_workflow {
            // Direct workflow trigger
            self.trigger_workflow(workflow_name, event).await
        } else {
            // Publish to event bus for normal event handling
            let publisher = EventPublisher::new()?;
            let event_msg = event.to_event_message();

            publisher
                .publish(&event_msg)
                .await
                .map_err(|e| EventError::Publish(e.to_string()))?;

            Ok(())
        }
    }

    /// Trigger a specific workflow with the delayed event.
    async fn trigger_workflow(
        &self,
        workflow_name: &str,
        event: &DelayedEvent,
    ) -> Result<(), EventError> {
        let stored = self
            .storage
            .get_workflow(workflow_name)
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?
            .ok_or_else(|| EventError::WorkflowNotFound(workflow_name.to_string()))?;

        let workflow =
            parse_workflow(&stored.definition).map_err(|e| EventError::WorkflowParse(e.to_string()))?;

        let input = serde_json::json!({
            "event": event.event,
            "data": event.data,
            "source": event.source,
            "correlation_id": event.correlation_id,
            "delayed": true,
            "scheduled_at": event.scheduled_at,
            "created_at": event.created_at,
        });

        let mut executor = Executor::new((*self.registry).clone(), self.storage.clone());
        if let Some(ref monitor) = self.monitor {
            executor = executor.with_monitor(monitor.clone());
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
    }

    /// Start the background polling task.
    pub async fn start(&mut self) -> Result<(), EventError> {
        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone for the spawned task
        let client = self.client.clone();
        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let monitor = self.monitor.clone();
        let poll_interval = self.poll_interval_ms;

        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(poll_interval));

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Delayed event queue received shutdown signal");
                        break;
                    }
                    _ = ticker.tick() => {
                        // Try to acquire lock for processing
                        if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
                            let lock_key = DELAYED_EVENTS_LOCK;
                            let lock_value = uuid::Uuid::new_v4().to_string();
                            let lock_acquired: bool = redis::cmd("SET")
                                .arg(lock_key)
                                .arg(&lock_value)
                                .arg("NX")
                                .arg("EX")
                                .arg(LOCK_TIMEOUT_SECONDS)
                                .query_async(&mut conn)
                                .await
                                .unwrap_or(false);

                            if lock_acquired {
                                // Process due events
                                if let Err(e) = process_due_events_internal(
                                    &client,
                                    &storage,
                                    &registry,
                                    monitor.clone(),
                                ).await {
                                    error!("Error processing delayed events: {}", e);
                                }

                                // Release lock
                                let _: Result<(), _> = redis::cmd("DEL")
                                    .arg(lock_key)
                                    .query_async(&mut conn)
                                    .await;
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        info!("Delayed event queue started with {}ms poll interval", self.poll_interval_ms);
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

        info!("Delayed event queue stopped");
        Ok(())
    }

    /// Check if the queue is running.
    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }
}

/// Lua script that atomically fetches and removes due events from a sorted set.
/// This prevents duplicate processing when multiple instances race.
const ATOMIC_POP_DUE_EVENTS_LUA: &str = r#"
local key = KEYS[1]
local max_score = ARGV[1]
local events = redis.call('ZRANGEBYSCORE', key, 0, max_score)
if #events > 0 then
    for _, ev in ipairs(events) do
        redis.call('ZREM', key, ev)
    end
end
return events
"#;

/// Internal function to process due events (used by background task).
async fn process_due_events_internal(
    client: &Client,
    storage: &SqliteStorage,
    registry: &Arc<NodeRegistry>,
    monitor: Option<Arc<Monitor>>,
) -> Result<usize, EventError> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| EventError::Connection(e.to_string()))?;

    let now = Utc::now().timestamp() as f64;

    // Atomically fetch and remove due events using a Lua script.
    // This prevents duplicate processing when the lock expires and
    // another instance picks up the same events.
    let script = redis::Script::new(ATOMIC_POP_DUE_EVENTS_LUA);
    let events: Vec<String> = script
        .key(DELAYED_EVENTS_KEY)
        .arg(now)
        .invoke_async(&mut conn)
        .await
        .map_err(|e| EventError::Storage(e.to_string()))?;

    if events.is_empty() {
        return Ok(0);
    }

    let mut processed = 0;

    for event_json in events {

        // Parse and process
        match serde_json::from_str::<DelayedEvent>(&event_json) {
            Ok(event) => {
                if let Err(e) =
                    process_single_event(&event, storage, registry, monitor.clone()).await
                {
                    error!("Failed to process delayed event '{}': {}", event.id, e);

                    // Schedule retry if applicable
                    if event.retry_count < event.max_retries {
                        let mut retry_event = event.clone();
                        retry_event.retry_count += 1;
                        retry_event.scheduled_at = Utc::now()
                            + ChronoDuration::seconds(60 * retry_event.retry_count as i64);

                        let retry_json = serde_json::to_string(&retry_event)
                            .map_err(|e| EventError::Serialization(e.to_string()))?;
                        let score = retry_event.scheduled_at.timestamp() as f64;

                        let _: i32 = conn
                            .zadd(DELAYED_EVENTS_KEY, retry_json, score)
                            .await
                            .map_err(|e| EventError::Storage(e.to_string()))?;

                        info!(
                            "Scheduled retry {} for event '{}' (id: {})",
                            retry_event.retry_count, retry_event.event, retry_event.id
                        );
                    }
                } else {
                    processed += 1;
                }
            }
            Err(e) => {
                warn!("Failed to parse delayed event: {}", e);
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

        let workflow =
            parse_workflow(&stored.definition).map_err(|e| EventError::WorkflowParse(e.to_string()))?;

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

        match executor.execute(&workflow, &stored.id, &trigger_type, input).await {
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
        let publisher = EventPublisher::new()?;
        let event_msg = event.to_event_message();

        publisher
            .publish(&event_msg)
            .await
            .map_err(|e| EventError::Publish(e.to_string()))?;

        Ok(())
    }
}

/// Errors specific to delayed event processing.
///
/// TODO(review): This enum is unused — all delayed event operations use EventError
/// from event.rs. Either use this type or remove it.
#[derive(Debug, thiserror::Error)]
pub enum DelayedEventError {
    #[error("Redis connection error: {0}")]
    Connection(String),

    #[error("Event not found: {0}")]
    NotFound(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Internal error: {0}")]
    Internal(String),
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
        assert!(event.delay_seconds() <= 3600); // May be slightly less due to processing time
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
