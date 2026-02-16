//! TUI application state and event handling.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::MonitorEvent;

use super::event::{ExecutionSummary, LogLevel, LogLine, TuiEvent, WorkflowSummary};

/// Maximum log lines to keep in the ring buffer.
const MAX_LOG_LINES: usize = 1000;

/// Which panel is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivePanel {
    LiveView,
    History,
    DagView,
    LogTail,
}

impl ActivePanel {
    pub fn next(self) -> Self {
        match self {
            Self::LiveView => Self::History,
            Self::History => Self::DagView,
            Self::DagView => Self::LogTail,
            Self::LogTail => Self::LiveView,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::LiveView => Self::LogTail,
            Self::History => Self::LiveView,
            Self::DagView => Self::History,
            Self::LogTail => Self::DagView,
        }
    }
}

/// Status of a single node in a live execution.
#[derive(Debug, Clone)]
pub enum NodeStatus {
    Running { started_at: DateTime<Utc> },
    Completed { duration_ms: i64 },
    Failed { error: String },
}

/// A node in a live execution.
#[derive(Debug, Clone)]
pub struct LiveNode {
    pub id: String,
    pub node_type: String,
    pub status: NodeStatus,
}

/// A live execution being tracked in real-time.
#[derive(Debug, Clone)]
pub struct LiveExecution {
    pub execution_id: String,
    pub workflow_name: String,
    pub trigger_type: String,
    pub nodes: Vec<LiveNode>,
}

/// A node in the DAG display.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub id: String,
    pub node_type: String,
    pub depth: usize,
    pub status: Option<NodeStatus>,
}

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
}

/// The main application state.
pub struct App {
    pub active_panel: ActivePanel,
    pub live_executions: VecDeque<LiveExecution>,
    pub history: Vec<ExecutionSummary>,
    pub dag_nodes: Vec<DagNode>,
    pub dag_workflow_name: Option<String>,
    pub log_lines: VecDeque<LogLine>,
    pub tick: u64,
    pub connection_state: ConnectionState,
    pub should_quit: bool,

    // Scroll state
    pub history_scroll: usize,
    pub history_selected: Option<usize>,
    pub log_scroll: usize,
    pub log_follow: bool,
    pub live_scroll: usize,
    pub dag_scroll: usize,

    // Workflow data for building DAG
    pub workflows: Vec<WorkflowSummary>,

    // Server URL for status bar
    pub server_url: String,
}

impl App {
    pub fn new(server_url: String) -> Self {
        Self {
            active_panel: ActivePanel::LiveView,
            live_executions: VecDeque::new(),
            history: Vec::new(),
            dag_nodes: Vec::new(),
            dag_workflow_name: None,
            log_lines: VecDeque::new(),
            tick: 0,
            connection_state: ConnectionState::Connecting,
            should_quit: false,
            history_scroll: 0,
            history_selected: None,
            log_scroll: 0,
            log_follow: true,
            live_scroll: 0,
            dag_scroll: 0,
            workflows: Vec::new(),
            server_url,
        }
    }

