# Enterprise Feature Gaps Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the remaining enterprise feature gaps in r8r — unified audit log, REST approval API, DLQ auto-push, backoff cap, fallback node routing, and parallel sibling execution — to meet SEA SME compliance and production-readiness requirements.

**Architecture:** Six independent workstreams that can be implemented in any order. Each adds a focused capability without breaking existing behavior. The unified audit log is the highest priority for SEA SME audit compliance. All changes follow TDD with tests written first.

**Tech Stack:** Rust, SQLite (rusqlite), Axum (REST API), Tokio (async), serde_json, chrono, uuid.

---

## Task 1: Unified Audit Log

**Why:** SEA SMEs need a single queryable table that answers "who did what, when, to which execution." Currently the audit data is scattered across `executions`, `node_executions`, and `approval_requests`.

**Files:**
- Create: `src/storage/audit.rs`
- Modify: `src/storage/mod.rs`
- Modify: `src/storage/sqlite.rs` (add table + methods)
- Modify: `src/engine/executor.rs` (emit audit events)
- Modify: `src/api/mod.rs` (add GET endpoint)
- Modify: `src/nodes/approval.rs` (emit audit on approval creation)
- Modify: `src/triggers/approval_timeout.rs` (emit audit on timeout decision)
- Test: `tests/audit_log_test.rs`

### Step 1: Define the audit log model

Create `src/storage/audit.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub execution_id: Option<String>,
    pub workflow_name: Option<String>,
    pub node_id: Option<String>,
    pub actor: String,            // "system", "agent", "timeout", "api:<ip>", user identity
    pub detail: String,           // human-readable description
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
```

### Step 2: Add the audit table to SQLite schema

In `src/storage/sqlite.rs`, add to `init_schema_sync()` after the existing tables (~line 200):

```sql
CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,
    execution_id TEXT,
    workflow_name TEXT,
    node_id TEXT,
    actor TEXT NOT NULL,
    detail TEXT NOT NULL,
    metadata TEXT
);
CREATE INDEX IF NOT EXISTS idx_audit_log_execution ON audit_log(execution_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_log_event_type ON audit_log(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_log_workflow ON audit_log(workflow_name);
```

### Step 3: Add storage methods

In `src/storage/sqlite.rs`:

```rust
pub async fn save_audit_entry(&self, entry: &AuditEntry) -> Result<()>

pub async fn list_audit_entries(
    &self,
    execution_id: Option<&str>,
    workflow_name: Option<&str>,
    event_type: Option<&str>,
    since: Option<DateTime<Utc>>,
    limit: u32,
    offset: u32,
) -> Result<Vec<AuditEntry>>
```

### Step 4: Write tests

Create `tests/audit_log_test.rs` with:
- `test_save_and_retrieve_audit_entry` — round-trip insert + query
- `test_filter_by_execution_id` — filter returns only matching entries
- `test_filter_by_workflow_name` — filter by workflow
- `test_filter_by_event_type` — filter by event type
- `test_filter_by_timestamp` — since filter
- `test_pagination` — limit + offset
- `test_audit_entry_serialization` — AuditEventType serde round-trip

### Step 5: Emit audit events from executor

In `src/engine/executor.rs`, add a helper:

```rust
async fn emit_audit(
    storage: &SqliteStorage,
    event_type: AuditEventType,
    execution_id: Option<&str>,
    workflow_name: Option<&str>,
    node_id: Option<&str>,
    actor: &str,
    detail: &str,
    metadata: Option<Value>,
) {
    let entry = AuditEntry {
        id: Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        event_type, execution_id: execution_id.map(String::from),
        workflow_name: workflow_name.map(String::from),
        node_id: node_id.map(String::from),
        actor: actor.to_string(),
        detail: detail.to_string(),
        metadata,
    };
    let _ = storage.save_audit_entry(&entry).await;
}
```

Call at these points in `execute_inner()` / `execute_with_checkpoint()`:
- After `storage.save_execution()` (status=Running) → `ExecutionStarted`
- After each `save_node_execution()` success → `NodeCompleted`
- After each node failure (before on_error) → `NodeFailed`
- After node skip → `NodeSkipped`
- When execution paused (approval) → `ExecutionPaused`
- After `update_execution()` with final status → `ExecutionCompleted` / `ExecutionFailed`
- In `resume_from_checkpoint()` → `ExecutionResumed`
- In `pause_execution()` → `ExecutionPaused`

Also emit from:
- `src/nodes/approval.rs` → `ApprovalRequested` after saving the request
- `src/triggers/approval_timeout.rs` → `ApprovalTimedOut` after timeout decision
- `src/mcp/tools.rs` → `ApprovalDecided` after agent approve/reject

### Step 6: Add REST API endpoint

In `src/api/mod.rs`, add route:

```
GET /api/audit?execution_id=&workflow_name=&event_type=&since=&limit=50&offset=0
```

### Step 7: Commit

```
feat: add unified audit log table with filtering and REST API
```

---

## Task 2: REST API for Approval Decisions

**Why:** Currently approvals can only be decided via MCP tools (agent) or auto-timeout. Human users need a REST endpoint to approve/reject from a web UI or curl.

**Files:**
- Modify: `src/api/mod.rs` (add routes)
- Modify: `src/storage/sqlite.rs` (add update method)
- Test: `tests/approval_api_test.rs`

### Step 1: Add approval decision request/response types

In `src/api/mod.rs`:

```rust
#[derive(Deserialize)]
struct ApprovalDecisionRequest {
    decision: String,      // "approve" or "reject"
    comment: Option<String>,
    decided_by: Option<String>,  // identity of the human
}
```

### Step 2: Add routes

```
GET  /api/approvals                         → list_approvals (query: ?status=pending)
GET  /api/approvals/{id}                    → get_approval
POST /api/approvals/{id}/decide             → decide_approval
```

### Step 3: Implement decide_approval handler

The logic mirrors `r8r_approve` in `src/mcp/tools.rs` (line 1097):
1. Validate decision is "approve" or "reject"
2. Load approval request by ID, verify status == "pending"
3. Load latest checkpoint for the execution
4. Inject decision into checkpoint outputs (same as MCP flow)
5. Save updated checkpoint
6. Update approval record: status, decided_by, decided_at, decision_comment
7. Call `executor.resume_from_checkpoint(execution_id)`
8. Emit `ApprovalDecided` audit event
9. Return 200 with the updated approval

### Step 4: Write tests

- `test_list_pending_approvals` — returns only pending
- `test_approve_decision` — approves, resumes execution
- `test_reject_decision` — rejects, resumes execution
- `test_decide_invalid_decision` — 400 for bad decision value
- `test_decide_already_decided` — 409 for non-pending approval
- `test_decide_not_found` — 404

### Step 5: Commit

```
feat: add REST API endpoints for approval decisions
```

---

## Task 3: Automatic DLQ Push on Execution Failure

**Why:** The DLQ storage layer is fully implemented but the executor never pushes failed executions into it. This means failed workflows silently die with no retry queue.

**Files:**
- Modify: `src/engine/executor.rs` (push to DLQ after final failure)
- Modify: `src/storage/dlq.rs` (ensure `init_dlq()` called at startup)
- Modify: `src/main.rs` or `src/lib.rs` (call `init_dlq()` during startup)
- Test: `tests/dlq_auto_push_test.rs`

### Step 1: Call `init_dlq()` at startup

Find where `storage.open()` is called in the server startup path and add `storage.init_dlq().await?` right after. Check both `src/main.rs` (server mode) and `src/repl/mod.rs` (REPL mode).

### Step 2: Add DLQ push to executor

In `src/engine/executor.rs`, after the execution is marked `Failed` in `execute_inner()` (and `execute_with_checkpoint()`), add:

```rust
// After: storage.update_execution(execution_id, Failed, ...).await
if let Err(dlq_err) = self.push_to_dlq(
    &execution_id,
    &workflow.id,
    &workflow.name,
    trigger_type,
    &input,
    error_msg,
    failed_node_id.as_deref(),
    workflow.settings.as_ref(),
).await {
    tracing::warn!("Failed to push to DLQ: {}", dlq_err);
}
```

Add helper method on `Executor`:

```rust
async fn push_to_dlq(
    &self,
    execution_id: &str,
    workflow_id: &str,
    workflow_name: &str,
    trigger_type: &str,
    input: &Value,
    error: &str,
    failed_node_id: Option<&str>,
    settings: Option<&WorkflowSettings>,
) -> Result<()> {
    let max_retries = settings
        .and_then(|s| s.dlq_max_retries)  // new field, defaults to 3
        .unwrap_or(3);
    let entry = NewDlqEntry {
        execution_id, workflow_id, workflow_name,
        trigger_type, input, error,
        failed_node_id, max_retries,
    };
    self.storage.add_to_dlq(entry).await?;
    Ok(())
}
```

### Step 3: Write tests

- `test_failed_execution_creates_dlq_entry` — run a workflow that fails, verify DLQ entry exists
- `test_dlq_entry_has_correct_fields` — verify execution_id, error, failed_node_id populated
- `test_dlq_not_created_on_success` — successful execution has no DLQ entry
- `test_dlq_not_created_on_cancel` — cancelled execution has no DLQ entry

### Step 4: Commit

```
feat: auto-push failed executions to dead letter queue
```

---

## Task 4: Backoff Cap (max_delay_seconds) and Jitter

