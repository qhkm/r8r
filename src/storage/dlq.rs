//! Dead Letter Queue for failed workflow executions.

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SqliteStorage;
use crate::error::Result;

/// A failed execution stored in the dead letter queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterEntry {
    pub id: String,
    pub execution_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub trigger_type: String,
    pub input: Value,
    pub error: String,
    pub failed_node_id: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub status: DlqStatus,
}

/// Parameters for creating a new DLQ entry.
#[derive(Debug, Clone)]
pub struct NewDlqEntry<'a> {
    pub execution_id: &'a str,
    pub workflow_id: &'a str,
    pub workflow_name: &'a str,
    pub trigger_type: &'a str,
    pub input: &'a Value,
    pub error: &'a str,
    pub failed_node_id: Option<&'a str>,
    pub max_retries: u32,
}

/// Status of a dead letter queue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DlqStatus {
    Pending,
    Scheduled,
    Resolved,
    Discarded,
    Acknowledged,
}

impl std::fmt::Display for DlqStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DlqStatus::Pending => write!(f, "pending"),
            DlqStatus::Scheduled => write!(f, "scheduled"),
            DlqStatus::Resolved => write!(f, "resolved"),
            DlqStatus::Discarded => write!(f, "discarded"),
            DlqStatus::Acknowledged => write!(f, "acknowledged"),
        }
    }
}

impl std::str::FromStr for DlqStatus {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(DlqStatus::Pending),
            "scheduled" => Ok(DlqStatus::Scheduled),
            "resolved" => Ok(DlqStatus::Resolved),
            "discarded" => Ok(DlqStatus::Discarded),
            "acknowledged" => Ok(DlqStatus::Acknowledged),
            _ => Err(()),
        }
    }
}

