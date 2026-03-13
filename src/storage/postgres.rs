/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! PostgreSQL storage backend.
//!
//! Enable with `--features storage-postgres`. Requires a `DATABASE_URL`
//! environment variable pointing at a PostgreSQL database:
//!
//! ```text
//! DATABASE_URL=postgres://user:pass@localhost/r8r
//! ```
//!
//! The schema is created automatically on first connection via
//! [`PostgresStorage::connect`].

#![cfg(feature = "storage-postgres")]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use tracing::info;

use super::audit::{AuditEntry, AuditEventType};
use super::dlq::{DeadLetterEntry, DlqStats, DlqStatus, NewDlqEntry};
use super::models::*;
use super::Storage;
use crate::error::{Error, Result};
use crate::llm::{LlmConfig, LlmProvider};

/// PostgreSQL-backed storage.
///
/// Internally uses a `sqlx::PgPool` (connection pool) which is already
/// reference-counted and `Clone`-able, making this struct cheap to clone.
#[derive(Clone)]
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Connect to PostgreSQL and initialise the schema.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|e| Error::Storage(format!("PostgreSQL connect error: {}", e)))?;

        let storage = Self { pool };
        storage.init_schema().await?;
        Ok(storage)
    }

    /// Create all tables and indexes if they don't already exist.
    async fn init_schema(&self) -> Result<()> {
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                definition TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT TRUE,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS workflow_versions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
                workflow_name TEXT NOT NULL,
                version INTEGER NOT NULL,
                definition TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                created_by TEXT,
                changelog TEXT,
                checksum TEXT NOT NULL,
                UNIQUE(workflow_id, version)
            );

            CREATE INDEX IF NOT EXISTS idx_workflow_versions_workflow
                ON workflow_versions(workflow_id, version DESC);
            CREATE INDEX IF NOT EXISTS idx_workflow_versions_name
                ON workflow_versions(workflow_name, version DESC);

            CREATE TABLE IF NOT EXISTS executions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                workflow_version INTEGER,
                status TEXT NOT NULL,
                trigger_type TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                started_at TIMESTAMPTZ NOT NULL,
                finished_at TIMESTAMPTZ,
                error TEXT,
                correlation_id TEXT,
                idempotency_key TEXT UNIQUE,
                origin TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_executions_workflow ON executions(workflow_id);
            CREATE INDEX IF NOT EXISTS idx_executions_workflow_name ON executions(workflow_name);
            CREATE INDEX IF NOT EXISTS idx_executions_correlation ON executions(correlation_id);

            CREATE TABLE IF NOT EXISTS node_executions (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
                node_id TEXT NOT NULL,
                status TEXT NOT NULL,
                input TEXT NOT NULL,
                output TEXT,
                started_at TIMESTAMPTZ NOT NULL,
                finished_at TIMESTAMPTZ,
                error TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_node_executions_execution
                ON node_executions(execution_id);

            CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
                node_id TEXT NOT NULL,
                node_outputs TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_checkpoints_execution
                ON checkpoints(execution_id, created_at DESC);

            CREATE TABLE IF NOT EXISTS approval_requests (
                id TEXT PRIMARY KEY,
                execution_id TEXT NOT NULL,
                node_id TEXT NOT NULL,
                workflow_name TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                decision_comment TEXT,
                decided_by TEXT,
                context_data TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                expires_at TIMESTAMPTZ,
                decided_at TIMESTAMPTZ,
                assignee TEXT,
                groups TEXT NOT NULL DEFAULT '[]'
            );

            CREATE INDEX IF NOT EXISTS idx_approval_requests_status
                ON approval_requests(status);
            CREATE INDEX IF NOT EXISTS idx_approval_requests_execution
                ON approval_requests(execution_id);
            CREATE INDEX IF NOT EXISTS idx_approval_requests_assignee
                ON approval_requests(assignee);

            CREATE TABLE IF NOT EXISTS repl_sessions (
                id TEXT PRIMARY KEY,
                model TEXT NOT NULL,
                summary TEXT,
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS repl_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES repl_sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                token_count BIGINT,
                run_id TEXT,
                redacted BOOLEAN NOT NULL DEFAULT FALSE,
                created_at TIMESTAMPTZ NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_repl_messages_session ON repl_messages(session_id, created_at ASC);
            CREATE INDEX IF NOT EXISTS idx_repl_messages_run_id ON repl_messages(run_id);

            CREATE TABLE IF NOT EXISTS repl_llm_config (
                id INTEGER PRIMARY KEY DEFAULT 1,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                api_key TEXT,
                endpoint TEXT,
                temperature DOUBLE PRECISION,
                max_tokens BIGINT,
                timeout_seconds BIGINT NOT NULL DEFAULT 30,
                updated_at TIMESTAMPTZ NOT NULL
            );

            CREATE TABLE IF NOT EXISTS delayed_events (
                id TEXT PRIMARY KEY,
                event_name TEXT NOT NULL,
                event_data TEXT NOT NULL,
                scheduled_at TIMESTAMPTZ NOT NULL,
                created_at TIMESTAMPTZ NOT NULL,
                retry_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TIMESTAMPTZ NOT NULL,
                event_type TEXT NOT NULL,
                execution_id TEXT,
                workflow_name TEXT,
                node_id TEXT,
                actor TEXT NOT NULL,
                detail TEXT NOT NULL,
                metadata TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_audit_log_execution ON audit_log(execution_id);
            CREATE INDEX IF NOT EXISTS idx_audit_log_workflow ON audit_log(workflow_name);

            CREATE TABLE IF NOT EXISTS dead_letter_queue (
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
                created_at TIMESTAMPTZ NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL,
                next_retry_at TIMESTAMPTZ,
                status TEXT NOT NULL DEFAULT 'pending'
            );
        "#)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Storage(format!("Schema init error: {}", e)))?;

        info!("PostgreSQL schema initialised");
        Ok(())
    }
}

// ============================================================================
// Helper: convert sqlx errors to crate Error
// ============================================================================

fn sqlx_err(e: sqlx::Error) -> Error {
    Error::Storage(e.to_string())
}

// ============================================================================
// Helper: map a PgRow to StoredWorkflow
// ============================================================================

fn row_to_workflow(r: &sqlx::postgres::PgRow) -> Result<StoredWorkflow> {
    Ok(StoredWorkflow {
        id: r.try_get("id").map_err(sqlx_err)?,
        name: r.try_get("name").map_err(sqlx_err)?,
        definition: r.try_get("definition").map_err(sqlx_err)?,
        enabled: r.try_get("enabled").map_err(sqlx_err)?,
        created_at: r.try_get("created_at").map_err(sqlx_err)?,
        updated_at: r.try_get("updated_at").map_err(sqlx_err)?,
    })
}

fn row_to_version(r: &sqlx::postgres::PgRow) -> Result<WorkflowVersion> {
    Ok(WorkflowVersion {
        id: r.try_get("id").map_err(sqlx_err)?,
        workflow_id: r.try_get("workflow_id").map_err(sqlx_err)?,
        workflow_name: r.try_get("workflow_name").map_err(sqlx_err)?,
        version: r.try_get::<i32, _>("version").map_err(sqlx_err)? as u32,
        definition: r.try_get("definition").map_err(sqlx_err)?,
        created_at: r.try_get("created_at").map_err(sqlx_err)?,
        created_by: r.try_get("created_by").ok(),
        changelog: r.try_get("changelog").ok(),
        checksum: r.try_get("checksum").map_err(sqlx_err)?,
    })
}

fn row_to_execution(r: &sqlx::postgres::PgRow) -> Result<Execution> {
    let input_str: String = r.try_get("input").map_err(sqlx_err)?;
    let output_str: Option<String> = r.try_get("output").ok().flatten();
    let status_str: String = r.try_get("status").map_err(sqlx_err)?;
    Ok(Execution {
        id: r.try_get("id").map_err(sqlx_err)?,
        workflow_id: r.try_get("workflow_id").map_err(sqlx_err)?,
        workflow_name: r.try_get("workflow_name").map_err(sqlx_err)?,
        workflow_version: r
            .try_get::<Option<i32>, _>("workflow_version")
            .ok()
            .flatten()
            .map(|v| v as u32),
        status: status_str.parse().unwrap_or(ExecutionStatus::Failed),
        trigger_type: r.try_get("trigger_type").map_err(sqlx_err)?,
        input: serde_json::from_str(&input_str).unwrap_or_default(),
        output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
        started_at: r.try_get("started_at").map_err(sqlx_err)?,
        finished_at: r.try_get("finished_at").ok().flatten(),
        error: r.try_get("error").ok().flatten(),
        correlation_id: r.try_get("correlation_id").ok().flatten(),
        idempotency_key: r.try_get("idempotency_key").ok().flatten(),
        origin: r.try_get("origin").ok().flatten(),
    })
}

fn row_to_approval(r: &sqlx::postgres::PgRow) -> Result<ApprovalRequest> {
    let ctx: Option<String> = r.try_get("context_data").ok().flatten();
    let grps: String = r.try_get("groups").unwrap_or_else(|_| "[]".to_string());
    Ok(ApprovalRequest {
        id: r.try_get("id").map_err(sqlx_err)?,
        execution_id: r.try_get("execution_id").map_err(sqlx_err)?,
        node_id: r.try_get("node_id").map_err(sqlx_err)?,
        workflow_name: r.try_get("workflow_name").map_err(sqlx_err)?,
        title: r.try_get("title").map_err(sqlx_err)?,
        description: r.try_get("description").ok().flatten(),
        status: r.try_get("status").map_err(sqlx_err)?,
        decision_comment: r.try_get("decision_comment").ok().flatten(),
        decided_by: r.try_get("decided_by").ok().flatten(),
        context_data: ctx.and_then(|s| serde_json::from_str(&s).ok()),
        created_at: r.try_get("created_at").map_err(sqlx_err)?,
        expires_at: r.try_get("expires_at").ok().flatten(),
        decided_at: r.try_get("decided_at").ok().flatten(),
        assignee: r.try_get("assignee").ok().flatten(),
        groups: serde_json::from_str(&grps).unwrap_or_default(),
    })
}

fn row_to_dlq(r: &sqlx::postgres::PgRow) -> Result<DeadLetterEntry> {
    let input_str: String = r.try_get("input").map_err(sqlx_err)?;
    let status_str: String = r.try_get("status").map_err(sqlx_err)?;
    Ok(DeadLetterEntry {
        id: r.try_get("id").map_err(sqlx_err)?,
        execution_id: r.try_get("execution_id").map_err(sqlx_err)?,
        workflow_id: r.try_get("workflow_id").map_err(sqlx_err)?,
        workflow_name: r.try_get("workflow_name").map_err(sqlx_err)?,
        trigger_type: r.try_get("trigger_type").map_err(sqlx_err)?,
        input: serde_json::from_str(&input_str).unwrap_or(Value::Null),
        error: r.try_get("error").map_err(sqlx_err)?,
        failed_node_id: r.try_get("failed_node_id").ok().flatten(),
        retry_count: r.try_get::<i32, _>("retry_count").map_err(sqlx_err)? as u32,
        max_retries: r.try_get::<i32, _>("max_retries").map_err(sqlx_err)? as u32,
        created_at: r.try_get("created_at").map_err(sqlx_err)?,
        updated_at: r.try_get("updated_at").map_err(sqlx_err)?,
        next_retry_at: r.try_get("next_retry_at").ok().flatten(),
        status: status_str.parse().unwrap_or(DlqStatus::Pending),
    })
}

// ============================================================================
// Storage trait implementation
// ============================================================================

#[async_trait]
impl Storage for PostgresStorage {
    // -------------------------------------------------------------------------
    // Health
    // -------------------------------------------------------------------------

    async fn check_health(&self) -> Result<DatabaseHealth> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;

        Ok(DatabaseHealth {
            foreign_keys_enabled: true,
            integrity_check: "ok".to_string(),
            foreign_key_violations: vec![],
            orphaned_executions: 0,
            orphaned_node_executions: 0,
            orphaned_workflow_versions: 0,
            journal_mode: "wal".to_string(),
            busy_timeout_ms: 0,
        })
    }

    // -------------------------------------------------------------------------
    // Workflows
    // -------------------------------------------------------------------------

    async fn save_workflow(&self, w: &StoredWorkflow) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO workflows (id, name, definition, enabled, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT(id) DO UPDATE SET
                   name = EXCLUDED.name,
                   definition = EXCLUDED.definition,
                   enabled = EXCLUDED.enabled,
                   updated_at = EXCLUDED.updated_at"#,
        )
        .bind(&w.id)
        .bind(&w.name)
        .bind(&w.definition)
        .bind(w.enabled)
        .bind(w.created_at)
        .bind(w.updated_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_workflow(&self, name: &str) -> Result<Option<StoredWorkflow>> {
        let row = sqlx::query(
            "SELECT id, name, definition, enabled, created_at, updated_at FROM workflows WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_workflow).transpose()
    }

    async fn get_workflow_by_id(&self, id: &str) -> Result<Option<StoredWorkflow>> {
        let row = sqlx::query(
            "SELECT id, name, definition, enabled, created_at, updated_at FROM workflows WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_workflow).transpose()
    }

    async fn list_workflows(&self) -> Result<Vec<StoredWorkflow>> {
        let rows = sqlx::query(
            "SELECT id, name, definition, enabled, created_at, updated_at FROM workflows ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;
        rows.iter().map(row_to_workflow).collect()
    }

    async fn delete_workflow(&self, name: &str) -> Result<()> {
        sqlx::query("DELETE FROM workflows WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Workflow versions
    // -------------------------------------------------------------------------

    async fn list_workflow_versions(&self, workflow_name: &str) -> Result<Vec<WorkflowVersion>> {
        let rows = sqlx::query(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions WHERE workflow_name = $1 ORDER BY version DESC",
        )
        .bind(workflow_name)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;
        rows.iter().map(row_to_version).collect()
    }

    async fn get_workflow_version(
        &self,
        workflow_name: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>> {
        let row = sqlx::query(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions WHERE workflow_name = $1 AND version = $2",
        )
        .bind(workflow_name)
        .bind(version as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_version).transpose()
    }

    async fn get_workflow_version_by_id(
        &self,
        workflow_id: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>> {
        let row = sqlx::query(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions WHERE workflow_id = $1 AND version = $2",
        )
        .bind(workflow_id)
        .bind(version as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_version).transpose()
    }

    async fn get_latest_workflow_version_number(&self, workflow_id: &str) -> Result<Option<u32>> {
        let row: Option<Option<i32>> = sqlx::query_scalar(
            "SELECT version FROM workflow_versions WHERE workflow_id = $1 ORDER BY version DESC LIMIT 1",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(row.flatten().map(|v| v as u32))
    }

    async fn get_workflow_version_at_or_before(
        &self,
        workflow_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<WorkflowVersion>> {
        let row = sqlx::query(
            "SELECT id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum
             FROM workflow_versions WHERE workflow_id = $1 AND created_at <= $2
             ORDER BY version DESC LIMIT 1",
        )
        .bind(workflow_id)
        .bind(timestamp)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_version).transpose()
    }

    async fn rollback_workflow(
        &self,
        workflow_name: &str,
        version: u32,
        created_by: Option<&str>,
    ) -> Result<StoredWorkflow> {
        let ver = self
            .get_workflow_version(workflow_name, version)
            .await?
            .ok_or_else(|| {
                Error::Storage(format!(
                    "Version {} not found for {}",
                    version, workflow_name
                ))
            })?;
        let wf = self
            .get_workflow(workflow_name)
            .await?
            .ok_or_else(|| Error::Storage(format!("Workflow '{}' not found", workflow_name)))?;

        let now = Utc::now();
        sqlx::query("UPDATE workflows SET definition = $1, updated_at = $2 WHERE name = $3")
            .bind(&ver.definition)
            .bind(now)
            .bind(workflow_name)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;

        let max_v = self
            .get_latest_workflow_version_number(&wf.id)
            .await?
            .unwrap_or(0);
        let new_version = max_v + 1;
        let new_ver_id = uuid::Uuid::new_v4().to_string();
        let checksum = format!("{:016x}", {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            ver.definition.hash(&mut h);
            h.finish()
        });

        sqlx::query(
            "INSERT INTO workflow_versions (id, workflow_id, workflow_name, version, definition, created_at, created_by, changelog, checksum)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(&new_ver_id)
        .bind(&wf.id)
        .bind(workflow_name)
        .bind(new_version as i32)
        .bind(&ver.definition)
        .bind(now)
        .bind(created_by)
        .bind(format!("Rollback to version {}", version))
        .bind(&checksum)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;

        Ok(StoredWorkflow {
            id: wf.id,
            name: wf.name,
            definition: ver.definition,
            enabled: wf.enabled,
            created_at: wf.created_at,
            updated_at: now,
        })
    }

    // -------------------------------------------------------------------------
    // Executions
    // -------------------------------------------------------------------------

    async fn save_execution(&self, e: &Execution) -> Result<()> {
        let status = e.status.to_string();
        let input_json = serde_json::to_string(&e.input)?;
        let output_json = e.output.as_ref().map(|v| v.to_string());

        sqlx::query(
            r#"INSERT INTO executions
               (id, workflow_id, workflow_name, workflow_version, status, trigger_type, input, output,
                started_at, finished_at, error, correlation_id, idempotency_key, origin)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               ON CONFLICT(id) DO UPDATE SET
                   status = EXCLUDED.status,
                   output = EXCLUDED.output,
                   finished_at = EXCLUDED.finished_at,
                   error = EXCLUDED.error"#,
        )
        .bind(&e.id)
        .bind(&e.workflow_id)
        .bind(&e.workflow_name)
        .bind(e.workflow_version.map(|v| v as i32))
        .bind(&status)
        .bind(&e.trigger_type)
        .bind(&input_json)
        .bind(&output_json)
        .bind(e.started_at)
        .bind(e.finished_at)
        .bind(&e.error)
        .bind(&e.correlation_id)
        .bind(&e.idempotency_key)
        .bind(&e.origin)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_execution(&self, id: &str) -> Result<Option<Execution>> {
        let row = sqlx::query(
            "SELECT id, workflow_id, workflow_name, workflow_version, status, trigger_type,
                    input, output, started_at, finished_at, error, correlation_id, idempotency_key, origin
             FROM executions WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_execution).transpose()
    }

    async fn check_idempotency_key(&self, key: &str) -> Result<Option<Execution>> {
        let row = sqlx::query(
            "SELECT id, workflow_id, workflow_name, workflow_version, status, trigger_type,
                    input, output, started_at, finished_at, error, correlation_id, idempotency_key, origin
             FROM executions WHERE idempotency_key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_execution).transpose()
    }

    async fn list_executions(&self, workflow_name: &str, limit: usize) -> Result<Vec<Execution>> {
        let query = ExecutionQuery {
            workflow_name: Some(workflow_name.to_string()),
            limit,
            ..ExecutionQuery::default()
        };
        self.query_executions(&query).await
    }

    async fn query_executions(&self, query: &ExecutionQuery) -> Result<Vec<Execution>> {
        let mut sql = String::from(
            "SELECT id, workflow_id, workflow_name, workflow_version, status, trigger_type,
                    input, output, started_at, finished_at, error, correlation_id, idempotency_key, origin
             FROM executions WHERE 1=1",
        );
        let mut idx = 1usize;
        let mut binds: Vec<String> = Vec::new();

        if let Some(wn) = &query.workflow_name {
            sql.push_str(&format!(" AND workflow_name = ${}", idx));
            binds.push(wn.clone());
            idx += 1;
        }
        if let Some(status) = &query.status {
            sql.push_str(&format!(" AND status = ${}", idx));
            binds.push(status.to_string());
            idx += 1;
        }
        if let Some(tt) = &query.trigger_type {
            sql.push_str(&format!(" AND trigger_type = ${}", idx));
            binds.push(tt.clone());
            idx += 1;
        }
        if let Some(after) = &query.started_after {
            sql.push_str(&format!(" AND started_at >= ${}", idx));
            binds.push(after.to_rfc3339());
            idx += 1;
        }
        if let Some(before) = &query.started_before {
            sql.push_str(&format!(" AND started_at <= ${}", idx));
            binds.push(before.to_rfc3339());
            idx += 1;
        }

        let limit = if query.limit == 0 {
            50
        } else {
            query.limit.min(1000)
        };
        let offset = query.offset;
        sql.push_str(&format!(
            " ORDER BY started_at DESC LIMIT ${} OFFSET ${}",
            idx,
            idx + 1
        ));

        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b.as_str());
        }
        q = q.bind(limit as i64).bind(offset as i64);

        let rows = q.fetch_all(&self.pool).await.map_err(sqlx_err)?;
        rows.iter().map(row_to_execution).collect()
    }

    async fn get_execution_trace(&self, execution_id: &str) -> Result<Option<ExecutionTrace>> {
        let execution = self.get_execution(execution_id).await?;
        let Some(execution) = execution else {
            return Ok(None);
        };
        let nodes = self.get_node_executions(execution_id).await?;
        Ok(Some(ExecutionTrace { execution, nodes }))
    }

    // -------------------------------------------------------------------------
    // Node executions
    // -------------------------------------------------------------------------

    async fn save_node_execution(&self, n: &NodeExecution) -> Result<()> {
        let status_str = n.status.to_string();
        let input_json = serde_json::to_string(&n.input)?;
        let output_json = n.output.as_ref().map(|v| v.to_string());

        sqlx::query(
            r#"INSERT INTO node_executions
               (id, execution_id, node_id, status, input, output, started_at, finished_at, error)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT(id) DO UPDATE SET
                   status = EXCLUDED.status,
                   output = EXCLUDED.output,
                   finished_at = EXCLUDED.finished_at,
                   error = EXCLUDED.error"#,
        )
        .bind(&n.id)
        .bind(&n.execution_id)
        .bind(&n.node_id)
        .bind(&status_str)
        .bind(&input_json)
        .bind(&output_json)
        .bind(n.started_at)
        .bind(n.finished_at)
        .bind(&n.error)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_node_executions(&self, execution_id: &str) -> Result<Vec<NodeExecution>> {
        let rows = sqlx::query(
            "SELECT id, execution_id, node_id, status, input, output, started_at, finished_at, error
             FROM node_executions WHERE execution_id = $1 ORDER BY started_at ASC",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;

        rows.iter()
            .map(|r| {
                let input_str: String = r.try_get("input").map_err(sqlx_err)?;
                let output_str: Option<String> = r.try_get("output").ok().flatten();
                let status_str: String = r.try_get("status").map_err(sqlx_err)?;
                Ok(NodeExecution {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    execution_id: r.try_get("execution_id").map_err(sqlx_err)?,
                    node_id: r.try_get("node_id").map_err(sqlx_err)?,
                    status: status_str.parse().unwrap_or(ExecutionStatus::Failed),
                    input: serde_json::from_str(&input_str).unwrap_or_default(),
                    output: output_str.and_then(|s| serde_json::from_str(&s).ok()),
                    started_at: r.try_get("started_at").map_err(sqlx_err)?,
                    finished_at: r.try_get("finished_at").ok().flatten(),
                    error: r.try_get("error").ok().flatten(),
                })
            })
            .collect()
    }

    // -------------------------------------------------------------------------
    // Checkpoints
    // -------------------------------------------------------------------------

    async fn save_checkpoint(&self, c: &Checkpoint) -> Result<()> {
        let node_outputs_json = serde_json::to_string(&c.node_outputs)?;
        sqlx::query(
            r#"INSERT INTO checkpoints (id, execution_id, node_id, node_outputs, created_at)
               VALUES ($1,$2,$3,$4,$5)
               ON CONFLICT(id) DO UPDATE SET node_outputs = EXCLUDED.node_outputs"#,
        )
        .bind(&c.id)
        .bind(&c.execution_id)
        .bind(&c.node_id)
        .bind(&node_outputs_json)
        .bind(c.created_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_latest_checkpoint(&self, execution_id: &str) -> Result<Option<Checkpoint>> {
        let row = sqlx::query(
            "SELECT id, execution_id, node_id, node_outputs, created_at
             FROM checkpoints WHERE execution_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(execution_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;

        row.as_ref()
            .map(|r| {
                let no_str: String = r.try_get("node_outputs").map_err(sqlx_err)?;
                Ok(Checkpoint {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    execution_id: r.try_get("execution_id").map_err(sqlx_err)?,
                    node_id: r.try_get("node_id").map_err(sqlx_err)?,
                    node_outputs: serde_json::from_str(&no_str).unwrap_or_default(),
                    created_at: r.try_get("created_at").map_err(sqlx_err)?,
                })
            })
            .transpose()
    }

    async fn list_checkpoints(&self, execution_id: &str) -> Result<Vec<Checkpoint>> {
        let rows = sqlx::query(
            "SELECT id, execution_id, node_id, node_outputs, created_at
             FROM checkpoints WHERE execution_id = $1 ORDER BY created_at ASC",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;

        rows.iter()
            .map(|r| {
                let no_str: String = r.try_get("node_outputs").map_err(sqlx_err)?;
                Ok(Checkpoint {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    execution_id: r.try_get("execution_id").map_err(sqlx_err)?,
                    node_id: r.try_get("node_id").map_err(sqlx_err)?,
                    node_outputs: serde_json::from_str(&no_str).unwrap_or_default(),
                    created_at: r.try_get("created_at").map_err(sqlx_err)?,
                })
            })
            .collect()
    }

    // -------------------------------------------------------------------------
    // Approval requests
    // -------------------------------------------------------------------------

    async fn save_approval_request(&self, r: &ApprovalRequest) -> Result<()> {
        let context_json = r.context_data.as_ref().map(|v| v.to_string());
        let groups_json = serde_json::to_string(&r.groups)?;

        sqlx::query(
            r#"INSERT INTO approval_requests
               (id, execution_id, node_id, workflow_name, title, description, status,
                decision_comment, decided_by, context_data, created_at, expires_at, decided_at,
                assignee, groups)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
               ON CONFLICT(id) DO UPDATE SET
                   status = EXCLUDED.status,
                   decision_comment = EXCLUDED.decision_comment,
                   decided_by = EXCLUDED.decided_by,
                   decided_at = EXCLUDED.decided_at,
                   node_id = EXCLUDED.node_id"#,
        )
        .bind(&r.id)
        .bind(&r.execution_id)
        .bind(&r.node_id)
        .bind(&r.workflow_name)
        .bind(&r.title)
        .bind(&r.description)
        .bind(&r.status)
        .bind(&r.decision_comment)
        .bind(&r.decided_by)
        .bind(&context_json)
        .bind(r.created_at)
        .bind(r.expires_at)
        .bind(r.decided_at)
        .bind(&r.assignee)
        .bind(&groups_json)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_approval_request(&self, id: &str) -> Result<Option<ApprovalRequest>> {
        let row = sqlx::query(
            "SELECT id, execution_id, node_id, workflow_name, title, description, status,
                    decision_comment, decided_by, context_data, created_at, expires_at, decided_at,
                    assignee, groups
             FROM approval_requests WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_approval).transpose()
    }

    async fn decide_approval_request(
        &self,
        id: &str,
        new_status: &str,
        decided_by: &str,
        decision_comment: Option<&str>,
        decided_at: chrono::DateTime<chrono::Utc>,
        node_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE approval_requests SET status=$2, decided_by=$3, decision_comment=$4, decided_at=$5, node_id=$6 WHERE id=$1 AND status='pending'",
        )
        .bind(id)
        .bind(new_status)
        .bind(decided_by)
        .bind(decision_comment)
        .bind(decided_at)
        .bind(node_id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_approval_requests(
        &self,
        status_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>> {
        self.list_approval_requests_filtered(status_filter, None, None, limit, offset)
            .await
    }

    async fn list_approval_requests_filtered(
        &self,
        status_filter: Option<&str>,
        assignee_filter: Option<&str>,
        group_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>> {
        let mut sql = String::from(
            "SELECT id, execution_id, node_id, workflow_name, title, description, status,
                    decision_comment, decided_by, context_data, created_at, expires_at, decided_at,
                    assignee, groups
             FROM approval_requests WHERE 1=1",
        );
        let mut idx = 1usize;
        let mut binds: Vec<String> = Vec::new();

        if let Some(s) = status_filter {
            sql.push_str(&format!(" AND status = ${}", idx));
            binds.push(s.to_string());
            idx += 1;
        }
        if let Some(a) = assignee_filter {
            sql.push_str(&format!(" AND assignee = ${}", idx));
            binds.push(a.to_string());
            idx += 1;
        }
        if let Some(g) = group_filter {
            sql.push_str(&format!(" AND groups LIKE ${}", idx));
            binds.push(format!("%\"{}\"% ", g));
            idx += 1;
        }
        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ${} OFFSET ${}",
            idx,
            idx + 1
        ));

        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b.as_str());
        }
        q = q.bind(limit.min(1000) as i64).bind(offset as i64);

        let rows = q.fetch_all(&self.pool).await.map_err(sqlx_err)?;
        rows.iter().map(row_to_approval).collect()
    }

    async fn list_expired_approvals(&self) -> Result<Vec<ApprovalRequest>> {
        let now = Utc::now();
        let rows = sqlx::query(
            "SELECT id, execution_id, node_id, workflow_name, title, description, status,
                    decision_comment, decided_by, context_data, created_at, expires_at, decided_at,
                    assignee, groups
             FROM approval_requests WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < $1
             ORDER BY expires_at ASC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;
        rows.iter().map(row_to_approval).collect()
    }

    // -------------------------------------------------------------------------
    // REPL sessions
    // -------------------------------------------------------------------------

    async fn create_repl_session(&self, model: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO repl_sessions (id, model, created_at, updated_at) VALUES ($1,$2,$3,$4)",
        )
        .bind(&id)
        .bind(model)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(id)
    }

    async fn get_repl_session(&self, session_id: &str) -> Result<Option<ReplSession>> {
        let row = sqlx::query(
            "SELECT id, model, summary, created_at, updated_at FROM repl_sessions WHERE id = $1",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref()
            .map(|r| {
                Ok(ReplSession {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    model: r.try_get("model").map_err(sqlx_err)?,
                    summary: r.try_get("summary").ok().flatten(),
                    created_at: r.try_get("created_at").map_err(sqlx_err)?,
                    updated_at: r.try_get("updated_at").map_err(sqlx_err)?,
                })
            })
            .transpose()
    }

    async fn list_repl_sessions(&self, limit: usize) -> Result<Vec<ReplSession>> {
        let rows = sqlx::query(
            "SELECT id, model, summary, created_at, updated_at FROM repl_sessions ORDER BY updated_at DESC LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;
        rows.iter()
            .map(|r| {
                Ok(ReplSession {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    model: r.try_get("model").map_err(sqlx_err)?,
                    summary: r.try_get("summary").ok().flatten(),
                    created_at: r.try_get("created_at").map_err(sqlx_err)?,
                    updated_at: r.try_get("updated_at").map_err(sqlx_err)?,
                })
            })
            .collect()
    }

    async fn update_repl_session_summary(&self, session_id: &str, summary: &str) -> Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE repl_sessions SET summary=$1, updated_at=$2 WHERE id=$3")
            .bind(summary)
            .bind(now)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    // NOTE: run_id and redacted columns require a manual migration:
    // ALTER TABLE repl_messages ADD COLUMN IF NOT EXISTS run_id TEXT;
    // ALTER TABLE repl_messages ADD COLUMN IF NOT EXISTS redacted BOOLEAN NOT NULL DEFAULT FALSE;
    // CREATE INDEX IF NOT EXISTS idx_repl_messages_run_id ON repl_messages(run_id);
    async fn save_repl_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        token_count: Option<i64>,
        run_id: Option<&str>,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO repl_messages (id, session_id, role, content, token_count, run_id, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(role)
        .bind(content)
        .bind(token_count)
        .bind(run_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        sqlx::query("UPDATE repl_sessions SET updated_at=$1 WHERE id=$2")
            .bind(now)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    async fn list_repl_messages(&self, session_id: &str, limit: usize) -> Result<Vec<ReplMessage>> {
        let rows = sqlx::query(
            "SELECT id, session_id, role, content, token_count, run_id, redacted, created_at
             FROM repl_messages WHERE session_id = $1 ORDER BY created_at ASC LIMIT $2",
        )
        .bind(session_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;
        rows.iter()
            .map(|r| {
                Ok(ReplMessage {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    session_id: r.try_get("session_id").map_err(sqlx_err)?,
                    role: r.try_get("role").map_err(sqlx_err)?,
                    content: r.try_get("content").map_err(sqlx_err)?,
                    token_count: r.try_get("token_count").ok().flatten(),
                    run_id: r.try_get("run_id").ok(),
                    redacted: r.try_get::<bool, _>("redacted").unwrap_or(false),
                    created_at: r.try_get("created_at").map_err(sqlx_err)?,
                })
            })
            .collect()
    }

    async fn save_repl_llm_config(&self, config: &LlmConfig) -> Result<()> {
        let provider = match config.provider {
            LlmProvider::Openai => "openai",
            LlmProvider::Anthropic => "anthropic",
            LlmProvider::Ollama => "ollama",
            LlmProvider::Custom => "custom",
        };
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO repl_llm_config
               (id, provider, model, api_key, endpoint, temperature, max_tokens, timeout_seconds, updated_at)
               VALUES (1,$1,$2,$3,$4,$5,$6,$7,$8)
               ON CONFLICT(id) DO UPDATE SET
                   provider=EXCLUDED.provider, model=EXCLUDED.model, api_key=EXCLUDED.api_key,
                   endpoint=EXCLUDED.endpoint, temperature=EXCLUDED.temperature,
                   max_tokens=EXCLUDED.max_tokens, timeout_seconds=EXCLUDED.timeout_seconds,
                   updated_at=EXCLUDED.updated_at"#,
        )
        .bind(provider)
        .bind(&config.model)
        .bind(&config.api_key)
        .bind(&config.endpoint)
        .bind(config.temperature)
        .bind(config.max_tokens.map(|v| v as i64))
        .bind(config.timeout_seconds as i64)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn get_repl_llm_config(&self) -> Result<Option<LlmConfig>> {
        let row = sqlx::query(
            "SELECT provider, model, api_key, endpoint, temperature, max_tokens, timeout_seconds
             FROM repl_llm_config WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;

        row.as_ref()
            .map(|r| {
                let provider_str: String = r.try_get("provider").map_err(sqlx_err)?;
                let provider = match provider_str.to_ascii_lowercase().as_str() {
                    "anthropic" => LlmProvider::Anthropic,
                    "ollama" => LlmProvider::Ollama,
                    "custom" => LlmProvider::Custom,
                    _ => LlmProvider::Openai,
                };
                let timeout: i64 = r.try_get("timeout_seconds").unwrap_or(30);
                Ok(LlmConfig {
                    provider,
                    model: r.try_get("model").ok(),
                    api_key: r.try_get("api_key").ok().flatten(),
                    endpoint: r.try_get("endpoint").ok().flatten(),
                    temperature: r.try_get("temperature").ok().flatten(),
                    max_tokens: r
                        .try_get::<Option<i64>, _>("max_tokens")
                        .ok()
                        .flatten()
                        .map(|v| v as u32),
                    timeout_seconds: timeout.max(1) as u64,
                })
            })
            .transpose()
    }

    // -------------------------------------------------------------------------
    // Delayed events
    // -------------------------------------------------------------------------

    async fn store_delayed_event(
        &self,
        event_name: &str,
        event_data: &str,
        scheduled_at: DateTime<Utc>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            r#"INSERT INTO delayed_events (id, event_name, event_data, scheduled_at, created_at, retry_count)
               VALUES ($1,$2,$3,$4,$5,0)"#,
        )
        .bind(&id)
        .bind(event_name)
        .bind(event_data)
        .bind(scheduled_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(id)
    }

    async fn get_due_delayed_events(&self) -> Result<Vec<(String, String, String)>> {
        let now = Utc::now();
        let rows = sqlx::query(
            "SELECT id, event_name, event_data FROM delayed_events WHERE scheduled_at <= $1 ORDER BY scheduled_at ASC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(sqlx_err)?;

        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get("id").map_err(sqlx_err)?,
                    r.try_get("event_name").map_err(sqlx_err)?,
                    r.try_get("event_data").map_err(sqlx_err)?,
                ))
            })
            .collect()
    }

    async fn delete_delayed_event(&self, event_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM delayed_events WHERE id = $1")
            .bind(event_id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    async fn count_pending_delayed_events(&self) -> Result<u64> {
        let now = Utc::now();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM delayed_events WHERE scheduled_at > $1")
                .bind(now)
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        Ok(count.max(0) as u64)
    }

    // -------------------------------------------------------------------------
    // Audit log
    // -------------------------------------------------------------------------

    async fn save_audit_entry(&self, entry: &AuditEntry) -> Result<()> {
        let metadata_json = entry.metadata.as_ref().map(|v| v.to_string());
        sqlx::query(
            "INSERT INTO audit_log (id, timestamp, event_type, execution_id, workflow_name, node_id, actor, detail, metadata)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&entry.id)
        .bind(entry.timestamp)
        .bind(entry.event_type.to_string())
        .bind(&entry.execution_id)
        .bind(&entry.workflow_name)
        .bind(&entry.node_id)
        .bind(&entry.actor)
        .bind(&entry.detail)
        .bind(&metadata_json)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn list_audit_entries(
        &self,
        execution_id: Option<&str>,
        workflow_name: Option<&str>,
        event_type: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEntry>> {
        let mut sql = String::from(
            "SELECT id, timestamp, event_type, execution_id, workflow_name, node_id, actor, detail, metadata
             FROM audit_log WHERE 1=1",
        );
        let mut idx = 1usize;
        let mut binds: Vec<String> = Vec::new();

        if let Some(v) = execution_id {
            sql.push_str(&format!(" AND execution_id = ${}", idx));
            binds.push(v.to_string());
            idx += 1;
        }
        if let Some(v) = workflow_name {
            sql.push_str(&format!(" AND workflow_name = ${}", idx));
            binds.push(v.to_string());
            idx += 1;
        }
        if let Some(v) = event_type {
            sql.push_str(&format!(" AND event_type = ${}", idx));
            binds.push(v.to_string());
            idx += 1;
        }
        if let Some(v) = since {
            sql.push_str(&format!(" AND timestamp >= ${}", idx));
            binds.push(v.to_rfc3339());
            idx += 1;
        }
        sql.push_str(&format!(
            " ORDER BY timestamp DESC LIMIT ${} OFFSET ${}",
            idx,
            idx + 1
        ));

        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b.as_str());
        }
        q = q.bind(limit.min(1000) as i64).bind(offset as i64);

        let rows = q.fetch_all(&self.pool).await.map_err(sqlx_err)?;

        rows.iter()
            .map(|r| {
                let event_str: String = r.try_get("event_type").map_err(sqlx_err)?;
                let event_type: AuditEventType = event_str.parse().map_err(|_| {
                    Error::Storage(format!("Unknown audit event type: {}", event_str))
                })?;
                let metadata_str: Option<String> = r.try_get("metadata").ok().flatten();
                Ok(AuditEntry {
                    id: r.try_get("id").map_err(sqlx_err)?,
                    timestamp: r.try_get("timestamp").map_err(sqlx_err)?,
                    event_type,
                    execution_id: r.try_get("execution_id").ok().flatten(),
                    workflow_name: r.try_get("workflow_name").ok().flatten(),
                    node_id: r.try_get("node_id").ok().flatten(),
                    actor: r.try_get("actor").map_err(sqlx_err)?,
                    detail: r.try_get("detail").map_err(sqlx_err)?,
                    metadata: metadata_str.and_then(|s| serde_json::from_str(&s).ok()),
                })
            })
            .collect()
    }

    // -------------------------------------------------------------------------
    // Dead Letter Queue
    // -------------------------------------------------------------------------

    async fn add_to_dlq(&self, entry: NewDlqEntry<'_>) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let input_json = serde_json::to_string(entry.input)?;
        sqlx::query(
            "INSERT INTO dead_letter_queue
             (id, execution_id, workflow_id, workflow_name, trigger_type, input, error, failed_node_id, retry_count, max_retries, created_at, updated_at, status)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,0,$9,$10,$10,'pending')",
        )
        .bind(&id)
        .bind(entry.execution_id)
        .bind(entry.workflow_id)
        .bind(entry.workflow_name)
        .bind(entry.trigger_type)
        .bind(&input_json)
        .bind(entry.error)
        .bind(entry.failed_node_id)
        .bind(entry.max_retries as i32)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(id)
    }

    async fn get_dlq_entry(&self, id: &str) -> Result<Option<DeadLetterEntry>> {
        let row = sqlx::query(
            "SELECT id, execution_id, workflow_id, workflow_name, trigger_type, input, error,
                    failed_node_id, retry_count, max_retries, created_at, updated_at, next_retry_at, status
             FROM dead_letter_queue WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_err)?;
        row.as_ref().map(row_to_dlq).transpose()
    }

    async fn list_dlq_entries(
        &self,
        status: Option<DlqStatus>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeadLetterEntry>> {
        let rows = match &status {
            Some(s) => {
                let s_str = s.to_string();
                sqlx::query(
                    "SELECT id, execution_id, workflow_id, workflow_name, trigger_type, input, error,
                            failed_node_id, retry_count, max_retries, created_at, updated_at, next_retry_at, status
                     FROM dead_letter_queue WHERE status = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
                )
                .bind(s_str)
                .bind(limit as i64)
                .bind(offset as i64)
                .fetch_all(&self.pool)
                .await
                .map_err(sqlx_err)?
            }
            None => sqlx::query(
                "SELECT id, execution_id, workflow_id, workflow_name, trigger_type, input, error,
                        failed_node_id, retry_count, max_retries, created_at, updated_at, next_retry_at, status
                 FROM dead_letter_queue ORDER BY created_at DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit as i64)
            .bind(offset as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(sqlx_err)?,
        };

        rows.iter().map(row_to_dlq).collect()
    }

    async fn schedule_dlq_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE dead_letter_queue SET status='scheduled', next_retry_at=$1, updated_at=$2 WHERE id=$3",
        )
        .bind(next_retry_at)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        Ok(())
    }

    async fn resolve_dlq_entry(&self, id: &str) -> Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE dead_letter_queue SET status='resolved', updated_at=$1 WHERE id=$2")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    async fn discard_dlq_entry(&self, id: &str) -> Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE dead_letter_queue SET status='discarded', updated_at=$1 WHERE id=$2")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(sqlx_err)?;
        Ok(())
    }

    async fn increment_dlq_retry(&self, id: &str) -> Result<u32> {
        let now = Utc::now();
        sqlx::query(
            "UPDATE dead_letter_queue SET retry_count = retry_count + 1, updated_at=$1 WHERE id=$2",
        )
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(sqlx_err)?;
        let count: i32 =
            sqlx::query_scalar("SELECT retry_count FROM dead_letter_queue WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        Ok(count as u32)
    }

    async fn get_dlq_stats(&self) -> Result<DlqStats> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dead_letter_queue")
            .fetch_one(&self.pool)
            .await
            .map_err(sqlx_err)?;
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dead_letter_queue WHERE status='pending'")
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        let scheduled: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dead_letter_queue WHERE status='scheduled'")
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        let resolved: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dead_letter_queue WHERE status='resolved'")
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        let discarded: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dead_letter_queue WHERE status='discarded'")
                .fetch_one(&self.pool)
                .await
                .map_err(sqlx_err)?;
        Ok(DlqStats {
            total: total as u64,
            pending: pending as u64,
            scheduled: scheduled as u64,
            resolved: resolved as u64,
            discarded: discarded as u64,
        })
    }
}
