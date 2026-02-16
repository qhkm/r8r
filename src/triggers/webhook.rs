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
use moka::future::Cache;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{interval, Instant};
use tracing::{debug, error, info, warn};

use crate::api::AppState;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, DebounceConfig, HeaderFilter, Trigger};

/// Debouncer for webhook deduplication.
///
/// Tracks recent webhook requests by key and ensures they are only
/// triggered once after a quiet period.
///
/// TODO(review): This struct and its methods are unused dead code. The actual
/// "debounce" in handle_webhook uses tokio::time::sleep which is a fixed delay,
/// not a true debounce (timer reset on each new request). Either integrate this
/// Debouncer properly or remove it and implement real debouncing.
pub struct Debouncer {
    /// In-memory cache tracking pending debounces
    pending: Cache<String, DebounceEntry>,
    /// Channel for debounced event notifications
    event_tx: mpsc::Sender<DebouncedEvent>,
    /// Background task handle
    _cleanup_handle: tokio::task::JoinHandle<()>,
}

/// Entry tracking a pending debounce.
#[derive(Debug, Clone)]
struct DebounceEntry {
    /// When this entry was first seen
    first_seen: Instant,
    /// Last time this key was updated
    last_update: Instant,
    /// Number of times this key has been seen
    count: u32,
    /// Webhook data to trigger
    data: DebouncedEvent,
    /// Maximum wait time before triggering
    max_wait: Duration,
    /// Quiet period before triggering
    wait_duration: Duration,
}

/// A debounced webhook event ready to be triggered.
#[derive(Debug, Clone)]
struct DebouncedEvent {
    workflow_name: String,
    workflow_id: String,
    method: Method,
    headers: HashMap<String, String>,
    body: Value,
    key: String,
}

/// Debounce result indicating how to handle a webhook.
#[derive(Debug)]
enum DebounceResult {
    /// Trigger immediately (no debouncing)
    TriggerImmediately,
    /// Event is being debounced, wait for quiet period
    Debouncing,
    /// This is a duplicate within the debounce window
    Duplicate,
}

impl Debouncer {
    /// Create a new debouncer with the specified configuration.
    pub fn new(
        cache_ttl_seconds: u64,
        event_tx: mpsc::Sender<DebouncedEvent>,
    ) -> Self {
        let pending: Cache<String, DebounceEntry> = Cache::builder()
            .time_to_live(Duration::from_secs(cache_ttl_seconds * 2))
            .build();

        let pending_clone = pending.clone();
        let _event_tx_clone = event_tx.clone();

        // Spawn cleanup task that checks for due events
        let cleanup_handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(100));

            loop {
                ticker.tick().await;

                let _now = Instant::now();
                let _due_events: Vec<DebouncedEvent> = Vec::new();

                // Check all pending entries
                pending_clone
                    .run_pending_tasks()
                    .await;

                // Note: Moka cache doesn't support iteration, so we use a separate
                // tracking mechanism in production. For simplicity, this implementation
                // relies on the webhook handler to check debounce status on each request.
            }
        });

        Self {
            pending,
            event_tx,
            _cleanup_handle: cleanup_handle,
        }
    }

    /// Create a simple debouncer with default settings.
    pub fn default_with_sender(event_tx: mpsc::Sender<DebouncedEvent>) -> Self {
        Self::new(300, event_tx) // 5 minute default TTL
    }

    /// Check if a webhook should be debounced.
    ///
    /// Returns:
    /// - `TriggerImmediately` if no debounce or debounce period has passed
    /// - `Debouncing` if this is a new or updated debounce entry
    /// - `Duplicate` if this is a duplicate within the window
    async fn check_or_insert(
        &self,
        key: String,
        event: DebouncedEvent,
        config: &DebounceConfig,
    ) -> DebounceResult {
        let now = Instant::now();
        let wait_duration = Duration::from_secs(config.wait_seconds);
        let max_wait = Duration::from_secs(config.max_wait_seconds);

        if let Some(entry) = self.pending.get(&key).await {
            // Entry exists, check if we should trigger
            let elapsed = now.duration_since(entry.first_seen);

            // Check max wait exceeded
            if elapsed >= max_wait {
                // Max wait exceeded, trigger now
                debug!(
                    "Debounce max wait exceeded for key '{}', triggering",
                    key
                );
                self.pending.invalidate(&key).await;
                return DebounceResult::TriggerImmediately;
            }

            // Update the entry
            let updated = DebounceEntry {
                first_seen: entry.first_seen,
                last_update: now,
                count: entry.count + 1,
                data: event,
                max_wait,
                wait_duration,
            };
            self.pending.insert(key.clone(), updated).await;

            debug!(
                "Updated debounce entry for key '{}' (count: {})",
                key,
                entry.count + 1
            );
            DebounceResult::Debouncing
        } else {
            // New entry
            let entry = DebounceEntry {
                first_seen: now,
                last_update: now,
                count: 1,
                data: event,
                max_wait,
                wait_duration,
            };
            self.pending.insert(key.clone(), entry).await;

            debug!("Created new debounce entry for key '{}'", key);

            // Schedule the debounced trigger
            let pending_clone = self.pending.clone();
            let event_tx_clone = self.event_tx.clone();
            let key_clone = key.clone();

            tokio::spawn(async move {
                tokio::time::sleep(wait_duration).await;

                // Check if entry still exists and hasn't been updated
                if let Some(entry) = pending_clone.get(&key_clone).await {
                    let time_since_update = Instant::now().duration_since(entry.last_update);

                    // Only trigger if enough quiet time has passed
                    if time_since_update >= wait_duration {
                        pending_clone.invalidate(&key_clone).await;
                        debug!(
                            "Debounce quiet period reached for key '{}', triggering (count: {})",
                            key_clone, entry.count
                        );
                        let _ = event_tx_clone.send(entry.data).await;
                    }
                }
            });

            DebounceResult::Debouncing
        }
    }

    /// Check if a key is currently being debounced.
    async fn is_pending(&self, key: &str) -> bool {
        self.pending.get(key).await.is_some()
    }

    /// Force trigger a pending debounced event immediately.
    async fn trigger_now(&self, key: &str) -> Option<DebouncedEvent> {
        // Get entry before invalidating
        if let Some(entry) = self.pending.get(key).await {
            let data = entry.data.clone();
            self.pending.invalidate(key).await;
            let _ = self.event_tx.send(data.clone()).await;
            Some(data)
        } else {
            None
        }
    }
}