impl SqliteStorage {
    pub async fn init_dlq(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS dead_letter_queue (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                input TEXT NOT NULL,
                error TEXT NOT NULL,
                failed_node_id TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER NOT NULL DEFAULT 3,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                next_retry_at TEXT,
                status TEXT NOT NULL DEFAULT 'pending'
            )",
            [],
        )?;
        conn.execute("CREATE INDEX IF NOT EXISTS idx_dlq_status ON dead_letter_queue(status)", [])?;
        Ok(())
    }

    pub async fn add_to_dlq(&self, entry: NewDlqEntry<'_>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let input_json = serde_json::to_string(entry.input)?;

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO dead_letter_queue (id, execution_id, workflow_id, workflow_name, trigger_type, input, error, failed_node_id, retry_count, max_retries, created_at, updated_at, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?10, 'pending')",
            params![id, entry.execution_id, entry.workflow_id, entry.workflow_name, entry.trigger_type, input_json, entry.error, entry.failed_node_id, entry.max_retries, now],
        )?;
        Ok(id)
    }

    pub async fn get_dlq_entry(&self, id: &str) -> Result<Option<DeadLetterEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM dead_letter_queue WHERE id = ?1")?;
        let entry = stmt.query_row([id], row_to_dlq_entry).optional()?;
        Ok(entry)
    }

    pub async fn list_dlq_entries(&self, status: Option<DlqStatus>, limit: u32, offset: u32) -> Result<Vec<DeadLetterEntry>> {
        let conn = self.conn.lock().await;
        let entries = match status {
            Some(s) => {
                let mut stmt = conn.prepare("SELECT * FROM dead_letter_queue WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3")?;
                let rows = stmt.query_map(params![s.to_string(), limit, offset], row_to_dlq_entry)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = conn.prepare("SELECT * FROM dead_letter_queue ORDER BY created_at DESC LIMIT ?1 OFFSET ?2")?;
                let rows = stmt.query_map(params![limit, offset], row_to_dlq_entry)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            }
        };
        Ok(entries)
    }

    pub async fn schedule_dlq_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute("UPDATE dead_letter_queue SET status = 'scheduled', next_retry_at = ?1, updated_at = ?2 WHERE id = ?3", params![next_retry_at.to_rfc3339(), now, id])?;
        Ok(())
    }

    pub async fn resolve_dlq_entry(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute("UPDATE dead_letter_queue SET status = 'resolved', updated_at = ?1 WHERE id = ?2", params![now, id])?;
        Ok(())
    }

    pub async fn discard_dlq_entry(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute("UPDATE dead_letter_queue SET status = 'discarded', updated_at = ?1 WHERE id = ?2", params![now, id])?;
        Ok(())
    }

    pub async fn increment_dlq_retry(&self, id: &str) -> Result<u32> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute("UPDATE dead_letter_queue SET retry_count = retry_count + 1, updated_at = ?1 WHERE id = ?2", params![now, id])?;
        let count: u32 = conn.query_row("SELECT retry_count FROM dead_letter_queue WHERE id = ?1", [id], |row| row.get(0))?;
        Ok(count)
    }

    pub async fn get_dlq_stats(&self) -> Result<DlqStats> {
        let conn = self.conn.lock().await;
        let total: u64 = conn.query_row("SELECT COUNT(*) FROM dead_letter_queue", [], |row| row.get(0))?;
        let pending: u64 = conn.query_row("SELECT COUNT(*) FROM dead_letter_queue WHERE status = 'pending'", [], |row| row.get(0))?;
        let scheduled: u64 = conn.query_row("SELECT COUNT(*) FROM dead_letter_queue WHERE status = 'scheduled'", [], |row| row.get(0))?;
        let resolved: u64 = conn.query_row("SELECT COUNT(*) FROM dead_letter_queue WHERE status = 'resolved'", [], |row| row.get(0))?;
        let discarded: u64 = conn.query_row("SELECT COUNT(*) FROM dead_letter_queue WHERE status = 'discarded'", [], |row| row.get(0))?;
        Ok(DlqStats { total, pending, scheduled, resolved, discarded })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqStats {
    pub total: u64,
    pub pending: u64,
    pub scheduled: u64,
    pub resolved: u64,
    pub discarded: u64,
}

fn row_to_dlq_entry(row: &Row) -> rusqlite::Result<DeadLetterEntry> {
    let input_str: String = row.get("input")?;
    let status_str: String = row.get("status")?;
    let next_retry_str: Option<String> = row.get("next_retry_at")?;
    Ok(DeadLetterEntry {
        id: row.get("id")?,
        execution_id: row.get("execution_id")?,
        workflow_id: row.get("workflow_id")?,
        workflow_name: row.get("workflow_name")?,
        trigger_type: row.get("trigger_type")?,
        input: serde_json::from_str(&input_str).unwrap_or(Value::Null),
        error: row.get("error")?,
        failed_node_id: row.get("failed_node_id")?,
        retry_count: row.get("retry_count")?,
        max_retries: row.get("max_retries")?,
        created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>("created_at")?).map(|dt| dt.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
        updated_at: DateTime::parse_from_rfc3339(&row.get::<_, String>("updated_at")?).map(|dt| dt.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
        next_retry_at: next_retry_str.and_then(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&Utc)).ok()),
        status: status_str.parse().unwrap_or(DlqStatus::Pending),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dlq_add_and_get() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.init_dlq().await.unwrap();
        let id = storage
            .add_to_dlq(NewDlqEntry {
                execution_id: "exec-1",
                workflow_id: "wf-1",
                workflow_name: "test-workflow",
                trigger_type: "api",
                input: &serde_json::json!({"key": "value"}),
                error: "Test error",
                failed_node_id: Some("node-1"),
                max_retries: 3,
            })
            .await
            .unwrap();
        let entry = storage.get_dlq_entry(&id).await.unwrap().unwrap();
        assert_eq!(entry.execution_id, "exec-1");
        assert_eq!(entry.workflow_name, "test-workflow");
        assert_eq!(entry.status, DlqStatus::Pending);
    }

    #[tokio::test]
    async fn test_dlq_status_transitions() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.init_dlq().await.unwrap();
        let id = storage
            .add_to_dlq(NewDlqEntry {
                execution_id: "exec-1",
                workflow_id: "wf-1",
                workflow_name: "test",
                trigger_type: "api",
                input: &serde_json::json!({}),
                error: "Error",
                failed_node_id: None,
                max_retries: 3,
            })
            .await
            .unwrap();
        storage.schedule_dlq_retry(&id, Utc::now()).await.unwrap();
        let entry = storage.get_dlq_entry(&id).await.unwrap().unwrap();
        assert_eq!(entry.status, DlqStatus::Scheduled);
        storage.resolve_dlq_entry(&id).await.unwrap();
        let entry = storage.get_dlq_entry(&id).await.unwrap().unwrap();
        assert_eq!(entry.status, DlqStatus::Resolved);
    }

    #[tokio::test]
    async fn test_dlq_stats() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.init_dlq().await.unwrap();
        let id1 = storage
            .add_to_dlq(NewDlqEntry {
                execution_id: "exec-1",
                workflow_id: "wf-1",
                workflow_name: "test",
                trigger_type: "api",
                input: &serde_json::json!({}),
                error: "Error",
                failed_node_id: None,
                max_retries: 3,
            })
            .await
            .unwrap();
        let id2 = storage
            .add_to_dlq(NewDlqEntry {
                execution_id: "exec-2",
                workflow_id: "wf-1",
                workflow_name: "test",
                trigger_type: "api",
                input: &serde_json::json!({}),
                error: "Error",
                failed_node_id: None,
                max_retries: 3,
            })
            .await
            .unwrap();
        storage.resolve_dlq_entry(&id1).await.unwrap();
        storage.discard_dlq_entry(&id2).await.unwrap();
        let stats = storage.get_dlq_stats().await.unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.resolved, 1);
        assert_eq!(stats.discarded, 1);
    }
}
