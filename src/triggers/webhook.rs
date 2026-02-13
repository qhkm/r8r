//! Webhook trigger handler.
//!
//! Handles HTTP webhook triggers for workflows.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::api::AppState;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, Trigger};

/// Create webhook routes for all workflows with webhook triggers.
pub async fn create_webhook_routes(
    storage: SqliteStorage,
    _registry: Arc<NodeRegistry>,
) -> Router<AppState> {
    let mut router = Router::new();

    // Load workflows and register webhook routes
    let workflows = match storage.list_workflows().await {
        Ok(w) => w,
        Err(e) => {
            error!("Failed to load workflows for webhook routes: {}", e);
            return router;
        }
    };

    let mut webhook_count = 0;

    for stored in workflows {
        if !stored.enabled {
            continue;
        }

        let workflow = match parse_workflow(&stored.definition) {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "Failed to parse workflow '{}' for webhooks: {}",
                    stored.name, e
                );
                continue;
            }
        };

        for trigger in &workflow.triggers {
            if let Trigger::Webhook { path, method } = trigger {
                let route_path = path
                    .clone()
                    .unwrap_or_else(|| format!("/webhooks/{}", workflow.name));

                let route_method = method.as_deref().unwrap_or("POST").to_uppercase();

                let workflow_name = workflow.name.clone();
                let workflow_id = stored.id.clone();

                // Create handler for this webhook
                let handler = move |State(state): State<AppState>,
                                    method: Method,
                                    headers: HeaderMap,
                                    body: Bytes| {
                    let workflow_name = workflow_name.clone();
                    let workflow_id = workflow_id.clone();
                    async move {
                        handle_webhook(state, &workflow_name, &workflow_id, method, headers, body)
                            .await
                    }
                };

                // Register route based on method
                router = match route_method.as_str() {
                    "GET" => router.route(&route_path, get(handler)),
                    "POST" => router.route(&route_path, post(handler)),
                    "PUT" => router.route(&route_path, axum::routing::put(handler)),
                    "DELETE" => router.route(&route_path, axum::routing::delete(handler)),
                    "PATCH" => router.route(&route_path, axum::routing::patch(handler)),
                    _ => router.route(&route_path, any(handler)),
                };

                info!("Registered webhook: {} {}", route_method, route_path);
                webhook_count += 1;
            }
        }
    }

    info!("Registered {} webhook route(s)", webhook_count);
    router
}

/// Handle an incoming webhook request.
async fn handle_webhook(
    state: AppState,
    workflow_name: &str,
    workflow_id: &str,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    info!("Webhook triggered for workflow '{}'", workflow_name);

    // Get workflow
    let stored = match state.storage.get_workflow(workflow_name).await {
        Ok(Some(w)) => w,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Workflow '{}' not found", workflow_name)})),
            )
                .into_response();
        }
        Err(e) => {
            error!("Failed to load workflow '{}': {}", workflow_name, e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
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
                .into_response();
        }
    };

    // Build input from request
    let body_value: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => {
                // Treat as raw string if not valid JSON
                Value::String(String::from_utf8_lossy(&body).to_string())
            }
        }
    };

    // Extract headers as JSON object
    let headers_value: Value = {
        let mut map = serde_json::Map::new();
        for (key, value) in headers.iter() {
            if let Ok(v) = value.to_str() {
                map.insert(key.to_string(), Value::String(v.to_string()));
            }
        }
        Value::Object(map)
    };

    let input = json!({
        "method": method.to_string(),
        "headers": headers_value,
        "body": body_value,
    });

    // Execute workflow
    let mut executor = Executor::new((*state.registry).clone(), state.storage.clone());
    if let Some(monitor) = state.monitor.clone() {
        executor = executor.with_monitor(monitor);
    }

    match executor
        .execute(&workflow, workflow_id, "webhook", input)
        .await
    {
        Ok(execution) => {
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
        Err(e) => {
            error!("Webhook execution failed for '{}': {}", workflow_name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    }
}
