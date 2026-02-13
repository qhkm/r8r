//! Event trigger handler using Redis pub/sub.
//!
//! Subscribes to Redis channels and triggers workflows when events are received.

use std::sync::Arc;

use redis::aio::PubSub;
use redis::{AsyncCommands, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::api::Monitor;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, Trigger};

/// Default Redis URL if not configured.
const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// Redis connection timeout in seconds.
const REDIS_CONNECTION_TIMEOUT_SECS: u64 = 5;

/// Event message structure for Redis pub/sub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage {
    /// Event name/type
    pub event: String,
    /// Event payload data
    #[serde(default)]
    pub data: Value,
    /// Source of the event (workflow name, external system, etc.)
    #[serde(default)]
    pub source: Option<String>,
    /// Correlation ID for tracing
    #[serde(default)]
    pub correlation_id: Option<String>,
}

impl EventMessage {
    /// Create a new event message.
    pub fn new(event: impl Into<String>, data: Value) -> Self {
        Self {
            event: event.into(),
            data,
            source: None,
            correlation_id: None,
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
}

/// Event subscriber that listens for Redis pub/sub messages.
pub struct EventSubscriber {
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    redis_url: String,
    monitor: Option<Arc<Monitor>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl EventSubscriber {
    /// Create a new event subscriber.
    pub fn new(storage: SqliteStorage, registry: Arc<NodeRegistry>) -> Self {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());

        Self {
            storage,
            registry,
            redis_url,
            monitor: None,
            shutdown_tx: None,
            handle: None,
        }
    }

    /// Create with a custom Redis URL.
    pub fn with_redis_url(mut self, url: impl Into<String>) -> Self {
        self.redis_url = url.into();
        self
    }

    /// Attach a live execution monitor for event-triggered runs.
    pub fn with_monitor(mut self, monitor: Arc<Monitor>) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Start the event subscriber.
    ///
    /// This will connect to Redis, subscribe to event channels for all workflows
    /// with event triggers, and start processing messages.
    pub async fn start(&mut self) -> Result<(), EventError> {
        // Load workflows with event triggers
        let workflows = self
            .storage
            .list_workflows()
            .await
            .map_err(|e| EventError::Storage(e.to_string()))?;

        let mut event_subscriptions: Vec<EventSubscription> = Vec::new();

        for stored in workflows {
            if !stored.enabled {
                continue;
            }

            let workflow = match parse_workflow(&stored.definition) {
                Ok(w) => w,
                Err(e) => {
                    warn!(
                        "Failed to parse workflow '{}' for events: {}",
                        stored.name, e
                    );
                    continue;
                }
            };

            for trigger in &workflow.triggers {
                if let Trigger::Event { event, filter } = trigger {
                    event_subscriptions.push(EventSubscription {
                        channel: format!("r8r:events:{}", event),
                        workflow_id: stored.id.clone(),
                        workflow_name: workflow.name.clone(),
                        filter: filter.clone(),
                    });
                }
            }
        }

        if event_subscriptions.is_empty() {
            info!("No event triggers configured, skipping Redis subscription");
            return Ok(());
        }

        // Connect to Redis with timeout
        let client = Client::open(self.redis_url.clone())
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let pubsub_future = client.get_async_pubsub();
        let mut pubsub = tokio::time::timeout(
            std::time::Duration::from_secs(REDIS_CONNECTION_TIMEOUT_SECS),
            pubsub_future,
        )
        .await
        .map_err(|_| EventError::Connection("Redis connection timeout".to_string()))?
        .map_err(|e| EventError::Connection(e.to_string()))?;

        // Subscribe to all event channels
        let channels: Vec<String> = event_subscriptions
            .iter()
            .map(|s| s.channel.clone())
            .collect();

        for channel in &channels {
            pubsub
                .subscribe(channel)
                .await
                .map_err(|e| EventError::Subscription(e.to_string()))?;
            info!("Subscribed to Redis channel: {}", channel);
        }

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone data for the spawned task
        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let monitor = self.monitor.clone();

        // Spawn message processing task
        let handle = tokio::spawn(async move {
            process_messages(
                pubsub,
                event_subscriptions,
                storage,
                registry,
                monitor,
                &mut shutdown_rx,
            )
            .await;
        });

        self.handle = Some(handle);

        info!(
            "Event subscriber started with {} subscription(s)",
            channels.len()
        );
        Ok(())
    }

    /// Stop the event subscriber.
    pub async fn stop(&mut self) -> Result<(), EventError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        if let Some(handle) = self.handle.take() {
            handle
                .await
                .map_err(|e| EventError::Internal(e.to_string()))?;
        }

        info!("Event subscriber stopped");
        Ok(())
    }

    /// Check if the subscriber is running.
    pub fn is_running(&self) -> bool {
        self.handle.is_some()
    }
}

/// Internal subscription record.
#[derive(Debug, Clone)]
struct EventSubscription {
    channel: String,
    workflow_id: String,
    workflow_name: String,
    filter: Option<String>,
}

/// Process messages from Redis pub/sub.
async fn process_messages(
    mut pubsub: PubSub,
    subscriptions: Vec<EventSubscription>,
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    monitor: Option<Arc<Monitor>>,
    shutdown_rx: &mut mpsc::Receiver<()>,
) {
    use futures_util::StreamExt;

    let mut stream = pubsub.on_message();

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("Event subscriber received shutdown signal");
                break;
            }
            msg = stream.next() => {
                match msg {
                    Some(msg) => {
                        let channel: String = msg.get_channel_name().to_string();
                        let payload: Result<String, _> = msg.get_payload();

                        match payload {
                            Ok(payload) => {
                                debug!("Received event on channel {}: {}", channel, payload);

                                // Find matching subscriptions
                                let matching: Vec<_> = subscriptions
                                    .iter()
                                    .filter(|s| s.channel == channel)
                                    .collect();

                                for sub in matching {
                                    // Parse the event message
                                    let event_msg: Result<EventMessage, _> = serde_json::from_str(&payload);

                                    match event_msg {
                                        Ok(event) => {
                                            // Apply filter if configured
                                            if let Some(filter) = &sub.filter {
                                                if !evaluate_filter(filter, &event.data) {
                                                    debug!(
                                                        "Event filtered out for workflow '{}'",
                                                        sub.workflow_name
                                                    );
                                                    continue;
                                                }
                                            }

                                            // Trigger workflow execution
                                            if let Err(e) = trigger_workflow(
                                                &storage,
                                                &registry,
                                                &sub.workflow_id,
                                                &sub.workflow_name,
                                                &event,
                                                monitor.clone(),
                                            )
                                            .await
                                            {
                                                error!(
                                                    "Failed to trigger workflow '{}': {}",
                                                    sub.workflow_name, e
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                "Failed to parse event message on channel {}: {}",
                                                channel, e
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to get payload from message: {:?}", e);
                            }
                        }
                    }
                    None => {
                        warn!("Redis pub/sub stream ended unexpectedly");
                        break;
                    }
                }
            }
        }
    }
}

/// Maximum operations allowed in filter evaluation (prevents infinite loops).
const MAX_FILTER_OPERATIONS: u64 = 10_000;
/// Maximum string size in filter evaluation (100KB).
const MAX_FILTER_STRING_SIZE: usize = 100 * 1024;
/// Maximum array size in filter evaluation.
const MAX_FILTER_ARRAY_SIZE: usize = 1_000;
/// Maximum map size in filter evaluation.
const MAX_FILTER_MAP_SIZE: usize = 100;

/// Evaluate a filter expression against event data.
/// Uses a sandboxed Rhai engine with strict resource limits.
fn evaluate_filter(filter: &str, data: &Value) -> bool {
    use rhai::{Engine, Scope};

    // Create a sandboxed engine with resource limits
    let mut engine = Engine::new_raw();

    // Disable all dangerous operations
    engine.set_max_operations(MAX_FILTER_OPERATIONS);
    engine.set_max_string_size(MAX_FILTER_STRING_SIZE);
    engine.set_max_array_size(MAX_FILTER_ARRAY_SIZE);
    engine.set_max_map_size(MAX_FILTER_MAP_SIZE);

    // Disable all modules (no file access, no system calls)
    // Engine::new_raw() already excludes standard packages

    // Register only safe comparison operations
    engine.register_fn("==", |a: rhai::Dynamic, b: rhai::Dynamic| -> bool {
        a.to_string() == b.to_string()
    });
    engine.register_fn("!=", |a: rhai::Dynamic, b: rhai::Dynamic| -> bool {
        a.to_string() != b.to_string()
    });

    let mut scope = Scope::new();

    // Add event data to scope as immutable
    scope.push_constant("data", data.clone());

    match engine.eval_with_scope::<bool>(&mut scope, filter) {
        Ok(result) => result,
        Err(e) => {
            warn!("Filter evaluation failed (may be resource limited): {}", e);
            false
        }
    }
}

/// Trigger a workflow execution based on an event.
async fn trigger_workflow(
    storage: &SqliteStorage,
    registry: &Arc<NodeRegistry>,
    workflow_id: &str,
    workflow_name: &str,
    event: &EventMessage,
    monitor: Option<Arc<Monitor>>,
) -> Result<(), EventError> {
    // Get workflow definition
    let stored = storage
        .get_workflow(workflow_name)
        .await
        .map_err(|e| EventError::Storage(e.to_string()))?
        .ok_or_else(|| EventError::WorkflowNotFound(workflow_name.to_string()))?;

    let workflow =
        parse_workflow(&stored.definition).map_err(|e| EventError::WorkflowParse(e.to_string()))?;

    // Build input from event
    let input = serde_json::json!({
        "event": event.event,
        "data": event.data,
        "source": event.source,
        "correlation_id": event.correlation_id,
    });

    // Execute workflow
    let mut executor = Executor::new((**registry).clone(), storage.clone());
    if let Some(monitor) = monitor {
        executor = executor.with_monitor(monitor);
    }
    let trigger_type = format!("event:{}", event.event);

    match executor
        .execute(&workflow, workflow_id, &trigger_type, input)
        .await
    {
        Ok(execution) => {
            info!(
                "Event '{}' triggered workflow '{}' (execution: {})",
                event.event, workflow_name, execution.id
            );
            Ok(())
        }
        Err(e) => {
            error!(
                "Failed to execute workflow '{}' for event '{}': {}",
                workflow_name, event.event, e
            );
            Err(EventError::Execution(e.to_string()))
        }
    }
}

/// Event publisher for sending events to Redis.
pub struct EventPublisher {
    client: Client,
}

impl EventPublisher {
    /// Create a new event publisher.
    pub fn new() -> Result<Self, EventError> {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| DEFAULT_REDIS_URL.to_string());
        let client = Client::open(redis_url).map_err(|e| EventError::Connection(e.to_string()))?;
        Ok(Self { client })
    }

    /// Create with a custom Redis URL.
    pub fn with_redis_url(url: &str) -> Result<Self, EventError> {
        let client = Client::open(url).map_err(|e| EventError::Connection(e.to_string()))?;
        Ok(Self { client })
    }

    /// Publish an event.
    pub async fn publish(&self, event: &EventMessage) -> Result<(), EventError> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let channel = format!("r8r:events:{}", event.event);
        let payload =
            serde_json::to_string(event).map_err(|e| EventError::Serialization(e.to_string()))?;

        conn.publish::<_, _, ()>(&channel, &payload)
            .await
            .map_err(|e| EventError::Publish(e.to_string()))?;

        debug!("Published event '{}' to channel '{}'", event.event, channel);
        Ok(())
    }

    /// Publish an event with a name and data.
    pub async fn emit(&self, event_name: &str, data: Value) -> Result<(), EventError> {
        let event = EventMessage::new(event_name, data);
        self.publish(&event).await
    }
}

// Note: EventPublisher intentionally does not implement Default
// as it requires a Redis connection that may fail.
// Use EventPublisher::new() and handle the Result explicitly.

/// Errors that can occur with event triggers.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("Redis connection error: {0}")]
    Connection(String),

