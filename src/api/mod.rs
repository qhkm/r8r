//! HTTP API server for r8r.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::parse_workflow;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub storage: SqliteStorage,
    pub registry: Arc<NodeRegistry>,
}

/// Create the API router.
pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/health", get(health_check))
        .route("/api/workflows", get(list_workflows))
        .route("/api/workflows/{name}", get(get_workflow))
        .route("/api/workflows/{name}/execute", post(execute_workflow))
        .route("/api/executions/{id}", get(get_execution))
        .route("/api/executions/{id}/trace", get(get_execution_trace))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

// ============================================================================
// Health Check
// ============================================================================

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    match state.storage.check_health().await {
        Ok(health) => Json(json!({
            "status": "ok",
            "foreign_keys_enabled": health.foreign_keys_enabled,
            "integrity_check": health.integrity_check,
            "orphaned_executions": health.orphaned_executions,
            "orphaned_node_executions": health.orphaned_node_executions,
            "orphaned_workflow_versions": health.orphaned_workflow_versions,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "message": e.to_string()})),
        )
            .into_response(),
    }
}

// ============================================================================
// Workflow Endpoints
// ============================================================================

#[derive(Serialize)]
struct WorkflowResponse {
    id: String,
    name: String,
    enabled: bool,
    created_at: String,
    updated_at: String,
    node_count: usize,
    trigger_count: usize,
}

async fn list_workflows(State(state): State<AppState>) -> impl IntoResponse {
    match state.storage.list_workflows().await {
        Ok(workflows) => {
            let responses: Vec<WorkflowResponse> = workflows
                .into_iter()
                .filter_map(|w| {
                    let parsed = parse_workflow(&w.definition).ok()?;
                    Some(WorkflowResponse {
                        id: w.id,
                        name: w.name,
                        enabled: w.enabled,
                        created_at: w.created_at.to_rfc3339(),
                        updated_at: w.updated_at.to_rfc3339(),
                        node_count: parsed.nodes.len(),
                        trigger_count: parsed.triggers.len(),
                    })
                })
                .collect();
            Json(json!({"workflows": responses})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.storage.get_workflow(&name).await {
        Ok(Some(w)) => {
            let parsed = parse_workflow(&w.definition);
            let (node_count, trigger_count) = match &parsed {
                Ok(p) => (p.nodes.len(), p.triggers.len()),
                Err(_) => (0, 0),
            };
            Json(json!({
                "id": w.id,
                "name": w.name,
                "enabled": w.enabled,
                "created_at": w.created_at.to_rfc3339(),
                "updated_at": w.updated_at.to_rfc3339(),
                "node_count": node_count,
                "trigger_count": trigger_count,
                "definition": w.definition,
            }))
            .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Workflow '{}' not found", name)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ============================================================================
// Execution Endpoints
// ============================================================================

#[derive(Deserialize)]
struct ExecuteRequest {
    #[serde(default)]
    input: Value,
    /// If true, wait for execution to complete before returning (future use).
    #[serde(default)]
    #[allow(dead_code)]
    wait: bool,
}

#[derive(Serialize)]
struct ExecuteResponse {
    execution_id: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    duration_ms: Option<i64>,
}

async fn execute_workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<ExecuteRequest>,
) -> impl IntoResponse {
    // Get workflow
    let stored = match state.storage.get_workflow(&name).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Workflow '{}' not found", name)})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };

    // Parse workflow
    let workflow = match parse_workflow(&stored.definition) {
        Ok(w) => w,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid workflow definition: {}", e)})),
            )
                .into_response()
        }
    };

    // Create executor
    let executor = Executor::new((*state.registry).clone(), state.storage.clone());

    // Execute
    let input = if request.input.is_null() {
        json!({})
    } else {
        request.input
    };

    let result = executor.execute(&workflow, &stored.id, "api", input).await;

    match result {
        Ok(execution) => {
            let duration_ms = execution
                .finished_at
                .map(|f| (f - execution.started_at).num_milliseconds());

            Json(ExecuteResponse {
                execution_id: execution.id,
                status: execution.status.to_string(),
                output: execution.output,
                error: execution.error,
                duration_ms,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_execution(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.storage.get_execution(&id).await {
        Ok(Some(e)) => {
            let duration_ms = e.finished_at.map(|f| (f - e.started_at).num_milliseconds());

            Json(json!({
                "id": e.id,
                "workflow_id": e.workflow_id,
                "workflow_name": e.workflow_name,
                "status": e.status.to_string(),
                "trigger_type": e.trigger_type,
                "input": e.input,
                "output": e.output,
                "error": e.error,
                "started_at": e.started_at.to_rfc3339(),
                "finished_at": e.finished_at.map(|f| f.to_rfc3339()),
                "duration_ms": duration_ms,
            }))
            .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Execution '{}' not found", id)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_execution_trace(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.storage.get_execution_trace(&id).await {
        Ok(Some(trace)) => {
            let nodes: Vec<Value> = trace
                .nodes
                .into_iter()
                .map(|n| {
                    let duration_ms = n.finished_at.map(|f| (f - n.started_at).num_milliseconds());
                    json!({
                        "node_id": n.node_id,
                        "status": n.status.to_string(),
                        "started_at": n.started_at.to_rfc3339(),
                        "finished_at": n.finished_at.map(|f| f.to_rfc3339()),
                        "duration_ms": duration_ms,
                        "error": n.error,
                    })
                })
                .collect();

            Json(json!({
                "execution_id": trace.execution.id,
                "workflow_name": trace.execution.workflow_name,
                "status": trace.execution.status.to_string(),
                "nodes": nodes,
            }))
            .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Execution '{}' not found", id)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
