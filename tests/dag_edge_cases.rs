//! Integration tests for DAG edge cases, error handling, retry, and timeout.
//!
//! These tests use inline YAML, in-memory SQLite, and the real Executor to
//! verify end-to-end behavior.

use chrono::Utc;
use r8r::engine::Executor;
use r8r::nodes::NodeRegistry;
use r8r::storage::{ExecutionStatus, SqliteStorage, StoredWorkflow};
use r8r::workflow::parse_workflow;
use serde_json::json;

/// Helper: parse YAML, build executor, run workflow, return execution.
async fn run_workflow_yaml(yaml: &str) -> r8r::storage::Execution {
    let workflow = parse_workflow(yaml).expect("YAML should parse");
    let storage: std::sync::Arc<dyn r8r::storage::Storage> = std::sync::Arc::new(SqliteStorage::open_in_memory().expect("storage should open"));

    // The executor saves an execution record that references the workflow_id
    // via a foreign key, so the workflow must exist in storage first.
    let stored = StoredWorkflow {
        id: "test-wf-id".to_string(),
        name: workflow.name.clone(),
        definition: yaml.to_string(),
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    storage
        .save_workflow(&stored)
        .await
        .expect("should save workflow");

    let registry = NodeRegistry::new();
    let executor = Executor::new(registry, storage);
    executor
        .execute(&workflow, "test-wf-id", "test", json!({}))
        .await
        .expect("execution should not panic")
}

/// Helper: parse YAML and validate — returns Ok(()) or Err.
fn validate_yaml(yaml: &str) -> r8r::Result<()> {
    let workflow = parse_workflow(yaml)?;
    r8r::workflow::validate_workflow(&workflow)
}

// ─── Test 1: Diamond DAG ────────────────────────────────────────────

#[tokio::test]
async fn test_diamond_dag() {
    // Diamond shape: root -> left, root -> right, left+right -> join
    //
    // The transform node injects each dependency's output as a Rhai variable
    // (with dashes replaced by underscores). For multi-dependency nodes,
    // `input` is a JSON object keyed by dependency name.
    //
    // Node outputs flow as JSON strings through the Rhai scope, so we use
    // simple numeric expressions to avoid quoting complexity.
    let yaml = r#"
name: diamond-test
nodes:
  - id: root
    type: transform
    config:
      expression: "100"
  - id: left
    type: transform
    config:
      expression: "200"
    depends_on: [root]
  - id: right
    type: transform
    config:
      expression: "300"
    depends_on: [root]
  - id: join
    type: transform
    config:
      expression: '"left=" + left + ",right=" + right'
    depends_on: [left, right]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
    let output = exec.output.expect("should have output");
    let s = output.as_str().unwrap_or("");
    assert!(
        s.contains("left"),
        "output should contain left branch: {}",
        s
    );
    assert!(
        s.contains("right"),
        "output should contain right branch: {}",
        s
    );
}

// ─── Test 2: Fan-out / Fan-in ───────────────────────────────────────

#[tokio::test]
async fn test_fan_out_fan_in() {
    let yaml = r#"
name: fan-test
nodes:
  - id: source
    type: transform
    config:
      expression: "1"
  - id: branch-a
    type: transform
    config:
      expression: "10"
    depends_on: [source]
  - id: branch-b
    type: transform
    config:
      expression: "20"
    depends_on: [source]
  - id: branch-c
    type: transform
    config:
      expression: "30"
    depends_on: [source]
  - id: merge
    type: transform
    config:
      expression: '"a=" + branch_a + ",b=" + branch_b + ",c=" + branch_c'
    depends_on: [branch-a, branch-b, branch-c]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
    let output = exec.output.expect("should have output");
    let s = output.as_str().unwrap_or("");
    assert!(s.contains("a="), "output should contain branch-a: {}", s);
    assert!(s.contains("b="), "output should contain branch-b: {}", s);
    assert!(s.contains("c="), "output should contain branch-c: {}", s);
}

// ─── Test 3: Disconnected node ──────────────────────────────────────

#[tokio::test]
async fn test_disconnected_node() {
    let yaml = r#"
name: disconnected-test
nodes:
  - id: main-chain-1
    type: transform
    config:
      expression: "1"
  - id: main-chain-2
    type: transform
    config:
      expression: "2"
    depends_on: [main-chain-1]
  - id: island
    type: transform
    config:
      expression: "99"
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
}

// ─── Test 4: Cycle detection ────────────────────────────────────────

#[test]
fn test_cycle_detection() {
    let yaml = r#"
name: cycle-test
nodes:
  - id: node-a
    type: transform
    config:
      expression: "1"
    depends_on: [node-b]
  - id: node-b
    type: transform
    config:
      expression: "2"
    depends_on: [node-a]
"#;

    let result = validate_yaml(yaml);
    assert!(result.is_err(), "circular dependency should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("circular") || err.to_lowercase().contains("cycle"),
        "error should mention circular dependency: {}",
        err
    );
}

// ─── Test 5: on_error continue ──────────────────────────────────────
//
// Uses an HTTP node targeting localhost (blocked by SSRF protection) to
// trigger an instant, deterministic failure without needing network access.

#[tokio::test]
async fn test_on_error_continue() {
    let yaml = r#"
name: error-continue-test
settings:
  timeout_seconds: 10
nodes:
  - id: start
    type: transform
    config:
      expression: "1"
  - id: fail-node
    type: http
    config:
      url: http://localhost:1/will-fail
      method: GET
    on_error:
      continue: true
    depends_on: [start]
  - id: after-fail
    type: transform
    config:
      expression: '"continued-past-failure"'
    depends_on: [fail-node]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(
        exec.status,
        ExecutionStatus::Completed,
        "workflow should complete despite node failure: {:?}",
        exec.error
    );
}

// ─── Test 6: on_error fallback ──────────────────────────────────────

#[tokio::test]
async fn test_on_error_fallback() {
    let yaml = r#"
name: error-fallback-test
settings:
  timeout_seconds: 10
nodes:
  - id: start
    type: transform
    config:
      expression: "1"
  - id: fail-node
    type: http
    config:
      url: http://localhost:1/will-fail
      method: GET
    on_error:
      action: fallback
      fallback_value:
        status: "recovered"
        data: "fallback-data"
    depends_on: [start]
  - id: use-fallback
    type: transform
    config:
      expression: '"got-fallback"'
    depends_on: [fail-node]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(
        exec.status,
        ExecutionStatus::Completed,
        "workflow should complete with fallback: {:?}",
        exec.error
    );
}

// ─── Test 7: Node timeout ───────────────────────────────────────────
//
// Uses a wait node (10s) with a short workflow timeout (3s). The
// executor wraps each node execution in tokio::time::timeout so the
// wait is interrupted and the workflow fails.

#[tokio::test]
async fn test_node_timeout() {
    let yaml = r#"
name: timeout-test
settings:
  timeout_seconds: 3
nodes:
  - id: start
    type: transform
    config:
      expression: "1"
  - id: slow-node
    type: wait
    config:
      seconds: 10
    depends_on: [start]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(
        exec.status,
        ExecutionStatus::Failed,
        "workflow should fail due to timeout"
    );
    let err = exec.error.unwrap_or_default();
    assert!(
        err.to_lowercase().contains("timeout") || err.to_lowercase().contains("timed out"),
        "error should mention timeout: {}",
        err
    );
}

// ─── Test 8: Retry with backoff ─────────────────────────────────────
//
// Uses an HTTP node targeting localhost (SSRF-blocked, instant failure)
// with retry config. After retries are exhausted the fallback fires.

#[tokio::test]
async fn test_retry_with_backoff() {
    let yaml = r#"
name: retry-test
settings:
  timeout_seconds: 30
nodes:
  - id: start
    type: transform
    config:
      expression: "1"
  - id: retry-node
    type: http
    config:
      url: http://localhost:1/will-fail
      method: GET
    retry:
      max_attempts: 2
      delay_seconds: 1
      backoff: exponential
    on_error:
      action: fallback
      fallback_value: "retries-exhausted"
    depends_on: [start]
  - id: after-retry
    type: transform
    config:
      expression: '"done"'
    depends_on: [retry-node]
"#;

    let start = std::time::Instant::now();
    let exec = run_workflow_yaml(yaml).await;
    let elapsed = start.elapsed();

    assert_eq!(
        exec.status,
        ExecutionStatus::Completed,
        "workflow should complete via fallback after retries: {:?}",
        exec.error
    );

    assert!(
        elapsed.as_secs() >= 1,
        "should have waited for retry delay, elapsed: {:?}",
        elapsed
    );
}
