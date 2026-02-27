# Human-in-the-Loop Node Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `approval` node type that pauses workflow execution until approved/rejected via MCP tools, with configurable timeout and default action.

**Architecture:** New `ApprovalNode` implementing the `Node` trait stores an approval request in SQLite, then signals the executor to pause. Two MCP tools (`r8r_list_approvals`, `r8r_approve`) let agents manage approvals. A background task handles timeouts.

**Tech Stack:** Rust, async-trait, rusqlite, rmcp (MCP), tokio (background tasks), serde, chrono

---

### Task 1: Add `ApprovalRequest` model and storage schema

**Files:**
- Modify: `src/storage/models.rs` (add struct after Checkpoint)
- Modify: `src/storage/sqlite.rs` (add CREATE TABLE + CRUD methods)

**Step 1: Add `ApprovalRequest` struct to models.rs**

After the `Checkpoint` struct (~line 176), add:

```rust
/// A human-in-the-loop approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub execution_id: String,
    pub node_id: String,
    pub workflow_name: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub decision_comment: Option<String>,
    pub decided_by: Option<String>,
    pub context_data: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub decided_at: Option<DateTime<Utc>>,
}
```

**Step 2: Add CREATE TABLE to `init_schema_sync` in sqlite.rs**

After the checkpoints table creation (~line 159), add:

```sql
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
    created_at TEXT NOT NULL,
    expires_at TEXT,
    decided_at TEXT,
    FOREIGN KEY (execution_id) REFERENCES executions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_approval_requests_status ON approval_requests(status);
```

**Step 3: Add storage methods to SqliteStorage in sqlite.rs**

After the checkpoint methods (~line 1108), add these methods:

```rust
pub async fn save_approval_request(&self, request: &ApprovalRequest) -> Result<()> {
    let conn = self.conn.lock().await;
    let context_json = request
        .context_data
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default());
    conn.execute(
        "INSERT INTO approval_requests (id, execution_id, node_id, workflow_name, title, description, status, decision_comment, decided_by, context_data, created_at, expires_at, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            decision_comment = excluded.decision_comment,
            decided_by = excluded.decided_by,
            decided_at = excluded.decided_at",
        rusqlite::params![
            request.id,
            request.execution_id,
            request.node_id,
            request.workflow_name,
            request.title,
            request.description,
            request.status,
            request.decision_comment,
            request.decided_by,
            context_json,
            request.created_at.to_rfc3339(),
            request.expires_at.map(|t| t.to_rfc3339()),
            request.decided_at.map(|t| t.to_rfc3339()),
        ],
    )?;
    Ok(())
}

pub async fn get_approval_request(&self, id: &str) -> Result<Option<ApprovalRequest>> {
    let conn = self.conn.lock().await;
    let mut stmt = conn.prepare(
        "SELECT id, execution_id, node_id, workflow_name, title, description, status, decision_comment, decided_by, context_data, created_at, expires_at, decided_at
         FROM approval_requests WHERE id = ?1",
    )?;
    let result = stmt
        .query_row(rusqlite::params![id], |row| {
            Ok(ApprovalRequest {
                id: row.get(0)?,
                execution_id: row.get(1)?,
                node_id: row.get(2)?,
                workflow_name: row.get(3)?,
                title: row.get(4)?,
                description: row.get(5)?,
                status: row.get(6)?,
                decision_comment: row.get(7)?,
                decided_by: row.get(8)?,
                context_data: row
                    .get::<_, Option<String>>(9)?
                    .and_then(|s| serde_json::from_str(&s).ok()),
                created_at: row
                    .get::<_, String>(10)?
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now()),
                expires_at: row
                    .get::<_, Option<String>>(11)?
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
                decided_at: row
                    .get::<_, Option<String>>(12)?
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            })
        })
        .optional()?;
    Ok(result)
}

pub async fn list_approval_requests(&self, status_filter: Option<&str>) -> Result<Vec<ApprovalRequest>> {
    let conn = self.conn.lock().await;
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
        Some(status) => (
            "SELECT id, execution_id, node_id, workflow_name, title, description, status, decision_comment, decided_by, context_data, created_at, expires_at, decided_at
             FROM approval_requests WHERE status = ?1 ORDER BY created_at DESC",
            vec![Box::new(status.to_string())],
        ),
        None => (
            "SELECT id, execution_id, node_id, workflow_name, title, description, status, decision_comment, decided_by, context_data, created_at, expires_at, decided_at
             FROM approval_requests ORDER BY created_at DESC",
            vec![],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
        Ok(ApprovalRequest {
            id: row.get(0)?,
            execution_id: row.get(1)?,
            node_id: row.get(2)?,
            workflow_name: row.get(3)?,
            title: row.get(4)?,
            description: row.get(5)?,
            status: row.get(6)?,
            decision_comment: row.get(7)?,
            decided_by: row.get(8)?,
            context_data: row
                .get::<_, Option<String>>(9)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row
                .get::<_, String>(10)?
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now()),
            expires_at: row
                .get::<_, Option<String>>(11)?
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            decided_at: row
                .get::<_, Option<String>>(12)?
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

pub async fn list_expired_approvals(&self) -> Result<Vec<ApprovalRequest>> {
    let conn = self.conn.lock().await;
    let now = Utc::now().to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT id, execution_id, node_id, workflow_name, title, description, status, decision_comment, decided_by, context_data, created_at, expires_at, decided_at
         FROM approval_requests WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![now], |row| {
        Ok(ApprovalRequest {
            id: row.get(0)?,
            execution_id: row.get(1)?,
            node_id: row.get(2)?,
            workflow_name: row.get(3)?,
            title: row.get(4)?,
            description: row.get(5)?,
            status: row.get(6)?,
            decision_comment: row.get(7)?,
            decided_by: row.get(8)?,
            context_data: row
                .get::<_, Option<String>>(9)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row
                .get::<_, String>(10)?
                .parse::<DateTime<Utc>>()
                .unwrap_or_else(|_| Utc::now()),
            expires_at: row
                .get::<_, Option<String>>(11)?
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
            decided_at: row
                .get::<_, Option<String>>(12)?
                .and_then(|s| s.parse::<DateTime<Utc>>().ok()),
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
```

