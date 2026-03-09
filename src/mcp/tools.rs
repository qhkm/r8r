//! MCP Tool definitions for r8r.
//!
//! This module contains the tool implementations that are exposed via MCP.

use rmcp::{model::*, tool, Error as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

// ============================================================================
// Structured error helpers (Task #4)
// ============================================================================

/// Machine-readable error codes for MCP tools.
/// Agents can branch on `error_code` to decide how to recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Workflow name does not exist in the registry.
    WorkflowNotFound,
    /// Workflow YAML could not be parsed.
    YamlParseError,
    /// Workflow failed structural validation.
    ValidationError,
    /// Execution engine returned a failure status.
    ExecutionFailed,
    /// Execution timed out waiting for a result.
    ExecutionTimeout,
    /// Execution is paused waiting for human approval.
    ApprovalRequired,
    /// Requested resource (execution, approval, etc.) was not found.
    ResourceNotFound,
    /// Input parameters are missing or malformed.
    InvalidInput,
    /// Internal storage or I/O error.
    InternalError,
}

impl ErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            ErrorCode::WorkflowNotFound  => "workflow_not_found",
            ErrorCode::YamlParseError    => "yaml_parse_error",
            ErrorCode::ValidationError   => "validation_error",
            ErrorCode::ExecutionFailed   => "execution_failed",
            ErrorCode::ExecutionTimeout  => "execution_timeout",
            ErrorCode::ApprovalRequired  => "approval_required",
            ErrorCode::ResourceNotFound  => "resource_not_found",
            ErrorCode::InvalidInput      => "invalid_input",
            ErrorCode::InternalError     => "internal_error",
        }
    }
}

/// Build a structured error `CallToolResult` agents can parse.
///
/// Response shape:
/// ```json
/// { "error_code": "workflow_not_found", "message": "...", "details": { ... } }
/// ```
fn mcp_error(code: ErrorCode, message: impl Into<String>) -> CallToolResult {
    mcp_error_with_details(code, message, json!(null))
}

fn mcp_error_with_details(code: ErrorCode, message: impl Into<String>, details: Value) -> CallToolResult {
    let body = json!({
        "error_code": code.as_str(),
        "message": message.into(),
        "details": details,
    });
    CallToolResult::error(vec![Content::text(serde_json::to_string_pretty(&body).unwrap_or_default())])
}

use crate::engine::executor::emit_audit;
use crate::engine::Executor;
use crate::storage::{AuditEventType, SqliteStorage, Storage};
use crate::workflow::{parse_workflow, validate_workflow};

/// R8r MCP Service - handles all tool calls
#[derive(Clone)]
pub struct R8rService {
    pub storage: Arc<dyn Storage>,
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

/// Parameters for listing approval requests.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListApprovalsParams {
    /// Filter by status: pending, approved, rejected, expired.
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by assignee identity (exact match).
    #[serde(default)]
    pub assignee: Option<String>,
    /// Filter by group name (approval must include this group).
    #[serde(default)]
    pub group: Option<String>,
}

/// Parameters for approving or rejecting an approval request.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApproveParams {
    /// Approval request ID.
    pub approval_id: String,
    /// Decision value: approve or reject.
    pub decision: String,
    /// Optional decision comment.
    #[serde(default)]
    pub comment: Option<String>,
    /// Identity of the decision-maker (default: "agent").
    #[serde(default)]
    pub decided_by: Option<String>,
}

/// Parameters for generating or refining a workflow from a natural language prompt.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateParams {
    /// Natural language description of the workflow to create or change
    pub prompt: String,
    /// If provided, loads and refines this existing workflow instead of creating from scratch
    #[serde(default)]
    pub workflow_name: Option<String>,
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

