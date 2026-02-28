# Workflow Testing & Mocking Framework Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an `r8r_test` MCP tool that lets agents validate workflows with test input, mock external calls via `pinned_data`, and assert expected output.

**Architecture:** Single new MCP tool method on `R8rService` using isolated in-memory storage per test run. Comparison logic (exact/contains) as a standalone function. Mocking via the existing `pinned_data` mechanism in the executor.

**Tech Stack:** Rust, rmcp (MCP framework), serde_json, schemars (JsonSchema)

---

### Task 1: Add `TestParams` struct

**Files:**
- Modify: `src/mcp/tools.rs:93` (after `DiscoverParams`)

**Step 1: Write the failing test**

Add this test at the end of the `mod tests` block in `src/mcp/tools.rs` (before the closing `}`):

```rust
#[tokio::test]
async fn test_r8r_test_dry_run() {
    let service = test_service();
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: Some(json!({"message": "hello"})),
        expected_output: None,
        mode: None,
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["status"], "completed");
    assert!(data["output"].is_object());
    assert!(data["nodes_executed"].is_array());
    assert!(data["duration_ms"].is_number());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test test_r8r_test_dry_run -- --nocapture 2>&1 | tail -20`
Expected: FAIL — `TestParams` doesn't exist, `r8r_test` method doesn't exist.

**Step 3: Add `TestParams` struct**

Insert after line 93 in `src/mcp/tools.rs` (after `DiscoverParams`):

```rust
/// Parameters for testing a workflow
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TestParams {
    /// Workflow YAML definition (may include pinned_data on nodes for mocking)
    pub workflow_yaml: String,
    /// Test input to pass to the workflow (default: {})
    #[serde(default)]
    pub input: Option<Value>,
    /// Expected output — if set, compare against actual output
    #[serde(default)]
    pub expected_output: Option<Value>,
    /// Comparison mode: "exact" (default) or "contains"
    #[serde(default)]
    pub mode: Option<String>,
}
```

