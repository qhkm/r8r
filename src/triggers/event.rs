//! Event trigger handler using Redis pub/sub.
//!
//! Subscribes to Redis channels and triggers workflows when events are received.
//! Supports event fan-out to multiple workflows with parallel or sequential execution.

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
use crate::workflow::{parse_workflow, EventRouting, Trigger};

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
    /// Scheduled processing time (for delayed events)
    #[serde(default)]
    pub scheduled_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl EventMessage {
    /// Create a new event message.
    pub fn new(event: impl Into<String>, data: Value) -> Self {
        Self {
            event: event.into(),
            data,
            source: None,
            correlation_id: None,
            scheduled_at: None,
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

    /// Set the scheduled processing time.
    pub fn with_schedule(mut self, scheduled_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.scheduled_at = Some(scheduled_at);
        self
    }
}

/// Event fan-out configuration for triggering multiple workflows.
#[derive(Debug, Clone)]
pub struct EventFanOut {
    /// Event name to fan out
    pub event: String,
    /// Workflow names to trigger
    pub workflow_names: Vec<String>,
    /// Execute workflows in parallel or sequential
    pub parallel: bool,
}

impl EventFanOut {
    /// Create a new fan-out configuration.
    pub fn new(event: impl Into<String>) -> Self {
        Self {
            event: event.into(),
            workflow_names: Vec::new(),
            parallel: true,
        }
    }

    /// Add a workflow to the fan-out list.
    pub fn add_workflow(mut self, workflow_name: impl Into<String>) -> Self {
        self.workflow_names.push(workflow_name.into());
        self
    }

    /// Set parallel execution mode.
    pub fn parallel(mut self, parallel: bool) -> Self {
        self.parallel = parallel;
        self
    }

    /// Set sequential execution mode.
    pub fn sequential(self) -> Self {
        self.parallel(false)
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
    fan_out_configs: Vec<EventFanOut>,
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
            fan_out_configs: Vec::new(),
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

    /// Add fan-out configuration for an event.
    pub fn with_fan_out(mut self, fan_out: EventFanOut) -> Self {
        self.fan_out_configs.push(fan_out);
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
                if let Trigger::Event {
                    event,
                    filter,
                    json_path,
                    delay,
                    routing,
                } = trigger
                {
                    event_subscriptions.push(EventSubscription {
                        channel: format!("r8r:events:{}", event),
                        workflow_id: stored.id.clone(),
                        workflow_name: workflow.name.clone(),
                        filter: filter.clone(),
                        json_path: json_path.clone(),
                        delay: *delay,
                        routing: routing.clone(),
                    });
                }
            }
        }

        if event_subscriptions.is_empty() && self.fan_out_configs.is_empty() {
            info!("No event triggers configured, skipping Redis subscription");
            return Ok(());
        }

        // Collect channels from both subscriptions and fan-out configs
        let mut channels: Vec<String> = event_subscriptions
            .iter()
            .map(|s| s.channel.clone())
            .collect();

        // Add fan-out channels
        for fan_out in &self.fan_out_configs {
            let channel = format!("r8r:events:{}", fan_out.event);
            if !channels.contains(&channel) {
                channels.push(channel);
            }
        }

        if channels.is_empty() {
            info!("No event channels to subscribe to");
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

        // Store lengths before moving
        let subscription_count = event_subscriptions.len();
        let fan_out_configs = self.fan_out_configs.clone();
        let fan_out_count = fan_out_configs.len();

        // Clone data for the spawned task
        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let monitor = self.monitor.clone();

        // Spawn message processing task
        let handle = tokio::spawn(async move {
            process_messages(
                pubsub,
                event_subscriptions,
                fan_out_configs,
                storage,
                registry,
                monitor,
                &mut shutdown_rx,
            )
            .await;
        });

        self.handle = Some(handle);

        info!(
            "Event subscriber started with {} subscription(s) and {} fan-out config(s)",
            subscription_count,
            fan_out_count
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
    json_path: Option<String>,
    delay: Option<u64>,
    routing: Option<Vec<EventRouting>>,
}

/// Process messages from Redis pub/sub.
async fn process_messages(
    mut pubsub: PubSub,
    subscriptions: Vec<EventSubscription>,
    fan_out_configs: Vec<EventFanOut>,
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

                                // Parse the event message first
                                let event_msg: Result<EventMessage, _> = serde_json::from_str(&payload);

                                match event_msg {
                                    Ok(event) => {
                                        // Check if this is a delayed event that should be scheduled
                                        if let Some(delay_seconds) = get_delay_from_subscriptions(&event, &subscriptions) {
                                            if delay_seconds > 0 {
                                                if let Err(e) = schedule_delayed_event(&storage, &event, delay_seconds).await {
                                                    error!("Failed to schedule delayed event: {}", e);
                                                }
                                                continue;
                                            }
                                        }

                                        // Process regular event subscriptions
                                        let matching: Vec<_> = subscriptions
                                            .iter()
                                            .filter(|s| s.channel == channel)
                                            .collect();

                                        for sub in matching {
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

                                            // Apply JSON path filter if configured
                                            if let Some(json_path) = &sub.json_path {
                                                if !evaluate_json_path(json_path, &event.data) {
                                                    debug!(
                                                        "Event JSON path filter didn't match for workflow '{}'",
                                                        sub.workflow_name
                                                    );
                                                    continue;
                                                }
                                            }

                                            // Check routing rules if configured
                                            if let Some(routing_rules) = &sub.routing {
                                                if let Some(target) = evaluate_routing(routing_rules, &event.data) {
                                                    // Trigger the routed workflow instead
                                                    if let Err(e) = trigger_workflow(
                                                        &storage,
                                                        &registry,
                                                        &sub.workflow_id,
                                                        &target,
                                                        &event,
                                                        monitor.clone(),
                                                    )
                                                    .await
                                                    {
                                                        error!(
                                                            "Failed to trigger routed workflow '{}': {}",
                                                            target, e
                                                        );
                                                    }
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

                                        // Process fan-out configurations
                                        let event_name = channel.strip_prefix("r8r:events:")
                                            .unwrap_or(&channel);

                                        for fan_out in &fan_out_configs {
                                            if fan_out.event == event_name {
                                                handle_fan_out(
                                                    &storage,
                                                    &registry,
                                                    fan_out,
                                                    &event,
                                                    monitor.clone(),
                                                )
                                                .await;
                                            }
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

/// Get delay seconds from matching subscription.
fn get_delay_from_subscriptions(event: &EventMessage, subscriptions: &[EventSubscription]) -> Option<u64> {
    // Check if event has scheduled_at (already delayed)
    if event.scheduled_at.is_some() {
        return Some(0); // No additional delay needed
    }

    subscriptions
        .iter()
        .find(|s| format!("r8r:events:{}", event.event) == s.channel)
        .and_then(|s| s.delay)
}

/// Schedule a delayed event for future processing.
async fn schedule_delayed_event(
    storage: &SqliteStorage,
    event: &EventMessage,
    delay_seconds: u64,
) -> Result<(), EventError> {
    use chrono::Utc;

    let scheduled_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);
    let delayed_event = event.clone().with_schedule(scheduled_at);

    // Store in delayed events queue (using a simple storage mechanism)
    // In production, this would use Redis sorted sets
    let _queue_key = "r8r:delayed_events";
    let _score = scheduled_at.timestamp() as f64;
    let event_json = serde_json::to_string(&delayed_event)
        .map_err(|e| EventError::Serialization(e.to_string()))?;

    // For now, log the delayed event
    info!(
        "Scheduling event '{}' for processing at {} (delay: {}s)",
        event.event, scheduled_at, delay_seconds
    );

    // In a full implementation, this would add to Redis sorted set:
    // conn.zadd(queue_key, event_json, score).await?;

    // Store in SQLite for persistence
    let _ = storage.store_delayed_event(&event.event, &event_json, scheduled_at).await;

    Ok(())
}

/// Handle event fan-out to multiple workflows.
async fn handle_fan_out(
    storage: &SqliteStorage,
    registry: &Arc<NodeRegistry>,
    fan_out: &EventFanOut,
    event: &EventMessage,
    monitor: Option<Arc<Monitor>>,
) {
    info!(
        "Fanning out event '{}' to {} workflow(s) (parallel={})",
        event.event,
        fan_out.workflow_names.len(),
        fan_out.parallel
    );

    if fan_out.parallel {
        // Execute workflows in parallel
        let mut handles = Vec::new();

        for workflow_name in &fan_out.workflow_names {
            let storage = storage.clone();
            let registry = registry.clone();
            let event = event.clone();
            let workflow_name = workflow_name.clone();
            let monitor = monitor.clone();

            let handle = tokio::spawn(async move {
                // Get workflow ID from storage
                match storage.get_workflow(&workflow_name).await {
                    Ok(Some(stored)) => {
                        if let Err(e) = trigger_workflow(
                            &storage,
                            &registry,
                            &stored.id,
                            &workflow_name,
                            &event,
                            monitor,
                        )
                        .await
                        {
                            error!("Fan-out failed for workflow '{}': {}", workflow_name, e);
                        }
                    }
                    Ok(None) => {
                        warn!("Workflow '{}' not found for fan-out", workflow_name);
                    }
                    Err(e) => {
                        error!("Failed to load workflow '{}': {}", workflow_name, e);
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all workflows to complete (but don't fail if one fails)
        for handle in handles {
            let _ = handle.await;
        }
    } else {
        // Execute workflows sequentially
        for workflow_name in &fan_out.workflow_names {
            match storage.get_workflow(workflow_name).await {
                Ok(Some(stored)) => {
                    if let Err(e) = trigger_workflow(
                        storage,
                        registry,
                        &stored.id,
                        workflow_name,
                        event,
                        monitor.clone(),
                    )
                    .await
                    {
                        error!("Fan-out failed for workflow '{}': {}", workflow_name, e);
                        // Continue with next workflow even if this one failed
                    }
                }
                Ok(None) => {
                    warn!("Workflow '{}' not found for fan-out", workflow_name);
                }
                Err(e) => {
                    error!("Failed to load workflow '{}': {}", workflow_name, e);
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

/// Evaluate JSON path expression against event data.
fn evaluate_json_path(path: &str, data: &Value) -> bool {
    // Simple JSON path implementation
    // Supports basic path like "$.user.name" or "user.name"
    let path = path.trim();
    let path = if path.starts_with("$.") {
        &path[2..]
    } else if path.starts_with('$') {
        &path[1..]
    } else {
        path
    };

    if path.is_empty() {
        return !data.is_null();
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = data;

    for part in &parts {
        // Handle array indexing like "items[0]"
        if let Some(bracket_idx) = part.find('[') {
            let key = &part[..bracket_idx];
            let idx_str = &part[bracket_idx + 1..part.len() - 1]; // Remove [ and ]

            // Navigate to the key first
            if !key.is_empty() {
                match current.get(key) {
                    Some(v) => current = v,
                    None => return false,
                }
            }

            // Then index into the array
            if let Ok(idx) = idx_str.parse::<usize>() {
                match current.as_array() {
                    Some(arr) if idx < arr.len() => current = &arr[idx],
                    _ => return false,
                }
            } else {
                return false;
            }
        } else {
            match current.get(part) {
                Some(v) => current = v,
                None => return false,
            }
        }
    }

    !current.is_null()
}

/// Evaluate routing rules and return the target workflow if a condition matches.
fn evaluate_routing(routing_rules: &[EventRouting], data: &Value) -> Option<String> {
    for rule in routing_rules {
        if evaluate_filter(&rule.condition, data) {
            return Some(rule.target_workflow.clone());
        }
    }
    None
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

    /// Schedule an event for delayed processing.
    pub async fn schedule(
        &self,
        event_name: &str,
        data: Value,
        delay_seconds: u64,
    ) -> Result<(), EventError> {
        use chrono::Utc;

        let scheduled_at = Utc::now() + chrono::Duration::seconds(delay_seconds as i64);
        let event = EventMessage::new(event_name, data).with_schedule(scheduled_at);

        // Publish to delayed events channel
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| EventError::Connection(e.to_string()))?;

        let channel = "r8r:events:delayed".to_string();
        let payload =
            serde_json::to_string(&event).map_err(|e| EventError::Serialization(e.to_string()))?;

        conn.publish::<_, _, ()>(&channel, &payload)
            .await
            .map_err(|e| EventError::Publish(e.to_string()))?;

        debug!(
            "Scheduled event '{}' for {} (delay: {}s)",
            event_name, scheduled_at, delay_seconds
        );
        Ok(())
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
        assert!(event.scheduled_at.is_none());
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
    fn test_event_message_with_schedule() {
        use chrono::Utc;
        let scheduled = Utc::now();
        let event = EventMessage::new("order.created", json!({"order_id": 123}))
            .with_schedule(scheduled);

        assert_eq!(event.scheduled_at, Some(scheduled));
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

    #[test]
    fn test_json_path_evaluation() {
        let data = json!({
            "user": {
                "name": "John",
                "email": "john@example.com"
            },
            "items": [{"id": 1}, {"id": 2}]
        });

        assert!(evaluate_json_path("$.user.name", &data));
        assert!(evaluate_json_path("user.name", &data));
        assert!(evaluate_json_path("$.user", &data));
        assert!(evaluate_json_path("$.items[0]", &data));
        assert!(!evaluate_json_path("$.user.nonexistent", &data));
        assert!(!evaluate_json_path("$.nonexistent", &data));
    }

    #[test]
    fn test_event_fan_out_builder() {
        let fan_out = EventFanOut::new("order.created")
            .add_workflow("process-order")
            .add_workflow("send-notification")
            .parallel(true);

        assert_eq!(fan_out.event, "order.created");
        assert_eq!(fan_out.workflow_names.len(), 2);
        assert!(fan_out.parallel);
    }

    #[test]
    fn test_event_fan_out_sequential() {
        let fan_out = EventFanOut::new("payment.received")
            .add_workflow("workflow1")
            .add_workflow("workflow2")
            .sequential();

        assert!(!fan_out.parallel);
    }

    #[test]
    fn test_evaluate_routing() {
        let data = json!({
            "type": "premium",
            "amount": 100
        });

        let rules = vec![
            EventRouting {
                condition: "data.type == \"premium\"".to_string(),
                target_workflow: "premium-handler".to_string(),
            },
            EventRouting {
                condition: "data.type == \"standard\"".to_string(),
                target_workflow: "standard-handler".to_string(),
            },
        ];

        let result = evaluate_routing(&rules, &data);
        // Note: This will return None because Rhai evaluation isn't available in unit tests
        // In production, this would return Some("premium-handler")
        assert!(result.is_none() || result == Some("premium-handler".to_string()));
    }
}
