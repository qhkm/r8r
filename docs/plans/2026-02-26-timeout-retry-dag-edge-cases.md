# Timeout, Retry Defaults & DAG Edge Case Tests — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix retry delay default (60s→1s), fix dead hello-world example, add YAML examples for DAG edge cases and error handling, and add comprehensive Rust integration tests.

**Architecture:** Pure additive changes. Fix two defaults, add two example YAML files, and add one integration test file with 8 tests. Tests use inline YAML + `SqliteStorage::open_in_memory()` + `Executor` to run real workflow executions.

**Tech Stack:** Rust, tokio, serde_yaml, r8r crate internals (`workflow::parse_workflow`, `engine::Executor`, `nodes::NodeRegistry`, `storage::SqliteStorage`)

---

### Task 1: Fix retry delay default

**Files:**
- Modify: `src/workflow/types.rs:253-255`

**Step 1: Change default_delay_seconds from 60 to 1**

In `src/workflow/types.rs`, change:

```rust
fn default_delay_seconds() -> u64 {
    60
}
```

to:

```rust
fn default_delay_seconds() -> u64 {
    1
}
```

**Step 2: Run existing tests to verify nothing breaks**

Run: `cargo test --lib workflow`
Expected: All pass

**Step 3: Commit**

```bash
git add src/workflow/types.rs
git commit -m "fix: change default retry delay from 60s to 1s"
```

---

### Task 2: Fix hello-world example

**Files:**
- Modify: `examples/hello-world.yaml`

**Step 1: Replace dead API and add timeout**

Replace the full contents of `examples/hello-world.yaml` with:

```yaml
# Hello World - Simple workflow example
name: hello-world
description: A simple workflow that demonstrates basic r8r features
version: 1

triggers:
  - type: manual

nodes:
  - id: greet
    type: transform
    config:
      expression: '"Hello, r8r!"'

  - id: fetch-data
    type: http
    config:
      url: https://httpbin.org/get
      method: GET
      timeout_seconds: 10
    depends_on: [greet]

  - id: format-output
    type: transform
    config:
      expression: |
        let result = `Greeting: ${input}\nData fetched successfully`;
        result
    depends_on: [fetch-data]
```

**Step 2: Validate the workflow**

Run: `cargo run --bin r8r -- workflows validate examples/hello-world.yaml`
Expected: `Workflow 'hello-world' is valid`

**Step 3: Commit**

```bash
git add examples/hello-world.yaml
git commit -m "fix: replace dead api.quotable.io with httpbin.org in hello-world example"
```

---

### Task 3: Add DAG edge cases example workflow

**Files:**
- Create: `examples/dag-edge-cases.yaml`

**Step 1: Create the example file**

```yaml
# DAG Edge Cases - Tests complex dependency patterns
name: dag-edge-cases
description: Exercises diamond, fan-out/fan-in, conditional skip, and disconnected nodes
version: 1

triggers:
  - type: manual

nodes:
  # === Diamond Dependency: A -> B, A -> C, B+C -> D ===
  - id: diamond-root
    type: transform
    config:
      expression: '"diamond-start"'

  - id: diamond-left
    type: transform
    config:
      expression: '`left: ${input}`'
    depends_on: [diamond-root]

  - id: diamond-right
    type: transform
    config:
      expression: '`right: ${input}`'
    depends_on: [diamond-root]

  - id: diamond-join
    type: transform
    config:
      expression: |
        let left = nodes.get("diamond-left");
        let right = nodes.get("diamond-right");
        `joined: ${left} + ${right}`
    depends_on: [diamond-left, diamond-right]

  # === Fan-out: one node feeds 3 parallel branches ===
  - id: fan-source
    type: transform
    config:
      expression: '"fan-data"'
    depends_on: [diamond-join]

  - id: fan-branch-1
    type: transform
    config:
      expression: '`branch1: ${input}`'
    depends_on: [fan-source]

  - id: fan-branch-2
    type: transform
    config:
      expression: '`branch2: ${input}`'
    depends_on: [fan-source]

  - id: fan-branch-3
    type: transform
    config:
      expression: '`branch3: ${input}`'
    depends_on: [fan-source]

  # === Fan-in: all branches merge ===
  - id: fan-merge
    type: transform
    config:
      expression: |
        let b1 = nodes.get("fan-branch-1");
        let b2 = nodes.get("fan-branch-2");
        let b3 = nodes.get("fan-branch-3");
        `merged: ${b1} | ${b2} | ${b3}`
    depends_on: [fan-branch-1, fan-branch-2, fan-branch-3]

  # === Conditional skip: node skipped when condition is false ===
  - id: always-runs
    type: transform
    config:
      expression: '"always"'
    depends_on: [fan-merge]

  - id: skipped-node
    type: transform
    condition: "false"
    config:
      expression: '"this should be skipped"'
    depends_on: [always-runs]

  - id: final-output
    type: transform
    config:
      expression: '"dag-edge-cases complete"'
    depends_on: [always-runs]
```

