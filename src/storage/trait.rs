//! `Storage` trait — backend-agnostic interface for r8r persistence.
//!
//! Both [`SqliteStorage`] and [`PostgresStorage`] implement this trait.
//! All callers that previously depended on `SqliteStorage` directly should
//! now accept `Arc<dyn Storage>` to remain backend-agnostic.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::audit::{AuditEntry, AuditEventType};
use super::dlq::{DeadLetterEntry, DlqStats, DlqStatus, NewDlqEntry};
use super::models::*;
use crate::error::Result;
use crate::llm::LlmConfig;

/// Backend-agnostic persistence interface for the r8r engine.
///
/// Implement this trait to support additional storage backends (e.g. PostgreSQL,
/// MySQL, DynamoDB). All async methods require `Send` so they are usable across
/// Tokio task boundaries.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    // -------------------------------------------------------------------------
    // Health
    // -------------------------------------------------------------------------

    async fn check_health(&self) -> Result<DatabaseHealth>;

    // -------------------------------------------------------------------------
    // Workflows
    // -------------------------------------------------------------------------

    async fn save_workflow(&self, workflow: &StoredWorkflow) -> Result<()>;
    async fn get_workflow(&self, name: &str) -> Result<Option<StoredWorkflow>>;
    async fn get_workflow_by_id(&self, id: &str) -> Result<Option<StoredWorkflow>>;
    async fn list_workflows(&self) -> Result<Vec<StoredWorkflow>>;
    async fn delete_workflow(&self, name: &str) -> Result<()>;

    // -------------------------------------------------------------------------
    // Workflow versions
    // -------------------------------------------------------------------------

    async fn list_workflow_versions(&self, workflow_name: &str) -> Result<Vec<WorkflowVersion>>;
    async fn get_workflow_version(
        &self,
        workflow_name: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>>;
    async fn get_workflow_version_by_id(
        &self,
        workflow_id: &str,
        version: u32,
    ) -> Result<Option<WorkflowVersion>>;
    async fn get_latest_workflow_version_number(&self, workflow_id: &str) -> Result<Option<u32>>;
    async fn get_workflow_version_at_or_before(
        &self,
        workflow_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<Option<WorkflowVersion>>;
    async fn rollback_workflow(
        &self,
        workflow_name: &str,
        version: u32,
        created_by: Option<&str>,
    ) -> Result<StoredWorkflow>;

    // -------------------------------------------------------------------------
    // Executions
    // -------------------------------------------------------------------------

    async fn save_execution(&self, execution: &Execution) -> Result<()>;
    async fn get_execution(&self, id: &str) -> Result<Option<Execution>>;
    async fn check_idempotency_key(&self, key: &str) -> Result<Option<Execution>>;
    async fn list_executions(&self, workflow_name: &str, limit: usize) -> Result<Vec<Execution>>;
    async fn query_executions(&self, query: &ExecutionQuery) -> Result<Vec<Execution>>;
    async fn get_execution_trace(&self, execution_id: &str) -> Result<Option<ExecutionTrace>>;

    // -------------------------------------------------------------------------
    // Node executions
    // -------------------------------------------------------------------------

    async fn save_node_execution(&self, node_exec: &NodeExecution) -> Result<()>;
    async fn get_node_executions(&self, execution_id: &str) -> Result<Vec<NodeExecution>>;

    // -------------------------------------------------------------------------
    // Checkpoints
    // -------------------------------------------------------------------------

    async fn save_checkpoint(&self, checkpoint: &Checkpoint) -> Result<()>;
    async fn get_latest_checkpoint(&self, execution_id: &str) -> Result<Option<Checkpoint>>;
    async fn list_checkpoints(&self, execution_id: &str) -> Result<Vec<Checkpoint>>;

    // -------------------------------------------------------------------------
    // Approval requests
    // -------------------------------------------------------------------------

    async fn save_approval_request(&self, request: &ApprovalRequest) -> Result<()>;
    async fn get_approval_request(&self, id: &str) -> Result<Option<ApprovalRequest>>;
    async fn decide_approval_request(
        &self,
        id: &str,
        new_status: &str,
        decided_by: &str,
        decision_comment: Option<&str>,
        decided_at: chrono::DateTime<chrono::Utc>,
        node_id: &str,
    ) -> Result<bool>;
    async fn list_approval_requests(
        &self,
        status_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>>;
    async fn list_approval_requests_filtered(
        &self,
        status_filter: Option<&str>,
        assignee_filter: Option<&str>,
        group_filter: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>>;
    async fn list_expired_approvals(&self) -> Result<Vec<ApprovalRequest>>;

    // -------------------------------------------------------------------------
    // REPL sessions
    // -------------------------------------------------------------------------

    async fn create_repl_session(&self, model: &str) -> Result<String>;
    async fn get_repl_session(&self, session_id: &str) -> Result<Option<ReplSession>>;
    async fn list_repl_sessions(&self, limit: usize) -> Result<Vec<ReplSession>>;
    async fn update_repl_session_summary(&self, session_id: &str, summary: &str) -> Result<()>;
    async fn save_repl_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        token_count: Option<i64>,
        run_id: Option<&str>,
    ) -> Result<()>;
    async fn list_repl_messages(&self, session_id: &str, limit: usize) -> Result<Vec<ReplMessage>>;
    async fn save_repl_llm_config(&self, config: &LlmConfig) -> Result<()>;
    async fn get_repl_llm_config(&self) -> Result<Option<LlmConfig>>;

    // -------------------------------------------------------------------------
    // Delayed events
    // -------------------------------------------------------------------------

    async fn store_delayed_event(
        &self,
        event_name: &str,
        event_data: &str,
        scheduled_at: DateTime<Utc>,
    ) -> Result<String>;
    async fn get_due_delayed_events(&self) -> Result<Vec<(String, String, String)>>;
    async fn delete_delayed_event(&self, event_id: &str) -> Result<()>;
    async fn count_pending_delayed_events(&self) -> Result<u64>;

    // -------------------------------------------------------------------------
    // Audit log
    // -------------------------------------------------------------------------

    async fn save_audit_entry(&self, entry: &AuditEntry) -> Result<()>;
    async fn list_audit_entries(
        &self,
        execution_id: Option<&str>,
        workflow_name: Option<&str>,
        event_type: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEntry>>;

    // -------------------------------------------------------------------------
    // Dead Letter Queue
    // -------------------------------------------------------------------------

    async fn add_to_dlq(&self, entry: NewDlqEntry<'_>) -> Result<String>;
    async fn get_dlq_entry(&self, id: &str) -> Result<Option<DeadLetterEntry>>;
    async fn list_dlq_entries(
        &self,
        status: Option<DlqStatus>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeadLetterEntry>>;
    async fn schedule_dlq_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<()>;
    async fn resolve_dlq_entry(&self, id: &str) -> Result<()>;
    async fn discard_dlq_entry(&self, id: &str) -> Result<()>;
    async fn increment_dlq_retry(&self, id: &str) -> Result<u32>;
    async fn get_dlq_stats(&self) -> Result<DlqStats>;
}

// Blanket impl: Arc<S> implements Storage when S does.
// This lets callers use Arc<dyn Storage> and also Arc<SqliteStorage> directly.
#[async_trait]
impl<S: Storage + ?Sized> Storage for std::sync::Arc<S> {
    async fn check_health(&self) -> Result<DatabaseHealth> {
        (**self).check_health().await
    }
    async fn save_workflow(&self, w: &StoredWorkflow) -> Result<()> {
        (**self).save_workflow(w).await
    }
    async fn get_workflow(&self, name: &str) -> Result<Option<StoredWorkflow>> {
        (**self).get_workflow(name).await
    }
    async fn get_workflow_by_id(&self, id: &str) -> Result<Option<StoredWorkflow>> {
        (**self).get_workflow_by_id(id).await
    }
    async fn list_workflows(&self) -> Result<Vec<StoredWorkflow>> {
        (**self).list_workflows().await
    }
    async fn delete_workflow(&self, name: &str) -> Result<()> {
        (**self).delete_workflow(name).await
    }
    async fn list_workflow_versions(&self, n: &str) -> Result<Vec<WorkflowVersion>> {
        (**self).list_workflow_versions(n).await
    }
    async fn get_workflow_version(&self, n: &str, v: u32) -> Result<Option<WorkflowVersion>> {
        (**self).get_workflow_version(n, v).await
    }
    async fn get_workflow_version_by_id(&self, id: &str, v: u32) -> Result<Option<WorkflowVersion>> {
        (**self).get_workflow_version_by_id(id, v).await
    }
    async fn get_latest_workflow_version_number(&self, id: &str) -> Result<Option<u32>> {
        (**self).get_latest_workflow_version_number(id).await
    }
    async fn get_workflow_version_at_or_before(
        &self,
        id: &str,
        ts: DateTime<Utc>,
    ) -> Result<Option<WorkflowVersion>> {
        (**self).get_workflow_version_at_or_before(id, ts).await
    }
    async fn rollback_workflow(
        &self,
        name: &str,
        version: u32,
        created_by: Option<&str>,
    ) -> Result<StoredWorkflow> {
        (**self).rollback_workflow(name, version, created_by).await
    }
    async fn save_execution(&self, e: &Execution) -> Result<()> {
        (**self).save_execution(e).await
    }
    async fn get_execution(&self, id: &str) -> Result<Option<Execution>> {
        (**self).get_execution(id).await
    }
    async fn check_idempotency_key(&self, key: &str) -> Result<Option<Execution>> {
        (**self).check_idempotency_key(key).await
    }
    async fn list_executions(&self, name: &str, limit: usize) -> Result<Vec<Execution>> {
        (**self).list_executions(name, limit).await
    }
    async fn query_executions(&self, q: &ExecutionQuery) -> Result<Vec<Execution>> {
        (**self).query_executions(q).await
    }
    async fn get_execution_trace(&self, id: &str) -> Result<Option<ExecutionTrace>> {
        (**self).get_execution_trace(id).await
    }
    async fn save_node_execution(&self, n: &NodeExecution) -> Result<()> {
        (**self).save_node_execution(n).await
    }
    async fn get_node_executions(&self, id: &str) -> Result<Vec<NodeExecution>> {
        (**self).get_node_executions(id).await
    }
    async fn save_checkpoint(&self, c: &Checkpoint) -> Result<()> {
        (**self).save_checkpoint(c).await
    }
    async fn get_latest_checkpoint(&self, id: &str) -> Result<Option<Checkpoint>> {
        (**self).get_latest_checkpoint(id).await
    }
    async fn list_checkpoints(&self, id: &str) -> Result<Vec<Checkpoint>> {
        (**self).list_checkpoints(id).await
    }
    async fn save_approval_request(&self, r: &ApprovalRequest) -> Result<()> {
        (**self).save_approval_request(r).await
    }
    async fn get_approval_request(&self, id: &str) -> Result<Option<ApprovalRequest>> {
        (**self).get_approval_request(id).await
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
        (**self).decide_approval_request(id, new_status, decided_by, decision_comment, decided_at, node_id).await
    }
    async fn list_approval_requests(
        &self,
        status: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>> {
        (**self).list_approval_requests(status, limit, offset).await
    }
    async fn list_approval_requests_filtered(
        &self,
        status: Option<&str>,
        assignee: Option<&str>,
        group: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRequest>> {
        (**self)
            .list_approval_requests_filtered(status, assignee, group, limit, offset)
            .await
    }
    async fn list_expired_approvals(&self) -> Result<Vec<ApprovalRequest>> {
        (**self).list_expired_approvals().await
    }
    async fn create_repl_session(&self, model: &str) -> Result<String> {
        (**self).create_repl_session(model).await
    }
    async fn get_repl_session(&self, id: &str) -> Result<Option<ReplSession>> {
        (**self).get_repl_session(id).await
    }
    async fn list_repl_sessions(&self, limit: usize) -> Result<Vec<ReplSession>> {
        (**self).list_repl_sessions(limit).await
    }
    async fn update_repl_session_summary(&self, id: &str, summary: &str) -> Result<()> {
        (**self).update_repl_session_summary(id, summary).await
    }
    async fn save_repl_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        token_count: Option<i64>,
        run_id: Option<&str>,
    ) -> Result<()> {
        (**self).save_repl_message(session_id, role, content, token_count, run_id).await
    }
    async fn list_repl_messages(&self, session_id: &str, limit: usize) -> Result<Vec<ReplMessage>> {
        (**self).list_repl_messages(session_id, limit).await
    }
    async fn save_repl_llm_config(&self, config: &LlmConfig) -> Result<()> {
        (**self).save_repl_llm_config(config).await
    }
    async fn get_repl_llm_config(&self) -> Result<Option<LlmConfig>> {
        (**self).get_repl_llm_config().await
    }
    async fn store_delayed_event(
        &self,
        name: &str,
        data: &str,
        scheduled_at: DateTime<Utc>,
    ) -> Result<String> {
        (**self).store_delayed_event(name, data, scheduled_at).await
    }
    async fn get_due_delayed_events(&self) -> Result<Vec<(String, String, String)>> {
        (**self).get_due_delayed_events().await
    }
    async fn delete_delayed_event(&self, id: &str) -> Result<()> {
        (**self).delete_delayed_event(id).await
    }
    async fn count_pending_delayed_events(&self) -> Result<u64> {
        (**self).count_pending_delayed_events().await
    }
    async fn save_audit_entry(&self, entry: &AuditEntry) -> Result<()> {
        (**self).save_audit_entry(entry).await
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
        (**self)
            .list_audit_entries(execution_id, workflow_name, event_type, since, limit, offset)
            .await
    }
    async fn add_to_dlq(&self, entry: NewDlqEntry<'_>) -> Result<String> {
        (**self).add_to_dlq(entry).await
    }
    async fn get_dlq_entry(&self, id: &str) -> Result<Option<DeadLetterEntry>> {
        (**self).get_dlq_entry(id).await
    }
    async fn list_dlq_entries(
        &self,
        status: Option<DlqStatus>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DeadLetterEntry>> {
        (**self).list_dlq_entries(status, limit, offset).await
    }
    async fn schedule_dlq_retry(&self, id: &str, next_retry_at: DateTime<Utc>) -> Result<()> {
        (**self).schedule_dlq_retry(id, next_retry_at).await
    }
    async fn resolve_dlq_entry(&self, id: &str) -> Result<()> {
        (**self).resolve_dlq_entry(id).await
    }
    async fn discard_dlq_entry(&self, id: &str) -> Result<()> {
        (**self).discard_dlq_entry(id).await
    }
    async fn increment_dlq_retry(&self, id: &str) -> Result<u32> {
        (**self).increment_dlq_retry(id).await
    }
    async fn get_dlq_stats(&self) -> Result<DlqStats> {
        (**self).get_dlq_stats().await
    }
}