**Step 4: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All existing tests pass (new code is additive).

**Step 5: Commit**

```bash
git add src/storage/models.rs src/storage/sqlite.rs
git commit -m "feat: add ApprovalRequest model and storage schema"
```

---

### Task 2: Create ApprovalNode

**Files:**
- Create: `src/nodes/approval.rs`
- Modify: `src/nodes/mod.rs` (add module declaration)
- Modify: `src/nodes/registry.rs` (register the node)

**Step 1: Create `src/nodes/approval.rs`**

```rust
//! Approval node — pauses workflow for human/agent approval.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use crate::error::Result;
use crate::storage::models::ApprovalRequest;

use super::types::{Node, NodeContext, NodeResult};

/// Node that pauses execution until a human or agent approves.
pub struct ApprovalNode;

impl ApprovalNode {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct ApprovalConfig {
    /// Short title for the approval request.
    title: String,
    /// Detailed description (supports template expressions).
    #[serde(default)]
    description: Option<String>,
    /// Timeout in seconds. If set, requires default_action.
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Action when timeout expires: "approve" or "reject".
    #[serde(default)]
    default_action: Option<String>,
}

#[async_trait]
impl Node for ApprovalNode {
    fn node_type(&self) -> &str {
        "approval"
    }

    fn description(&self) -> &str {
        "Pause execution for human or agent approval"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: ApprovalConfig = serde_json::from_value(config.clone()).map_err(|e| {
            crate::error::Error::Node(format!("Invalid approval config: {}", e))
        })?;

        // Validate: if timeout_seconds set, default_action must be set
        if config.timeout_seconds.is_some() && config.default_action.is_none() {
            return Err(crate::error::Error::Node(
                "timeout_seconds requires default_action to be set (\"approve\" or \"reject\")"
                    .to_string(),
            ));
        }

        if let Some(ref action) = config.default_action {
            if action != "approve" && action != "reject" {
                return Err(crate::error::Error::Node(format!(
                    "default_action must be \"approve\" or \"reject\", got \"{}\"",
                    action
                )));
            }
        }

        let approval_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = config
            .timeout_seconds
            .map(|s| now + chrono::Duration::seconds(s as i64));

        // Create the approval request
        let request = ApprovalRequest {
            id: approval_id.clone(),
            execution_id: ctx.execution_id.to_string(),
            node_id: String::new(), // Will be set by executor
            workflow_name: ctx.workflow_name.to_string(),
            title: config.title.clone(),
            description: config.description.clone(),
            status: "pending".to_string(),
            decision_comment: None,
            decided_by: None,
            context_data: Some(ctx.input.clone()),
            created_at: now,
            expires_at,
            decided_at: None,
        };

        // Save to storage if available
        if let Some(ref storage) = ctx.storage {
            storage.save_approval_request(&request).await?;
            info!(
                "Created approval request {} for workflow '{}'",
                approval_id, ctx.workflow_name
            );
        }

        // Return result that signals executor to pause
        Ok(NodeResult::new(json!({
            "approval_id": approval_id,
            "status": "pending",
            "title": config.title,
            "description": config.description,
            "expires_at": expires_at.map(|t| t.to_rfc3339()),
            "default_action": config.default_action,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::types::NodeContext;

    #[tokio::test]
    async fn test_approval_node_basic() {
        let node = ApprovalNode::new();
        assert_eq!(node.node_type(), "approval");

        let config = json!({
            "title": "Test approval",
            "description": "Please approve this"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await.unwrap();
        let data = result.data;
        assert_eq!(data["status"], "pending");
        assert!(data["approval_id"].is_string());
        assert_eq!(data["title"], "Test approval");
    }

    #[tokio::test]
    async fn test_approval_node_with_timeout() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Timed approval",
            "timeout_seconds": 3600,
            "default_action": "reject"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await.unwrap();
        let data = result.data;
        assert_eq!(data["default_action"], "reject");
        assert!(data["expires_at"].is_string());
    }

    #[tokio::test]
    async fn test_approval_node_timeout_requires_default_action() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Bad config",
            "timeout_seconds": 3600
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_approval_node_invalid_default_action() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Bad action",
            "timeout_seconds": 3600,
            "default_action": "maybe"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
```

