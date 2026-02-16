//! HTTP API server for r8r.

mod middleware;
mod openapi;
mod webhook;
mod websocket;

pub use middleware::{
    access_log_middleware, api_auth_middleware, health_auth_middleware, request_id_middleware,
    ApiAuthConfig, HealthAuthConfig, RequestId, REQUEST_ID_HEADER,
};
pub use openapi::generate_openapi_spec;
pub use webhook::{compute_signature, verify_signature, SignatureConfig, SignatureScheme};

use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, warn};

use crate::engine::{Executor, PauseRegistry};
use crate::error::Error;
use crate::nodes::NodeRegistry;
use crate::shutdown::ShutdownCoordinator;
use crate::storage::{ExecutionStatus, SqliteStorage};
use crate::workflow::parse_workflow;

/// Create a sanitized error response for external consumers.
///
/// This logs the full error internally but returns only safe information
/// to external clients to prevent information leakage.
fn external_error_response(e: Error) -> (StatusCode, Json<Value>) {
    // Log full error for debugging
    error!("API error: {:?}", e);

    // Return sanitized message to client
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": e.external_message()})),
    )
}

pub use websocket::{Monitor, MonitorEvent, MonitoredAppState};

/// Create CORS layer based on environment configuration.
///
/// This is exported for use by the main server.
///
/// - R8R_CORS_ORIGINS: Comma-separated list of allowed origins (default: http://localhost:3000)
/// - R8R_CORS_ALLOW_ALL: Set to "true" to allow all origins (NOT recommended for production)
pub fn create_cors_layer() -> CorsLayer {
    let allow_all = std::env::var("R8R_CORS_ALLOW_ALL")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    if allow_all {
        warn!("CORS configured to allow all origins - this is NOT secure for production!");
        return CorsLayer::very_permissive();
    }

    let origins_str =
        std::env::var("R8R_CORS_ORIGINS").unwrap_or_else(|_| "http://localhost:3000".to_string());

    let origins: Vec<HeaderValue> = origins_str
        .split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }
            match trimmed.parse::<HeaderValue>() {
                Ok(hv) => Some(hv),
                Err(e) => {
                    warn!("Invalid CORS origin '{}': {}", trimmed, e);
                    None
                }
            }
        })
        .collect();

    if origins.is_empty() {
        warn!("No valid CORS origins configured, using localhost:3000");
        CorsLayer::new()
            .allow_origin("http://localhost:3000".parse::<HeaderValue>().unwrap())
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    } else {
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([
                axum::http::header::CONTENT_TYPE,
                axum::http::header::AUTHORIZATION,
            ])
    }
}

/// Default maximum concurrent requests.
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 100;
/// Default maximum request body size in bytes (1 MiB).
const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 1_048_576;

