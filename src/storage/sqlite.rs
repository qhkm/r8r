//! SQLite storage implementation.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use tokio::sync::Mutex;

use super::models::*;
use crate::error::{Error, Result};

/// Parse an RFC 3339 datetime string into a `chrono::DateTime<Utc>`.
///
/// Returns a `rusqlite::Error` on parse failure instead of panicking,
/// so it is safe to use inside `query_row` / `query_map` closures.
fn parse_datetime_utc(s: &str) -> rusqlite::Result<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

/// Escape SQL LIKE pattern special characters to prevent injection.
/// Characters % and _ have special meaning in LIKE patterns.
fn escape_like_pattern(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Default query limit.
const DEFAULT_QUERY_LIMIT: usize = 50;
/// Maximum query limit to prevent abuse.
const MAX_QUERY_LIMIT: usize = 1000;

/// SQLite-based storage.
#[derive(Clone)]
pub struct SqliteStorage {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open or create a database at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;

        // Initialize schema synchronously before wrapping in async mutex
        Self::init_schema_sync(&mut conn)?;

        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        Ok(storage)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;

        // Initialize schema synchronously before wrapping in async mutex
        Self::init_schema_sync(&mut conn)?;

        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        Ok(storage)
    }

    fn init_schema_sync(conn: &mut Connection) -> Result<()> {
        // Configure SQLite for better concurrent access and reliability
        // Note: WAL mode must be set before any transaction begins
        conn.execute_batch(
            r#"
            -- Enable WAL mode for better concurrent reads during writes
            PRAGMA journal_mode = WAL;
            -- Wait up to 5 seconds when database is locked instead of failing immediately
            PRAGMA busy_timeout = 5000;
            -- Balance between safety and performance (fsync at critical moments)
            PRAGMA synchronous = NORMAL;
            -- Enable foreign key enforcement
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                definition TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workflow_versions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                version INTEGER NOT NULL,
                definition TEXT NOT NULL,
                created_at TEXT NOT NULL,
                created_by TEXT,
                changelog TEXT,
                checksum TEXT NOT NULL,
                FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE,
                UNIQUE(workflow_id, version)
            );

            CREATE TABLE IF NOT EXISTS executions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                workflow_version INTEGER,
                status TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                error TEXT,
                FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS node_executions (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                status TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                started_at TEXT NOT NULL,
                finished_at TEXT,
                error TEXT,
                FOREIGN KEY (execution_id) REFERENCES executions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_workflow_versions_workflow
                ON workflow_versions(workflow_id, version DESC);
            CREATE INDEX IF NOT EXISTS idx_workflow_versions_name
                ON workflow_versions(workflow_name, version DESC);
            CREATE INDEX IF NOT EXISTS idx_executions_workflow ON executions(workflow_id);
            CREATE INDEX IF NOT EXISTS idx_executions_workflow_name ON executions(workflow_name);
            CREATE INDEX IF NOT EXISTS idx_node_executions_execution ON node_executions(execution_id);

            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                node_outputs TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (execution_id) REFERENCES executions(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_checkpoints_execution ON checkpoints(execution_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS delayed_events (
                id TEXT PRIMARY KEY,
                event_name TEXT NOT NULL,
                event_data TEXT NOT NULL,
                scheduled_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                retry_count INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_delayed_events_scheduled ON delayed_events(scheduled_at);
            CREATE INDEX IF NOT EXISTS idx_delayed_events_name ON delayed_events(event_name);
            "#,
        )?;

        Self::migrate_foreign_keys_to_cascade(conn)?;
        Self::ensure_execution_workflow_version_column(conn)?;
        Self::repair_orphans(conn)?;
        Ok(())
    }

    fn has_cascade_fk(
        conn: &Connection,
        table: &str,
        from_column: &str,
        parent_table: &str,
    ) -> Result<bool> {
        let sql = format!("PRAGMA foreign_key_list({})", table);
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let referenced_table: String = row.get(2)?;
            let from: String = row.get(3)?;
            let on_delete: String = row.get(6)?;
            if referenced_table == parent_table && from == from_column {
                return Ok(on_delete.eq_ignore_ascii_case("CASCADE"));
            }
        }

        Ok(false)
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
        let sql = format!("PRAGMA table_info({})", table);
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn ensure_execution_workflow_version_column(conn: &Connection) -> Result<()> {
        if !Self::has_column(conn, "executions", "workflow_version")? {
            conn.execute(
                "ALTER TABLE executions ADD COLUMN workflow_version INTEGER",
                [],
            )?;
        }
        Ok(())
    }

    fn migrate_foreign_keys_to_cascade(conn: &mut Connection) -> Result<()> {
        let executions_has_cascade =
            Self::has_cascade_fk(conn, "executions", "workflow_id", "workflows")?;
        let node_exec_has_cascade =
            Self::has_cascade_fk(conn, "node_executions", "execution_id", "executions")?;
        let versions_has_cascade =
            Self::has_cascade_fk(conn, "workflow_versions", "workflow_id", "workflows")?;

        if executions_has_cascade && node_exec_has_cascade && versions_has_cascade {
            return Ok(());
        }

        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let migration_result = (|| -> Result<()> {
            let tx = conn.transaction()?;

            tx.execute_batch(
                r#"
                DROP TABLE IF EXISTS node_executions_old;
                DROP TABLE IF EXISTS executions_old;
                DROP TABLE IF EXISTS workflow_versions_old;

                ALTER TABLE node_executions RENAME TO node_executions_old;
                ALTER TABLE executions RENAME TO executions_old;
                ALTER TABLE workflow_versions RENAME TO workflow_versions_old;

                CREATE TABLE workflow_versions (
                    id TEXT PRIMARY KEY,
                    workflow_id TEXT NOT NULL,
                    workflow_name TEXT NOT NULL,
                    version INTEGER NOT NULL,
                    definition TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    created_by TEXT,
                    changelog TEXT,
                    checksum TEXT NOT NULL,
                    FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE,
                    UNIQUE(workflow_id, version)
                );

                CREATE TABLE executions (
                    id TEXT PRIMARY KEY,
                    workflow_id TEXT NOT NULL,
                    workflow_name TEXT NOT NULL,
                    workflow_version INTEGER,
                    status TEXT NOT NULL,
                    trigger_type TEXT NOT NULL,
                    input TEXT NOT NULL,
                    output TEXT,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    error TEXT,
                    FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
                );

                CREATE TABLE node_executions (
                    id TEXT PRIMARY KEY,
                    execution_id TEXT NOT NULL,
                    node_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    input TEXT NOT NULL,
                    output TEXT,
                    started_at TEXT NOT NULL,
                    finished_at TEXT,
                    error TEXT,
                    FOREIGN KEY (execution_id) REFERENCES executions(id) ON DELETE CASCADE
                );

                INSERT INTO workflow_versions
                    (id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum)
                SELECT v.id, v.workflow_id, v.workflow_name, v.version, v.definition, v.created_at, v.created_by, v.changelog, v.checksum
                FROM workflow_versions_old v
                JOIN workflows w ON w.id = v.workflow_id;

                INSERT INTO executions
                    (id, workflow_id, workflow_name, workflow_version, status, trigger_type, input, output, started_at, finished_at, error)
                SELECT e.id, e.workflow_id, e.workflow_name, NULL, e.status, e.trigger_type, e.input, e.output, e.started_at, e.finished_at, e.error
                FROM executions_old e
                JOIN workflows w ON w.id = e.workflow_id;

                INSERT INTO node_executions
                    (id, execution_id, node_id, status, input, output, started_at, finished_at, error)
                SELECT n.id, n.execution_id, n.node_id, n.status, n.input, n.output, n.started_at, n.finished_at, n.error
                FROM node_executions_old n
                JOIN executions e ON e.id = n.execution_id;

                DROP TABLE workflow_versions_old;
                DROP TABLE node_executions_old;
                DROP TABLE executions_old;

                CREATE INDEX IF NOT EXISTS idx_workflow_versions_workflow
                    ON workflow_versions(workflow_id, version DESC);
                CREATE INDEX IF NOT EXISTS idx_workflow_versions_name
                    ON workflow_versions(workflow_name, version DESC);
                CREATE INDEX IF NOT EXISTS idx_executions_workflow ON executions(workflow_id);
                CREATE INDEX IF NOT EXISTS idx_executions_workflow_name ON executions(workflow_name);
                CREATE INDEX IF NOT EXISTS idx_node_executions_execution ON node_executions(execution_id);
                "#,
            )?;

            tx.commit()?;
            Ok(())
        })();
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        migration_result
    }

    fn repair_orphans(conn: &Connection) -> Result<()> {
        conn.execute(
            "DELETE FROM node_executions
             WHERE execution_id NOT IN (SELECT id FROM executions)",
            [],
        )?;
        conn.execute(
            "DELETE FROM executions
             WHERE workflow_id NOT IN (SELECT id FROM workflows)",
            [],
        )?;
        conn.execute(
            "DELETE FROM workflow_versions
             WHERE workflow_id NOT IN (SELECT id FROM workflows)",
            [],
        )?;
        conn.execute(
            "DELETE FROM checkpoints
             WHERE execution_id NOT IN (SELECT id FROM executions)",
            [],
        )?;
        Ok(())
    }

    // ========================================================================
    // Workflow operations
    // ========================================================================

    pub async fn save_workflow(&self, workflow: &StoredWorkflow) -> Result<()> {
        let conn = self.conn.lock().await;
        let existing: Option<(String, String)> = conn
            .query_row(
                "SELECT id, created_at FROM workflows WHERE name = ?1",
                [workflow.name.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let effective = if let Some((existing_id, existing_created_at)) = existing {
            conn.execute(
                "UPDATE workflows
                 SET definition = ?1, enabled = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![
                    workflow.definition,
                    workflow.enabled,
                    workflow.updated_at.to_rfc3339(),
                    existing_id
                ],
            )?;

            StoredWorkflow {
                id: existing_id,
                name: workflow.name.clone(),
                definition: workflow.definition.clone(),
                enabled: workflow.enabled,
                created_at: parse_datetime_utc(&existing_created_at)
                    .unwrap_or_else(|_| Utc::now()),
                updated_at: workflow.updated_at,
            }
        } else {
            conn.execute(
                "INSERT INTO workflows (id, name, definition, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    workflow.id,
                    workflow.name,
                    workflow.definition,
                    workflow.enabled,
                    workflow.created_at.to_rfc3339(),
                    workflow.updated_at.to_rfc3339(),
                ],
            )?;

            workflow.clone()
        };

        let _ = Self::record_workflow_version_if_changed(&conn, &effective, None, None)?;
        Ok(())
    }

    pub async fn get_workflow(&self, name: &str) -> Result<Option<StoredWorkflow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at
             FROM workflows WHERE name = ?1",
        )?;

        let workflow = stmt
            .query_row([name], |row| {
                Ok(StoredWorkflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    definition: row.get(2)?,
                    enabled: row.get(3)?,
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                    updated_at: parse_datetime_utc(&row.get::<_, String>(5)?)?,
                })
            })
            .optional()?;

        Ok(workflow)
    }

    pub async fn get_workflow_by_id(&self, id: &str) -> Result<Option<StoredWorkflow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at
             FROM workflows WHERE id = ?1",
        )?;

        let workflow = stmt
            .query_row([id], |row| {
                Ok(StoredWorkflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    definition: row.get(2)?,
                    enabled: row.get(3)?,
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                    updated_at: parse_datetime_utc(&row.get::<_, String>(5)?)?,
                })
            })
            .optional()?;

        Ok(workflow)
    }

    pub async fn list_workflows(&self) -> Result<Vec<StoredWorkflow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at
             FROM workflows ORDER BY name",
        )?;

        let workflows = stmt
            .query_map([], |row| {
                Ok(StoredWorkflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    definition: row.get(2)?,
                    enabled: row.get(3)?,
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                    updated_at: parse_datetime_utc(&row.get::<_, String>(5)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(workflows)
    }

    pub async fn delete_workflow(&self, name: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute("DELETE FROM workflows WHERE name = ?1", [name])?;
        Ok(())
    }

    pub async fn check_health(&self) -> Result<DatabaseHealth> {
        let conn = self.conn.lock().await;

        let foreign_keys_enabled: i64 =
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
        let integrity_check: String =
            conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let busy_timeout_ms: i64 = conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;

        let mut violations_stmt = conn.prepare("PRAGMA foreign_key_check")?;
        let violations_iter = violations_stmt.query_map([], |row| {
            let table: String = row.get(0)?;
            let rowid: Option<i64> = row.get(1)?;
            let parent: String = row.get(2)?;
            let fk_id: i64 = row.get(3)?;
            Ok(format!(
                "table={} rowid={} parent={} fk_id={}",
                table,
                rowid
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                parent,
                fk_id
            ))
        })?;
        let foreign_key_violations = violations_iter.collect::<std::result::Result<Vec<_>, _>>()?;

        let orphaned_executions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM executions e
             LEFT JOIN workflows w ON w.id = e.workflow_id
             WHERE w.id IS NULL",
            [],
            |row| row.get(0),
        )?;

        let orphaned_node_executions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM node_executions n
             LEFT JOIN executions e ON e.id = n.execution_id
             WHERE e.id IS NULL",
            [],
            |row| row.get(0),
        )?;

        let orphaned_workflow_versions: i64 = conn.query_row(
            "SELECT COUNT(*) FROM workflow_versions v
             LEFT JOIN workflows w ON w.id = v.workflow_id
             WHERE w.id IS NULL",
            [],
            |row| row.get(0),
        )?;

        Ok(DatabaseHealth {
            foreign_keys_enabled: foreign_keys_enabled == 1,
            integrity_check,
            foreign_key_violations,
            orphaned_executions: orphaned_executions.max(0) as u64,
            orphaned_node_executions: orphaned_node_executions.max(0) as u64,
            orphaned_workflow_versions: orphaned_workflow_versions.max(0) as u64,
            journal_mode,
            busy_timeout_ms,
        })
    }

    pub async fn list_workflow_versions(
        &self,
        workflow_name: &str,
    ) -> Result<Vec<WorkflowVersion>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions
             WHERE workflow_name = ?1
             ORDER BY version DESC",
        )?;

        let versions = stmt
            .query_map([workflow_name], Self::row_to_workflow_version)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(versions)
    }

    pub async fn get_workflow_version(
        &self,
        workflow_name: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions
             WHERE workflow_name = ?1 AND version = ?2",
        )?;

        let record = stmt
            .query_row(
                params![workflow_name, version],
                Self::row_to_workflow_version,
            )
            .optional()?;

        Ok(record)
    }

    pub async fn get_workflow_version_by_id(
        &self,
        workflow_id: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions
             WHERE workflow_id = ?1 AND version = ?2",
        )?;

        let record = stmt
            .query_row(params![workflow_id, version], Self::row_to_workflow_version)
            .optional()?;

        Ok(record)
    }

    pub async fn get_latest_workflow_version_number(
        &self,
        workflow_id: &str,
    ) -> Result<Option<u32>> {
        let conn = self.conn.lock().await;
        let version = conn
            .query_row(
                "SELECT version FROM workflow_versions
                 WHERE workflow_id = ?1
                 ORDER BY version DESC
                 LIMIT 1",
                [workflow_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(version)
    }

    pub async fn get_workflow_version_at_or_before(
        &self,
        workflow_id: &str,
        timestamp: chrono::DateTime<Utc>,
    ) -> Result<Option<WorkflowVersion>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions
             WHERE workflow_id = ?1 AND created_at <= ?2
             ORDER BY version DESC
             LIMIT 1",
        )?;

        let record = stmt
            .query_row(
                params![workflow_id, timestamp.to_rfc3339()],
                Self::row_to_workflow_version,
            )
            .optional()?;

        Ok(record)
    }

    pub async fn rollback_workflow(
        &self,
        workflow_name: &str,
        version: u32,
        created_by: Option<&str>,
    ) -> Result<StoredWorkflow> {
        let conn = self.conn.lock().await;

        let mut wf_stmt = conn.prepare(
            "SELECT id, name, definition, enabled, created_at, updated_at
             FROM workflows WHERE name = ?1",
        )?;

        let mut workflow = wf_stmt
            .query_row([workflow_name], |row| {
                Ok(StoredWorkflow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    definition: row.get(2)?,
                    enabled: row.get(3)?,
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                    updated_at: parse_datetime_utc(&row.get::<_, String>(5)?)?,
                })
            })
            .optional()?
            .ok_or_else(|| Error::Storage(format!("Workflow not found: {}", workflow_name)))?;

        let mut version_stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions WHERE workflow_name = ?1 AND version = ?2",
        )?;

        let target = version_stmt
            .query_row(
                params![workflow_name, version],
                Self::row_to_workflow_version,
            )
            .optional()?
            .ok_or_else(|| {
                Error::Storage(format!(
                    "Workflow version not found: {} v{}",
                    workflow_name, version
                ))
            })?;

        workflow.definition = target.definition;
        workflow.updated_at = Utc::now();

        conn.execute(
            "UPDATE workflows SET definition = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                workflow.definition,
                workflow.updated_at.to_rfc3339(),
                workflow.id
            ],
        )?;

        let changelog = format!("Rollback to version {}", version);
        let _ = Self::record_workflow_version_if_changed(
            &conn,
            &workflow,
            created_by,
            Some(changelog.as_str()),
        )?;

        Ok(workflow)
    }

    // ========================================================================
    // Execution operations
    // ========================================================================

    pub async fn save_execution(&self, execution: &Execution) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO executions
             (id, workflow_id, workflow_name, workflow_version, status, trigger_type, input, output, started_at, finished_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                workflow_id = excluded.workflow_id,
                workflow_name = excluded.workflow_name,
                workflow_version = excluded.workflow_version,
                status = excluded.status,
                trigger_type = excluded.trigger_type,
                input = excluded.input,
                output = excluded.output,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                error = excluded.error",
            params![
                execution.id,
                execution.workflow_id,
                execution.workflow_name,
                execution.workflow_version,
                execution.status.to_string(),
                execution.trigger_type,
                serde_json::to_string(&execution.input).unwrap_or_default(),
                execution
                    .output
                    .as_ref()
                    .map(|o| serde_json::to_string(o).unwrap_or_default()),
                execution.started_at.to_rfc3339(),
                execution.finished_at.map(|t| t.to_rfc3339()),
                execution.error,
            ],
        )?;
        Ok(())
    }

    pub async fn get_execution(&self, id: &str) -> Result<Option<Execution>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, workflow_id, workflow_name, workflow_version, status, trigger_type, input, output, started_at, finished_at, error
             FROM executions WHERE id = ?1",
        )?;

        let execution = stmt
            .query_row([id], |row| {
                let status_str: String = row.get(4)?;
                let status = status_str.parse().unwrap_or(ExecutionStatus::Failed);
                let input_str: String = row.get(6)?;
                let output_str: Option<String> = row.get(7)?;

                Ok(Execution {
                    id: row.get(0)?,
                    workflow_id: row.get(1)?,
                    workflow_name: row.get(2)?,
                    workflow_version: row.get(3)?,
                    status,
                    trigger_type: row.get(5)?,
                    input: serde_json::from_str(&input_str).unwrap_or(serde_json::Value::Null),
                    output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
                    started_at: parse_datetime_utc(&row.get::<_, String>(8)?)?,
                    finished_at: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|t| t.with_timezone(&Utc)),
                    error: row.get(10)?,
                })
            })
            .optional()?;

        Ok(execution)
    }

    pub async fn list_executions(
        &self,
        workflow_name: &str,
        limit: usize,
    ) -> Result<Vec<Execution>> {
        let query = ExecutionQuery {
            workflow_name: Some(workflow_name.to_string()),
            limit,
            ..ExecutionQuery::default()
        };
        self.query_executions(&query).await
    }

    pub async fn query_executions(&self, query: &ExecutionQuery) -> Result<Vec<Execution>> {
        let conn = self.conn.lock().await;

        let mut sql = String::from(
            "SELECT id, workflow_id, workflow_name, workflow_version, status, trigger_type, input, output, started_at, finished_at, error
             FROM executions WHERE 1=1",
        );
        let mut bind: Vec<SqlValue> = Vec::new();

        if let Some(workflow_name) = &query.workflow_name {
            sql.push_str(" AND workflow_name = ?");
            bind.push(SqlValue::Text(workflow_name.clone()));
        }

        if let Some(status) = &query.status {
            sql.push_str(" AND status = ?");
            bind.push(SqlValue::Text(status.to_string()));
        }

        if let Some(trigger_type) = &query.trigger_type {
            sql.push_str(" AND trigger_type = ?");
            bind.push(SqlValue::Text(trigger_type.clone()));
        }

        if let Some(started_after) = &query.started_after {
            sql.push_str(" AND started_at >= ?");
            bind.push(SqlValue::Text(started_after.to_rfc3339()));
        }

        if let Some(started_before) = &query.started_before {
            sql.push_str(" AND started_at <= ?");
            bind.push(SqlValue::Text(started_before.to_rfc3339()));
        }

        if let Some(search) = &query.search {
            sql.push_str(
                " AND (input LIKE ? ESCAPE '\\' OR COALESCE(output, '') LIKE ? ESCAPE '\\' OR COALESCE(error, '') LIKE ? ESCAPE '\\')",
            );
            let pattern = format!("%{}%", escape_like_pattern(search));
            bind.push(SqlValue::Text(pattern.clone()));
            bind.push(SqlValue::Text(pattern.clone()));
            bind.push(SqlValue::Text(pattern));
        }

        sql.push_str(" ORDER BY started_at DESC LIMIT ? OFFSET ?");
        let limit = if query.limit == 0 {
            DEFAULT_QUERY_LIMIT
        } else {
            query.limit.min(MAX_QUERY_LIMIT)
        };
        bind.push(SqlValue::Integer(limit as i64));
        bind.push(SqlValue::Integer(query.offset as i64));

        let mut stmt = conn.prepare(&sql)?;
        let executions = stmt
            .query_map(params_from_iter(bind.iter()), Self::row_to_execution)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(executions)
    }

    pub async fn get_execution_trace(&self, execution_id: &str) -> Result<Option<ExecutionTrace>> {
        let execution = self.get_execution(execution_id).await?;
        let Some(execution) = execution else {
            return Ok(None);
        };

        let nodes = self.get_node_executions(execution_id).await?;
        Ok(Some(ExecutionTrace { execution, nodes }))
    }

    // ========================================================================
    // Node execution operations
    // ========================================================================

    pub async fn save_node_execution(&self, node_exec: &NodeExecution) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO node_executions
             (id, execution_id, node_id, status, input, output, started_at, finished_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                execution_id = excluded.execution_id,
                node_id = excluded.node_id,
                status = excluded.status,
                input = excluded.input,
                output = excluded.output,
                started_at = excluded.started_at,
                finished_at = excluded.finished_at,
                error = excluded.error",
            params![
                node_exec.id,
                node_exec.execution_id,
                node_exec.node_id,
                node_exec.status.to_string(),
                serde_json::to_string(&node_exec.input).unwrap_or_default(),
                node_exec
                    .output
                    .as_ref()
                    .map(|o| serde_json::to_string(o).unwrap_or_default()),
                node_exec.started_at.to_rfc3339(),
                node_exec.finished_at.map(|t| t.to_rfc3339()),
                node_exec.error,
            ],
        )?;
        Ok(())
    }

    pub async fn get_node_executions(&self, execution_id: &str) -> Result<Vec<NodeExecution>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, node_id, status, input, output, started_at, finished_at, error
             FROM node_executions WHERE execution_id = ?1 ORDER BY started_at",
        )?;

        let node_execs = stmt
            .query_map([execution_id], |row| {
                let status_str: String = row.get(3)?;
                let status = status_str.parse().unwrap_or(ExecutionStatus::Failed);
                let input_str: String = row.get(4)?;
                let output_str: Option<String> = row.get(5)?;

                Ok(NodeExecution {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    node_id: row.get(2)?,
                    status,
                    input: serde_json::from_str(&input_str).unwrap_or(serde_json::Value::Null),
                    output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
                    started_at: parse_datetime_utc(&row.get::<_, String>(6)?)?,
                    finished_at: row
                        .get::<_, Option<String>>(7)?
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|t| t.with_timezone(&Utc)),
                    error: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(node_execs)
    }

    // ========================================================================
    // Checkpoint operations
    // ========================================================================

    /// Save a checkpoint for an execution.
    pub async fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO checkpoints (id, execution_id, node_id, node_outputs, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                execution_id = excluded.execution_id,
                node_id = excluded.node_id,
                node_outputs = excluded.node_outputs,
                created_at = excluded.created_at",
            params![
                checkpoint.id,
                checkpoint.execution_id,
                checkpoint.node_id,
                serde_json::to_string(&checkpoint.node_outputs).unwrap_or_default(),
                checkpoint.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get the latest checkpoint for an execution.
    pub async fn get_latest_checkpoint(&self, execution_id: &str) -> Result<Option<Checkpoint>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, node_id, node_outputs, created_at
             FROM checkpoints
             WHERE execution_id = ?1
             ORDER BY created_at DESC
             LIMIT 1",
        )?;

        let checkpoint = stmt
            .query_row([execution_id], |row| {
                let outputs_str: String = row.get(3)?;
                Ok(Checkpoint {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    node_id: row.get(2)?,
                    node_outputs: serde_json::from_str(&outputs_str).unwrap_or(serde_json::Value::Null),
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                })
            })
            .optional()?;

        Ok(checkpoint)
    }

    /// List all checkpoints for an execution, ordered by creation time (newest first).
    pub async fn list_checkpoints(&self, execution_id: &str) -> Result<Vec<Checkpoint>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, execution_id, node_id, node_outputs, created_at
             FROM checkpoints
             WHERE execution_id = ?1
             ORDER BY created_at DESC",
        )?;

        let checkpoints = stmt
            .query_map([execution_id], |row| {
                let outputs_str: String = row.get(3)?;
                Ok(Checkpoint {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    node_id: row.get(2)?,
                    node_outputs: serde_json::from_str(&outputs_str).unwrap_or(serde_json::Value::Null),
                    created_at: parse_datetime_utc(&row.get::<_, String>(4)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(checkpoints)
    }

    fn row_to_workflow_version(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowVersion> {
        Ok(WorkflowVersion {
            id: row.get(0)?,
            workflow_id: row.get(1)?,
            workflow_name: row.get(2)?,
            version: row.get(3)?,
            definition: row.get(4)?,
            created_at: parse_datetime_utc(&row.get::<_, String>(5)?)?,
            created_by: row.get(6)?,
            changelog: row.get(7)?,
            checksum: row.get(8)?,
        })
    }

    fn row_to_execution(row: &rusqlite::Row<'_>) -> rusqlite::Result<Execution> {
        let status_str: String = row.get(4)?;
        let status = status_str.parse().unwrap_or(ExecutionStatus::Failed);
        let input_str: String = row.get(6)?;
        let output_str: Option<String> = row.get(7)?;

        Ok(Execution {
            id: row.get(0)?,
            workflow_id: row.get(1)?,
            workflow_name: row.get(2)?,
            workflow_version: row.get(3)?,
            status,
            trigger_type: row.get(5)?,
            input: serde_json::from_str(&input_str).unwrap_or(serde_json::Value::Null),
            output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
            started_at: parse_datetime_utc(&row.get::<_, String>(8)?)?,
            finished_at: row
                .get::<_, Option<String>>(9)?
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|t| t.with_timezone(&Utc)),
            error: row.get(10)?,
        })
    }

    fn record_workflow_version_if_changed(
        conn: &Connection,
        workflow: &StoredWorkflow,
        created_by: Option<&str>,
        changelog: Option<&str>,
    ) -> Result<Option<WorkflowVersion>> {
        let checksum = definition_checksum(&workflow.definition);

        let latest: Option<(u32, String)> = conn
            .query_row(
                "SELECT version, checksum FROM workflow_versions
                 WHERE workflow_id = ?1
                 ORDER BY version DESC
                 LIMIT 1",
                [workflow.id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((_, latest_checksum)) = &latest {
            if latest_checksum == &checksum {
                return Ok(None);
            }
        }

        let next_version = latest.map(|(version, _)| version + 1).unwrap_or(1);
        let version = WorkflowVersion {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_id: workflow.id.clone(),
            workflow_name: workflow.name.clone(),
            version: next_version,
            definition: workflow.definition.clone(),
            created_at: Utc::now(),
            created_by: created_by.map(|s| s.to_string()),
            changelog: changelog.map(|s| s.to_string()),
            checksum,
        };

        conn.execute(
            "INSERT INTO workflow_versions
             (id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                version.id,
                version.workflow_id,
                version.workflow_name,
                version.version,
                version.definition,
                version.created_at.to_rfc3339(),
                version.created_by,
                version.changelog,
                version.checksum,
            ],
        )?;

        Ok(Some(version))
    }

    // ========================================================================
    // Delayed events operations
    // ========================================================================

    /// Store a delayed event for future processing.
    pub async fn store_delayed_event(
        &self,
        event_name: &str,
        event_data: &str,
        scheduled_at: chrono::DateTime<Utc>,
    ) -> Result<String> {
        let conn = self.conn.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        
        conn.execute(
            "INSERT INTO delayed_events (id, event_name, event_data, scheduled_at, created_at, retry_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                event_name = excluded.event_name,
                event_data = excluded.event_data,
                scheduled_at = excluded.scheduled_at,
                retry_count = excluded.retry_count",
            params![
                &id,
                event_name,
                event_data,
                scheduled_at.to_rfc3339(),
                Utc::now().to_rfc3339(),
                0,
            ],
        )?;
        
        Ok(id)
    }

    /// Get delayed events that are due for processing.
    pub async fn get_due_delayed_events(&self) -> Result<Vec<(String, String, String)>> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        
        let mut stmt = conn.prepare(
            "SELECT id, event_name, event_data 
             FROM delayed_events 
             WHERE scheduled_at <= ?1 
             ORDER BY scheduled_at ASC"
        )?;
        
        let events = stmt
            .query_map([&now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        
        Ok(events)
    }

    /// Delete a delayed event by ID.
    pub async fn delete_delayed_event(&self, event_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM delayed_events WHERE id = ?1",
            [event_id],
        )?;
        Ok(())
    }

    /// Get count of pending delayed events.
    pub async fn count_pending_delayed_events(&self) -> Result<u64> {
        let conn = self.conn.lock().await;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM delayed_events WHERE scheduled_at > ?1",
            [Utc::now().to_rfc3339()],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u64)
    }
}

fn definition_checksum(definition: &str) -> String {
    let mut hasher = DefaultHasher::new();
    definition.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_workflow_crud() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let workflow = StoredWorkflow {
            id: "wf-123".to_string(),
            name: "test-workflow".to_string(),
            definition: "name: test".to_string(),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        storage.save_workflow(&workflow).await.unwrap();

        let loaded = storage.get_workflow("test-workflow").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().name, "test-workflow");

        let versions = storage
            .list_workflow_versions("test-workflow")
            .await
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
    }

    #[tokio::test]
    async fn test_workflow_versions_and_rollback() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let now = Utc::now();

        let mut workflow = StoredWorkflow {
            id: "wf-v".to_string(),
            name: "versioned".to_string(),
            definition: "name: versioned\nnodes:\n  - id: step\n    type: transform\n    config:\n      expression: '\"v1\"'".to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        storage.save_workflow(&workflow).await.unwrap();

        workflow.definition = "name: versioned\nnodes:\n  - id: step\n    type: transform\n    config:\n      expression: '\"v2\"'"
            .to_string();
        workflow.updated_at = Utc::now();
        storage.save_workflow(&workflow).await.unwrap();

        let versions = storage.list_workflow_versions("versioned").await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 2);
        assert_eq!(versions[1].version, 1);

        let rolled_back = storage
            .rollback_workflow("versioned", 1, Some("test"))
            .await
            .unwrap();
        assert!(rolled_back.definition.contains("\"v1\""));

        let versions_after = storage.list_workflow_versions("versioned").await.unwrap();
        assert_eq!(versions_after.len(), 3);
        assert_eq!(versions_after[0].version, 3);
        assert_eq!(
            versions_after[0].changelog.as_deref(),
            Some("Rollback to version 1")
        );
    }

    #[tokio::test]
    async fn test_execution_crud() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let workflow = StoredWorkflow {
            id: "wf-123".to_string(),
            name: "test-workflow".to_string(),
            definition: "name: test-workflow\nnodes:\n  - id: n1\n    type: transform\n    config:\n      expression: '1'".to_string(),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        storage.save_workflow(&workflow).await.unwrap();

        let execution = Execution {
            id: "exec-123".to_string(),
            workflow_id: "wf-123".to_string(),
            workflow_name: "test-workflow".to_string(),
            workflow_version: None,
            status: ExecutionStatus::Completed,
            trigger_type: "manual".to_string(),
            input: serde_json::json!({"key": "value"}),
            output: Some(serde_json::json!({"result": "ok"})),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            error: None,
        };

        storage.save_execution(&execution).await.unwrap();

        let loaded = storage.get_execution("exec-123").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().status, ExecutionStatus::Completed);

        let trace = storage.get_execution_trace("exec-123").await.unwrap();
        assert!(trace.is_some());
        assert_eq!(trace.unwrap().execution.id, "exec-123");
    }

    #[tokio::test]
    async fn test_query_executions_filters() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let now = Utc::now();

        let workflow = StoredWorkflow {
            id: "wf-q".to_string(),
            name: "query-workflow".to_string(),
            definition:
                "name: query-workflow\nnodes:\n  - id: n1\n    type: transform\n    config:\n      expression: '1'"
                    .to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        storage.save_workflow(&workflow).await.unwrap();

        let exec1 = Execution {
            id: "exec-q1".to_string(),
            workflow_id: "wf-q".to_string(),
            workflow_name: "query-workflow".to_string(),
            workflow_version: None,
            status: ExecutionStatus::Completed,
            trigger_type: "manual".to_string(),
            input: serde_json::json!({"note": "alpha"}),
            output: Some(serde_json::json!({"result": "ok"})),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            error: None,
        };
        storage.save_execution(&exec1).await.unwrap();

        let exec2 = Execution {
            id: "exec-q2".to_string(),
            workflow_id: "wf-q".to_string(),
            workflow_name: "query-workflow".to_string(),
            workflow_version: None,
            status: ExecutionStatus::Failed,
            trigger_type: "replay".to_string(),
            input: serde_json::json!({"note": "beta"}),
            output: None,
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            error: Some("boom".to_string()),
        };
        storage.save_execution(&exec2).await.unwrap();

        let only_failed = storage
            .query_executions(&ExecutionQuery {
                workflow_name: Some("query-workflow".to_string()),
                status: Some(ExecutionStatus::Failed),
                ..ExecutionQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(only_failed.len(), 1);
        assert_eq!(only_failed[0].id, "exec-q2");

        let search_boom = storage
            .query_executions(&ExecutionQuery {
                workflow_name: Some("query-workflow".to_string()),
                search: Some("boom".to_string()),
                ..ExecutionQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(search_boom.len(), 1);
        assert_eq!(search_boom[0].id, "exec-q2");
    }

    #[tokio::test]
    async fn test_foreign_keys_safe_updates() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        {
            let conn = storage.conn.lock().await;
            conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        }

        let now = Utc::now();
        let mut workflow = StoredWorkflow {
            id: "wf-fk".to_string(),
            name: "fk-workflow".to_string(),
            definition:
                "name: fk-workflow\nnodes:\n  - id: n1\n    type: transform\n    config:\n      expression: '1'"
                    .to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        storage.save_workflow(&workflow).await.unwrap();

        // Update workflow by same name with a different incoming id.
        workflow.id = "wf-fk-new".to_string();
        workflow.definition = "name: fk-workflow\nnodes:\n  - id: n1\n    type: transform\n    config:\n      expression: '2'"
            .to_string();
        workflow.updated_at = Utc::now();
        storage.save_workflow(&workflow).await.unwrap();

        let stored = storage.get_workflow("fk-workflow").await.unwrap().unwrap();
        assert_eq!(stored.id, "wf-fk");

        let execution = Execution {
            id: "exec-fk".to_string(),
            workflow_id: "wf-fk".to_string(),
            workflow_name: "fk-workflow".to_string(),
            workflow_version: None,
            status: ExecutionStatus::Running,
            trigger_type: "manual".to_string(),
            input: serde_json::json!({"x": 1}),
            output: None,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
        };
        storage.save_execution(&execution).await.unwrap();

        let node_exec = NodeExecution {
            id: "node-fk".to_string(),
            execution_id: "exec-fk".to_string(),
            node_id: "n1".to_string(),
            status: ExecutionStatus::Running,
            input: serde_json::json!({"x": 1}),
            output: None,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
        };
        storage.save_node_execution(&node_exec).await.unwrap();

        // Update parent and child records (would fail with INSERT OR REPLACE + FK ON).
        let mut updated_exec = execution.clone();
        updated_exec.status = ExecutionStatus::Completed;
        updated_exec.finished_at = Some(Utc::now());
        updated_exec.output = Some(serde_json::json!({"ok": true}));
        storage.save_execution(&updated_exec).await.unwrap();

        let mut updated_node = node_exec.clone();
        updated_node.status = ExecutionStatus::Completed;
        updated_node.finished_at = Some(Utc::now());
        updated_node.output = Some(serde_json::json!({"ok": true}));
        storage.save_node_execution(&updated_node).await.unwrap();

        let health = storage.check_health().await.unwrap();
        assert!(health.foreign_keys_enabled);
        assert_eq!(health.integrity_check.to_lowercase(), "ok");
        assert!(health.foreign_key_violations.is_empty());
        // In-memory databases use "memory" journal mode (WAL only applies to file DBs)
        assert_eq!(health.journal_mode.to_lowercase(), "memory");
        // Verify busy_timeout is set to handle lock contention
        assert_eq!(health.busy_timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_wal_mode_for_file_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let storage = SqliteStorage::open(&db_path).unwrap();

        let health = storage.check_health().await.unwrap();
        // File-based databases should use WAL mode for better concurrent access
        assert_eq!(health.journal_mode.to_lowercase(), "wal");
        // Verify busy_timeout is set to handle lock contention
        assert_eq!(health.busy_timeout_ms, 5000);
    }

    #[tokio::test]
    async fn test_cascade_delete_cleanup() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let now = Utc::now();

        let workflow = StoredWorkflow {
            id: "wf-cascade".to_string(),
            name: "cascade-workflow".to_string(),
            definition:
                "name: cascade-workflow\nnodes:\n  - id: n1\n    type: transform\n    config:\n      expression: '1'"
                    .to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        storage.save_workflow(&workflow).await.unwrap();

        let execution = Execution {
            id: "exec-cascade".to_string(),
            workflow_id: "wf-cascade".to_string(),
            workflow_name: "cascade-workflow".to_string(),
            workflow_version: None,
            status: ExecutionStatus::Running,
            trigger_type: "manual".to_string(),
            input: serde_json::json!({"x": 1}),
            output: None,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
        };
        storage.save_execution(&execution).await.unwrap();

        let node_exec = NodeExecution {
            id: "node-cascade".to_string(),
            execution_id: "exec-cascade".to_string(),
            node_id: "n1".to_string(),
            status: ExecutionStatus::Running,
            input: serde_json::json!({"x": 1}),
            output: None,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
        };
        storage.save_node_execution(&node_exec).await.unwrap();

        storage.delete_workflow("cascade-workflow").await.unwrap();

        assert!(storage
            .get_execution("exec-cascade")
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_node_executions("exec-cascade")
            .await
            .unwrap()
            .is_empty());
        assert!(storage
            .list_workflow_versions("cascade-workflow")
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_escape_like_pattern() {
        // Test that special LIKE characters are escaped
        assert_eq!(escape_like_pattern("hello"), "hello");
        assert_eq!(escape_like_pattern("100%"), "100\\%");
        assert_eq!(escape_like_pattern("test_value"), "test\\_value");
        assert_eq!(escape_like_pattern("%_%"), "\\%\\_\\%");
        assert_eq!(escape_like_pattern("back\\slash"), "back\\\\slash");

        // Combined special characters
        assert_eq!(escape_like_pattern("10% of_total"), "10\\% of\\_total");
    }
}