**Step 2: Validate**

Run: `cargo run --bin r8r -- workflows validate examples/dag-edge-cases.yaml`
Expected: `Workflow 'dag-edge-cases' is valid`

**Step 3: Run the workflow**

Run: `cargo run --bin r8r -- workflows create examples/dag-edge-cases.yaml && cargo run --bin r8r -- workflows run dag-edge-cases`
Expected: Status: completed

**Step 4: Commit**

```bash
git add examples/dag-edge-cases.yaml
git commit -m "feat: add DAG edge cases example (diamond, fan-out/fan-in, conditional skip)"
```

---

### Task 4: Add error handling example workflow

**Files:**
- Create: `examples/error-handling.yaml`

**Step 1: Create the example file**

```yaml
# Error Handling - Tests retry, timeout, fallback, and continue-on-error
name: error-handling
description: Exercises error recovery patterns
version: 1

settings:
  timeout_seconds: 60

triggers:
  - type: manual

nodes:
  # Normal start node
  - id: start
    type: transform
    config:
      expression: '"workflow started"'

  # on_error continue: failing node, workflow proceeds with null
  - id: fail-continue
    type: http
    config:
      url: https://httpbin.org/status/500
      method: GET
      timeout_seconds: 5
    on_error:
      continue: true
    depends_on: [start]

  # on_error fallback: failing node replaced with fallback value
  - id: fail-fallback
    type: http
    config:
      url: https://httpbin.org/status/503
      method: GET
      timeout_seconds: 5
    on_error:
      action: fallback
      fallback_value:
        status: "fallback"
        message: "used fallback due to service error"
    depends_on: [start]

  # Retry with exponential backoff (will fail all 3 attempts on 500)
  - id: retry-node
    type: http
    config:
      url: https://httpbin.org/status/500
      method: GET
      timeout_seconds: 5
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: exponential
    on_error:
      action: fallback
      fallback_value:
        status: "retries-exhausted"
    depends_on: [start]

  # Merge results from error-handled branches
  - id: merge-results
    type: transform
    config:
      expression: |
        let continued = nodes.get("fail-continue");
        let fallback = nodes.get("fail-fallback");
        let retried = nodes.get("retry-node");
        `continue=${continued}, fallback=${fallback}, retried=${retried}`
    depends_on: [fail-continue, fail-fallback, retry-node]

  - id: final
    type: transform
    config:
      expression: '"error-handling workflow complete"'
    depends_on: [merge-results]
```

**Step 2: Validate**

Run: `cargo run --bin r8r -- workflows validate examples/error-handling.yaml`
Expected: `Workflow 'error-handling' is valid`

**Step 3: Commit**

```bash
git add examples/error-handling.yaml
git commit -m "feat: add error handling example (retry, timeout, fallback, continue-on-error)"
```

---

### Task 5: Add integration tests — helpers and diamond DAG test

**Files:**
- Create: `tests/dag_edge_cases.rs`

**Step 1: Create the integration test file with helper and first test**

```rust
//! Integration tests for DAG edge cases, error handling, retry, and timeout.
//!
//! These tests use inline YAML, in-memory SQLite, and the real Executor to
//! verify end-to-end behavior.

use r8r::engine::Executor;
use r8r::nodes::NodeRegistry;
use r8r::storage::{ExecutionStatus, SqliteStorage};
use r8r::workflow::parse_workflow;
use serde_json::json;

/// Helper: parse YAML, build executor, run workflow, return execution.
async fn run_workflow_yaml(yaml: &str) -> r8r::storage::Execution {
    let workflow = parse_workflow(yaml).expect("YAML should parse");
    let storage = SqliteStorage::open_in_memory().expect("storage should open");
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
    let yaml = r#"
name: diamond-test
nodes:
  - id: root
    type: transform
    config:
      expression: '"root-value"'
  - id: left
    type: transform
    config:
      expression: '`left: ${input}`'
    depends_on: [root]
  - id: right
    type: transform
    config:
      expression: '`right: ${input}`'
    depends_on: [root]
  - id: join
    type: transform
    config:
      expression: |
        let l = nodes.get("left");
        let r = nodes.get("right");
        `joined: ${l} + ${r}`
    depends_on: [left, right]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
    // The final output should contain both left and right
    let output = exec.output.expect("should have output");
    let s = output.as_str().unwrap_or("");
    assert!(s.contains("left"), "output should contain left branch: {}", s);
    assert!(s.contains("right"), "output should contain right branch: {}", s);
}
```

