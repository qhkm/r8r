//! MCP Tool definitions for r8r.
//!
//! This module contains the tool implementations that are exposed via MCP.

use rmcp::{model::*, tool, Error as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::engine::Executor;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, validate_workflow};

/// R8r MCP Service - handles all tool calls
#[derive(Clone)]
pub struct R8rService {
    pub storage: SqliteStorage,
    pub executor: Arc<Executor>,
}

// ============================================================================
// Tool Parameter Types
// ============================================================================

/// Parameters for executing a workflow
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExecuteParams {
    /// Name of the workflow to execute
    pub workflow: String,
    /// Input data for the workflow (JSON object)
    #[serde(default)]
    pub input: Option<Value>,
}

/// Parameters for getting a workflow
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWorkflowParams {
    /// Workflow name
    pub name: String,
}

/// Parameters for getting an execution
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetExecutionParams {
    /// Execution ID
    pub execution_id: String,
}

/// Parameters for listing executions
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListExecutionsParams {
    /// Workflow name
    pub workflow: String,
    /// Maximum number of executions to return (default: 10)
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

/// Parameters for validating a workflow
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateParams {
    /// Workflow YAML definition
    pub yaml: String,
}

/// Parameters for creating a workflow
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateWorkflowParams {
    /// Workflow YAML definition
    pub yaml: String,
}

/// Parameters for running a workflow and waiting for the result
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunAndWaitParams {
    /// Name of the workflow to execute
    pub workflow: String,
    /// Input data for the workflow (JSON object)
    #[serde(default)]
    pub input: Option<Value>,
}

/// Parameters for discovering workflow details
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverParams {
    /// Name of the workflow to discover
    pub workflow: String,
}

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

// ============================================================================
// Tool Implementations
// ============================================================================

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

#[tool(tool_box)]
impl R8rService {
    /// Execute an r8r workflow and return the result.
    #[tool(
        description = "Execute an r8r workflow. Returns execution result with status, output, and execution ID."
    )]
    pub async fn r8r_execute(
        &self,
        #[tool(aggr)] params: ExecuteParams,
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

        // Execute
        match self
            .executor
            .execute(&workflow, &stored.id, "mcp", input)
            .await
        {
            Ok(execution) => {
                let duration_ms = execution
                    .finished_at
                    .map(|f| (f - execution.started_at).num_milliseconds());

                let result = json!({
                    "execution_id": execution.id,
                    "workflow": execution.workflow_name,
                    "status": execution.status.to_string(),
                    "output": execution.output,
                    "error": execution.error,
                    "duration_ms": duration_ms
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Workflow execution failed: {}",
                e
            ))])),
        }
    }

    /// List all available r8r workflows.
    #[tool(description = "List all available r8r workflows with their status and description.")]
    pub async fn r8r_list_workflows(&self) -> Result<CallToolResult, McpError> {
        match self.storage.list_workflows().await {
            Ok(workflows) => {
                let list: Vec<Value> = workflows
                    .iter()
                    .map(|wf| {
                        let desc = parse_workflow(&wf.definition)
                            .map(|w| w.description)
                            .unwrap_or_default();

                        json!({
                            "name": wf.name,
                            "enabled": wf.enabled,
                            "description": desc,
                            "updated_at": wf.updated_at.to_rfc3339()
                        })
                    })
                    .collect();

                let result = json!({
                    "workflows": list,
                    "count": list.len()
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Err(McpError::internal_error(
                "storage_error",
                Some(json!({"error": e.to_string()})),
            )),
        }
    }

    /// Get the full definition of a workflow.
    #[tool(
        description = "Get the full definition of a workflow including all nodes and configuration."
    )]
    pub async fn r8r_get_workflow(
        &self,
        #[tool(aggr)] params: GetWorkflowParams,
    ) -> Result<CallToolResult, McpError> {
        match self.storage.get_workflow(&params.name).await {
            Ok(Some(stored)) => match parse_workflow(&stored.definition) {
                Ok(workflow) => {
                    let result = json!({
                        "name": workflow.name,
                        "description": workflow.description,
                        "version": workflow.version,
                        "enabled": stored.enabled,
                        "triggers": workflow.triggers,
                        "nodes": workflow.nodes,
                        "nodes_count": workflow.nodes.len(),
                        "definition_yaml": stored.definition
                    });

                    Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )]))
                }
                Err(e) => Err(McpError::internal_error(
                    "parse_error",
                    Some(json!({"error": e.to_string()})),
                )),
            },
            Ok(None) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Workflow not found: {}",
                params.name
            ))])),
            Err(e) => Err(McpError::internal_error(
                "storage_error",
                Some(json!({"error": e.to_string()})),
            )),
        }
    }

    /// Get the status and result of a workflow execution.
    #[tool(description = "Get the status and result of a workflow execution.")]
    pub async fn r8r_get_execution(
        &self,
        #[tool(aggr)] params: GetExecutionParams,
    ) -> Result<CallToolResult, McpError> {
        match self.storage.get_execution(&params.execution_id).await {
            Ok(Some(execution)) => {
                let duration_ms = execution
                    .finished_at
                    .map(|f| (f - execution.started_at).num_milliseconds());

                let result = json!({
                    "execution_id": execution.id,
                    "workflow_name": execution.workflow_name,
                    "status": execution.status.to_string(),
                    "trigger_type": execution.trigger_type,
                    "input": execution.input,
                    "output": execution.output,
                    "error": execution.error,
                    "started_at": execution.started_at.to_rfc3339(),
                    "finished_at": execution.finished_at.map(|t| t.to_rfc3339()),
                    "duration_ms": duration_ms
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Ok(None) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Execution not found: {}",
                params.execution_id
            ))])),
            Err(e) => Err(McpError::internal_error(
                "storage_error",
                Some(json!({"error": e.to_string()})),
            )),
        }
    }

    /// Get detailed execution trace for debugging.
    #[tool(
        description = "Get detailed execution trace for debugging. Shows input/output of each node."
    )]
    pub async fn r8r_get_trace(
        &self,
        #[tool(aggr)] params: GetExecutionParams,
    ) -> Result<CallToolResult, McpError> {
        // Get execution
        let execution = match self.storage.get_execution(&params.execution_id).await {
            Ok(Some(e)) => e,
            Ok(None) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Execution not found: {}",
                    params.execution_id
                ))]));
            }
            Err(e) => {
                return Err(McpError::internal_error(
                    "storage_error",
                    Some(json!({"error": e.to_string()})),
                ));
            }
        };

        // Get node executions
        let node_execs = match self.storage.get_node_executions(&params.execution_id).await {
            Ok(n) => n,
            Err(e) => {
                return Err(McpError::internal_error(
                    "storage_error",
                    Some(json!({"error": e.to_string()})),
                ));
            }
        };

        let trace: Vec<Value> = node_execs
            .iter()
            .map(|ne| {
                let duration_ms = ne
                    .finished_at
                    .map(|f| (f - ne.started_at).num_milliseconds());

                json!({
                    "node_id": ne.node_id,
                    "status": ne.status.to_string(),
                    "input": ne.input,
                    "output": ne.output,
                    "error": ne.error,
                    "started_at": ne.started_at.to_rfc3339(),
                    "finished_at": ne.finished_at.map(|t| t.to_rfc3339()),
                    "duration_ms": duration_ms
                })
            })
            .collect();

        let failed_node = node_execs
            .iter()
            .find(|n| n.error.is_some())
            .map(|n| n.node_id.clone());

        let result = json!({
            "execution_id": execution.id,
            "workflow_name": execution.workflow_name,
            "status": execution.status.to_string(),
            "error": execution.error,
            "nodes": trace,
            "nodes_count": trace.len(),
            "failed_node": failed_node
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    /// List recent executions for a workflow.
    #[tool(description = "List recent executions for a workflow with status and timing.")]
    pub async fn r8r_list_executions(
        &self,
        #[tool(aggr)] params: ListExecutionsParams,
    ) -> Result<CallToolResult, McpError> {
        match self
            .storage
            .list_executions(&params.workflow, params.limit)
            .await
        {
            Ok(executions) => {
                let list: Vec<Value> = executions
                    .iter()
                    .map(|e| {
                        let duration_ms =
                            e.finished_at.map(|f| (f - e.started_at).num_milliseconds());

                        json!({
                            "execution_id": e.id,
                            "status": e.status.to_string(),
                            "trigger_type": e.trigger_type,
                            "started_at": e.started_at.to_rfc3339(),
                            "duration_ms": duration_ms,
                            "error": e.error
                        })
                    })
                    .collect();

                let result = json!({
                    "workflow": params.workflow,
                    "executions": list,
                    "count": list.len()
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Err(McpError::internal_error(
                "storage_error",
                Some(json!({"error": e.to_string()})),
            )),
        }
    }

    /// Validate a workflow YAML definition without saving it.
    #[tool(description = "Validate a workflow YAML definition without saving it.")]
    pub async fn r8r_validate(
        &self,
        #[tool(aggr)] params: ValidateParams,
    ) -> Result<CallToolResult, McpError> {
        match parse_workflow(&params.yaml) {
            Ok(workflow) => match validate_workflow(&workflow) {
                Ok(()) => {
                    let result = json!({
                        "valid": true,
                        "name": workflow.name,
                        "description": workflow.description,
                        "nodes_count": workflow.nodes.len(),
                        "triggers_count": workflow.triggers.len()
                    });

                    Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )]))
                }
                Err(e) => {
                    let result = json!({
                        "valid": false,
                        "error": e.to_string(),
                        "error_type": "validation"
                    });

                    Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )]))
                }
            },
            Err(e) => {
                let result = json!({
                    "valid": false,
                    "error": e.to_string(),
                    "error_type": "parse"
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
        }
    }

    /// Create or update a workflow from YAML definition.
    #[tool(description = "Create or update a workflow from YAML definition.")]
    pub async fn r8r_create_workflow(
        &self,
        #[tool(aggr)] params: CreateWorkflowParams,
    ) -> Result<CallToolResult, McpError> {
        // Parse
        let workflow = match parse_workflow(&params.yaml) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Parse error: {}",
                    e
                ))]));
            }
        };

        // Validate
        if let Err(e) = validate_workflow(&workflow) {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Validation error: {}",
                e
            ))]));
        }

        // Save
        let now = chrono::Utc::now();
        let stored = crate::storage::StoredWorkflow {
            id: uuid::Uuid::new_v4().to_string(),
            name: workflow.name.clone(),
            definition: params.yaml,
            enabled: true,
            created_at: now,
            updated_at: now,
        };

        match self.storage.save_workflow(&stored).await {
            Ok(()) => {
                let result = json!({
                    "created": true,
                    "name": workflow.name,
                    "id": stored.id,
                    "nodes_count": workflow.nodes.len()
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Err(McpError::internal_error(
                "storage_error",
                Some(json!({"error": e.to_string()})),
            )),
        }
    }

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
                    if error_str.contains("dependsOn") {
                        issue["suggestion"] =
                            json!("Use 'depends_on' (snake_case) for node dependencies");
                    }
                    if error_str.contains("nodeType") {
                        issue["suggestion"] = json!("Use 'type' for node type, not 'nodeType'");
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

    /// Test a workflow with given input and optionally assert expected output.
    /// Use pinned_data on nodes to mock external calls (HTTP, agent, etc.).
    /// Runs in full isolation — no side effects to the real database.
    #[tool(
        description = "Test a workflow with input and optional expected output assertion. Use pinned_data on nodes to mock external calls. Fully isolated execution."
    )]
    pub async fn r8r_test(
        &self,
        #[tool(aggr)] params: TestParams,
    ) -> Result<CallToolResult, McpError> {
        // Parse the workflow
        let workflow = match parse_workflow(&params.workflow_yaml) {
            Ok(w) => w,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to parse workflow YAML: {}",
                    e
                ))]));
            }
        };

        // Create isolated test executor
        let test_storage = match SqliteStorage::open_in_memory() {
            Ok(s) => s,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to create test storage: {}",
                    e
                ))]));
            }
        };
        let test_registry = crate::nodes::NodeRegistry::new();
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
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to save test workflow: {}",
                e
            ))]));
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
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Workflow execution failed: {}",
                    e
                ))]));
            }
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        // Gather nodes_executed
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
}

