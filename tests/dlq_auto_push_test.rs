//! Integration tests for automatic DLQ push on workflow execution failure.
//!
//! These tests verify that:
//! 1. A failed execution automatically creates a DLQ entry.
//! 2. The DLQ entry fields are correct.
//! 3. A successful execution does NOT create a DLQ entry.

use chrono::Utc;
use r8r::engine::Executor;
use r8r::error::{Error, Result};
use r8r::nodes::{Node, NodeContext, NodeRegistry, NodeResult};
use r8r::storage::{DlqStatus, ExecutionStatus, SqliteStorage, Storage, StoredWorkflow};
use r8r::workflow::parse_workflow;
use serde_json::Value;
use std::sync::Arc;

// ============================================================================
// Helper: AlwaysFailNode — a node that always errors
// ============================================================================

struct AlwaysFailNode;

#[async_trait::async_trait]
impl Node for AlwaysFailNode {
    fn node_type(&self) -> &str {
        "always_fail"
    }

    async fn execute(&self, _config: &Value, _ctx: &NodeContext) -> Result<NodeResult> {
        Err(Error::Execution("Simulated node failure".to_string()))
    }

    fn description(&self) -> &str {
        "A test node that always fails"
    }
}

// ============================================================================
// Helper: save a workflow to storage
// ============================================================================

async fn save_test_workflow(storage: &dyn Storage, id: &str, yaml: &str) {
    let workflow = parse_workflow(yaml).unwrap();
    let now = Utc::now();
    storage
        .save_workflow(&StoredWorkflow {
            id: id.to_string(),
            name: workflow.name.clone(),
            definition: yaml.to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

// ============================================================================
// Test 1: Failed execution creates a DLQ entry
// ============================================================================

#[tokio::test]
async fn test_failed_execution_creates_dlq_entry() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());
    // DLQ table is created by init_schema_sync — no separate init_dlq() call needed.

    let mut registry = NodeRegistry::new();
    registry.register(Arc::new(AlwaysFailNode));
    let executor = Executor::new(registry, storage.clone());

    let yaml = r#"
name: failing-workflow
nodes:
  - id: fail-node
    type: always_fail
"#;

    let workflow = parse_workflow(yaml).unwrap();
    save_test_workflow(&storage, "wf-fail-1", yaml).await;

    let execution = executor
        .execute(&workflow, "wf-fail-1", "api", Value::Null)
        .await
        .unwrap();

    // The execution should be marked as failed
    assert_eq!(
        execution.status,
        ExecutionStatus::Failed,
        "Execution should have failed"
    );

    // There should be exactly one DLQ entry
    let entries = storage.list_dlq_entries(None, 100, 0).await.unwrap();
    assert_eq!(entries.len(), 1, "Expected exactly one DLQ entry");

    let entry = &entries[0];
    assert_eq!(
        entry.execution_id, execution.id,
        "DLQ entry execution_id should match"
    );
    assert_eq!(
        entry.workflow_name, "failing-workflow",
        "DLQ entry workflow_name should match"
    );
    assert!(
        !entry.error.is_empty(),
        "DLQ entry should have a non-empty error"
    );
}

// ============================================================================
// Test 2: DLQ entry has all the correct fields
// ============================================================================

#[tokio::test]
async fn test_dlq_entry_has_correct_fields() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());

    let mut registry = NodeRegistry::new();
    registry.register(Arc::new(AlwaysFailNode));
    let executor = Executor::new(registry, storage.clone());

    let yaml = r#"
name: field-check-workflow
nodes:
  - id: fail-node
    type: always_fail
"#;

    let workflow = parse_workflow(yaml).unwrap();
    save_test_workflow(&storage, "wf-field-check", yaml).await;

    let input = serde_json::json!({"user": "test", "value": 42});

    let execution = executor
        .execute(&workflow, "wf-field-check", "api", input.clone())
        .await
        .unwrap();

    assert_eq!(execution.status, ExecutionStatus::Failed);

    let entries = storage.list_dlq_entries(None, 100, 0).await.unwrap();
    assert_eq!(entries.len(), 1, "Expected exactly one DLQ entry");

    let entry = &entries[0];

    // execution_id
    assert_eq!(entry.execution_id, execution.id, "execution_id mismatch");

    // workflow_id
    assert_eq!(
        entry.workflow_id, "wf-field-check",
        "workflow_id mismatch"
    );

    // workflow_name
    assert_eq!(
        entry.workflow_name, "field-check-workflow",
        "workflow_name mismatch"
    );

    // trigger_type
    assert_eq!(entry.trigger_type, "api", "trigger_type mismatch");

    // input — the DLQ stores the original input
    assert_eq!(entry.input, input, "input mismatch");

    // error — should contain the error message
    assert!(
        entry.error.contains("Simulated node failure"),
        "error should contain 'Simulated node failure', got: '{}'",
        entry.error
    );

    // failed_node_id — should be the node that failed
    assert_eq!(
        entry.failed_node_id.as_deref(),
        Some("fail-node"),
        "failed_node_id mismatch"
    );

    // status — should be pending (default)
    assert_eq!(entry.status, DlqStatus::Pending, "status should be pending");

    // retry_count — newly created entries should have 0 retries
    assert_eq!(entry.retry_count, 0, "retry_count should be 0");

    // max_retries — we set this to 3
    assert_eq!(entry.max_retries, 3, "max_retries should be 3");
}

// ============================================================================
// Test 3: Successful execution does NOT create a DLQ entry
// ============================================================================

#[tokio::test]
async fn test_dlq_not_created_on_success() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());

    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage.clone());

    // Use the built-in transform node which succeeds
    let yaml = r#"
name: success-workflow
nodes:
  - id: transform-node
    type: transform
    config:
      expression: '"success"'
"#;

    let workflow = parse_workflow(yaml).unwrap();
    save_test_workflow(&storage, "wf-success", yaml).await;

    let execution = executor
        .execute(&workflow, "wf-success", "api", Value::Null)
        .await
        .unwrap();

    assert_eq!(
        execution.status,
        ExecutionStatus::Completed,
        "Execution should have completed successfully"
    );

    // DLQ should have zero entries
    let stats = storage.get_dlq_stats().await.unwrap();
    assert_eq!(
        stats.total, 0,
        "No DLQ entries should be created on success"
    );
}

// ============================================================================
// Test 4: Unknown node type also pushes to DLQ
// ============================================================================

#[tokio::test]
async fn test_unknown_node_type_creates_dlq_entry() {
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().unwrap());

    // Default registry does not have 'nonexistent_node_type_xyz'
    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage.clone());

    let yaml = r#"
name: unknown-node-workflow
nodes:
  - id: unknown-node
    type: nonexistent_node_type_xyz
"#;

    let workflow = parse_workflow(yaml).unwrap();
    save_test_workflow(&storage, "wf-unknown", yaml).await;

    let execution = executor
        .execute(&workflow, "wf-unknown", "webhook", Value::Null)
        .await
        .unwrap();

    assert_eq!(
        execution.status,
        ExecutionStatus::Failed,
        "Execution should have failed due to unknown node type"
    );

    let entries = storage.list_dlq_entries(None, 100, 0).await.unwrap();
    assert_eq!(entries.len(), 1, "Expected exactly one DLQ entry");

    let entry = &entries[0];
    assert!(
        entry.error.contains("Unknown node type"),
        "Error should mention unknown node type, got: '{}'",
        entry.error
    );
    assert_eq!(
        entry.failed_node_id.as_deref(),
        Some("unknown-node"),
        "failed_node_id should be the unknown node"
    );
}