pub(crate) fn inject_approval_decision_into_checkpoint(
    checkpoint_outputs: &mut serde_json::Map<String, Value>,
    approval_id: &str,
    node_id_hint: Option<&str>,
    decision: &str,
    comment: Option<&str>,
) -> Option<String> {
    let approved_status = if decision == "approve" {
        "approved"
    } else {
        "rejected"
    };

    let mut resolved_node_id = node_id_hint
        .filter(|id| checkpoint_outputs.contains_key(*id))
        .map(ToOwned::to_owned);

    if resolved_node_id.is_none() {
        resolved_node_id = checkpoint_outputs.iter().find_map(|(node_id, output)| {
            output
                .get("approval_id")
                .and_then(Value::as_str)
                .filter(|id| *id == approval_id)
                .map(|_| node_id.clone())
        });
    }

    let node_id = resolved_node_id?;
    let existing = checkpoint_outputs
        .get(&node_id)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut output_obj = existing.as_object().cloned().unwrap_or_default();
    output_obj.insert("approval_id".to_string(), json!(approval_id));
    output_obj.insert("status".to_string(), json!(approved_status));
    output_obj.insert("decision".to_string(), json!(decision));
    output_obj.insert(
        "comment".to_string(),
        comment.map(Value::from).unwrap_or(Value::Null),
    );
    checkpoint_outputs.insert(node_id.clone(), Value::Object(output_obj));
    Some(node_id)
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
                return Ok(mcp_error_with_details(
                    ErrorCode::WorkflowNotFound,
                    format!("Workflow not found: {}", params.workflow),
                    json!({ "workflow": params.workflow }),
                ));
            }
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Storage error: {}", e)));
            }
        };

        // Parse workflow
        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(mcp_error(ErrorCode::YamlParseError, format!("Invalid workflow definition: {}", e)));
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

                let status_str = execution.status.to_string();
                if status_str == "failed" {
                    return Ok(mcp_error_with_details(
                        ErrorCode::ExecutionFailed,
                        execution.error.as_deref().unwrap_or("Workflow execution failed"),
                        json!({ "execution_id": execution.id, "status": status_str }),
                    ));
                }
                if status_str == "waiting_for_approval" {
                    return Ok(mcp_error_with_details(
                        ErrorCode::ApprovalRequired,
                        "Workflow is paused waiting for human approval",
                        json!({ "execution_id": execution.id }),
                    ));
                }

                let result = json!({
                    "execution_id": execution.id,
                    "workflow": execution.workflow_name,
                    "status": status_str,
                    "output": execution.output,
                    "error": execution.error,
                    "duration_ms": duration_ms
                });

                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(mcp_error(ErrorCode::InternalError, format!("Workflow execution failed: {}", e))),
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
                return Ok(mcp_error_with_details(
                    ErrorCode::WorkflowNotFound,
                    format!("Workflow not found: {}", params.workflow),
                    json!({ "workflow": params.workflow }),
                ));
            }
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Storage error: {}", e)));
            }
        };

        // Parse workflow
        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(mcp_error(ErrorCode::YamlParseError, format!("Invalid workflow definition: {}", e)));
            }
        };

        // Execute and return just the output
        match self
            .executor
            .execute(&workflow, &stored.id, "mcp", input)
            .await
        {
            Ok(execution) => {
                let status_str = execution.status.to_string();
                if status_str == "failed" {
                    return Ok(mcp_error_with_details(
                        ErrorCode::ExecutionFailed,
                        execution.error.as_deref().unwrap_or("Workflow execution failed"),
                        json!({ "execution_id": execution.id }),
                    ));
                }
                if status_str == "waiting_for_approval" {
                    return Ok(mcp_error_with_details(
                        ErrorCode::ApprovalRequired,
                        "Workflow is paused waiting for human approval",
                        json!({ "execution_id": execution.id }),
                    ));
                }
                match execution.output {
                    Some(output) => Ok(CallToolResult::success(vec![Content::text(
                        serde_json::to_string_pretty(&output).unwrap_or_default(),
                    )])),
                    None => Ok(CallToolResult::success(vec![Content::text("null")])),
                }
            }
            Err(e) => Ok(mcp_error(ErrorCode::InternalError, format!("Workflow execution failed: {}", e))),
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
                return Ok(mcp_error_with_details(
                    ErrorCode::WorkflowNotFound,
                    format!("Workflow not found: {}", params.workflow),
                    json!({ "workflow": params.workflow }),
                ));
            }
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Storage error: {}", e)));
            }
        };

        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Ok(mcp_error(ErrorCode::YamlParseError, format!("Invalid workflow definition: {}", e)));
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
                return Ok(mcp_error(ErrorCode::YamlParseError, format!("Failed to parse workflow YAML: {}", e)));
            }
        };

        // Create isolated test executor
        let test_storage: Arc<dyn Storage> = match SqliteStorage::open_in_memory() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Failed to create test storage: {}", e)));
            }
        };
        let test_registry = crate::nodes::NodeRegistry::new();
        let test_executor = Executor::new(test_registry, Arc::clone(&test_storage));

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

    /// List approval requests, filtered by status (default: pending).
    #[tool(
        description = "List workflow approval requests. Default returns pending approvals requiring action."
    )]
    pub async fn r8r_list_approvals(
        &self,
        #[tool(aggr)] params: ListApprovalsParams,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.as_deref().unwrap_or("pending");
        let approvals = match self
            .storage
            .list_approval_requests_filtered(
                Some(status),
                params.assignee.as_deref(),
                params.group.as_deref(),
                100,
                0,
            )
            .await
        {
            Ok(list) => list,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "Failed to list approvals: {}",
                    e
                ))]));
            }
        };

        let items: Vec<Value> = approvals
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "workflow_name": a.workflow_name,
                    "node_id": a.node_id,
                    "title": a.title,
                    "description": a.description,
                    "status": a.status,
                    "created_at": a.created_at.to_rfc3339(),
                    "expires_at": a.expires_at.map(|t| t.to_rfc3339()),
                    "execution_id": a.execution_id,
                    "assignee": a.assignee,
                    "groups": a.groups,
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

    /// Approve/reject a pending approval request and resume execution.
    #[tool(
        description = "Approve or reject a pending approval request and resume the paused workflow execution."
    )]
    pub async fn r8r_approve(
        &self,
        #[tool(aggr)] params: ApproveParams,
    ) -> Result<CallToolResult, McpError> {
        if params.decision != "approve" && params.decision != "reject" {
            return Ok(mcp_error_with_details(
                ErrorCode::InvalidInput,
                format!("Decision must be \"approve\" or \"reject\", got \"{}\"", params.decision),
                json!({ "valid_values": ["approve", "reject"] }),
            ));
        }

        let mut approval = match self.storage.get_approval_request(&params.approval_id).await {
            Ok(Some(approval)) => approval,
            Ok(None) => {
                return Ok(mcp_error_with_details(
                    ErrorCode::ResourceNotFound,
                    format!("Approval request not found: {}", params.approval_id),
                    json!({ "approval_id": params.approval_id }),
                ));
            }
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Failed to load approval request: {}", e)));
            }
        };

        if approval.status != "pending" {
            return Ok(mcp_error_with_details(
                ErrorCode::InvalidInput,
                format!("Approval {} is already {}, cannot change", params.approval_id, approval.status),
                json!({ "approval_id": params.approval_id, "current_status": approval.status }),
            ));
        }

        let mut checkpoint = match self
            .storage
            .get_latest_checkpoint(&approval.execution_id)
            .await
        {
            Ok(Some(checkpoint)) => checkpoint,
            Ok(None) => {
                return Ok(mcp_error_with_details(
                    ErrorCode::ResourceNotFound,
                    format!("No checkpoint found for execution {}", approval.execution_id),
                    json!({ "execution_id": approval.execution_id }),
                ));
            }
            Err(e) => {
                return Ok(mcp_error(ErrorCode::InternalError, format!("Failed to load checkpoint: {}", e)));
            }
        };

        let mut outputs: serde_json::Map<String, Value> = checkpoint
            .node_outputs
            .as_object()
            .cloned()
            .unwrap_or_default();
        let resolved_node = inject_approval_decision_into_checkpoint(
            &mut outputs,
            &approval.id,
            if approval.node_id.is_empty() {
                None
            } else {
                Some(approval.node_id.as_str())
            },
            &params.decision,
            params.comment.as_deref(),
        );

        let Some(node_id) = resolved_node else {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Approval {} not found in latest checkpoint outputs",
                approval.id
            ))]));
        };

        checkpoint.node_outputs = Value::Object(outputs);
        if let Err(e) = self.storage.save_checkpoint(&checkpoint).await {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to update checkpoint with decision: {}",
                e
            ))]));
        }

        let decided_status = if params.decision == "approve" {
            "approved"
        } else {
            "rejected"
        };
        approval.status = decided_status.to_string();
        approval.node_id = node_id;
        approval.decision_comment = params.comment.clone();
        approval.decided_by = Some(params.decided_by.unwrap_or_else(|| "agent".to_string()));
        approval.decided_at = Some(chrono::Utc::now());

        if let Err(e) = self.storage.save_approval_request(&approval).await {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to persist approval decision: {}",
                e
            ))]));
        }

        emit_audit(
            &self.storage,
            AuditEventType::ApprovalDecided,
            Some(&approval.execution_id),
            Some(&approval.workflow_name),
            None,
            approval.decided_by.as_deref().unwrap_or("agent"),
            &format!(
                "Approval {}: {} by {}",
                decided_status, approval.id,
                approval.decided_by.as_deref().unwrap_or("agent")
            ),
            Some(json!({
                "approval_id": approval.id,
                "decision": decided_status,
                "comment": params.comment,
            })),
        )
        .await;

        match self
            .executor
            .resume_from_checkpoint(&approval.execution_id)
            .await
        {
            Ok(execution) => {
                let result = json!({
                    "approval_id": approval.id,
                    "decision": decided_status,
                    "comment": params.comment,
                    "execution_id": execution.id,
                    "execution_status": execution.status.to_string(),
                    "output": execution.output,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&result).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Approval recorded but failed to resume execution: {}",
                e
            ))])),
        }
    }

    #[tool(
        description = "Generate or refine a workflow from a natural language description. Returns the generated YAML and validation status. Use r8r_create_workflow to save it."
    )]
    pub async fn r8r_generate(
        &self,
        #[tool(aggr)] params: GenerateParams,
    ) -> Result<CallToolResult, McpError> {
        use crate::generator;

        let result = if let Some(name) = &params.workflow_name {
            // Refine existing workflow
            let stored = self.storage.get_workflow(name).await.map_err(|e| {
                McpError::internal_error("storage_error", Some(json!({"error": e.to_string()})))
            })?;

            let stored = match stored {
                Some(w) => w,
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(
                        json!({
                            "success": false,
                            "error": {
                                "code": "WORKFLOW_NOT_FOUND",
                                "message": format!("Workflow '{}' not found", name)
                            }
                        })
                        .to_string(),
                    )]));
                }
            };

            generator::refine(&stored.definition, &params.prompt).await
        } else {
            generator::generate(&params.prompt).await
        };

        match result {
            Ok(gen_result) => {
                let response = json!({
                    "success": true,
                    "data": {
                        "yaml": gen_result.yaml,
                        "summary": gen_result.summary,
                        "validation": {
                            "valid": gen_result.valid,
                            "errors": gen_result.errors,
                        }
                    }
                });
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap_or_default(),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(
                json!({
                    "success": false,
                    "error": {
                        "code": "GENERATION_FAILED",
                        "message": e.to_string()
                    }
                })
                .to_string(),
            )])),
        }
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
    use crate::storage::{SqliteStorage, Storage, StoredWorkflow};
    use std::sync::Arc;

    fn test_service() -> R8rService {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open_in_memory().unwrap());
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, Arc::clone(&storage)));
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
    async fn test_r8r_list_approvals_empty() {
        let service = test_service();
        let params = ListApprovalsParams { status: None, assignee: None, group: None };
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
            approval_id: "missing".to_string(),
            decision: "approve".to_string(),
            comment: None,
            decided_by: None,
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
            decided_by: None,
        };
        let result = service.r8r_approve(params).await.unwrap();
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn test_r8r_approval_workflow_e2e() {
        let service = test_service();
        let yaml = r#"
name: test-approval
description: Tests approval pause/resume flow
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
        save_test_workflow(&service, "test-approval", yaml).await;

        let execute = service
            .r8r_execute(ExecuteParams {
                workflow: "test-approval".to_string(),
                input: Some(json!({})),
            })
            .await
            .unwrap();
        assert!(!execute.is_error.unwrap_or(false));
        let execute_data: Value = serde_json::from_str(extract_text(&execute)).unwrap();
        assert_eq!(execute_data["status"], "paused");

        let listed = service
            .r8r_list_approvals(ListApprovalsParams { status: None, assignee: None, group: None })
            .await
            .unwrap();
        assert!(!listed.is_error.unwrap_or(false));
        let listed_data: Value = serde_json::from_str(extract_text(&listed)).unwrap();
        assert_eq!(listed_data["count"], 1);
        let approval_id = listed_data["approvals"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let approved = service
            .r8r_approve(ApproveParams {
                approval_id,
                decision: "approve".to_string(),
                comment: Some("Looks good".to_string()),
                decided_by: None,
            })
            .await
            .unwrap();
        assert!(!approved.is_error.unwrap_or(false));
        let approved_data: Value = serde_json::from_str(extract_text(&approved)).unwrap();
        assert_eq!(approved_data["decision"], "approved");
        assert_eq!(approved_data["execution_status"], "completed");
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
        assert!(!data["issues"].as_array().unwrap().is_empty());
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
        assert!(!data["diff"].as_array().unwrap().is_empty());
        assert!(data["expected"].is_object());
    }

    // =========================================================================
    // Task 5: Contains mode and pinned_data tests
    // =========================================================================

    #[tokio::test]
    async fn test_r8r_test_contains_mode_pass() {
        let service = test_service();
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
        // The `fetch` node has pinned_data so it never makes a real HTTP call.
        // The `tag` node depends on `fetch` and receives the pinned output as
        // its input, then sets a field using a template expression that reads
        // from the upstream node output via `nodes.fetch.output.*`.
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
      status: 200
      body:
        message: mocked
  - id: tag
    type: set
    config:
      fields:
        - name: source
          value: pinned
        - name: body
          value: "{{ nodes.fetch.output.body.message }}"
    depends_on: [fetch]
"#;
        // First, dry-run to see what the output actually is
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
        let nodes: Vec<String> = data["nodes_executed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(nodes.contains(&"fetch".to_string()));
        assert!(nodes.contains(&"tag".to_string()));
        // Verify pinned data actually propagated to the downstream node
        // The tag node should have received the mocked body.message value
        let output = &data["output"];
        assert_eq!(
            output["body"], "mocked",
            "Downstream node should receive pinned data value. Full output: {}",
            output
        );
    }

    // =========================================================================
    // Task 6: Error handling and edge case tests
    // =========================================================================

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
}