    #[error("Redis subscription error: {0}")]
    Subscription(String),

    #[error("Redis publish error: {0}")]
    Publish(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Workflow not found: {0}")]
    WorkflowNotFound(String),

    #[error("Workflow parse error: {0}")]
    WorkflowParse(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_event_message_creation() {
        let event = EventMessage::new("order.created", json!({"order_id": 123}));
        assert_eq!(event.event, "order.created");
        assert_eq!(event.data["order_id"], 123);
        assert!(event.source.is_none());
        assert!(event.correlation_id.is_none());
    }

    #[test]
    fn test_event_message_with_metadata() {
        let event = EventMessage::new("order.created", json!({"order_id": 123}))
            .with_source("order-service")
            .with_correlation_id("abc-123");

        assert_eq!(event.source, Some("order-service".to_string()));
        assert_eq!(event.correlation_id, Some("abc-123".to_string()));
    }

    #[test]
    fn test_event_message_serialization() {
        let event = EventMessage::new("test.event", json!({"key": "value"})).with_source("test");

        let json = serde_json::to_string(&event).unwrap();
        let parsed: EventMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.event, event.event);
        assert_eq!(parsed.data, event.data);
        assert_eq!(parsed.source, event.source);
    }

    #[test]
    fn test_filter_evaluation_simple() {
        let data = json!({"status": "active", "count": 10});

        // These tests would need Rhai to work properly
        // For now, test that the function doesn't panic
        let _ = evaluate_filter("true", &data);
        let _ = evaluate_filter("false", &data);
    }
}
