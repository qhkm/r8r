//! TUI event types merging terminal, monitor, and tick events.

use crate::api::MonitorEvent;
use chrono::{DateTime, Utc};
use crossterm::event::KeyEvent;

/// All events the TUI main loop handles.
pub enum TuiEvent {
    /// Terminal key press
    Key(KeyEvent),
    /// Monitor event from WebSocket
    Monitor(MonitorEvent),
    /// Tick for spinner animation
    Tick,
    /// WebSocket connected
    WsConnected,
    /// WebSocket disconnected (with optional error message)
    WsDisconnected(Option<String>),
    /// Initial data loaded from REST
    InitialData(InitialData),
}

/// Data loaded via REST on startup.
pub struct InitialData {
    pub executions: Vec<ExecutionSummary>,
    pub workflows: Vec<WorkflowSummary>,
}

/// Minimal execution info for history panel.
#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    pub id: String,
    pub workflow_name: String,
    pub status: String,
    pub trigger_type: String,
    #[allow(dead_code)]
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
}

/// Minimal workflow info for DAG panel.
#[derive(Debug, Clone)]
pub struct WorkflowSummary {
    pub name: String,
    pub definition: String,
}

/// A formatted log line for the event log panel.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLine {
    pub fn from_monitor_event(event: &MonitorEvent) -> Self {
        let (level, message) = match event {
            MonitorEvent::Connected { message } => {
                (LogLevel::Info, format!("connected: {}", message))
            }
            MonitorEvent::ExecutionStarted {
                execution_id,
                workflow_name,
                trigger_type,
            } => (
                LogLevel::Info,
                format!(
                    "exec_started {} workflow={} trigger={}",
                    &execution_id[..8.min(execution_id.len())],
                    workflow_name,
                    trigger_type
                ),
            ),
            MonitorEvent::NodeStarted {
                execution_id,
                node_id,
                node_type,
            } => (
                LogLevel::Info,
                format!(
                    "node_started {}:{} type={}",
                    &execution_id[..8.min(execution_id.len())],
                    node_id,
                    node_type
                ),
            ),
            MonitorEvent::NodeCompleted {
                execution_id,
                node_id,
                duration_ms,
                ..
            } => (
                LogLevel::Info,
                format!(
                    "node_completed {}:{} {}ms",
                    &execution_id[..8.min(execution_id.len())],
                    node_id,
                    duration_ms
                ),
            ),
            MonitorEvent::NodeFailed {
                execution_id,
                node_id,
                error,
            } => (
                LogLevel::Error,
                format!(
                    "node_failed {}:{} err={}",
                    &execution_id[..8.min(execution_id.len())],
                    node_id,
                    error
                ),
            ),
            MonitorEvent::ExecutionCompleted {
                execution_id,
                workflow_name,
                status,
                duration_ms,
            } => (
                LogLevel::Info,
                format!(
                    "exec_completed {} workflow={} status={} {}ms",
                    &execution_id[..8.min(execution_id.len())],
                    workflow_name,
                    status,
                    duration_ms
                ),
            ),
            MonitorEvent::ExecutionFailed {
                execution_id,
                workflow_name,
                error,
            } => (
                LogLevel::Error,
                format!(
                    "exec_failed {} workflow={} err={}",
                    &execution_id[..8.min(execution_id.len())],
                    workflow_name,
                    error
                ),
            ),
            MonitorEvent::Heartbeat { .. } => (LogLevel::Info, "heartbeat".to_string()),
            MonitorEvent::Error { message } => (LogLevel::Error, format!("error: {}", message)),
        };

        LogLine {
            timestamp: Utc::now(),
            level,
            message,
        }
    }
}
