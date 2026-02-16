//! WebSocket handler for live execution monitoring.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::{
    extract::{Query, State},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use super::AppState;

/// Maximum number of events to buffer in broadcast channel.
const BROADCAST_CAPACITY: usize = 1024;

/// Event types for WebSocket monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MonitorEvent {
    /// Connection established
    Connected { message: String },
    /// Execution started
    ExecutionStarted {
        execution_id: String,
        workflow_name: String,
        trigger_type: String,
    },
    /// Node execution started
    NodeStarted {
        execution_id: String,
        node_id: String,
        node_type: String,
    },
    /// Node execution completed
    NodeCompleted {
        execution_id: String,
        node_id: String,
        status: String,
        duration_ms: i64,
    },
    /// Node execution failed
    NodeFailed {
        execution_id: String,
        node_id: String,
        error: String,
    },
    /// Execution completed
    ExecutionCompleted {
        execution_id: String,
        workflow_name: String,
        status: String,
        duration_ms: i64,
    },
    /// Execution failed
    ExecutionFailed {
        execution_id: String,
        workflow_name: String,
        error: String,
    },
    /// Heartbeat to keep connection alive
    Heartbeat { timestamp: String },
    /// Error message
    Error { message: String },
}

/// Query parameters for WebSocket connection.
#[derive(Debug, Deserialize)]
pub struct MonitorQuery {
    /// Filter by workflow name
    #[serde(default)]
    pub workflow: Option<String>,
    /// Filter by execution ID
    #[serde(default)]
    pub execution: Option<String>,
    /// Authentication token (required unless R8R_MONITOR_PUBLIC=true)
    #[serde(default)]
    pub token: Option<String>,
}

/// Check if WebSocket monitoring requires authentication.
fn is_monitor_auth_required() -> bool {
    std::env::var("R8R_MONITOR_PUBLIC")
        .map(|v| v.to_lowercase() != "true")
        .unwrap_or(true)
}

/// Get the expected monitor token from environment.
fn get_monitor_token() -> Option<String> {
    std::env::var("R8R_MONITOR_TOKEN").ok()
}

/// Validate the monitor authentication token.
fn validate_monitor_token(provided: Option<&str>) -> bool {
    if !is_monitor_auth_required() {
        return true;
    }

    match (get_monitor_token(), provided) {
        (Some(expected), Some(provided)) => {
            // Constant-time comparison to prevent timing attacks
            if expected.len() != provided.len() {
                return false;
            }
            expected
                .bytes()
                .zip(provided.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        (None, _) => {
            // No token configured - allow access but log warning
            warn!("R8R_MONITOR_TOKEN not set - WebSocket monitor is unprotected!");
            true
        }
        (Some(_), None) => false,
    }
}

/// Shared monitor state for broadcasting events.
#[derive(Clone)]
pub struct Monitor {
    tx: broadcast::Sender<MonitorEvent>,
}

impl Monitor {
    /// Create a new monitor.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx }
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<MonitorEvent> {
        self.tx.subscribe()
    }

    /// Broadcast an event to all subscribers.
    pub fn broadcast(&self, event: MonitorEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.tx.send(event);
    }

    /// Broadcast execution started event.
    pub fn execution_started(&self, execution_id: &str, workflow_name: &str, trigger_type: &str) {
        self.broadcast(MonitorEvent::ExecutionStarted {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            trigger_type: trigger_type.to_string(),
        });
    }

    /// Broadcast node started event.
    pub fn node_started(&self, execution_id: &str, node_id: &str, node_type: &str) {
        self.broadcast(MonitorEvent::NodeStarted {
            execution_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            node_type: node_type.to_string(),
        });
    }

    /// Broadcast node completed event.
    pub fn node_completed(
        &self,
        execution_id: &str,
        node_id: &str,
        status: &str,
        duration_ms: i64,
    ) {
        self.broadcast(MonitorEvent::NodeCompleted {
            execution_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            status: status.to_string(),
            duration_ms,
        });
    }

    /// Broadcast node failed event.
    pub fn node_failed(&self, execution_id: &str, node_id: &str, error: &str) {
        self.broadcast(MonitorEvent::NodeFailed {
            execution_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            error: error.to_string(),
        });
    }

    /// Broadcast execution completed event.
    pub fn execution_completed(
        &self,
        execution_id: &str,
        workflow_name: &str,
        status: &str,
        duration_ms: i64,
    ) {
        self.broadcast(MonitorEvent::ExecutionCompleted {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            status: status.to_string(),
            duration_ms,
        });
    }

    /// Broadcast execution failed event.
    pub fn execution_failed(&self, execution_id: &str, workflow_name: &str, error: &str) {
        self.broadcast(MonitorEvent::ExecutionFailed {
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            error: error.to_string(),
        });
    }
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Extended app state with monitor.
#[derive(Clone)]
pub struct MonitoredAppState {
    pub inner: AppState,
    pub monitor: Arc<Monitor>,
}

/// WebSocket handler for live monitoring.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<MonitoredAppState>,
    Query(query): Query<MonitorQuery>,
) -> impl IntoResponse {
    // Validate authentication token
    if !validate_monitor_token(query.token.as_deref()) {
        warn!("WebSocket connection rejected: invalid or missing token");
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            "Invalid or missing authentication token. Set R8R_MONITOR_TOKEN and pass ?token=<value>",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state, query))
        .into_response()
}

/// Handle WebSocket connection.
async fn handle_socket(socket: WebSocket, state: MonitoredAppState, query: MonitorQuery) {
    let (mut sender, receiver) = socket.split();
    let mut rx = state.monitor.subscribe();

    info!(
        "WebSocket client connected (workflow: {:?}, execution: {:?})",
        query.workflow, query.execution
    );

    // Send connection confirmation
    let connected = MonitorEvent::Connected {
        message: "Connected to r8r monitor".to_string(),
    };
    if let Err(e) = send_event(&mut sender, &connected).await {
        error!("Failed to send connection message: {}", e);
        return;
    }

    // Reunite and split again to get proper ownership
    let heartbeat_sender = sender.reunite(receiver).expect("reunite failed");
    let (mut sender, mut receiver) = heartbeat_sender.split();

    // Clone filters for use in spawn
    let workflow_filter = query.workflow.clone();
    let execution_filter = query.execution.clone();

    // Main event loop
    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        debug!("Received message from client: {}", text);
                        // Could handle commands here (e.g., subscribe to specific workflow)
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = sender.send(Message::Pong(data)).await {
                            error!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client requested close");
                        break;
                    }
                    Some(Err(e)) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("Client disconnected");
                        break;
                    }
                    _ => {}
                }
            }
            // Handle broadcast events
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        // Apply filters
                        if !should_send_event(&event, &workflow_filter, &execution_filter) {
                            continue;
                        }

                        if let Err(e) = send_event(&mut sender, &event).await {
                            error!("Failed to send event: {}", e);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Client lagged behind by {} messages", n);
                        let error = MonitorEvent::Error {
                            message: format!("Dropped {} messages due to lag", n),
                        };
                        let _ = send_event(&mut sender, &error).await;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Monitor channel closed");
                        break;
                    }
                }
            }
        }
    }

    info!("WebSocket client disconnected");
}