**Step 4: Verify struct compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: Compiles (struct exists but unused — that's fine for now).

**Step 5: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add TestParams struct for r8r_test tool"
```

---

### Task 2: Implement JSON comparison functions

**Files:**
- Modify: `src/mcp/tools.rs` (add helper functions before the `#[tool(tool_box)]` impl block)

**Step 1: Write the failing tests**

Add these tests in `mod tests`:

```rust
#[test]
fn test_json_diff_exact_match() {
    let actual = json!({"greeting": "Hello Alice!", "source": "webhook"});
    let expected = json!({"greeting": "Hello Alice!", "source": "webhook"});
    let diffs = json_diff(&actual, &expected, "exact");
    assert!(diffs.is_empty(), "Expected no diffs, got: {:?}", diffs);
}

#[test]
fn test_json_diff_exact_mismatch() {
    let actual = json!({"greeting": "Hello Bob!"});
    let expected = json!({"greeting": "Hello Alice!"});
    let diffs = json_diff(&actual, &expected, "exact");
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].contains("greeting"));
    assert!(diffs[0].contains("Hello Alice!"));
    assert!(diffs[0].contains("Hello Bob!"));
}

#[test]
fn test_json_diff_exact_extra_keys() {
    let actual = json!({"greeting": "Hello", "extra": true});
    let expected = json!({"greeting": "Hello"});
    let diffs = json_diff(&actual, &expected, "exact");
    assert!(!diffs.is_empty(), "Exact mode should flag extra keys");
}

#[test]
fn test_json_diff_contains_subset() {
    let actual = json!({"greeting": "Hello", "source": "webhook", "ts": 123});
    let expected = json!({"greeting": "Hello"});
    let diffs = json_diff(&actual, &expected, "contains");
    assert!(diffs.is_empty(), "Contains mode should allow extra keys, got: {:?}", diffs);
}

#[test]
fn test_json_diff_contains_missing_key() {
    let actual = json!({"greeting": "Hello"});
    let expected = json!({"greeting": "Hello", "missing": true});
    let diffs = json_diff(&actual, &expected, "contains");
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].contains("missing"));
}

#[test]
fn test_json_diff_nested_objects() {
    let actual = json!({"data": {"name": "Alice", "age": 30}});
    let expected = json!({"data": {"name": "Bob", "age": 30}});
    let diffs = json_diff(&actual, &expected, "exact");
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].contains("data.name"));
}

#[test]
fn test_json_diff_array_mismatch() {
    let actual = json!({"items": [1, 2, 3]});
    let expected = json!({"items": [1, 2, 4]});
    let diffs = json_diff(&actual, &expected, "exact");
    assert_eq!(diffs.len(), 1);
    assert!(diffs[0].contains("items[2]"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_json_diff -- --nocapture 2>&1 | tail -20`
Expected: FAIL — `json_diff` function doesn't exist.

**Step 3: Implement `json_diff` function**

Add before the `#[tool(tool_box)]` line (before line 99):

```rust
/// Compare two JSON values and return a list of human-readable differences.
/// `mode` is "exact" (full equality) or "contains" (expected is a subset of actual).
fn json_diff(actual: &Value, expected: &Value, mode: &str) -> Vec<String> {
    let mut diffs = Vec::new();
    json_diff_recursive(actual, expected, mode, "", &mut diffs);
    diffs
}

fn json_diff_recursive(
    actual: &Value,
    expected: &Value,
    mode: &str,
    path: &str,
    diffs: &mut Vec<String>,
) {
    match (actual, expected) {
        (Value::Object(a_map), Value::Object(e_map)) => {
            // Check all expected keys exist and match
            for (key, e_val) in e_map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                match a_map.get(key) {
                    Some(a_val) => {
                        json_diff_recursive(a_val, e_val, mode, &child_path, diffs);
                    }
                    None => {
                        diffs.push(format!("{}: missing in actual output", child_path));
                    }
                }
            }
            // In exact mode, flag extra keys in actual
            if mode == "exact" {
                for key in a_map.keys() {
                    if !e_map.contains_key(key) {
                        let child_path = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{}.{}", path, key)
                        };
                        diffs.push(format!("{}: unexpected key in actual output", child_path));
                    }
                }
            }
        }
        (Value::Array(a_arr), Value::Array(e_arr)) => {
            if a_arr.len() != e_arr.len() {
                diffs.push(format!(
                    "{}: array length mismatch: expected {}, got {}",
                    if path.is_empty() { "root" } else { path },
                    e_arr.len(),
                    a_arr.len()
                ));
                return;
            }
            for (i, (a_val, e_val)) in a_arr.iter().zip(e_arr.iter()).enumerate() {
                let child_path = format!("{}[{}]", path, i);
                json_diff_recursive(a_val, e_val, mode, &child_path, diffs);
            }
        }
        _ => {
            if actual != expected {
                let display_path = if path.is_empty() { "root" } else { path };
                diffs.push(format!(
                    "{}: expected {}, got {}",
                    display_path, expected, actual
                ));
            }
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test test_json_diff -- --nocapture 2>&1 | tail -20`
Expected: All 7 tests PASS.

**Step 5: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: add json_diff comparison functions for test assertions"
```

---

### Task 3: Implement `r8r_test` tool method

**Files:**
- Modify: `src/mcp/tools.rs` (add method inside the `#[tool(tool_box)] impl R8rService` block, after `r8r_lint`)

**Step 1: The failing test already exists from Task 1** (`test_r8r_test_dry_run`)

Verify it still fails:
Run: `cargo test test_r8r_test_dry_run -- --nocapture 2>&1 | tail -10`
Expected: FAIL — `r8r_test` method doesn't exist.

**Step 2: Implement `r8r_test` method**

Add inside the `#[tool(tool_box)] impl R8rService` block, after the `r8r_lint` method (before the closing `}` of the impl):

```rust
    /// Test a workflow with given input and optionally assert expected output.
    /// Use pinned_data on nodes to mock external calls (HTTP, agent, etc.).
    /// Runs in full isolation — no side effects to the real database.
    #[tool(description = "Test a workflow with input and optional expected output assertion. Use pinned_data on nodes to mock external calls. Fully isolated execution.")]
    pub async fn r8r_test(
        &self,
        #[tool(aggr)] params: TestParams,
    ) -> Result<CallToolResult, McpError> {
        // Parse the workflow
        let workflow = match parse_workflow(&params.workflow_yaml) {
            Ok(w) => w,
            Err(e) => {
                return Ok(CallToolResult::error(format!(
                    "Failed to parse workflow YAML: {}",
                    e
                )));
            }
        };

        // Create isolated test executor
        let test_storage = match SqliteStorage::open_in_memory() {
            Ok(s) => s,
            Err(e) => {
                return Ok(CallToolResult::error(format!(
                    "Failed to create test storage: {}",
                    e
                )));
            }
        };
        let test_registry = NodeRegistry::new();
        let test_executor = Executor::new(test_registry, test_storage.clone());

        // Save workflow to test storage
        let workflow_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let stored = crate::storage::StoredWorkflow {
            id: workflow_id.clone(),
            name: workflow.name.clone(),
            definition: params.workflow_yaml.clone(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = test_storage.save_workflow(&stored).await {
            return Ok(CallToolResult::error(format!(
                "Failed to save test workflow: {}",
                e
            )));
        }

        // Execute
        let input = params.input.unwrap_or(json!({}));
        let start = std::time::Instant::now();
        let execution = match test_executor
            .execute(&workflow, &workflow_id, "test", input)
            .await
        {
            Ok(exec) => exec,
            Err(e) => {
                return Ok(CallToolResult::error(format!(
                    "Workflow execution failed: {}",
                    e
                )));
            }
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        // Gather nodes_executed from node executions
        let node_execs = test_storage
            .get_node_executions(&execution.id)
            .await
            .unwrap_or_default();
        let nodes_executed: Vec<String> = node_execs.iter().map(|ne| ne.node_id.clone()).collect();

        // Check execution status
        if execution.status == crate::storage::ExecutionStatus::Failed {
            let mut node_errors = serde_json::Map::new();
            for ne in &node_execs {
                if let Some(ref err) = ne.error {
                    node_errors.insert(ne.node_id.clone(), json!(err));
                }
            }
            let result = json!({
                "status": "failed",
                "error": execution.error,
                "nodes_executed": nodes_executed,
                "node_errors": node_errors,
            });
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            )]));
        }

        let actual_output = execution.output.clone().unwrap_or(json!(null));

        // If no expected_output, return dry-run result
        let Some(expected) = params.expected_output else {
            let result = json!({
                "status": "completed",
                "output": actual_output,
                "duration_ms": duration_ms,
                "nodes_executed": nodes_executed,
            });
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            )]));
        };

        // Compare output
        let mode = params.mode.as_deref().unwrap_or("exact");
        let diffs = json_diff(&actual_output, &expected, mode);
        let pass = diffs.is_empty();

        let mut result = json!({
            "pass": pass,
            "output": actual_output,
            "duration_ms": duration_ms,
            "nodes_executed": nodes_executed,
        });

        if !pass {
            result["expected"] = json!(expected);
            result["diff"] = json!(diffs);
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
```

**Step 3: Run the dry-run test**

Run: `cargo test test_r8r_test_dry_run -- --nocapture 2>&1 | tail -10`
Expected: PASS

**Step 4: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "feat: implement r8r_test MCP tool with isolated execution"
```

---

### Task 4: Add assertion tests (exact match pass/fail)

**Files:**
- Modify: `src/mcp/tools.rs` (add tests in `mod tests`)

**Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn test_r8r_test_exact_match_pass() {
    let service = test_service();
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: Some(json!({"message": "hello"})),
        expected_output: Some(json!({"result": "hello"})),
        mode: None, // default = "exact"
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["pass"], true);
    assert!(data.get("diff").is_none());
}

#[tokio::test]
async fn test_r8r_test_exact_match_fail() {
    let service = test_service();
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: Some(json!({"message": "hello"})),
        expected_output: Some(json!({"result": "wrong"})),
        mode: None,
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["pass"], false);
    assert!(data["diff"].is_array());
    assert!(data["diff"].as_array().unwrap().len() > 0);
    assert!(data["expected"].is_object());
}
```

**Step 2: Run tests**

Run: `cargo test test_r8r_test_exact_match -- --nocapture 2>&1 | tail -15`
Expected: Both PASS (implementation from Task 3 should handle this).

**Step 3: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "test: add exact match pass/fail tests for r8r_test"
```

---

### Task 5: Add contains mode and pinned_data tests

**Files:**
- Modify: `src/mcp/tools.rs` (add tests in `mod tests`)

**Step 1: Write the tests**

```rust
#[tokio::test]
async fn test_r8r_test_contains_mode_pass() {
    let service = test_service();
    // The simple workflow outputs {"result": "hello"} — exact one key
    // Use contains mode to only check that "result" key exists with right value
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: Some(json!({"message": "hello"})),
        expected_output: Some(json!({"result": "hello"})),
        mode: Some("contains".to_string()),
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["pass"], true);
}

