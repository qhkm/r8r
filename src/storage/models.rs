//! Storage models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Stored workflow record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWorkflow {
    pub id: String,
    pub name: String,
    pub definition: String, // YAML
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Stored workflow version snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub version: u32,
    pub definition: String,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub changelog: Option<String>,
    pub checksum: String,
}

/// Execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for ExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for ExecutionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

/// Execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub workflow_version: Option<u32>,
    pub status: ExecutionStatus,
    pub trigger_type: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// Node execution record (for detailed traces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExecution {
    pub id: String,
    pub execution_id: String,
    pub node_id: String,
    pub status: ExecutionStatus,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

/// Full execution trace for debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub execution: Execution,
    pub nodes: Vec<NodeExecution>,
}

/// Database health summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseHealth {
    pub foreign_keys_enabled: bool,
    pub integrity_check: String,
    pub foreign_key_violations: Vec<String>,
    pub orphaned_executions: u64,
    pub orphaned_node_executions: u64,
    pub orphaned_workflow_versions: u64,
}

/// Query filters for execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionQuery {
    pub workflow_name: Option<String>,
    pub status: Option<ExecutionStatus>,
    pub trigger_type: Option<String>,
    pub search: Option<String>,
    pub started_after: Option<DateTime<Utc>>,
    pub started_before: Option<DateTime<Utc>>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for ExecutionQuery {
    fn default() -> Self {
        Self {
            workflow_name: None,
            status: None,
            trigger_type: None,
            search: None,
            started_after: None,
            started_before: None,
            limit: 50,
            offset: 0,
        }
    }
}

/// Execution summary for agent responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub nodes_executed: u32,
    pub nodes_succeeded: u32,
    pub nodes_failed: u32,
    pub items_processed: u32,
    pub duration_ms: u64,
}