/// Send an event to the WebSocket client.
async fn send_event(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &MonitorEvent,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    sender
        .send(Message::Text(json.into()))
        .await
        .map_err(axum::Error::new)
}

/// Check if an event should be sent based on filters.
fn should_send_event(
    event: &MonitorEvent,
    workflow_filter: &Option<String>,
    execution_filter: &Option<String>,
) -> bool {
    // Always send connection and error events
    match event {
        MonitorEvent::Connected { .. }
        | MonitorEvent::Heartbeat { .. }
        | MonitorEvent::Error { .. } => {
            return true;
        }
        _ => {}
    }

    // Check execution filter
    if let Some(filter_exec) = execution_filter {
        let event_exec = match event {
            MonitorEvent::ExecutionStarted { execution_id, .. }
            | MonitorEvent::NodeStarted { execution_id, .. }
            | MonitorEvent::NodeCompleted { execution_id, .. }
            | MonitorEvent::NodeFailed { execution_id, .. }
            | MonitorEvent::ExecutionCompleted { execution_id, .. }
            | MonitorEvent::ExecutionFailed { execution_id, .. } => execution_id,
            _ => return false,
        };

        if event_exec != filter_exec {
            return false;
        }
    }

    // Check workflow filter
    if let Some(filter_workflow) = workflow_filter {
        let event_workflow = match event {
            MonitorEvent::ExecutionStarted { workflow_name, .. }
            | MonitorEvent::ExecutionCompleted { workflow_name, .. }
            | MonitorEvent::ExecutionFailed { workflow_name, .. } => Some(workflow_name.as_str()),
            _ => None,
        };

        if let Some(wf) = event_workflow {
            if wf != filter_workflow {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_broadcast() {
        let monitor = Monitor::new();
        let mut rx = monitor.subscribe();

        monitor.execution_started("exec-1", "test-workflow", "manual");

        let event = rx.try_recv().unwrap();
        match event {
            MonitorEvent::ExecutionStarted {
                execution_id,
                workflow_name,
                trigger_type,
            } => {
                assert_eq!(execution_id, "exec-1");
                assert_eq!(workflow_name, "test-workflow");
                assert_eq!(trigger_type, "manual");
            }
            _ => panic!("Unexpected event type"),
        }
    }

    #[test]
    fn test_filter_by_execution() {
        let event = MonitorEvent::NodeCompleted {
            execution_id: "exec-1".to_string(),
            node_id: "node-1".to_string(),
            status: "completed".to_string(),
            duration_ms: 100,
        };

        // Should pass with matching filter
        assert!(should_send_event(
            &event,
            &None,
            &Some("exec-1".to_string())
        ));

        // Should not pass with non-matching filter
        assert!(!should_send_event(
            &event,
            &None,
            &Some("exec-2".to_string())
        ));

        // Should pass with no filter
        assert!(should_send_event(&event, &None, &None));
    }

    #[test]
    fn test_filter_by_workflow() {
        let event = MonitorEvent::ExecutionStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "my-workflow".to_string(),
            trigger_type: "manual".to_string(),
        };

        // Should pass with matching filter
        assert!(should_send_event(
            &event,
            &Some("my-workflow".to_string()),
            &None
        ));

        // Should not pass with non-matching filter
        assert!(!should_send_event(
            &event,
            &Some("other-workflow".to_string()),
            &None
        ));
    }

    #[test]
    fn test_connection_always_passes() {
        let event = MonitorEvent::Connected {
            message: "test".to_string(),
        };

        // Should always pass regardless of filters
        assert!(should_send_event(
            &event,
            &Some("any".to_string()),
            &Some("any".to_string())
        ));
    }

    #[test]
    fn test_event_serialization() {
        let event = MonitorEvent::ExecutionStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test".to_string(),
            trigger_type: "api".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("execution_started"));
        assert!(json.contains("exec-1"));
    }
}