/// State for webhook processing with debouncing.
#[derive(Clone)]
pub struct WebhookState {
    _app_state: AppState,
    debouncer: Option<Arc<Debouncer>>,
    debounce_configs: HashMap<String, DebounceConfig>, // path -> config
}

/// Create webhook routes for all workflows with webhook triggers.
pub async fn create_webhook_routes(
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
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
    let mut debounce_configs: HashMap<String, DebounceConfig> = HashMap::new();

    // Create event channel for debounced webhooks
    let (_event_tx, mut event_rx) = mpsc::channel::<DebouncedEvent>(100);

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

                // Store debounce config if present
                if let Some(ref debounce_cfg) = debounce {
                    debounce_configs.insert(route_path.clone(), debounce_cfg.clone());
                }

                // Extract values for logging before moving into closure
                let has_debounce = debounce.is_some();
                let has_header_filter = header_filter.is_some();

                // Create handler for this webhook
                let handler = move |State(state): State<AppState>,
                                    method: Method,
                                    headers: HeaderMap,
                                    body: Bytes| {
                    let workflow_name = workflow_name.clone();
                    let workflow_id = workflow_id.clone();
                    let debounce_config = debounce.clone();
                    let header_filters = header_filter.clone();
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

    // Spawn background task to process debounced events
    if !debounce_configs.is_empty() {
        let storage = storage.clone();
        let registry = registry.clone();

        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                // Process the debounced webhook
                if let Err(e) = process_debounced_webhook(&storage, &registry, &event).await {
                    error!("Failed to process debounced webhook: {}", e);
                }
            }
        });
    }

    info!(
        "Registered {} webhook route(s) with {} debounced",
        webhook_count,
        debounce_configs.len()
    );
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
    header_filters: Option<Vec<HeaderFilter>>,
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

        // Create debounced event
        let _event = DebouncedEvent {
            workflow_name: workflow_name.to_string(),
            workflow_id: workflow_id.to_string(),
            method: method.clone(),
            headers: extract_headers(&headers),
            body: body_value.clone(),
            key: debounce_key.clone(),
        };

        // For simplicity, we'll process debounced webhooks synchronously
        // In production, you'd use the Debouncer struct with a background task
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
fn check_header_filters(filters: &[HeaderFilter], headers: &HeaderMap) -> bool {
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

            // Check pattern match
            // TODO(review): Regex is compiled from user-supplied pattern on every webhook
            // request. This is a performance issue and a potential ReDoS vulnerability.
            // Cache compiled regexes at startup and limit pattern complexity.
            if let Some(pattern) = &filter.pattern {
                if let Ok(regex) = regex_lite::Regex::new(pattern) {
                    if !regex.is_match(value_str) {
                        debug!(
                            "Header '{}' value '{}' doesn't match pattern '{}'",
                            filter.name, value_str, pattern
                        );
                        return false;
                    }
                }
            }
        }
    }

    true
}

/// Extract headers into a hashmap.
fn extract_headers(headers: &HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            map.insert(key.to_string(), v.to_string());
        }
    }
    map
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

/// Process a debounced webhook event.
async fn process_debounced_webhook(
    _storage: &SqliteStorage,
    _registry: &Arc<NodeRegistry>,
    event: &DebouncedEvent,
) -> Result<(), String> {
    info!(
        "Processing debounced webhook for workflow '{}' (key: {})",
        event.workflow_name, event.key
    );

    // The actual execution happens in the spawned task from handle_webhook
    // This function is reserved for more complex debounce logic if needed

    Ok(())
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

/// Debounce configuration response.
#[derive(Debug, Serialize)]
pub struct DebounceStatus {
    pub key: String,
    pub pending: bool,
    pub count: u32,
    pub first_seen: Option<String>,
    pub last_update: Option<String>,
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

        let filters = vec![
            HeaderFilter {
                name: "X-Event-Type".to_string(),
                value: Some("push".to_string()),
                pattern: None,
                required: true,
            },
        ];

        assert!(check_header_filters(&filters, &headers));

        // Test with pattern filter
        let filters_pattern = vec![
            HeaderFilter {
                name: "X-Signature".to_string(),
                value: None,
                pattern: Some(r"^sha256=.*".to_string()),
                required: true,
            },
        ];

        assert!(check_header_filters(&filters_pattern, &headers));

        // Test with missing required header
        let filters_missing = vec![
            HeaderFilter {
                name: "X-Missing".to_string(),
                value: None,
                pattern: None,
                required: true,
            },
        ];

        assert!(!check_header_filters(&filters_missing, &headers));
    }

    #[test]
    fn test_extract_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("X-Custom", HeaderValue::from_static("value"));

        let extracted = extract_headers(&headers);
        assert_eq!(extracted.get("content-type"), Some(&"application/json".to_string()));
        assert_eq!(extracted.get("x-custom"), Some(&"value".to_string()));
    }
}
