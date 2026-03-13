/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Tool bridge: converts r8r MCP tool definitions into LLM function calling
//! format and executes tool calls.

use crate::engine::Executor;
use crate::llm::LlmConfig;
use crate::storage::Storage;
use crate::workflow::{parse_workflow, validate_workflow};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

/// Build OpenAI-compatible function calling tool definitions for all r8r tools.
pub fn build_tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "r8r_execute",
            "Execute an r8r workflow. Returns execution result with status, output, and execution ID.",
            json!({
                "type": "object",
                "properties": {
                    "workflow": {"type": "string", "description": "Name of the workflow to execute"},
                    "input": {"type": "object", "description": "Input data for the workflow (JSON object)"}
                },
                "required": ["workflow"]
            }),
        ),
        tool_def(
            "r8r_run_and_wait",
            "Execute a workflow and wait for the result. Returns just the output.",
            json!({
                "type": "object",
                "properties": {
                    "workflow": {"type": "string", "description": "Name of the workflow to execute"},
                    "input": {"type": "object", "description": "Input data for the workflow"}
                },
                "required": ["workflow"]
            }),
        ),
        tool_def(
            "r8r_list_workflows",
            "List all available r8r workflows with their status and description.",
            json!({"type": "object", "properties": {}}),
        ),
        tool_def(
            "r8r_get_workflow",
            "Get full workflow definition (YAML) by name.",
            json!({
                "type": "object",
                "properties": {"name": {"type": "string", "description": "Workflow name"}},
                "required": ["name"]
            }),
        ),
        tool_def(
            "r8r_discover",
            "Discover workflow parameters, nodes, triggers, and dependencies.",
            json!({
                "type": "object",
                "properties": {"workflow": {"type": "string", "description": "Name of the workflow to discover"}},
                "required": ["workflow"]
            }),
        ),
        tool_def(
            "r8r_get_execution",
            "Get execution status and result by execution ID.",
            json!({
                "type": "object",
                "properties": {"execution_id": {"type": "string", "description": "Execution ID"}},
                "required": ["execution_id"]
            }),
        ),
        tool_def(
            "r8r_list_executions",
            "List recent executions for a workflow.",
            json!({
                "type": "object",
                "properties": {
                    "workflow": {"type": "string", "description": "Workflow name"},
                    "limit": {"type": "integer", "description": "Max results (default 10)"}
                },
                "required": ["workflow"]
            }),
        ),
        tool_def(
            "r8r_get_trace",
            "Get detailed execution trace with per-node timings.",
            json!({
                "type": "object",
                "properties": {"execution_id": {"type": "string", "description": "Execution ID"}},
                "required": ["execution_id"]
            }),
        ),
        tool_def(
            "r8r_validate",
            "Validate workflow YAML. Returns validation status and errors.",
            json!({
                "type": "object",
                "properties": {"yaml": {"type": "string", "description": "Workflow YAML definition"}},
                "required": ["yaml"]
            }),
        ),
        tool_def(
            "r8r_lint",
            "Lint workflow YAML with detailed error messages and suggestions.",
            json!({
                "type": "object",
                "properties": {"yaml": {"type": "string", "description": "Workflow YAML definition"}},
                "required": ["yaml"]
            }),
        ),
        tool_def(
            "r8r_create_workflow",
            "Create or update a workflow from YAML.",
            json!({
                "type": "object",
                "properties": {"yaml": {"type": "string", "description": "Workflow YAML definition"}},
                "required": ["yaml"]
            }),
        ),
        tool_def(
            "r8r_generate",
            "Generate or refine a workflow from a natural language description.",
            json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "Natural language description of the workflow"},
                    "workflow_name": {"type": "string", "description": "If provided, loads and refines this existing workflow"}
                },
                "required": ["prompt"]
            }),
        ),
        tool_def(
            "r8r_test",
            "Test a workflow with input and assert expected output. Supports mocking via pinned_data.",
            json!({
                "type": "object",
                "properties": {
                    "workflow_yaml": {"type": "string", "description": "Workflow YAML (may include pinned_data for mocking)"},
                    "input": {"type": "object", "description": "Test input"},
                    "expected_output": {"type": "object", "description": "Expected output to compare against"},
                    "mode": {"type": "string", "description": "Comparison mode: exact (default) or contains"}
                },
                "required": ["workflow_yaml"]
            }),
        ),
        tool_def(
            "r8r_list_approvals",
            "List pending approval requests that need action.",
            json!({
                "type": "object",
                "properties": {"status": {"type": "string", "description": "Filter: pending, approved, rejected, expired"}}
            }),
        ),
        tool_def(
            "r8r_approve",
            "Approve or reject a pending approval request.",
            json!({
                "type": "object",
                "properties": {
                    "approval_id": {"type": "string", "description": "Approval request ID"},
                    "decision": {"type": "string", "description": "approve or reject"},
                    "comment": {"type": "string", "description": "Optional decision comment"}
                },
                "required": ["approval_id", "decision"]
            }),
        ),
    ]
}

