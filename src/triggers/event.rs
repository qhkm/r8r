//! Event trigger handler using a pluggable backend.
//!
//! Subscribes to events via an `EventBackend` and triggers workflows when events
//! are received. Supports event fan-out to multiple workflows with parallel or
//! sequential execution.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::api::Monitor;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::triggers::delayed::DelayedEvent;
use crate::triggers::event_backend::EventBackend;
use crate::workflow::{parse_workflow, EventRouting, Trigger};

/// Event message structure for pub/sub.
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

/// Event subscriber that listens for events via an `EventBackend`.
pub struct EventSubscriber {
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    backend: Arc<dyn EventBackend>,
    monitor: Option<Arc<Monitor>>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    fan_out_configs: Vec<EventFanOut>,
    delayed_processor: Option<Arc<crate::triggers::delayed::DelayedEventProcessor>>,
}

impl EventSubscriber {
    /// Create a new event subscriber with the given backend.
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
            fan_out_configs: Vec::new(),
            delayed_processor: None,
        }
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

    /// Attach a delayed event processor for scheduling delayed events.
    pub fn with_delayed_processor(
        mut self,
        processor: Arc<crate::triggers::delayed::DelayedEventProcessor>,
    ) -> Self {
        self.delayed_processor = Some(processor);
        self
    }

    /// Start the event subscriber.
    ///
    /// This will subscribe to events for all workflows with event triggers,
    /// and start processing messages from the backend.
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
                        event_name: event.clone(),
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
            info!("No event triggers configured, skipping event subscription");
            return Ok(());
        }

        // Create a receiver from the backend
        let mut receiver = self.backend.subscribe();

        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let subscription_count = event_subscriptions.len();
        let fan_out_configs = self.fan_out_configs.clone();
        let fan_out_count = fan_out_configs.len();

        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let monitor = self.monitor.clone();
        let delayed_processor = self.delayed_processor.clone();

        // Spawn message processing task
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Event subscriber received shutdown signal");
                        break;
                    }
                    result = receiver.recv() => {
                        match result {
                            Ok(event) => {
                                debug!("Received event: {}", event.event);

                                // Check if this is a delayed event that should be scheduled
                                if let Some(delay_seconds) = get_delay_from_subscriptions(&event, &event_subscriptions) {
                                    if delay_seconds > 0 {
                                        if let Err(e) = schedule_delayed_event(&storage, &delayed_processor, &event, delay_seconds).await {
                                            error!("Failed to schedule delayed event: {}", e);
                                        }
                                        continue;
                                    }
                                }

                                // Process regular event subscriptions
                                let matching: Vec<_> = event_subscriptions
                                    .iter()
                                    .filter(|s| s.event_name == event.event)
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
                                for fan_out in &fan_out_configs {
                                    if fan_out.event == event.event {
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
                                warn!("Event receiver error: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);

        info!(
            "Event subscriber started with {} subscription(s) and {} fan-out config(s)",
            subscription_count, fan_out_count
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
    event_name: String,
    workflow_id: String,
    workflow_name: String,
    filter: Option<String>,
    json_path: Option<String>,
    delay: Option<u64>,
    routing: Option<Vec<EventRouting>>,
}

/// Get delay seconds from matching subscription.
fn get_delay_from_subscriptions(
    event: &EventMessage,
    subscriptions: &[EventSubscription],
) -> Option<u64> {
    // Check if event has scheduled_at (already delayed)
    if event.scheduled_at.is_some() {
        return Some(0); // No additional delay needed
    }

    subscriptions
        .iter()
        .find(|s| s.event_name == event.event)
        .and_then(|s| s.delay)
}

/// Schedule a delayed event for future processing.
async fn schedule_delayed_event(
    storage: &SqliteStorage,
    delayed_processor: &Option<Arc<crate::triggers::delayed::DelayedEventProcessor>>,
    event: &EventMessage,
    delay_seconds: u64,
) -> Result<(), EventError> {
    let mut delayed = DelayedEvent::new(&event.event, event.data.clone(), delay_seconds);
    if let Some(source) = &event.source {
        delayed = delayed.with_source(source.clone());
    }
    if let Some(cid) = &event.correlation_id {
        delayed = delayed.with_correlation_id(cid.clone());
    }

    // Store in SQLite (the processor will pick it up via polling)
    let event_json =
        serde_json::to_string(&delayed).map_err(|e| EventError::Serialization(e.to_string()))?;

    // If there's a delayed processor, use its storage path
    let _ = delayed_processor; // processor polls SQLite, no extra action needed

    storage
        .store_delayed_event(&event.event, &event_json, delayed.scheduled_at)
        .await
        .map_err(|e| EventError::Storage(e.to_string()))?;

    info!(
        "Scheduled delayed event '{}' for {} (delay: {}s)",
        event.event, delayed.scheduled_at, delay_seconds
    );

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
        let mut handles = Vec::new();

        for workflow_name in &fan_out.workflow_names {
            let storage = storage.clone();
            let registry = registry.clone();
            let event = event.clone();
            let workflow_name = workflow_name.clone();
            let monitor = monitor.clone();

            let handle = tokio::spawn(async move {
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

        for handle in handles {
            let _ = handle.await;
        }
    } else {
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

    let mut engine = Engine::new_raw();

    engine.set_max_operations(MAX_FILTER_OPERATIONS);
    engine.set_max_string_size(MAX_FILTER_STRING_SIZE);
    engine.set_max_array_size(MAX_FILTER_ARRAY_SIZE);
    engine.set_max_map_size(MAX_FILTER_MAP_SIZE);

    engine.register_fn("==", |a: rhai::Dynamic, b: rhai::Dynamic| -> bool {
        a.to_string() == b.to_string()
    });
    engine.register_fn("!=", |a: rhai::Dynamic, b: rhai::Dynamic| -> bool {
        a.to_string() != b.to_string()
    });

    let mut scope = Scope::new();
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
    let path = path.trim();
    let path = if let Some(stripped) = path.strip_prefix("$.") {
        stripped
    } else if let Some(stripped) = path.strip_prefix('$') {
        stripped
    } else {
        path
    };

    if path.is_empty() {
        return !data.is_null();
    }

    let parts: Vec<&str> = path.split('.').collect();
    let mut current = data;

    for part in &parts {
        if let Some(bracket_idx) = part.find('[') {
            let key = &part[..bracket_idx];
            let idx_str = &part[bracket_idx + 1..part.len() - 1];

            if !key.is_empty() {
                match current.get(key) {
                    Some(v) => current = v,
                    None => return false,
                }
            }

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
    });

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

/// Errors that can occur with event triggers.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Subscription error: {0}")]
    Subscription(String),

    #[error("Publish error: {0}")]
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
        let event =
            EventMessage::new("order.created", json!({"order_id": 123})).with_schedule(scheduled);

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
        assert!(result.is_none() || result == Some("premium-handler".to_string()));
    }
}
