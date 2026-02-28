# Synchronous Workflow Execution — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make r8r seamless for AI agents and external services by returning workflow results inline via webhook response mode and enhanced MCP tools.

**Architecture:** Enhance existing sync execution paths. Add `response_mode` to webhook trigger config. Add 3 new MCP tools (`r8r_run_and_wait`, `r8r_discover`, `r8r_lint`) and fix error handling on `r8r_execute`. No execution engine changes.

**Tech Stack:** Rust, axum, rmcp, serde, schemars

---

### Task 1: Add `response_mode` to Webhook Trigger Config

**Files:**
- Modify: `src/workflow/types.rs:119-132`
- Test: existing tests in `src/workflow/types.rs` or `tests/` directory

**Step 1: Write the failing test**

Add to the bottom of `src/workflow/types.rs` (or in the existing test module if one exists — check `src/workflow/mod.rs` for test location):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_trigger_with_response_mode() {
        let yaml = r#"
type: webhook
path: /api/summarize
method: POST
response_mode: output
"#;
        let trigger: Trigger = serde_yaml::from_str(yaml).unwrap();
        match trigger {
            Trigger::Webhook { response_mode, .. } => {
                assert_eq!(response_mode.as_deref(), Some("output"));
            }
            _ => panic!("Expected Webhook trigger"),
        }
    }

    #[test]
    fn test_webhook_trigger_default_response_mode() {
        let yaml = r#"
type: webhook
path: /api/test
method: POST
"#;
        let trigger: Trigger = serde_yaml::from_str(yaml).unwrap();
        match trigger {
            Trigger::Webhook { response_mode, .. } => {
                assert!(response_mode.is_none());
            }
            _ => panic!("Expected Webhook trigger"),
        }
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib test_webhook_trigger_with_response_mode -- --nocapture`
Expected: FAIL — `response_mode` field doesn't exist on `Trigger::Webhook`

**Step 3: Add `response_mode` field to Trigger::Webhook**

In `src/workflow/types.rs`, add the field to the `Webhook` variant (around line 119-132):

```rust
    Webhook {
        /// Custom path (default: /webhooks/{workflow_name})
        #[serde(default)]
        path: Option<String>,
        /// HTTP method (default: POST)
        #[serde(default)]
        method: Option<String>,
        /// Response mode: "full" (default) returns execution metadata, "output" returns raw workflow output
        #[serde(default)]
        response_mode: Option<String>,
        /// Debounce configuration for webhook deduplication
        #[serde(default)]
        debounce: Option<DebounceConfig>,
        /// Header-based filter conditions
        #[serde(default)]
        header_filter: Option<Vec<HeaderFilter>>,
    },
```

**Step 4: Fix compilation errors**

The `Trigger::Webhook` pattern match in `src/triggers/webhook.rs:99-104` destructures this enum. Update it to include `response_mode`:

```rust
            if let Trigger::Webhook {
                path,
                method,
                debounce,
                header_filter,
                response_mode,
            } = trigger
```

**Step 5: Run tests to verify they pass**

Run: `cargo test --lib test_webhook_trigger_with_response_mode test_webhook_trigger_default_response_mode -- --nocapture`
Expected: PASS

**Step 6: Commit**

```bash
git add src/workflow/types.rs src/triggers/webhook.rs
git commit -m "feat: add response_mode field to Webhook trigger config"
```

---

### Task 2: Implement Webhook Response Mode Logic

**Files:**
- Modify: `src/triggers/webhook.rs:65-182` (create_webhook_routes) and `src/triggers/webhook.rs:360-450` (execute_webhook_workflow)

**Step 1: Write the failing test**

Add to `src/triggers/webhook.rs` test module:

```rust
    #[test]
    fn test_debounce_with_output_mode_warning() {
        // This tests the validation logic: debounce + response_mode: output is invalid
        // The actual warning happens at route registration time, so we test the logic function
        let has_debounce = true;
        let response_mode = Some("output".to_string());
        let effective_mode = if has_debounce && response_mode.as_deref() == Some("output") {
            "full" // fallback
        } else {
            response_mode.as_deref().unwrap_or("full")
        };
        assert_eq!(effective_mode, "full");
    }
```

**Step 2: Run test to verify it passes (this is a logic test)**

Run: `cargo test --lib test_debounce_with_output_mode -- --nocapture`
Expected: PASS (this validates our logic before wiring it in)

**Step 3: Wire response_mode through to the handler**

In `src/triggers/webhook.rs`, update `create_webhook_routes()` (around line 98-159):

1. Extract `response_mode` from the destructured trigger:
```rust
            if let Trigger::Webhook {
                path,
                method,
                debounce,
                header_filter,
                response_mode,
            } = trigger
            {
```

2. Validate debounce + output mode (after `let has_debounce = debounce.is_some();` around line 134):
```rust
                // Validate: debounce + response_mode: output is invalid
                let effective_response_mode = if has_debounce
                    && response_mode.as_deref() == Some("output")
                {
                    warn!(
                        "Webhook '{}': response_mode 'output' is incompatible with debounce (fire-and-forget). Falling back to 'full'.",
                        workflow.name
                    );
                    None
                } else {
                    response_mode
                };
```

3. Pass `effective_response_mode` into the handler closure. Update the handler closure (around line 138-159) to capture and pass it:
```rust
                let response_mode_clone = effective_response_mode.clone();

                let handler = move |State(state): State<AppState>,
                                    method: Method,
                                    headers: HeaderMap,
                                    body: Bytes| {
                    let workflow_name = workflow_name.clone();
                    let workflow_id = workflow_id.clone();
                    let debounce_config = debounce.clone();
                    let header_filters = compiled_filters.clone();
                    let resp_mode = response_mode_clone.clone();
                    async move {
                        handle_webhook(
                            state,
                            &workflow_name,
                            &workflow_id,
                            method,
                            headers,
                            body,
                            debounce_config,
                            header_filters,
                            resp_mode,
                        )
                        .await
                    }
                };
```

4. Update `handle_webhook()` signature (around line 186-195) to accept `response_mode`:
```rust
async fn handle_webhook(
    state: AppState,
    workflow_name: &str,
    workflow_id: &str,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
    debounce_config: Option<DebounceConfig>,
    header_filters: Option<Vec<CompiledHeaderFilter>>,
    response_mode: Option<String>,
) -> impl IntoResponse {
```

5. Pass `response_mode` to `execute_webhook_workflow()` in the immediate execution path (around line 262):
```rust
    execute_webhook_workflow(
        state,
        workflow_name,
        workflow_id,
        method,
        headers,
        body_value,
        response_mode,
    )
    .await
    .into_response()
```

6. Update `execute_webhook_workflow()` signature (around line 360) and response logic (around line 427-449):
```rust
async fn execute_webhook_workflow(
    state: AppState,
    workflow_name: &str,
    workflow_id: &str,
    method: Method,
    headers: HeaderMap,
    body_value: Value,
    response_mode: Option<String>,
) -> impl IntoResponse {
```

And update the match on execution result:
```rust
    match executor
        .execute(&workflow, workflow_id, "webhook", input)
        .await
    {
        Ok(execution) => {
            if response_mode.as_deref() == Some("output") {
                // Return raw output only
                match execution.output {
                    Some(output) => Json(output).into_response(),
                    None => Json(json!(null)).into_response(),
                }
            } else {
                // Default: return full execution metadata
                let duration_ms = execution
                    .finished_at
                    .map(|f| (f - execution.started_at).num_milliseconds());

                Json(json!({
                    "execution_id": execution.id,
                    "status": execution.status.to_string(),
                    "output": execution.output,
                    "error": execution.error,
                    "duration_ms": duration_ms,
                }))
                .into_response()
            }
        }
        Err(e) => {
            error!("Webhook execution failed for '{}': {}", workflow_name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
```

**Step 4: Run all webhook tests**

Run: `cargo test --lib webhook -- --nocapture`
Expected: All existing tests PASS, no regressions

**Step 5: Run full test suite**

Run: `cargo test`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add src/triggers/webhook.rs
git commit -m "feat: implement webhook response_mode for raw output responses"
```

---

### Task 3: Fix `r8r_execute` Error Handling in MCP

**Files:**
- Modify: `src/mcp/tools.rs:82-151`

**Step 1: Identify the issue**

Currently at line 146-149, when execution fails, `r8r_execute` returns `Err(McpError::internal_error(...))` which is an MCP protocol error. For execution failures (workflow ran but failed), it should return `Ok(CallToolResult::error(...))` with `is_error: true` so the agent gets a readable error, not a protocol error.

**Step 2: Fix the error handling**

Replace lines 146-149:
```rust
            Err(e) => Err(McpError::internal_error(
                "execution_error",
                Some(json!({"error": e.to_string()})),
            )),
```

With:
```rust
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Workflow execution failed: {}",
                e
            ))])),
```

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests PASS (this is a behavior change but no test specifically validates the error path yet)

**Step 4: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "fix: return CallToolResult::error instead of McpError for execution failures"
```

---

### Task 4: Add `r8r_run_and_wait` MCP Tool

**Files:**
- Modify: `src/mcp/tools.rs`

**Step 1: Add parameter type**

After `CreateWorkflowParams` (around line 73-76), add:

```rust
/// Parameters for running a workflow and waiting for the result
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunAndWaitParams {
    /// Name of the workflow to execute
    pub workflow: String,
    /// Input data for the workflow (JSON object)
    #[serde(default)]
    pub input: Option<Value>,
}
```

**Step 2: Add the tool implementation**

After the `r8r_create_workflow` method (around line 495), add inside the `#[tool(tool_box)] impl R8rService` block:

```rust
    /// Execute a workflow and return just the output. Blocks until completion.
    /// Unlike r8r_execute, this returns only the workflow output (not execution metadata).
    /// Use this when you need the result of a workflow directly.
    #[tool(
        description = "Execute a workflow synchronously and return the output. Returns just the workflow result, not execution metadata. Use r8r_execute if you need execution_id and status."
    )]
    pub async fn r8r_run_and_wait(
        &self,
        #[tool(aggr)] params: RunAndWaitParams,
    ) -> Result<CallToolResult, McpError> {
        let input = params.input.unwrap_or(Value::Null);

        // Get workflow from storage
        let stored = match self.storage.get_workflow(&params.workflow).await {
            Ok(Some(wf)) => wf,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Workflow not found: {}",
                    params.workflow
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Storage error: {}",
                    e
                ))]));
            }
        };

        // Parse workflow
        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Invalid workflow definition: {}",
                    e
                ))]));
            }
        };

        // Execute and return just the output
        match self
            .executor
            .execute(&workflow, &stored.id, "mcp", input)
            .await
        {
            Ok(execution) => match execution.output {
                Some(output) => Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&output).unwrap_or_default(),
                )])),
                None => Ok(CallToolResult::success(vec![Content::text("null")])),
            },
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Workflow execution failed: {}",
                e
            ))])),
        }
    }
```

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests PASS

**Step 4: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add r8r_run_and_wait MCP tool for clean output-only responses"
```

---

### Task 5: Add `r8r_discover` MCP Tool

**Files:**
- Modify: `src/mcp/tools.rs`

**Step 1: Add parameter type**

After `RunAndWaitParams`, add:

```rust
/// Parameters for discovering workflow details
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverParams {
    /// Name of the workflow to discover
    pub workflow: String,
}
```

**Step 2: Add the tool implementation**

Inside the `#[tool(tool_box)] impl R8rService` block:

```rust
    /// Discover a workflow's parameters, nodes, and triggers.
    /// Use this before r8r_run_and_wait to understand what input a workflow expects.
    #[tool(
        description = "Discover a workflow's parameters, nodes, and triggers. Returns input schema, parameter definitions, node list, and trigger configuration. Use before r8r_run_and_wait to understand what input a workflow expects."
    )]
    pub async fn r8r_discover(
        &self,
        #[tool(aggr)] params: DiscoverParams,
    ) -> Result<CallToolResult, McpError> {
        let stored = match self.storage.get_workflow(&params.workflow).await {
            Ok(Some(wf)) => wf,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Workflow not found: {}",
                    params.workflow
                ))]));
            }
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Storage error: {}",
                    e
                ))]));
            }
        };

        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Invalid workflow definition: {}",
                    e
                ))]));
            }
        };

        // Build parameter info
        let parameters: Value = workflow
            .inputs
            .iter()
            .map(|(name, def)| {
                (
                    name.clone(),
                    json!({
                        "type": def.input_type,
                        "description": def.description,
                        "required": def.required,
                        "default": def.default,
                    }),
                )
            })
            .collect::<serde_json::Map<String, Value>>()
            .into();

        // Build node summary
        let nodes: Vec<Value> = workflow
            .nodes
            .iter()
            .map(|n| {
                json!({
                    "id": n.id,
                    "type": n.node_type,
                    "depends_on": n.depends_on,
                })
            })
            .collect();

        let result = json!({
            "name": workflow.name,
            "description": workflow.description,
            "version": workflow.version,
            "parameters": parameters,
            "input_schema": workflow.input_schema,
            "nodes": nodes,
            "nodes_count": nodes.len(),
            "triggers": workflow.triggers,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
```

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests PASS

**Step 4: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add r8r_discover MCP tool for parameter and schema introspection"
```

---

### Task 6: Add `r8r_lint` MCP Tool

**Files:**
- Modify: `src/mcp/tools.rs`

**Step 1: Add the tool implementation**

This reuses the existing `ValidateParams` type. Inside the `#[tool(tool_box)] impl R8rService` block:

```rust
    /// Lint a workflow YAML definition with detailed error messages.
    /// Returns a list of issues found, or confirms the workflow is valid.
    /// Use this to check YAML before submitting with r8r_create_workflow.
    #[tool(
        description = "Lint a workflow YAML definition. Returns detailed validation errors with suggestions, or confirms the workflow is valid. Use before r8r_create_workflow to catch issues early."
    )]
    pub async fn r8r_lint(
        &self,
        #[tool(aggr)] params: ValidateParams,
    ) -> Result<CallToolResult, McpError> {
        let mut issues: Vec<Value> = Vec::new();

        // Step 1: Parse YAML
        let workflow = match parse_workflow(&params.yaml) {
            Ok(wf) => wf,
            Err(e) => {
                let error_str = e.to_string();
                let mut issue = json!({
                    "level": "error",
                    "type": "parse",
                    "message": error_str,
                });

                // Add suggestions for common mistakes
                if error_str.contains("unknown field") {
                    if error_str.contains("dependsOn") || error_str.contains("depends_on") {
                        issue["suggestion"] =
                            json!("Use 'depends_on' (snake_case) for node dependencies");
                    }
                    if error_str.contains("nodeType") || error_str.contains("node_type") {
                        issue["suggestion"] =
                            json!("Use 'type' for node type, not 'node_type' or 'nodeType'");
                    }
                }

                issues.push(issue);

                let result = json!({
                    "valid": false,
                    "issues": issues,
                    "issues_count": issues.len(),
                });

                return Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]));
            }
        };

        // Step 2: Validate structure
        if let Err(e) = validate_workflow(&workflow) {
            issues.push(json!({
                "level": "error",
                "type": "validation",
                "message": e.to_string(),
            }));
        }

        // Step 3: Lint warnings (non-blocking)
        if workflow.description.is_empty() {
            issues.push(json!({
                "level": "warning",
                "type": "lint",
                "message": "Workflow has no description. Add a 'description' field for better discoverability.",
            }));
        }

        if workflow.triggers.is_empty() {
            issues.push(json!({
                "level": "warning",
                "type": "lint",
                "message": "Workflow has no triggers. It can only be executed via API or MCP.",
            }));
        }

        // Check for nodes with no depends_on (potential ordering issues)
        let has_deps = workflow.nodes.iter().any(|n| !n.depends_on.is_empty());
        if workflow.nodes.len() > 1 && !has_deps {
            issues.push(json!({
                "level": "warning",
                "type": "lint",
                "message": "Multiple nodes but none have 'depends_on'. Nodes will execute in definition order. Add explicit dependencies for clarity.",
            }));
        }

        let has_errors = issues.iter().any(|i| i["level"] == "error");

        let result = json!({
            "valid": !has_errors,
            "issues": issues,
            "issues_count": issues.len(),
            "name": workflow.name,
            "nodes_count": workflow.nodes.len(),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
```

**Step 2: Run tests**

Run: `cargo test`
Expected: All tests PASS

**Step 3: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add r8r_lint MCP tool with detailed validation and suggestions"
```

---

### Task 7: Update Documentation

**Files:**
- Modify: `docs/NODE_TYPES.md` — add `response_mode` to webhook trigger docs
- Modify: `README.md` — update MCP tools list

**Step 1: Update NODE_TYPES.md**

Find the webhook trigger section and add `response_mode` documentation:

```markdown
#### Response Mode

Control what the webhook HTTP response contains:

```yaml
triggers:
  - type: webhook
    config:
      path: /api/summarize
      method: POST
      response_mode: output  # return workflow output as response body
```

| Mode | Status Code | Response Body |
|------|-------------|---------------|
| `full` (default) | 200 | `{"execution_id": "...", "status": "...", "output": {...}, "duration_ms": ...}` |
| `output` | 200 | Raw workflow output (last node's output) |

**Note:** `response_mode: output` is incompatible with `debounce`. If both are set, `response_mode` falls back to `full` with a warning.
```

**Step 2: Update README.md**

In the MCP section, update the tools list to include the 3 new tools:

```markdown
| Tool | Description |
|------|-------------|
| `r8r_execute` | Execute a workflow, returns execution metadata |
| `r8r_run_and_wait` | Execute a workflow, returns just the output |
| `r8r_discover` | Discover workflow parameters, nodes, triggers |
| `r8r_lint` | Lint workflow YAML with detailed error messages |
| `r8r_list_workflows` | List all available workflows |
| `r8r_get_workflow` | Get full workflow definition |
| `r8r_get_execution` | Get execution status and result |
| `r8r_get_trace` | Get detailed execution trace |
| `r8r_list_executions` | List recent executions |
| `r8r_validate` | Validate workflow YAML |
| `r8r_create_workflow` | Create or update a workflow |
```

**Step 3: Commit**

```bash
git add docs/NODE_TYPES.md README.md
git commit -m "docs: update webhook response_mode and MCP tools documentation"
```

---

### Task 8: Final Verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests PASS

**Step 2: Run clippy**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings

**Step 3: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues (run `cargo fmt --all` to fix if needed)

**Step 4: Build release binary**

Run: `cargo build --release --features sandbox,sandbox-docker`
Expected: Compiles successfully

**Step 5: Commit any fixups, create PR**

```bash
git push -u origin <branch-name>
gh pr create --title "feat: synchronous workflow execution (webhook response mode + MCP tools)" --body "..."
```
