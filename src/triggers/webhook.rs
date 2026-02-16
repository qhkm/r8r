//! Webhook trigger handler.
//!
//! Handles HTTP webhook triggers for workflows with debouncing support.

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
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::api::AppState;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, DebounceConfig, HeaderFilter, Trigger};

/// Maximum allowed length for header filter regex patterns (ReDoS guard).
const MAX_HEADER_PATTERN_LEN: usize = 128;

/// A header filter with pre-compiled regex pattern.
#[derive(Debug, Clone)]
struct CompiledHeaderFilter {
    name: String,
    value: Option<String>,
    compiled_pattern: Option<regex_lite::Regex>,
    required: bool,
}

impl CompiledHeaderFilter {
    /// Compile a HeaderFilter, rejecting patterns that are too long.
    fn compile(filter: &HeaderFilter) -> Result<Self, String> {
        let compiled_pattern = match &filter.pattern {
            Some(pattern) => {
                if pattern.len() > MAX_HEADER_PATTERN_LEN {
                    return Err(format!(
                        "Header filter pattern for '{}' exceeds {} chars (ReDoS guard)",
                        filter.name, MAX_HEADER_PATTERN_LEN
                    ));
                }
                Some(regex_lite::Regex::new(pattern).map_err(|e| {
                    format!("Invalid regex for header '{}': {}", filter.name, e)
                })?)
            }
            None => None,
        };

        Ok(Self {
            name: filter.name.clone(),
            value: filter.value.clone(),
            compiled_pattern,
            required: filter.required,
        })
    }
}

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

        for trigger in workflow.triggers.clone() {
            if let Trigger::Webhook {
                path,
                method,
                debounce,
                header_filter,
            } = trigger
            {
                let route_path = path
                    .clone()
                    .unwrap_or_else(|| format!("/webhooks/{}", workflow.name.clone()));

                let route_method = method.as_deref().unwrap_or("POST").to_uppercase();

                let workflow_name = workflow.name.clone();
                let workflow_id = stored.id.clone();

                // Pre-compile header filter regexes at startup
                let compiled_filters: Option<Vec<CompiledHeaderFilter>> =
                    header_filter.as_ref().map(|filters| {
                        filters
                            .iter()
                            .filter_map(|f| match CompiledHeaderFilter::compile(f) {
                                Ok(cf) => Some(cf),
                                Err(e) => {
                                    warn!(
                                        "Skipping invalid header filter for webhook '{}': {}",
                                        workflow.name, e
                                    );
                                    None
                                }
                            })
                            .collect()
                    });

                // Extract values for logging before moving into closure
                let has_debounce = debounce.is_some();
                let has_header_filter = compiled_filters.is_some();

                // Create handler for this webhook
                let handler = move |State(state): State<AppState>,
                                    method: Method,
                                    headers: HeaderMap,
                                    body: Bytes| {
                    let workflow_name = workflow_name.clone();
                    let workflow_id = workflow_id.clone();
                    let debounce_config = debounce.clone();
                    let header_filters = compiled_filters.clone();
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
                        )
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

                info!(
                    "Registered webhook: {} {} (debounce: {}, header_filter: {})",
                    route_method,
                    route_path,
                    has_debounce,
                    has_header_filter
                );
                webhook_count += 1;
            }
        }
    }

    info!("Registered {} webhook route(s)", webhook_count);
    router
}

/// Handle an incoming webhook request.
#[allow(clippy::too_many_arguments)]
async fn handle_webhook(
    state: AppState,
    workflow_name: &str,
    workflow_id: &str,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
    debounce_config: Option<DebounceConfig>,
    header_filters: Option<Vec<CompiledHeaderFilter>>,
) -> impl IntoResponse {
    info!("Webhook triggered for workflow '{}'", workflow_name);

    // Check header filters first
    if let Some(filters) = header_filters {
        if !check_header_filters(&filters, &headers) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Header filter conditions not met"})),
            )
                .into_response();
        }
    }

    // Parse body
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

    // Handle debouncing if configured
    if let Some(config) = debounce_config {
        // Generate debounce key from template
        let debounce_key = generate_debounce_key(&config.key, &body_value, &headers);

        info!(
            "Webhook for '{}' is using debounce (key: {}, wait: {}s)",
            workflow_name, debounce_key, config.wait_seconds
        );

        // Return accepted status and process asynchronously
        let state_clone = state.clone();
        let workflow_name_clone = workflow_name.to_string();
        let workflow_id_clone = workflow_id.to_string();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(config.wait_seconds)).await;

            let _ = execute_webhook_workflow(
                state_clone,
                &workflow_name_clone,
                &workflow_id_clone,
                method,
                headers,
                body_value,
            )
            .await;
        });

        return (
            StatusCode::ACCEPTED,
            Json(json!({
                "status": "accepted",
                "message": "Webhook accepted and will be processed after debounce period"
            })),
        )
            .into_response();
    }

    // Execute workflow immediately
    execute_webhook_workflow(state, workflow_name, workflow_id, method, headers, body_value).await.into_response()
}