#[tool(tool_box)]
impl ServerHandler for R8rService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "R8r is an agent-first workflow automation engine. Use tools to execute, list, and debug workflows."
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "r8r".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Executor;
    use crate::nodes::NodeRegistry;
    use crate::storage::{SqliteStorage, StoredWorkflow};
    use std::sync::Arc;

    fn test_service() -> R8rService {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, storage.clone()));
        R8rService { storage, executor }
    }

    async fn save_test_workflow(service: &R8rService, name: &str, yaml: &str) {
        let now = chrono::Utc::now();
        let stored = StoredWorkflow {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            definition: yaml.to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        };
        service.storage.save_workflow(&stored).await.unwrap();
    }

    fn simple_workflow_yaml() -> &'static str {
        r#"
name: test-simple
description: A simple test workflow
version: 1
inputs:
  message:
    type: string
    description: The message to echo
    required: true
    default: "hello"
nodes:
  - id: echo
    type: set
    config:
      fields:
        - name: result
          value: "{{ input.message }}"
"#
    }

    /// Helper to extract text from the first content item of a CallToolResult.
    fn extract_text(result: &CallToolResult) -> &str {
        match &result.content[0].raw {
            RawContent::Text(t) => &t.text,
            _ => panic!("Expected text content"),
        }
    }

    #[tokio::test]
    async fn test_r8r_execute_workflow_not_found() {
        let service = test_service();
        let params = ExecuteParams {
            workflow: "nonexistent".to_string(),
            input: None,
        };
        let result = service.r8r_execute(params).await.unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_r8r_execute_success() {
        let service = test_service();
        save_test_workflow(&service, "test-simple", simple_workflow_yaml()).await;
        let params = ExecuteParams {
            workflow: "test-simple".to_string(),
            input: Some(json!({"message": "world"})),
        };
        let result = service.r8r_execute(params).await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        assert!(data["execution_id"].is_string());
        assert_eq!(data["status"], "completed");
    }

    #[tokio::test]
    async fn test_r8r_run_and_wait_returns_only_output() {
        let service = test_service();
        save_test_workflow(&service, "test-simple", simple_workflow_yaml()).await;
        let params = RunAndWaitParams {
            workflow: "test-simple".to_string(),
            input: Some(json!({"message": "test"})),
        };
        let result = service.r8r_run_and_wait(params).await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        // Should NOT have execution_id, status, duration_ms — only the output
        assert!(data.get("execution_id").is_none());
        assert!(data.get("status").is_none());
        // Should have the actual result
        assert_eq!(data["result"], "test");
    }

    #[tokio::test]
    async fn test_r8r_run_and_wait_not_found() {
        let service = test_service();
        let params = RunAndWaitParams {
            workflow: "missing".to_string(),
            input: None,
        };
        let result = service.r8r_run_and_wait(params).await.unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_r8r_discover_returns_parameters() {
        let service = test_service();
        save_test_workflow(&service, "test-simple", simple_workflow_yaml()).await;
        let params = DiscoverParams {
            workflow: "test-simple".to_string(),
        };
        let result = service.r8r_discover(params).await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["name"], "test-simple");
        assert_eq!(data["description"], "A simple test workflow");
        assert_eq!(data["version"], 1);
        assert_eq!(data["nodes_count"], 1);
        // Check parameters
        assert!(data["parameters"]["message"].is_object());
        assert_eq!(data["parameters"]["message"]["type"], "string");
        assert_eq!(data["parameters"]["message"]["required"], true);
    }

    #[tokio::test]
    async fn test_r8r_discover_not_found() {
        let service = test_service();
        let params = DiscoverParams {
            workflow: "nope".to_string(),
        };
        let result = service.r8r_discover(params).await.unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_r8r_lint_valid_workflow() {
        let service = test_service();
        let params = ValidateParams {
            yaml: simple_workflow_yaml().to_string(),
        };
        let result = service.r8r_lint(params).await.unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["valid"], true);
        assert_eq!(data["name"], "test-simple");
    }

    #[tokio::test]
    async fn test_r8r_lint_invalid_yaml() {
        let service = test_service();
        let params = ValidateParams {
            yaml: "not: valid: yaml: [".to_string(),
        };
        let result = service.r8r_lint(params).await.unwrap();
        // Lint results are NOT tool errors (is_error should be false)
        assert!(!result.is_error.unwrap_or(false));
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["valid"], false);
        assert!(data["issues"].as_array().unwrap().len() > 0);
        assert_eq!(data["issues"][0]["level"], "error");
        assert_eq!(data["issues"][0]["type"], "parse");
    }

    #[tokio::test]
    async fn test_r8r_lint_warnings_no_description_no_triggers() {
        let service = test_service();
        let yaml = r#"
name: bare-workflow
nodes:
  - id: step1
    type: set
    config:
      fields:
        ok: true
  - id: step2
    type: set
    config:
      fields:
        done: true
"#;
        let params = ValidateParams {
            yaml: yaml.to_string(),
        };
        let result = service.r8r_lint(params).await.unwrap();
        let text = extract_text(&result);
        let data: Value = serde_json::from_str(text).unwrap();
        // Should be valid (warnings don't make it invalid)
        assert_eq!(data["valid"], true);
        let issues = data["issues"].as_array().unwrap();
        // Should have warnings: no description, no triggers, no depends_on
        let warnings: Vec<_> = issues.iter().filter(|i| i["level"] == "warning").collect();
        assert!(
            warnings.len() >= 2,
            "Expected at least 2 warnings, got {}: {:?}",
            warnings.len(),
            warnings
        );
    }

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
        assert!(
            diffs.is_empty(),
            "Contains mode should allow extra keys, got: {:?}",
            diffs
        );
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

    // =========================================================================
    // Task 4: Exact match tests
    // =========================================================================

    #[tokio::test]
    async fn test_r8r_test_exact_match_pass() {
        let service = test_service();
        // The `set` node copies all input fields into the output map and then
        // adds the new `result` field, so the full output is
        // {"message": "hello", "result": "hello"}.
        let params = TestParams {
            workflow_yaml: simple_workflow_yaml().to_string(),
            input: Some(json!({"message": "hello"})),
            expected_output: Some(json!({"message": "hello", "result": "hello"})),
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

}