#[tokio::test]
async fn test_r8r_test_contains_mode_fail() {
    let service = test_service();
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: Some(json!({"message": "hello"})),
        expected_output: Some(json!({"result": "wrong_value"})),
        mode: Some("contains".to_string()),
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["pass"], false);
}

#[tokio::test]
async fn test_r8r_test_pinned_data_mocking() {
    let service = test_service();
    // Workflow with an HTTP node that would fail without mocking,
    // but pinned_data provides the output instead.
    let yaml = r#"
name: test-pinned
description: Tests pinned_data mocking
version: 1
nodes:
  - id: fetch
    type: http
    config:
      url: https://api.example.com/data
      method: GET
    pinned_data:
      users:
        - name: Alice
        - name: Bob
  - id: count
    type: transform
    config:
      expression: 'input.users.len()'
    depends_on: [fetch]
"#;
    let params = TestParams {
        workflow_yaml: yaml.to_string(),
        input: None,
        expected_output: Some(json!(2)),
        mode: Some("exact".to_string()),
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["pass"], true);
    // Verify both nodes executed
    let nodes: Vec<String> = data["nodes_executed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(nodes.contains(&"fetch".to_string()));
    assert!(nodes.contains(&"count".to_string()));
}
```

**Step 2: Run tests**

Run: `cargo test test_r8r_test_contains_mode -- --nocapture 2>&1 | tail -15`
Run: `cargo test test_r8r_test_pinned_data -- --nocapture 2>&1 | tail -15`
Expected: All PASS.

**Step 3: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "test: add contains mode and pinned_data mocking tests for r8r_test"
```

---

### Task 6: Add error handling tests

**Files:**
- Modify: `src/mcp/tools.rs` (add tests in `mod tests`)

**Step 1: Write the tests**

```rust
#[tokio::test]
async fn test_r8r_test_invalid_yaml() {
    let service = test_service();
    let params = TestParams {
        workflow_yaml: "not: valid: yaml: [".to_string(),
        input: None,
        expected_output: None,
        mode: None,
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    assert!(text.contains("parse") || text.contains("YAML") || text.contains("Invalid"));
}

#[tokio::test]
async fn test_r8r_test_empty_input_default() {
    let service = test_service();
    // When input is None, should default to {}
    let params = TestParams {
        workflow_yaml: simple_workflow_yaml().to_string(),
        input: None,
        expected_output: None,
        mode: None,
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["status"], "completed");
}

#[tokio::test]
async fn test_r8r_test_null_output_workflow() {
    let service = test_service();
    // A workflow with no set nodes — produces no output
    let yaml = r#"
name: empty-workflow
description: Produces no output
version: 1
nodes:
  - id: noop
    type: debug
    config:
      message: "hello"
"#;
    let params = TestParams {
        workflow_yaml: yaml.to_string(),
        input: None,
        expected_output: None,
        mode: None,
    };
    let result = service.r8r_test(params).await.unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let text = extract_text(&result);
    let data: Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["status"], "completed");
}
```

**Step 2: Run tests**

Run: `cargo test test_r8r_test_invalid_yaml test_r8r_test_empty_input test_r8r_test_null_output -- --nocapture 2>&1 | tail -20`
Expected: All PASS.

**Step 3: Commit**

```bash
git add src/mcp/tools.rs
git commit -m "test: add error handling and edge case tests for r8r_test"
```

---

### Task 7: Update documentation

**Files:**
- Modify: `README.md` (add r8r_test to MCP tools table)
- Modify: `docs/NODE_TYPES.md` (add testing section about pinned_data)

**Step 1: Update README.md**

Find the MCP tools table and add `r8r_test` row:

```markdown
| `r8r_test` | Test a workflow with input and assert expected output. Mocks external calls via `pinned_data`. Fully isolated. |
```

**Step 2: Update docs/NODE_TYPES.md**

Add a "Testing with pinned_data" section that explains:
- How `pinned_data` works on any node
- Example of mocking an HTTP node
- Example of using `r8r_test` for validation

**Step 3: Commit**

```bash
git add README.md docs/NODE_TYPES.md
git commit -m "docs: add r8r_test tool docs and pinned_data testing guide"
```

---

### Task 8: Final verification

**Step 1: Run all tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All tests pass.

**Step 2: Run clippy**

Run: `cargo clippy --all-targets --all-features 2>&1 | tail -10`
Expected: No warnings.

**Step 3: Run fmt check**

Run: `cargo fmt --all -- --check 2>&1`
Expected: No formatting issues. If there are, fix with `cargo fmt --all`.

**Step 4: Commit any fixes**

If fmt or clippy needed fixes:
```bash
git add -A
git commit -m "style: fix clippy/fmt issues"
```
