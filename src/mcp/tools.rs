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

// ============================================================================
// Tool Implementations
// ============================================================================

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
                return Err(McpError::internal_error(
                    "storage_error",
                    Some(json!({"error": e.to_string()})),
                ));
            }
        };

        // Parse workflow
        let workflow = match parse_workflow(&stored.definition) {
            Ok(wf) => wf,
            Err(e) => {
                return Err(McpError::internal_error(
                    "parse_error",
                    Some(json!({"error": e.to_string()})),
                ));
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
            Err(e) => Err(McpError::internal_error(
                "execution_error",
                Some(json!({"error": e.to_string()})),
            )),
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