    /// Handle all TUI events, returning true if the screen needs redrawing.
    pub fn handle_event(&mut self, event: TuiEvent) -> bool {
        match event {
            TuiEvent::Key(key) => self.handle_key_event(key),
            TuiEvent::Monitor(monitor_event) => self.handle_monitor_event(monitor_event),
            TuiEvent::Tick => {
                self.tick = self.tick.wrapping_add(1);
                true
            }
            TuiEvent::WsConnected => {
                self.connection_state = ConnectionState::Connected;
                self.add_log(LogLevel::Info, "WebSocket connected".to_string());
                true
            }
            TuiEvent::WsDisconnected(err) => {
                self.connection_state = ConnectionState::Disconnected;
                let msg = match err {
                    Some(e) => format!("WebSocket disconnected: {}", e),
                    None => "WebSocket disconnected".to_string(),
                };
                self.add_log(LogLevel::Warn, msg);
                true
            }
            TuiEvent::InitialData(data) => {
                self.history = data.executions;
                self.workflows = data.workflows;
                // Load DAG from first workflow if available
                if !self.workflows.is_empty() {
                    self.load_dag_from_workflow(0);
                }
                true
            }
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // Global keybindings
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return true;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return true;
            }
            KeyCode::Tab => {
                self.active_panel = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.active_panel.prev()
                } else {
                    self.active_panel.next()
                };
                return true;
            }
            KeyCode::Char('1') => {
                self.active_panel = ActivePanel::LiveView;
                return true;
            }
            KeyCode::Char('2') => {
                self.active_panel = ActivePanel::History;
                return true;
            }
            KeyCode::Char('3') => {
                self.active_panel = ActivePanel::DagView;
                return true;
            }
            KeyCode::Char('4') => {
                self.active_panel = ActivePanel::LogTail;
                return true;
            }
            KeyCode::Char('r') => {
                // Force reconnect handled externally via should_reconnect flag
                return true;
            }
            _ => {}
        }

        // Panel-specific keybindings
        match self.active_panel {
            ActivePanel::LiveView => self.handle_live_keys(key),
            ActivePanel::History => self.handle_history_keys(key),
            ActivePanel::DagView => self.handle_dag_keys(key),
            ActivePanel::LogTail => self.handle_log_keys(key),
        }
    }

    fn handle_live_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.live_scroll = self.live_scroll.saturating_add(1);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.live_scroll = self.live_scroll.saturating_sub(1);
                true
            }
            _ => false,
        }
    }

    fn handle_history_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.history.is_empty() {
                    let max = self.history.len().saturating_sub(1);
                    self.history_selected =
                        Some(self.history_selected.map(|s| (s + 1).min(max)).unwrap_or(0));
                    // Adjust scroll to keep selection visible
                    if let Some(sel) = self.history_selected {
                        if sel >= self.history_scroll + 20 {
                            self.history_scroll = sel.saturating_sub(19);
                        }
                    }
                }
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !self.history.is_empty() {
                    self.history_selected = Some(
                        self.history_selected
                            .map(|s| s.saturating_sub(1))
                            .unwrap_or(0),
                    );
                    if let Some(sel) = self.history_selected {
                        if sel < self.history_scroll {
                            self.history_scroll = sel;
                        }
                    }
                }
                true
            }
            KeyCode::Enter => {
                // Load the selected execution's workflow into the DAG
                if let Some(sel) = self.history_selected {
                    if let Some(exec) = self.history.get(sel) {
                        let workflow_name = exec.workflow_name.clone();
                        if let Some(idx) =
                            self.workflows.iter().position(|w| w.name == workflow_name)
                        {
                            self.load_dag_from_workflow(idx);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn handle_dag_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.dag_scroll = self.dag_scroll.saturating_add(1);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.dag_scroll = self.dag_scroll.saturating_sub(1);
                true
            }
            _ => false,
        }
    }

    fn handle_log_keys(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.log_follow = false;
                self.log_scroll = self.log_scroll.saturating_add(1);
                true
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.log_follow = false;
                self.log_scroll = self.log_scroll.saturating_sub(1);
                true
            }
            KeyCode::Char('g') => {
                self.log_follow = false;
                self.log_scroll = 0;
                true
            }
            KeyCode::Char('G') => {
                self.log_follow = true;
                true
            }
            _ => false,
        }
    }

    fn handle_monitor_event(&mut self, event: MonitorEvent) -> bool {
        // Add log line for every event (skip heartbeats from log)
        if !matches!(event, MonitorEvent::Heartbeat { .. }) {
            let log_line = LogLine::from_monitor_event(&event);
            self.add_log_line(log_line);
        }

        match &event {
            MonitorEvent::ExecutionStarted {
                execution_id,
                workflow_name,
                trigger_type,
            } => {
                let exec = LiveExecution {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow_name.clone(),
                    trigger_type: trigger_type.clone(),
                    nodes: Vec::new(),
                };
                self.live_executions.push_front(exec);

                // Also add to history
                self.history.insert(
                    0,
                    ExecutionSummary {
                        id: execution_id.clone(),
                        workflow_name: workflow_name.clone(),
                        status: "running".to_string(),
                        trigger_type: trigger_type.clone(),
                        started_at: Utc::now(),
                        finished_at: None,
                        duration_ms: None,
                        error: None,
                    },
                );

                // Adjust history selection
                if let Some(ref mut sel) = self.history_selected {
                    *sel += 1;
                }

                // Load DAG for this workflow
                if let Some(idx) = self.workflows.iter().position(|w| w.name == *workflow_name) {
                    self.load_dag_from_workflow(idx);
                }
            }
            MonitorEvent::NodeStarted {
                execution_id,
                node_id,
                node_type,
            } => {
                if let Some(exec) = self.find_live_execution_mut(execution_id) {
                    // Check if node already exists
                    if let Some(node) = exec.nodes.iter_mut().find(|n| n.id == *node_id) {
                        node.status = NodeStatus::Running {
                            started_at: Utc::now(),
                        };
                    } else {
                        exec.nodes.push(LiveNode {
                            id: node_id.clone(),
                            node_type: node_type.clone(),
                            status: NodeStatus::Running {
                                started_at: Utc::now(),
                            },
                        });
                    }
                    // Update DAG status
                    self.update_dag_node_status(
                        node_id,
                        NodeStatus::Running {
                            started_at: Utc::now(),
                        },
                    );
                }
            }
            MonitorEvent::NodeCompleted {
                execution_id,
                node_id,
                duration_ms,
                ..
            } => {
                if let Some(exec) = self.find_live_execution_mut(execution_id) {
                    if let Some(node) = exec.nodes.iter_mut().find(|n| n.id == *node_id) {
                        node.status = NodeStatus::Completed {
                            duration_ms: *duration_ms,
                        };
                    }
                    self.update_dag_node_status(
                        node_id,
                        NodeStatus::Completed {
                            duration_ms: *duration_ms,
                        },
                    );
                }
            }
            MonitorEvent::NodeFailed {
                execution_id,
                node_id,
                error,
            } => {
                if let Some(exec) = self.find_live_execution_mut(execution_id) {
                    if let Some(node) = exec.nodes.iter_mut().find(|n| n.id == *node_id) {
                        node.status = NodeStatus::Failed {
                            error: error.clone(),
                        };
                    }
                    self.update_dag_node_status(
                        node_id,
                        NodeStatus::Failed {
                            error: error.clone(),
                        },
                    );
                }
            }
            MonitorEvent::ExecutionCompleted {
                execution_id,
                status,
                duration_ms,
                ..
            } => {
                // Remove from live executions
                self.live_executions
                    .retain(|e| e.execution_id != *execution_id);
                // Update history
                if let Some(h) = self.history.iter_mut().find(|h| h.id == *execution_id) {
                    h.status = status.clone();
                    h.finished_at = Some(Utc::now());
                    h.duration_ms = Some(*duration_ms);
                }
            }
            MonitorEvent::ExecutionFailed {
                execution_id,
                error,
                ..
            } => {
                self.live_executions
                    .retain(|e| e.execution_id != *execution_id);
                if let Some(h) = self.history.iter_mut().find(|h| h.id == *execution_id) {
                    h.status = "failed".to_string();
                    h.finished_at = Some(Utc::now());
                    h.error = Some(error.clone());
                }
            }
            _ => {}
        }

        true
    }

    fn find_live_execution_mut(&mut self, execution_id: &str) -> Option<&mut LiveExecution> {
        self.live_executions
            .iter_mut()
            .find(|e| e.execution_id == execution_id)
    }

    fn update_dag_node_status(&mut self, node_id: &str, status: NodeStatus) {
        if let Some(dag_node) = self.dag_nodes.iter_mut().find(|n| n.id == node_id) {
            dag_node.status = Some(status);
        }
    }

    fn add_log(&mut self, level: LogLevel, message: String) {
        self.add_log_line(LogLine {
            timestamp: Utc::now(),
            level,
            message,
        });
    }

    fn add_log_line(&mut self, line: LogLine) {
        self.log_lines.push_back(line);
        while self.log_lines.len() > MAX_LOG_LINES {
            self.log_lines.pop_front();
        }
        if self.log_follow {
            self.log_scroll = self.log_lines.len().saturating_sub(1);
        }
    }

    /// Load DAG from a workflow by index in self.workflows.
    fn load_dag_from_workflow(&mut self, idx: usize) {
        let wf = &self.workflows[idx];
        self.dag_workflow_name = Some(wf.name.clone());

        match crate::workflow::parse_workflow(&wf.definition) {
            Ok(workflow) => {
                // Build DAG nodes with depth via simple BFS
                let mut dag_nodes = Vec::new();
                let mut depths: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();

                // Calculate depths
                for node in &workflow.nodes {
                    if node.depends_on.is_empty() {
                        depths.insert(node.id.clone(), 0);
                    }
                }

                // Iterate until stable
                let mut changed = true;
                while changed {
                    changed = false;
                    for node in &workflow.nodes {
                        if depths.contains_key(&node.id) {
                            continue;
                        }
                        let all_deps_resolved =
                            node.depends_on.iter().all(|dep| depths.contains_key(dep));
                        if all_deps_resolved {
                            let max_dep_depth = node
                                .depends_on
                                .iter()
                                .filter_map(|dep| depths.get(dep))
                                .max()
                                .copied()
                                .unwrap_or(0);
                            depths.insert(node.id.clone(), max_dep_depth + 1);
                            changed = true;
                        }
                    }
                }

                for node in &workflow.nodes {
                    dag_nodes.push(DagNode {
                        id: node.id.clone(),
                        node_type: node.node_type.clone(),
                        depth: depths.get(&node.id).copied().unwrap_or(0),
                        status: None,
                    });
                }

                // Sort by depth for rendering
                dag_nodes.sort_by_key(|n| n.depth);
                self.dag_nodes = dag_nodes;
            }
            Err(_) => {
                self.dag_nodes.clear();
            }
        }
    }

    /// Spinner frame based on tick counter.
    pub fn spinner_frame(&self) -> &'static str {
        const FRAMES: &[&str] = &[
            "\u{2801}", "\u{2809}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{2830}", "\u{2820}",
            "\u{2800}",
        ];
        FRAMES[(self.tick as usize / 2) % FRAMES.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_cycling() {
        assert_eq!(ActivePanel::LiveView.next(), ActivePanel::History);
        assert_eq!(ActivePanel::History.next(), ActivePanel::DagView);
        assert_eq!(ActivePanel::DagView.next(), ActivePanel::LogTail);
        assert_eq!(ActivePanel::LogTail.next(), ActivePanel::LiveView);
    }

    #[test]
    fn test_panel_reverse_cycling() {
        assert_eq!(ActivePanel::LiveView.prev(), ActivePanel::LogTail);
        assert_eq!(ActivePanel::LogTail.prev(), ActivePanel::DagView);
    }

    #[test]
    fn test_log_ring_buffer_cap() {
        let mut app = App::new("http://localhost:8080".to_string());
        for i in 0..1200 {
            app.add_log(LogLevel::Info, format!("line {}", i));
        }
        assert_eq!(app.log_lines.len(), MAX_LOG_LINES);
    }

    #[test]
    fn test_quit_key() {
        let mut app = App::new("http://localhost:8080".to_string());
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        app.handle_event(TuiEvent::Key(key));
        assert!(app.should_quit);
    }

    #[test]
    fn test_panel_number_keys() {
        let mut app = App::new("http://localhost:8080".to_string());
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
        app.handle_event(TuiEvent::Key(key));
        assert_eq!(app.active_panel, ActivePanel::DagView);
    }

    #[test]
    fn test_execution_lifecycle() {
        let mut app = App::new("http://localhost:8080".to_string());

        // Start execution
        app.handle_event(TuiEvent::Monitor(MonitorEvent::ExecutionStarted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            trigger_type: "manual".to_string(),
        }));
        assert_eq!(app.live_executions.len(), 1);
        assert_eq!(app.history.len(), 1);

        // Start a node
        app.handle_event(TuiEvent::Monitor(MonitorEvent::NodeStarted {
            execution_id: "exec-1".to_string(),
            node_id: "node-1".to_string(),
            node_type: "http".to_string(),
        }));
        assert_eq!(app.live_executions[0].nodes.len(), 1);

        // Complete the execution
        app.handle_event(TuiEvent::Monitor(MonitorEvent::ExecutionCompleted {
            execution_id: "exec-1".to_string(),
            workflow_name: "test-wf".to_string(),
            status: "completed".to_string(),
            duration_ms: 150,
        }));
        assert!(app.live_executions.is_empty());
        assert_eq!(app.history[0].status, "completed");
    }
}