**Step 2: Add module declaration to `src/nodes/mod.rs`**

Add `mod approval;` after other module declarations and `pub use approval::ApprovalNode;` in the exports section.

**Step 3: Register in `src/nodes/registry.rs`**

Add `registry.register(Arc::new(ApprovalNode::new()));` after the last node registration (~line 54).

Update the tests:
- In `test_registry_default_nodes`: add `assert!(registry.has("approval"));`
- In `test_registry_list`: update the expected count and add `assert!(types.contains(&"approval".to_string()));`

**Step 4: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass including the 4 new approval node tests.

**Step 5: Commit**

```bash
git add src/nodes/approval.rs src/nodes/mod.rs src/nodes/registry.rs
git commit -m "feat: add ApprovalNode type with validation and tests"
```

---

### Task 3: Add executor pause-on-approval logic

**Files:**
- Modify: `src/engine/executor.rs` (add approval node pause detection in main loop)

**Step 1: Find the node execution completion point in the main loop**

In `executor.rs`, after a node's output is stored and the node execution is saved, add a check: if the node type is "approval" and the output contains `"status": "pending"`, pause the execution.

Look for where `last_output` is set after node execution (around lines 640-660). After the node output is committed, add:

```rust
// Check if this is an approval node requesting pause
if node.node_type == "approval" {
    if let Some(status) = last_output.get("status").and_then(|v| v.as_str()) {
        if status == "pending" {
            info!(
                "Approval node '{}' requires approval, pausing execution {}",
                node_id, execution_id
            );

            // Save checkpoint before pausing
            self.save_checkpoint(
                &execution_id,
                node_id,
                &ctx.node_outputs,
                checkpoints_enabled,
            )
            .await?;

            // Update execution status to Paused
            execution.status = ExecutionStatus::Paused;
            execution.finished_at = Some(Utc::now());
            self.storage.save_execution(&execution).await?;

            // Emit monitoring event
            emit_execution_finished(self.monitor.as_ref(), &execution);

            // Unregister pause signal
            if let Some(ref registry) = self.pause_registry {
                registry.unregister(&execution_id).await;
            }

            return Ok(execution);
        }
    }
}
```

**Important:** Unlike the existing pause mechanism which returns an `Err`, this should return `Ok(execution)` with `status: Paused` so callers can inspect the result.

**Step 2: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass.

**Step 3: Commit**

```bash
git add src/engine/executor.rs
git commit -m "feat: executor pauses on approval node pending status"
```

---

### Task 4: Add MCP tools (r8r_list_approvals + r8r_approve)