**Why:** The JSON schema documents `max_delay_seconds` but the runtime ignores it. Exponential backoff without a cap can lead to 60+ minute delays. Jitter prevents thundering herd.

**Files:**
- Modify: `src/workflow/types.rs` (add field to `RetryConfig`)
- Modify: `src/engine/executor.rs` (respect cap + add jitter in `retry_delay()`)
- Modify: `src/workflow/schema.rs` (already in schema, verify)
- Test: `tests/retry_backoff_test.rs` (new or extend existing)

### Step 1: Add `max_delay_seconds` to RetryConfig

In `src/workflow/types.rs`, line ~238:

```rust
pub struct RetryConfig {
    pub max_attempts: u32,
    pub delay_seconds: u64,
    pub backoff: BackoffType,
    #[serde(default)]
    pub max_delay_seconds: Option<u64>,  // cap for exponential/linear backoff
    #[serde(default)]
    pub jitter: bool,                     // add random jitter (±25%)
}
```

### Step 2: Update retry_delay()

In `src/engine/executor.rs`, line ~1869:

```rust
fn retry_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let base = config.delay_seconds;
    let raw = match config.backoff {
        BackoffType::Fixed => base,
        BackoffType::Linear => base.saturating_mul(attempt as u64),
        BackoffType::Exponential => {
            base.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)))
        }
    };
    let capped = match config.max_delay_seconds {
        Some(max) if max > 0 => raw.min(max),
        _ => raw,
    };
    let final_secs = if config.jitter && capped > 0 {
        let jitter_range = capped / 4;  // ±25%
        let offset = (rand_u64_simple() % (jitter_range * 2 + 1)).saturating_sub(jitter_range);
        capped.saturating_add(offset)
    } else {
        capped
    };
    Duration::from_secs(final_secs)
}
```

For jitter without adding a `rand` dependency, use a simple XorShift or derive from `Instant::now().elapsed().subsec_nanos()`.

### Step 3: Write tests

- `test_exponential_backoff_capped` — verify delay never exceeds max_delay_seconds
- `test_linear_backoff_capped` — same for linear
- `test_no_cap_when_none` — verify old behavior when max_delay_seconds is None
- `test_jitter_varies_delay` — verify jitter produces different values (run 10x, check not all equal)
- `test_backoff_cap_deserialization` — verify YAML with max_delay_seconds parses correctly

### Step 4: Commit

```
feat: add max_delay_seconds cap and jitter to retry backoff
```

---

## Task 5: Fallback Node Routing

**Why:** `ErrorConfig.fallback_node` is declared in the struct but never wired in the executor. When a node fails with `action: fallback` and `fallback_node: "cleanup-node"`, the cleanup node should actually run.

**Files:**
- Modify: `src/engine/executor.rs` (wire fallback_node in error recovery path)
- Test: `tests/fallback_node_test.rs`

### Step 1: Update error recovery in executor

In `execute_inner()` and `execute_with_checkpoint()`, after a node fails and `on_error_recovery_output()` returns a value, check if `fallback_node` is set:

```rust
// After: let recovery = on_error_recovery_output(node);
if let Some(fallback_node_id) = node.error.as_ref().and_then(|e| e.fallback_node.as_ref()) {
    if let Some(fallback_node_def) = workflow.nodes.iter().find(|n| &n.id == fallback_node_id) {
        // Execute the fallback node with the failed node's input + error context
        let fallback_input = json!({
            "failed_node_id": node.id,
            "error": error_message,
            "original_input": node_input,
        });
        // Run fallback node (with retry if configured)
        let fallback_result = execute_node_with_retry(...).await;
        match fallback_result {
            Ok(result) => {
                // Use fallback node's output as recovery value
                node_outputs.insert(node.id.clone(), result.output);
            }
            Err(fallback_err) => {
                // Fallback itself failed — use static fallback_value or fail
                if let Some(val) = &node.error.as_ref().and_then(|e| e.fallback_value.clone()) {
                    node_outputs.insert(node.id.clone(), val.clone());
                } else {
                    return Err(fallback_err);
                }
            }
        }
    }
}
```

### Step 2: Write tests

- `test_fallback_node_executes_on_failure` — node A fails, fallback node B runs, B's output used
- `test_fallback_node_not_found_fails_gracefully` — bad fallback_node reference logs error, falls back to fallback_value
- `test_fallback_node_itself_fails` — fallback node also fails, uses fallback_value if set
- `test_no_fallback_node_uses_recovery_output` — existing behavior preserved when fallback_node is None

### Step 3: Commit

```
feat: wire fallback_node execution in error recovery path
```

---

## Task 6: Parallel Sibling Node Execution

**Why:** Currently all nodes execute serially in topological order. Independent sibling nodes (no dependency between them) can safely run in parallel, significantly reducing workflow latency.

