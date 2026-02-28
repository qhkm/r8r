//! Unified audit log for workflow engine events.

use chrono::{DateTime, Utc};
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};

use super::sqlite::parse_datetime_utc;
use super::SqliteStorage;
use crate::error::Result;

/// Maximum number of audit entries returned in a single query.
const MAX_AUDIT_LIMIT: u32 = 1000;

/// A single entry in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub execution_id: Option<String>,
    pub workflow_name: Option<String>,
    pub node_id: Option<String>,
    pub actor: String,
    pub detail: String,
    pub metadata: Option<serde_json::Value>,
}

/// All distinguishable event kinds that the engine can emit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    ExecutionStarted,
    ExecutionCompleted,
    ExecutionFailed,
    ExecutionPaused,
    ExecutionResumed,
    ExecutionCancelled,
    NodeStarted,
    NodeCompleted,
    NodeFailed,
    NodeSkipped,
    ApprovalRequested,
    ApprovalDecided,
    ApprovalTimedOut,
    WorkflowCreated,
    WorkflowUpdated,
    WorkflowDeleted,
    DlqEntryCreated,
    CheckpointSaved,
}

impl std::fmt::Display for AuditEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AuditEventType::ExecutionStarted => "execution_started",
            AuditEventType::ExecutionCompleted => "execution_completed",
            AuditEventType::ExecutionFailed => "execution_failed",
            AuditEventType::ExecutionPaused => "execution_paused",
            AuditEventType::ExecutionResumed => "execution_resumed",
            AuditEventType::ExecutionCancelled => "execution_cancelled",
            AuditEventType::NodeStarted => "node_started",
            AuditEventType::NodeCompleted => "node_completed",
            AuditEventType::NodeFailed => "node_failed",
            AuditEventType::NodeSkipped => "node_skipped",
            AuditEventType::ApprovalRequested => "approval_requested",
            AuditEventType::ApprovalDecided => "approval_decided",
            AuditEventType::ApprovalTimedOut => "approval_timed_out",
            AuditEventType::WorkflowCreated => "workflow_created",
            AuditEventType::WorkflowUpdated => "workflow_updated",
            AuditEventType::WorkflowDeleted => "workflow_deleted",
            AuditEventType::DlqEntryCreated => "dlq_entry_created",
            AuditEventType::CheckpointSaved => "checkpoint_saved",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for AuditEventType {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "execution_started" => Ok(AuditEventType::ExecutionStarted),
            "execution_completed" => Ok(AuditEventType::ExecutionCompleted),
            "execution_failed" => Ok(AuditEventType::ExecutionFailed),
            "execution_paused" => Ok(AuditEventType::ExecutionPaused),
            "execution_resumed" => Ok(AuditEventType::ExecutionResumed),
            "execution_cancelled" => Ok(AuditEventType::ExecutionCancelled),
            "node_started" => Ok(AuditEventType::NodeStarted),
            "node_completed" => Ok(AuditEventType::NodeCompleted),
            "node_failed" => Ok(AuditEventType::NodeFailed),
            "node_skipped" => Ok(AuditEventType::NodeSkipped),
            "approval_requested" => Ok(AuditEventType::ApprovalRequested),
            "approval_decided" => Ok(AuditEventType::ApprovalDecided),
            "approval_timed_out" => Ok(AuditEventType::ApprovalTimedOut),
            "workflow_created" => Ok(AuditEventType::WorkflowCreated),
            "workflow_updated" => Ok(AuditEventType::WorkflowUpdated),
            "workflow_deleted" => Ok(AuditEventType::WorkflowDeleted),
            "dlq_entry_created" => Ok(AuditEventType::DlqEntryCreated),
            "checkpoint_saved" => Ok(AuditEventType::CheckpointSaved),
            _ => Err(()),
        }
    }
}

impl SqliteStorage {
    /// Persist a single audit entry.
    pub async fn save_audit_entry(&self, entry: &AuditEntry) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO audit_log
                (id, timestamp, event_type, execution_id, workflow_name, node_id, actor, detail, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.id,
                entry.timestamp.to_rfc3339(),
                entry.event_type.to_string(),
                entry.execution_id,
                entry.workflow_name,
                entry.node_id,
                entry.actor,
                entry.detail,
                entry.metadata.as_ref().map(|v| v.to_string()),
            ],
        )?;
        Ok(())
    }

    /// Query audit entries with optional filters.
    ///
    /// Filters are ANDed together. Results are ordered newest-first.
    /// `limit` is capped at [`MAX_AUDIT_LIMIT`].
    pub async fn list_audit_entries(
        &self,
        execution_id: Option<&str>,
        workflow_name: Option<&str>,
        event_type: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().await;

        let capped_limit = limit.min(MAX_AUDIT_LIMIT);

        // Build the query dynamically based on which filters are present.
        let mut sql = String::from(
            "SELECT id, timestamp, event_type, execution_id, workflow_name, node_id, actor, detail, metadata \
             FROM audit_log \
             WHERE 1=1",
        );

        // Collect the runtime filter values as owned strings so the borrows
        // live long enough for `query_map`.
        let mut filter_values: Vec<String> = Vec::new();

        if let Some(eid) = execution_id {
            filter_values.push(eid.to_owned());
            sql.push_str(&format!(" AND execution_id = ?{}", filter_values.len()));
        }
        if let Some(wn) = workflow_name {
            filter_values.push(wn.to_owned());
            sql.push_str(&format!(" AND workflow_name = ?{}", filter_values.len()));
        }
        if let Some(et) = event_type {
            filter_values.push(et.to_owned());
            sql.push_str(&format!(" AND event_type = ?{}", filter_values.len()));
        }
        if let Some(s) = since {
            filter_values.push(s.to_rfc3339());
            sql.push_str(&format!(" AND timestamp >= ?{}", filter_values.len()));
        }

        // Pagination parameters always come last.
        let limit_pos = filter_values.len() + 1;
        let offset_pos = filter_values.len() + 2;
        sql.push_str(&format!(
            " ORDER BY timestamp DESC LIMIT ?{limit_pos} OFFSET ?{offset_pos}"
        ));

        let mut stmt = conn.prepare(&sql)?;

        let limit_i = capped_limit as i64;
        let offset_i = offset as i64;

        let entries = {
            use rusqlite::types::Value;
            let mut values: Vec<Value> = filter_values
                .iter()
                .map(|s| Value::Text(s.clone()))
                .collect();
            values.push(Value::Integer(limit_i));
            values.push(Value::Integer(offset_i));

            let rows = stmt.query_map(
                rusqlite::params_from_iter(values.iter()),
                Self::row_to_audit_entry,
            )?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        Ok(entries)
    }

    /// Map a database row to an [`AuditEntry`], propagating parse errors
    /// instead of silently defaulting.
    fn row_to_audit_entry(row: &Row) -> rusqlite::Result<AuditEntry> {
        let ts_str: String = row.get(1)?;
        let event_str: String = row.get(2)?;
        let metadata_str: Option<String> = row.get(8)?;

        let timestamp = parse_datetime_utc(&ts_str)?;

        let event_type: AuditEventType = event_str.parse().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                format!("unknown audit event type: {}", event_str).into(),
            )
        })?;

        Ok(AuditEntry {
            id: row.get(0)?,
            timestamp,
            event_type,
            execution_id: row.get(3)?,
            workflow_name: row.get(4)?,
            node_id: row.get(5)?,
            actor: row.get(6)?,
            detail: row.get(7)?,
            metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
        })
    }
}