/// Check if header filters match the incoming request.
/// Regexes are pre-compiled at startup via CompiledHeaderFilter.
fn check_header_filters(filters: &[CompiledHeaderFilter], headers: &HeaderMap) -> bool {
    for filter in filters {
        let header_value = headers.get(&filter.name);

        if filter.required && header_value.is_none() {
            debug!("Required header '{}' not found", filter.name);
            return false;
        }

        if let Some(value) = header_value {
            let value_str = match value.to_str() {
                Ok(s) => s,
                Err(_) => {
                    debug!("Header '{}' has invalid UTF-8 value", filter.name);
                    return false;
                }
            };

            // Check exact match
            if let Some(expected) = &filter.value {
                if value_str != expected {
                    debug!(
                        "Header '{}' value '{}' doesn't match expected '{}'",
                        filter.name, value_str, expected
                    );
                    return false;
                }
            }

            // Check pre-compiled pattern match
            if let Some(regex) = &filter.compiled_pattern {
                if !regex.is_match(value_str) {
                    debug!(
                        "Header '{}' value '{}' doesn't match pattern",
                        filter.name, value_str
                    );
                    return false;
                }
            }
        }
    }

    true
}

/// Generate a debounce key from the template.
fn generate_debounce_key(template: &str, body: &Value, headers: &HeaderMap) -> String {
    use crate::nodes::template::render_template_simple;

    // Build context for template rendering
    let mut context = serde_json::Map::new();
    context.insert("input".to_string(), body.clone());

    // Add headers to context
    let headers_value: Value = {
        let mut map = serde_json::Map::new();
        for (key, value) in headers.iter() {
            if let Ok(v) = value.to_str() {
                map.insert(key.to_string(), Value::String(v.to_string()));
            }
        }
        Value::Object(map)
    };
    context.insert("headers".to_string(), headers_value);

    let context_value = Value::Object(context);

    // Render the template
    match render_template_simple(template, &context_value) {
        Ok(key) => key,
        Err(_) => {
            // Fallback: use a hash of the template + body
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            template.hash(&mut hasher);
            body.to_string().hash(&mut hasher);
            format!("fallback-{}", hasher.finish())
        }
    }
}

/// Execute the webhook workflow.
async fn execute_webhook_workflow(
    state: AppState,
    workflow_name: &str,
    workflow_id: &str,
    method: Method,
    headers: HeaderMap,
    body_value: Value,
) -> impl IntoResponse {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    #[test]
    fn test_generate_debounce_key() {
        let body = json!({
            "repository": {
                "id": 12345
            }
        });
        let headers = HeaderMap::new();

        let key = generate_debounce_key("{{ input.repository.id }}", &body, &headers);
        assert_eq!(key, "12345");
    }

    #[test]
    fn test_generate_debounce_key_fallback() {
        let body = json!({"test": "value"});
        let headers = HeaderMap::new();

        // Invalid template syntax should use fallback
        let key = generate_debounce_key("{{ invalid", &body, &headers);
        // The template "{{ invalid" contains {{ but no matching data
        // After processing it remains unchanged and should trigger fallback
        assert!(key.starts_with("fallback-"), "Expected fallback key but got: {}", key);
    }

    #[test]
    fn test_check_header_filters() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Event-Type", HeaderValue::from_static("push"));
        headers.insert("X-Signature", HeaderValue::from_static("sha256=abc123"));

        let filters = vec![CompiledHeaderFilter::compile(&HeaderFilter {
            name: "X-Event-Type".to_string(),
            value: Some("push".to_string()),
            pattern: None,
            required: true,
        })
        .unwrap()];

        assert!(check_header_filters(&filters, &headers));

        // Test with pattern filter
        let filters_pattern = vec![CompiledHeaderFilter::compile(&HeaderFilter {
            name: "X-Signature".to_string(),
            value: None,
            pattern: Some(r"^sha256=.*".to_string()),
            required: true,
        })
        .unwrap()];

        assert!(check_header_filters(&filters_pattern, &headers));

        // Test with missing required header
        let filters_missing = vec![CompiledHeaderFilter::compile(&HeaderFilter {
            name: "X-Missing".to_string(),
            value: None,
            pattern: None,
            required: true,
        })
        .unwrap()];

        assert!(!check_header_filters(&filters_missing, &headers));
    }

    #[test]
    fn test_pattern_length_limit() {
        let long_pattern = "a".repeat(MAX_HEADER_PATTERN_LEN + 1);
        let result = CompiledHeaderFilter::compile(&HeaderFilter {
            name: "X-Test".to_string(),
            value: None,
            pattern: Some(long_pattern),
            required: false,
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("ReDoS guard"));
    }

}