**Files:**
- Modify: `src/mcp/tools.rs` (add params structs + tool methods + tests)

**Step 1: Add param structs** (after TestParams)

```rust
/// Parameters for listing approval requests
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListApprovalsParams {
    /// Filter by status: "pending", "approved", "rejected", "expired". Default: "pending"
    #[serde(default)]
    pub status: Option<String>,
}

/// Parameters for approving or rejecting an approval request
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApproveParams {
    /// The approval request ID
    pub approval_id: String,
    /// Decision: "approve" or "reject"
    pub decision: String,
    /// Optional comment explaining the decision
    #[serde(default)]
    pub comment: Option<String>,
}
```

**Step 2: Add tool methods** (inside `#[tool(tool_box)] impl R8rService`, after `r8r_test`)

```rust
    /// List approval requests, optionally filtered by status.
    /// Default: shows pending approvals that need action.
    #[tool(description = "List workflow approval requests. Default: pending approvals needing action.")]
    pub async fn r8r_list_approvals(
        &self,
        #[tool(aggr)] params: ListApprovalsParams,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.as_deref().unwrap_or("pending");
        let approvals = match self.storage.list_approval_requests(Some(status)).await {
            Ok(list) => list,
            Err(e) => {
                return Ok(CallToolResult::error(format!(
                    "Failed to list approvals: {}",
                    e
                )));
            }
        };

        let items: Vec<Value> = approvals
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "workflow_name": a.workflow_name,
                    "title": a.title,
                    "description": a.description,
                    "status": a.status,
                    "created_at": a.created_at.to_rfc3339(),
                    "expires_at": a.expires_at.map(|t| t.to_rfc3339()),
                    "execution_id": a.execution_id,
                })
            })
            .collect();

        let result = json!({
            "count": items.len(),
            "approvals": items,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    /// Approve or reject a pending approval request, resuming the paused workflow.
    #[tool(description = "Approve or reject a pending approval. Resumes the paused workflow execution.")]
    pub async fn r8r_approve(
        &self,
        #[tool(aggr)] params: ApproveParams,
    ) -> Result<CallToolResult, McpError> {
        // Validate decision
        if params.decision != "approve" && params.decision != "reject" {
            return Ok(CallToolResult::error(format!(
                "Decision must be \"approve\" or \"reject\", got \"{}\"",
                params.decision
            )));
        }

        // Get the approval request
        let mut approval = match self.storage.get_approval_request(&params.approval_id).await {
            Ok(Some(a)) => a,
            Ok(None) => {
                return Ok(CallToolResult::error(format!(
                    "Approval request not found: {}",
                    params.approval_id
                )));
            }
            Err(e) => {
                return Ok(CallToolResult::error(format!(
                    "Failed to get approval: {}",
                    e
                )));
            }
        };

        if approval.status != "pending" {
            return Ok(CallToolResult::error(format!(
                "Approval {} is already {}, cannot change",
                params.approval_id, approval.status
            )));
        }

        // Update the approval
        let decided_status = if params.decision == "approve" {
            "approved"
        } else {
            "rejected"
        };
        approval.status = decided_status.to_string();
        approval.decision_comment = params.comment.clone();
        approval.decided_by = Some("agent".to_string());
        approval.decided_at = Some(chrono::Utc::now());

        if let Err(e) = self.storage.save_approval_request(&approval).await {
            return Ok(CallToolResult::error(format!(
                "Failed to save approval: {}",
                e
            )));
        }

        // Update the node output in checkpoint to include the decision
        // Then resume the execution
        let execution_id = &approval.execution_id;
        match self
            .executor
            .resume_from_checkpoint(execution_id)
            .await
        {
            Ok(exec) => {
                let result = json!({
                    "approval_id": approval.id,
                    "decision": decided_status,
                    "comment": params.comment,
                    "execution_id": exec.id,
                    "execution_status": exec.status.to_string(),
                    "output": exec.output,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(format!(
                "Approval recorded but failed to resume execution: {}",
                e
            ))),
        }
    }
```

**Step 3: Add tests** (in `mod tests`)