/// Get the maximum concurrent requests limit from environment.
///
/// - R8R_MAX_CONCURRENT_REQUESTS: Maximum concurrent requests (default: 100)
pub fn get_max_concurrent_requests() -> usize {
    std::env::var("R8R_MAX_CONCURRENT_REQUESTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT_REQUESTS)
}

/// Create a concurrency limit layer to prevent resource exhaustion.
pub fn create_concurrency_limit() -> tower::limit::ConcurrencyLimitLayer {
    let max = get_max_concurrent_requests();
    tower::limit::ConcurrencyLimitLayer::new(max)
}

/// Get the maximum request body size limit from environment.
///
/// - R8R_MAX_REQUEST_BODY_BYTES: Maximum request body size in bytes (default: 1048576)
pub fn get_max_request_body_bytes() -> usize {
    std::env::var("R8R_MAX_REQUEST_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_REQUEST_BODY_BYTES)
}

/// Create request body size limit layer to prevent memory exhaustion.
pub fn create_request_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(get_max_request_body_bytes())
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub storage: SqliteStorage,
    pub registry: Arc<NodeRegistry>,
    pub monitor: Option<Arc<Monitor>>,
    pub shutdown: Arc<ShutdownCoordinator>,
    pub pause_registry: PauseRegistry,
}

impl AppState {
    /// Create a new executor with the current state.
    pub fn create_executor(&self) -> Executor {
        let mut executor = Executor::new(
            (*self.registry).clone(),
            self.storage.clone(),
        );
        if let Some(monitor) = self.monitor.clone() {
            executor = executor.with_monitor(monitor);
        }
        executor = executor.with_pause_registry(self.pause_registry.clone());
        executor
    }
}

/// Create the API router (without state applied - call with_state on the result).
pub fn create_api_routes() -> Router<AppState> {
    Router::new()
        .route("/api/health", get(health_check))
        .route("/api/metrics", get(prometheus_metrics))
        .route("/api/openapi.json", get(openapi_spec))
        .route("/api/workflows", get(list_workflows))
        .route("/api/workflows/{name}", get(get_workflow))
        .route("/api/workflows/{name}/execute", post(execute_workflow))
        .route("/api/executions", get(list_executions))
        .route("/api/executions/{id}", get(get_execution))
        .route("/api/executions/{id}/trace", get(get_execution_trace))
        .route("/api/executions/{id}/pause", post(pause_execution_handler))
        .route("/api/executions/{id}/resume", post(resume_execution_handler))
}

/// Create routes that require the monitored app state (WebSocket).
pub fn create_monitored_routes() -> Router<MonitoredAppState> {
    Router::new().route("/api/monitor", get(websocket::ws_handler))
}

/// Create the complete API router with state.
///
/// Middleware stack (bottom to top execution order):
/// 1. CORS - Handle cross-origin requests
/// 2. Request ID - Generate/propagate X-Request-ID
/// 3. Access Log - Log requests in structured JSON format
/// 4. Health Auth - Optional authentication for health endpoints
/// 5. API Auth - Optional API key authentication for all API endpoints
/// 6. Tracing - OpenTelemetry-compatible request tracing
/// 7. Concurrency - Limit concurrent requests
/// 8. Body Limit - Limit request body size
pub fn create_router(state: AppState) -> Router {
    let health_auth_config = HealthAuthConfig::default();
    let api_auth_config = ApiAuthConfig::default();

    create_api_routes()
        .layer(create_request_body_limit())
        .layer(create_concurrency_limit())
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn_with_state(
            api_auth_config,
            api_auth_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            health_auth_config,
            health_auth_middleware,
        ))
        .layer(axum::middleware::from_fn(access_log_middleware))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(create_cors_layer())
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
            "journal_mode": health.journal_mode,
            "busy_timeout_ms": health.busy_timeout_ms,
        }))
        .into_response(),
        Err(e) => {
            error!("Health check failed: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"status": "error", "message": "Health check failed"})),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Prometheus Metrics
// ============================================================================

async fn prometheus_metrics() -> impl IntoResponse {
    use axum::http::header::CONTENT_TYPE;

    let metrics = crate::metrics::render_metrics();
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        metrics,
    )
}

// ============================================================================
// OpenAPI Specification
// ============================================================================

async fn openapi_spec() -> impl IntoResponse {
    Json(openapi::generate_openapi_spec())
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
        Err(e) => external_error_response(e).into_response(),
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
        Err(e) => external_error_response(e).into_response(),
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

#[derive(Deserialize)]
struct ListExecutionsQuery {
    #[serde(default)]
    workflow: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default = "default_exec_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_exec_limit() -> usize {
    50
}

async fn list_executions(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<ListExecutionsQuery>,
) -> impl IntoResponse {
    use crate::storage::{ExecutionQuery, ExecutionStatus};

    let status_filter = params
        .status
        .and_then(|s| s.parse::<ExecutionStatus>().ok());

    let query = ExecutionQuery {
        workflow_name: params.workflow,
        status: status_filter,
        trigger_type: params.trigger,
        search: None,
        started_after: None,
        started_before: None,
        limit: params.limit,
        offset: params.offset,
    };

    match state.storage.query_executions(&query).await {
        Ok(executions) => {
            let items: Vec<Value> = executions
                .into_iter()
                .map(|e| {
                    let duration_ms = e.finished_at.map(|f| (f - e.started_at).num_milliseconds());
                    json!({
                        "id": e.id,
                        "workflow_id": e.workflow_id,
                        "workflow_name": e.workflow_name,
                        "status": e.status.to_string(),
                        "trigger_type": e.trigger_type,
                        "started_at": e.started_at.to_rfc3339(),
                        "finished_at": e.finished_at.map(|f| f.to_rfc3339()),
                        "duration_ms": duration_ms,
                        "error": e.error,
                    })
                })
                .collect();
            Json(json!({"executions": items})).into_response()
        }
        Err(e) => external_error_response(e).into_response(),
    }
}

async fn execute_workflow(
    State(state): State<AppState>,
    Path(name): Path<String>,
    request_id: Option<axum::Extension<RequestId>>,
    Json(request): Json<ExecuteRequest>,
) -> impl IntoResponse {
    let _wait_requested = request.wait;
    let correlation_id = request_id.map(|r| r.0 .0.clone());

    // Check if shutdown is requested
    if state.shutdown.is_shutdown_requested() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "Server is shutting down, cannot start new executions"})),
        )
            .into_response();
    }

    // Log execution start with correlation
    if let Some(ref id) = correlation_id {
        tracing::info!(request_id = %id, workflow = %name, "Starting workflow execution");
    }

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
        Err(e) => return external_error_response(e).into_response(),
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
    let executor = state.create_executor();

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
        Err(e) => external_error_response(e).into_response(),
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
        Err(e) => external_error_response(e).into_response(),
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
        Err(e) => external_error_response(e).into_response(),
    }
}