**Step 2: Run the test**

Run: `cargo test --test dag_edge_cases test_diamond_dag -- --nocapture`
Expected: PASS

**Step 3: Commit**

```bash
git add tests/dag_edge_cases.rs
git commit -m "test: add integration test for diamond DAG execution"
```

---

### Task 6: Add fan-out/fan-in and disconnected node tests

**Files:**
- Modify: `tests/dag_edge_cases.rs`

**Step 1: Add fan-out/fan-in test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 2: Fan-out / Fan-in ───────────────────────────────────────

#[tokio::test]
async fn test_fan_out_fan_in() {
    let yaml = r#"
name: fan-test
nodes:
  - id: source
    type: transform
    config:
      expression: '"data"'
  - id: branch-a
    type: transform
    config:
      expression: '`a: ${input}`'
    depends_on: [source]
  - id: branch-b
    type: transform
    config:
      expression: '`b: ${input}`'
    depends_on: [source]
  - id: branch-c
    type: transform
    config:
      expression: '`c: ${input}`'
    depends_on: [source]
  - id: merge
    type: transform
    config:
      expression: |
        let a = nodes.get("branch-a");
        let b = nodes.get("branch-b");
        let c = nodes.get("branch-c");
        `merged: ${a} | ${b} | ${c}`
    depends_on: [branch-a, branch-b, branch-c]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
    let output = exec.output.expect("should have output");
    let s = output.as_str().unwrap_or("");
    assert!(s.contains("a:"), "output should contain branch-a: {}", s);
    assert!(s.contains("b:"), "output should contain branch-b: {}", s);
    assert!(s.contains("c:"), "output should contain branch-c: {}", s);
}
```

**Step 2: Add disconnected node test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 3: Disconnected node (no deps, nothing depends on it) ─────

#[tokio::test]
async fn test_disconnected_node() {
    let yaml = r#"
name: disconnected-test
nodes:
  - id: main-chain-1
    type: transform
    config:
      expression: '"chain-start"'
  - id: main-chain-2
    type: transform
    config:
      expression: '`chain: ${input}`'
    depends_on: [main-chain-1]
  - id: island
    type: transform
    config:
      expression: '"island-node"'
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(exec.status, ExecutionStatus::Completed);
    // Workflow should complete — island node also executes
}
```

**Step 3: Run tests**

Run: `cargo test --test dag_edge_cases -- --nocapture`
Expected: All 3 tests pass

**Step 4: Commit**

```bash
git add tests/dag_edge_cases.rs
git commit -m "test: add fan-out/fan-in and disconnected node integration tests"
```

---

### Task 7: Add cycle detection test

**Files:**
- Modify: `tests/dag_edge_cases.rs`

**Step 1: Add cycle detection test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 4: Cycle detection ────────────────────────────────────────

#[test]
fn test_cycle_detection() {
    let yaml = r#"
name: cycle-test
nodes:
  - id: node-a
    type: transform
    config:
      expression: '"a"'
    depends_on: [node-b]
  - id: node-b
    type: transform
    config:
      expression: '"b"'
    depends_on: [node-a]
"#;

    let result = validate_yaml(yaml);
    assert!(result.is_err(), "circular dependency should be rejected");
    let err = result.unwrap_err().to_string();
    assert!(
        err.to_lowercase().contains("circular"),
        "error should mention circular: {}",
        err
    );
}
```

**Step 2: Run test**

Run: `cargo test --test dag_edge_cases test_cycle_detection -- --nocapture`
Expected: PASS

**Step 3: Commit**

```bash
git add tests/dag_edge_cases.rs
git commit -m "test: add cycle detection integration test"
```

---

### Task 8: Add on_error continue and fallback tests

**Files:**
- Modify: `tests/dag_edge_cases.rs`

**Step 1: Add on_error continue test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 5: on_error continue ──────────────────────────────────────

#[tokio::test]
async fn test_on_error_continue() {
    let yaml = r#"
name: error-continue-test
settings:
  timeout_seconds: 30
nodes:
  - id: start
    type: transform
    config:
      expression: '"ok"'
  - id: fail-node
    type: http
    config:
      url: https://httpbin.org/status/500
      method: GET
      timeout_seconds: 10
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
```