```rust
#[tokio::test]
async fn test_r8r_list_approvals_empty() {
    let service = test_service();
    let params = ListApprovalsParams { status: None };
    let result = service.r8r_list_approvals(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["count"], 0);
    assert_eq!(data["approvals"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_r8r_approve_not_found() {
    let service = test_service();
    let params = ApproveParams {
        approval_id: "nonexistent".to_string(),
        decision: "approve".to_string(),
        comment: None,
    };
    let result = service.r8r_approve(params).await.unwrap();
    assert!(result.is_error.unwrap_or(false));
}

#[tokio::test]
async fn test_r8r_approve_invalid_decision() {
    let service = test_service();
    let params = ApproveParams {
        approval_id: "any".to_string(),
        decision: "maybe".to_string(),
        comment: None,
    };
    let result = service.r8r_approve(params).await.unwrap();
    assert!(result.is_error.unwrap_or(false));
}
```

**Step 4: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add r8r_list_approvals and r8r_approve MCP tools"
```

---

### Task 5: Add integration test for full approval workflow

**Files:**
- Modify: `src/mcp/tools.rs` (add E2E test in `mod tests`)

**Step 1: Write the E2E test**

This test creates a workflow with an approval node, executes it, verifies it pauses, then approves it and verifies it resumes.

```rust
#[tokio::test]
async fn test_r8r_approval_workflow_e2e() {
    let service = test_service();

    // Create a workflow with an approval node
    let yaml = r#"
name: test-approval
description: Tests the approval node
version: 1
nodes:
  - id: prepare
    type: set
    config:
      fields:
        - name: amount
          value: "1500"
  - id: get-approval
    type: approval
    config:
      title: "Approve large order"
      description: "Order exceeds $1000"
    depends_on: [prepare]
  - id: finalize
    type: set
    config:
      fields:
        - name: status
          value: "processed"
        - name: approved
          value: "{{ nodes.get-approval.output.decision }}"
    depends_on: [get-approval]
"#;

    // Save workflow
    save_test_workflow(&service, "test-approval", yaml).await;

    // Execute — should pause at approval node
    let params = ExecuteParams {
        workflow: "test-approval".to_string(),
        input: Some(json!({})),
    };
    let result = service.r8r_execute(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["status"], "paused");

    // List pending approvals
    let list_params = ListApprovalsParams { status: None };
    let list_result = service.r8r_list_approvals(list_params).await.unwrap();
    let list_text = extract_text(&list_result);
    let list_data: Value = serde_json::from_str(list_text).unwrap();
    assert_eq!(list_data["count"], 1);
    let approval_id = list_data["approvals"][0]["id"].as_str().unwrap().to_string();

    // Approve
    let approve_params = ApproveParams {
        approval_id,
        decision: "approve".to_string(),
        comment: Some("Looks good".to_string()),
    };
    let approve_result = service.r8r_approve(approve_params).await.unwrap();
    assert!(!approve_result.is_error.unwrap_or(false));
    let approve_text = extract_text(&approve_result);
    let approve_data: Value = serde_json::from_str(approve_text).unwrap();
    assert_eq!(approve_data["decision"], "approved");
    assert_eq!(approve_data["execution_status"], "completed");
}
```

**Step 2: Run the test**

Run: `cargo test test_r8r_approval_workflow_e2e -- --nocapture 2>&1 | tail -20`
Expected: PASS. If it fails, debug the issue — likely the executor return path for paused executions or the resume flow.

**Step 3: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "test: add E2E approval workflow integration test"
```

---

### Task 6: Update documentation

**Files:**
- Modify: `README.md` (add r8r_list_approvals and r8r_approve to MCP tools table)
- Modify: `docs/NODE_TYPES.md` (add approval node section)

**Step 1: Update README.md MCP tools table**

Add two rows:
```markdown
| `r8r_list_approvals` | List pending approval requests that need action |
| `r8r_approve` | Approve or reject a pending approval, resuming the paused workflow |
```

**Step 2: Update docs/NODE_TYPES.md**

Add an Approval Node section with YAML config example, description of behavior, and how to approve via MCP tools.

**Step 3: Commit**

```bash
git add README.md docs/NODE_TYPES.md
git commit -m "docs: add approval node and MCP tools documentation"
```

---

### Task 7: Final verification

**Step 1:** Run: `cargo test 2>&1 | tail -5` — All tests pass
**Step 2:** Run: `cargo clippy --all-targets --all-features 2>&1 | tail -10` — No warnings
**Step 3:** Run: `cargo fmt --all -- --check` — No formatting issues
**Step 4:** Fix any issues and commit