// ============================================================================
// Pause/Resume Endpoints
// ============================================================================

/// Pause a running execution.
///
/// Returns 200 if the execution was paused successfully.
/// Returns 404 if the execution is not found.
/// Returns 409 if the execution is not in a running state.
async fn pause_execution_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // First check if execution exists
    let execution = match state.storage.get_execution(&id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Execution '{}' not found", id)})),
            )
                .into_response();
        }
        Err(e) => return external_error_response(e).into_response(),
    };

    // Check if execution is in a pausable state (running)
    if execution.status != ExecutionStatus::Running {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!(
                    "Cannot pause execution '{}': status is '{}', expected 'running'",
                    id, execution.status
                )
            })),
        )
            .into_response();
    }

    // Create executor and pause the execution
    let executor = state.create_executor();

    match executor.pause_execution(&id).await {
        Ok(paused_execution) => {
            let duration_ms = paused_execution
                .finished_at
                .map(|f| (f - paused_execution.started_at).num_milliseconds());

            Json(json!({
                "execution_id": paused_execution.id,
                "status": paused_execution.status.to_string(),
                "workflow_name": paused_execution.workflow_name,
                "started_at": paused_execution.started_at.to_rfc3339(),
                "finished_at": paused_execution.finished_at.map(|f| f.to_rfc3339()),
                "duration_ms": duration_ms,
                "message": "Execution paused successfully",
            }))
            .into_response()
        }
        Err(Error::Execution(msg)) if msg.contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": msg})),
        )
            .into_response(),
        Err(Error::Execution(msg)) if msg.contains("status is") => (
            StatusCode::CONFLICT,
            Json(json!({"error": msg})),
        )
            .into_response(),
        Err(e) => external_error_response(e).into_response(),
    }
}

/// Resume a paused execution from its checkpoint.
///
/// Returns 200 if the execution was resumed successfully.
/// Returns 404 if the execution is not found.
/// Returns 409 if the execution is not in a paused state.
async fn resume_execution_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Check if shutdown is requested
    if state.shutdown.is_shutdown_requested() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "Server is shutting down, cannot resume executions"})),
        )
            .into_response();
    }

    // First check if execution exists
    let execution = match state.storage.get_execution(&id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Execution '{}' not found", id)})),
            )
                .into_response();
        }
        Err(e) => return external_error_response(e).into_response(),
    };

    // Check if execution is in a resumable state (paused)
    if execution.status != ExecutionStatus::Paused {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!(
                    "Cannot resume execution '{}': status is '{}', expected 'paused'",
                    id, execution.status
                )
            })),
        )
            .into_response();
    }

    // Create executor and resume the execution
    let executor = state.create_executor();

    match executor.resume_from_checkpoint(&id).await {
        Ok(resumed_execution) => {
            let duration_ms = resumed_execution
                .finished_at
                .map(|f| (f - resumed_execution.started_at).num_milliseconds());

            Json(json!({
                "execution_id": resumed_execution.id,
                "status": resumed_execution.status.to_string(),
                "workflow_name": resumed_execution.workflow_name,
                "started_at": resumed_execution.started_at.to_rfc3339(),
                "finished_at": resumed_execution.finished_at.map(|f| f.to_rfc3339()),
                "duration_ms": duration_ms,
                "message": "Execution resumed successfully",
            }))
            .into_response()
        }
        Err(Error::Execution(msg)) if msg.contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": msg})),
        )
            .into_response(),
        Err(Error::Execution(msg)) if msg.contains("No checkpoint found") => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": msg})),
        )
            .into_response(),
        Err(Error::Execution(msg)) if msg.contains("status is") => (
            StatusCode::CONFLICT,
            Json(json!({"error": msg})),
        )
            .into_response(),
        Err(e) => external_error_response(e).into_response(),
    }
}
