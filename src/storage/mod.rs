//! Storage layer for workflows and executions.

mod audit;
mod dlq;
mod models;
mod pool;
mod sqlite;
mod r#trait;

#[cfg(feature = "storage-postgres")]
mod postgres;

pub use audit::{AuditEntry, AuditEventType};
pub use dlq::{DeadLetterEntry, DlqStats, DlqStatus, NewDlqEntry};
pub use models::{
    ApprovalRequest, Checkpoint, DatabaseHealth, Execution, ExecutionQuery, ExecutionStatus,
    ExecutionSummary, ExecutionTrace, NodeExecution, ReplMessage, ReplSession, StoredWorkflow,
    WorkflowVersion,
};
pub use pool::{ConnectionPool, PoolConfig, PoolStats};
pub use r#trait::Storage;
pub use sqlite::SqliteStorage;

#[cfg(feature = "storage-postgres")]
pub use postgres::PostgresStorage;