**Files:**
- Modify: `src/engine/executor.rs` (group siblings by depth, run groups concurrently)
- Test: `tests/parallel_execution_test.rs`

### Step 1: Add depth-grouped execution

Replace the serial `for node in sorted_nodes` loop with a depth-grouped approach:

```rust
// Group nodes by topological depth
let mut depth_map: HashMap<usize, Vec<&WorkflowNode>> = HashMap::new();
let mut node_depths: HashMap<String, usize> = HashMap::new();
// ... compute depths (same algorithm as TUI's workflow_to_dag_lines)

let max_depth = node_depths.values().max().copied().unwrap_or(0);
for depth in 0..=max_depth {
    let nodes_at_depth = depth_map.get(&depth).unwrap_or(&vec![]);
    if nodes_at_depth.len() == 1 {
        // Single node — execute directly (no overhead)
        execute_single_node(...).await?;
    } else {
        // Multiple independent siblings — execute concurrently
        let mut join_set = tokio::task::JoinSet::new();
        for node in nodes_at_depth {
            join_set.spawn(async move { execute_single_node(...).await });
        }
        // Collect results, fail fast on first error (unless on_error configured)
        while let Some(result) = join_set.join_next().await {
            // merge into node_outputs
        }
    }
}
```

### Step 2: Respect max_concurrency

Use `WorkflowSettings.max_concurrency` to limit parallel node count. If a depth level has more nodes than `max_concurrency`, chunk them:

```rust
let max_concurrent = workflow.settings.as_ref()
    .map(|s| s.max_concurrency)
    .unwrap_or(10);
for chunk in nodes_at_depth.chunks(max_concurrent) {
    // Run chunk concurrently, await all before next chunk
}
```

### Step 3: Handle checkpoint compatibility

When saving checkpoints in parallel mode, each completed node still calls `save_checkpoint()` with the full accumulated `node_outputs`. Use a `Mutex<HashMap<String, Value>>` to safely accumulate outputs from concurrent tasks.

### Step 4: Write tests

- `test_parallel_siblings_run_concurrently` — diamond DAG: A → (B, C) → D. Verify B and C overlap in time
- `test_serial_dependencies_respected` — A → B → C still runs in order
- `test_max_concurrency_respected` — 10 independent nodes with max_concurrency=3, verify max 3 run at once
- `test_parallel_failure_propagates` — one sibling fails, verify correct error handling
- `test_checkpoint_correctness_with_parallel` — verify checkpoints contain all completed node outputs

### Step 5: Commit

```
feat: parallel execution of independent sibling nodes in DAG
```

---

## Task 7: Minor Fixes (store_traces flag, decide_by identity)

**Why:** Small gaps that should be cleaned up for completeness.

**Files:**
- Modify: `src/engine/executor.rs` (gate node trace writes on `store_traces`)
- Modify: `src/mcp/tools.rs` (allow `decided_by` parameter in `r8r_approve`)
- Test: existing tests should cover these

### Step 1: Gate trace writes on store_traces setting

In `executor.rs`, before each `storage.save_node_execution()` call, check:

```rust
let should_store = workflow.settings.as_ref()
    .map(|s| s.store_traces)
    .unwrap_or(true);
if should_store {
    storage.save_node_execution(&node_execution).await?;
}
```

### Step 2: Add decided_by to r8r_approve tool

In `src/mcp/tools.rs`, the `r8r_approve` tool definition should accept an optional `decided_by` parameter instead of hardcoding `"agent"`.

### Step 3: Commit

```
fix: respect store_traces setting, allow decided_by in approval tool
```

---

## Priority Order

| Priority | Task | Effort | Impact |
|----------|------|--------|--------|
| 1 (highest) | Task 1: Unified Audit Log | Medium | Critical for SEA SME compliance |
| 2 | Task 2: REST Approval API | Small | Enables human-in-the-loop without MCP |
| 3 | Task 3: DLQ Auto-Push | Small | Wires up existing code, high value |
| 4 | Task 4: Backoff Cap + Jitter | Small | Prevents runaway retry delays |
| 5 | Task 5: Fallback Node Routing | Medium | Completes error handling story |
| 6 | Task 6: Parallel Siblings | Large | Performance optimization |
| 7 | Task 7: Minor Fixes | Small | Cleanup |

---

## Dependencies Between Tasks

```
Task 1 (Audit Log) ← independent, do first
Task 2 (Approval API) ← independent, depends on Task 1 for audit events
Task 3 (DLQ Auto-Push) ← independent
Task 4 (Backoff Cap) ← independent
Task 5 (Fallback Node) ← independent
Task 6 (Parallel Exec) ← independent, but most complex
Task 7 (Minor Fixes) ← independent, can be done anytime
```

Tasks 1-5 and 7 can be done in any order. Task 6 is the most complex and can be deferred.