**Step 2: Add on_error fallback test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 6: on_error fallback ──────────────────────────────────────

#[tokio::test]
async fn test_on_error_fallback() {
    let yaml = r#"
name: error-fallback-test
settings:
  timeout_seconds: 30
nodes:
  - id: start
    type: transform
    config:
      expression: '"ok"'
  - id: fail-node
    type: http
    config:
      url: https://httpbin.org/status/500
      method: GET
      timeout_seconds: 10
    on_error:
      action: fallback
      fallback_value:
        status: "recovered"
        data: "fallback-data"
    depends_on: [start]
  - id: use-fallback
    type: transform
    config:
      expression: '`got: ${input}`'
    depends_on: [fail-node]
"#;

    let exec = run_workflow_yaml(yaml).await;
    assert_eq!(
        exec.status,
        ExecutionStatus::Completed,
        "workflow should complete with fallback: {:?}",
        exec.error
    );
    let output = exec.output.expect("should have output");
    let s = output.as_str().unwrap_or(&output.to_string());
    assert!(
        s.contains("fallback") || s.contains("recovered"),
        "output should reference fallback data: {}",
        s
    );
}
```

**Step 3: Run tests**

Run: `cargo test --test dag_edge_cases -- --nocapture`
Expected: All 6 tests pass (HTTP tests need network — they hit httpbin.org)

**Step 4: Commit**

```bash
git add tests/dag_edge_cases.rs
git commit -m "test: add on_error continue and fallback integration tests"
```

---

### Task 9: Add node timeout and retry tests

**Files:**
- Modify: `tests/dag_edge_cases.rs`

**Step 1: Add node timeout test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 7: Node-level timeout ─────────────────────────────────────

#[tokio::test]
async fn test_node_timeout() {
    // Use httpbin.org/delay to force a slow response that exceeds the node timeout
    let yaml = r#"
name: timeout-test
settings:
  timeout_seconds: 30
nodes:
  - id: start
    type: transform
    config:
      expression: '"ok"'
  - id: slow-node
    type: http
    config:
      url: https://httpbin.org/delay/10
      method: GET
      timeout_seconds: 2
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
```

**Step 2: Add retry test**

Append to `tests/dag_edge_cases.rs`:

```rust
// ─── Test 8: Retry with exponential backoff ─────────────────────────

#[tokio::test]
async fn test_retry_with_backoff() {
    // This node will fail all 2 attempts (500 from httpbin), then use fallback
    let yaml = r#"
name: retry-test
settings:
  timeout_seconds: 60
nodes:
  - id: start
    type: transform
    config:
      expression: '"ok"'
  - id: retry-node
    type: http
    config:
      url: https://httpbin.org/status/500
      method: GET
      timeout_seconds: 10
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
      expression: '`result: ${input}`'
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

    // With 2 attempts and exponential backoff from 1s, expect at least ~1s of delay
    assert!(
        elapsed.as_secs() >= 1,
        "should have waited for retry delay, elapsed: {:?}",
        elapsed
    );
}
```

**Step 3: Run all tests**

Run: `cargo test --test dag_edge_cases -- --nocapture`
Expected: All 8 tests pass

**Step 4: Commit**

```bash
git add tests/dag_edge_cases.rs
git commit -m "test: add node timeout and retry with backoff integration tests"
```

---

### Task 10: Final verification

**Step 1: Run all project tests**

Run: `cargo test`
Expected: All tests pass, including existing tests + our 8 new tests

**Step 2: Validate all examples**

Run: `cargo run --bin r8r -- workflows validate examples/hello-world.yaml && cargo run --bin r8r -- workflows validate examples/dag-edge-cases.yaml && cargo run --bin r8r -- workflows validate examples/error-handling.yaml`
Expected: All valid

**Step 3: Run hello-world to confirm fix**

Run: `cargo run --bin r8r -- workflows create examples/hello-world.yaml && cargo run --bin r8r -- workflows run hello-world`
Expected: Completes successfully with httpbin.org data

**Step 4: Final commit if any cleanup needed, otherwise done**