fn tool_def(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

/// Parse a text-based tool call from LLM output (Ollama fallback).
///
/// Looks for `<tool_call>{"name": "...", "args": {...}}</tool_call>`.
pub fn parse_text_tool_call(text: &str) -> Option<(String, Value)> {
    let start = text.find("<tool_call>")?;
    let end = text.find("</tool_call>")?;
    let json_str = &text[start + "<tool_call>".len()..end];
    let v: Value = serde_json::from_str(json_str.trim()).ok()?;
    let name = v["name"].as_str()?.to_string();
    let args = v.get("args").cloned().unwrap_or(json!({}));
    Some((name, args))
}

/// Execute a tool by name with JSON args.
pub async fn execute_tool(
    name: &str,
    args: &Value,
    storage: &dyn Storage,
    executor: &Arc<Executor>,
) -> String {
    execute_tool_with_config(name, args, storage, executor, None).await
}

/// Execute a tool by name with JSON args and optional LLM config override.
pub async fn execute_tool_with_config(
    name: &str,
    args: &Value,
    storage: &dyn Storage,
    executor: &Arc<Executor>,
    llm_config: Option<&LlmConfig>,
) -> String {
    debug!(tool = name, "Executing REPL tool call");

    match name {
        "r8r_list_workflows" => exec_list_workflows(storage).await,
        "r8r_get_workflow" => exec_get_workflow(storage, args).await,
        "r8r_execute" => exec_execute(storage, executor, args).await,
        "r8r_run_and_wait" => exec_run_and_wait(storage, executor, args).await,
        "r8r_discover" => exec_discover(storage, args).await,
        "r8r_get_execution" => exec_get_execution(storage, args).await,
        "r8r_list_executions" => exec_list_executions(storage, args).await,
        "r8r_get_trace" => exec_get_trace(storage, args).await,
        "r8r_validate" => exec_validate(args).await,
        "r8r_lint" => exec_validate(args).await,
        "r8r_create_workflow" => exec_create_workflow(storage, args).await,
        "r8r_generate" => exec_generate(args, llm_config).await,
        "r8r_test" => exec_test(args).await,
        "r8r_list_approvals" => exec_list_approvals(storage, args).await,
        "r8r_approve" => exec_approve(storage, args).await,
        _ => json!({"error": format!("Unknown tool: {}", name)}).to_string(),
    }
}

async fn exec_list_workflows(storage: &dyn Storage) -> String {
    match storage.list_workflows().await {
        Ok(workflows) => {
            let list: Vec<Value> = workflows
                .iter()
                .map(|wf| {
                    let desc = parse_workflow(&wf.definition)
                        .map(|w| w.description)
                        .unwrap_or_default();
                    json!({"name": wf.name, "enabled": wf.enabled, "description": desc})
                })
                .collect();
            json!({"workflows": list, "count": list.len()}).to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_get_workflow(storage: &dyn Storage, args: &Value) -> String {
    let name = args["name"].as_str().unwrap_or("");
    match storage.get_workflow(name).await {
        Ok(Some(wf)) => {
            json!({"name": wf.name, "definition": wf.definition, "enabled": wf.enabled}).to_string()
        }
        Ok(None) => json!({"error": format!("Workflow not found: {}", name)}).to_string(),
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_execute(storage: &dyn Storage, executor: &Arc<Executor>, args: &Value) -> String {
    let workflow_name = args["workflow"].as_str().unwrap_or("");
    let input = args.get("input").cloned().unwrap_or(Value::Null);

    let stored = match storage.get_workflow(workflow_name).await {
        Ok(Some(wf)) => wf,
        Ok(None) => {
            return json!({"error": format!("Workflow not found: {}", workflow_name)}).to_string()
        }
        Err(e) => return json!({"error": e.to_string()}).to_string(),
    };

    let workflow = match parse_workflow(&stored.definition) {
        Ok(wf) => wf,
        Err(e) => return json!({"error": format!("Invalid workflow: {}", e)}).to_string(),
    };

    match executor.execute(&workflow, &stored.id, "repl", input).await {
        Ok(execution) => {
            let duration_ms = execution
                .finished_at
                .map(|f| (f - execution.started_at).num_milliseconds());
            json!({
                "execution_id": execution.id,
                "status": execution.status.to_string(),
                "output": execution.output,
                "error": execution.error,
                "duration_ms": duration_ms
            })
            .to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_run_and_wait(
    storage: &dyn Storage,
    executor: &Arc<Executor>,
    args: &Value,
) -> String {
    let result = exec_execute(storage, executor, args).await;
    let v: Value = serde_json::from_str(&result).unwrap_or(json!({"error": "parse failed"}));
    if let Some(output) = v.get("output") {
        output.to_string()
    } else {
        result
    }
}

async fn exec_discover(storage: &dyn Storage, args: &Value) -> String {
    let name = args["workflow"].as_str().unwrap_or("");
    match storage.get_workflow(name).await {
        Ok(Some(wf)) => match parse_workflow(&wf.definition) {
            Ok(workflow) => {
                let nodes: Vec<Value> = workflow
                    .nodes
                    .iter()
                    .map(|n| json!({"id": n.id, "type": n.node_type, "depends_on": n.depends_on}))
                    .collect();
                json!({
                    "name": workflow.name,
                    "description": workflow.description,
                    "nodes": nodes,
                    "triggers": workflow.triggers
                })
                .to_string()
            }
            Err(e) => json!({"error": e.to_string()}).to_string(),
        },
        Ok(None) => json!({"error": format!("Workflow not found: {}", name)}).to_string(),
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_get_execution(storage: &dyn Storage, args: &Value) -> String {
    let exec_id = args["execution_id"].as_str().unwrap_or("");
    match storage.get_execution(exec_id).await {
        Ok(Some(exec)) => json!({
            "execution_id": exec.id,
            "workflow": exec.workflow_name,
            "status": exec.status.to_string(),
            "output": exec.output,
            "error": exec.error,
            "started_at": exec.started_at.to_rfc3339()
        })
        .to_string(),
        Ok(None) => json!({"error": format!("Execution not found: {}", exec_id)}).to_string(),
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_list_executions(storage: &dyn Storage, args: &Value) -> String {
    let workflow = args["workflow"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(10) as usize;
    match storage.list_executions(workflow, limit).await {
        Ok(execs) => {
            let list: Vec<Value> = execs
                .iter()
                .map(|e| {
                    json!({
                        "execution_id": e.id,
                        "status": e.status.to_string(),
                        "started_at": e.started_at.to_rfc3339()
                    })
                })
                .collect();
            json!({"executions": list, "count": list.len()}).to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_get_trace(storage: &dyn Storage, args: &Value) -> String {
    let exec_id = args["execution_id"].as_str().unwrap_or("");
    match storage.get_node_executions(exec_id).await {
        Ok(nodes) => {
            let list: Vec<Value> = nodes
                .iter()
                .map(|n| {
                    json!({
                        "node_id": n.node_id,
                        "status": n.status.to_string(),
                        "started_at": n.started_at.to_rfc3339(),
                        "output": n.output,
                        "error": n.error
                    })
                })
                .collect();
            json!({"nodes": list}).to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_validate(args: &Value) -> String {
    let yaml = args["yaml"].as_str().unwrap_or("");
    match parse_workflow(yaml) {
        Ok(workflow) => match validate_workflow(&workflow) {
            Ok(()) => json!({"valid": true, "name": workflow.name}).to_string(),
            Err(e) => json!({"valid": false, "errors": [e.to_string()]}).to_string(),
        },
        Err(e) => json!({"valid": false, "errors": [format!("Parse error: {}", e)]}).to_string(),
    }
}

async fn exec_create_workflow(storage: &dyn Storage, args: &Value) -> String {
    let yaml = args["yaml"].as_str().unwrap_or("");
    match parse_workflow(yaml) {
        Ok(workflow) => {
            let now = chrono::Utc::now();
            let stored = crate::storage::StoredWorkflow {
                id: uuid::Uuid::new_v4().to_string(),
                name: workflow.name.clone(),
                definition: yaml.to_string(),
                enabled: true,
                created_at: now,
                updated_at: now,
            };
            match storage.save_workflow(&stored).await {
                Ok(()) => json!({"created": true, "name": workflow.name}).to_string(),
                Err(e) => json!({"error": e.to_string()}).to_string(),
            }
        }
        Err(e) => json!({"error": format!("Invalid YAML: {}", e)}).to_string(),
    }
}

async fn exec_generate(args: &Value, llm_config: Option<&LlmConfig>) -> String {
    let prompt = args["prompt"].as_str().unwrap_or("");

    let generated = if let Some(cfg) = llm_config {
        crate::generator::generate_with_config(cfg, prompt).await
    } else {
        crate::generator::generate(prompt).await
    };

    match generated {
        Ok(generated) => json!({
            "yaml": generated.yaml,
            "summary": generated.summary,
            "valid": generated.valid,
            "errors": generated.errors
        })
        .to_string(),
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_test(args: &Value) -> String {
    let yaml = args["workflow_yaml"].as_str().unwrap_or("");

    match parse_workflow(yaml) {
        Ok(workflow) => match validate_workflow(&workflow) {
            Ok(()) => {
                json!({"valid": true, "name": workflow.name, "message": "Workflow is valid and ready to test"})
                    .to_string()
            }
            Err(e) => json!({"valid": false, "errors": [e.to_string()]}).to_string(),
        },
        Err(e) => json!({"error": format!("Invalid YAML: {}", e)}).to_string(),
    }
}

async fn exec_list_approvals(storage: &dyn Storage, args: &Value) -> String {
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("pending");
    match storage.list_approval_requests(Some(status), 100, 0).await {
        Ok(approvals) => {
            let list: Vec<Value> = approvals
                .iter()
                .map(|a| json!({"id": a.id, "workflow": a.workflow_name, "title": a.title, "status": a.status}))
                .collect();
            json!({"approvals": list, "count": list.len()}).to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}

async fn exec_approve(storage: &dyn Storage, args: &Value) -> String {
    let approval_id = args["approval_id"].as_str().unwrap_or("");
    let decision = args["decision"].as_str().unwrap_or("");
    let comment = args.get("comment").and_then(|v| v.as_str());

    let mut approval = match storage.get_approval_request(approval_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return json!({"error": format!("Approval request not found: {}", approval_id)})
                .to_string()
        }
        Err(e) => return json!({"error": e.to_string()}).to_string(),
    };

    if approval.status != "pending" {
        return json!({"error": format!("Approval {} is already {}", approval_id, approval.status)}).to_string();
    }

    approval.status = if decision == "approve" {
        "approved".to_string()
    } else {
        "rejected".to_string()
    };
    approval.decision_comment = comment.map(|c| c.to_string());
    approval.decided_by = Some("repl".to_string());
    approval.decided_at = Some(chrono::Utc::now());

    match storage.save_approval_request(&approval).await {
        Ok(()) => {
            json!({"approved": decision == "approve", "approval_id": approval_id, "status": approval.status})
                .to_string()
        }
        Err(e) => json!({"error": e.to_string()}).to_string(),
    }
}
