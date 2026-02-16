//! API middleware for request tracing, logging, and authentication.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header::HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::time::Instant;
use tracing::{info, warn, Span};
use uuid::Uuid;

// ============================================================================
// Request ID Middleware
// ============================================================================

/// Header name for request ID.
pub static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Middleware that ensures every request has a unique request ID.
///
/// If the incoming request has an `X-Request-ID` header, it is preserved.
/// Otherwise, a new UUID is generated. The request ID is:
/// - Added to the response headers
/// - Recorded in the tracing span for log correlation
///
/// Environment:
/// - R8R_TRUST_REQUEST_ID: If "true", trust incoming X-Request-ID headers (default: false)
pub async fn request_id_middleware(mut request: Request<Body>, next: Next) -> Response {
    let trust_incoming = std::env::var("R8R_TRUST_REQUEST_ID")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false);

    // Get or generate request ID
    let request_id = if trust_incoming {
        request
            .headers()
            .get(&REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string())
    } else {
        Uuid::new_v4().to_string()
    };

    // Record in current span
    Span::current().record("request_id", &request_id);

    // Add to request extensions for use by handlers
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    // Process request
    let mut response = next.run(request).await;

    // Add request ID to response headers
    if let Ok(header_value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER.clone(), header_value);
    }

    response
}

/// Request ID extension for extracting in handlers.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

// ============================================================================
// Structured Access Logging Middleware
// ============================================================================

/// Middleware that logs each request/response in structured JSON format.
///
/// Logs include:
/// - Request method, path, query string
/// - Response status code
/// - Request duration in milliseconds
/// - Request ID (if present)
/// - User agent
/// - Client IP (from X-Forwarded-For or socket)
///
/// Environment:
/// - R8R_ACCESS_LOG: Set to "true" to enable access logging (default: true)
/// - R8R_ACCESS_LOG_BODY: Set to "true" to log request/response bodies (default: false, NOT recommended)
pub async fn access_log_middleware(request: Request<Body>, next: Next) -> Response {
    let enabled = std::env::var("R8R_ACCESS_LOG")
        .map(|v| v.to_lowercase() != "false")
        .unwrap_or(true);

    if !enabled {
        return next.run(request).await;
    }

    let start = Instant::now();

    // Extract request metadata
    let method = request.method().clone();
    let uri = request.uri().clone();
    let path = uri.path().to_string();
    let query = uri.query().map(|q| q.to_string());

    let user_agent = request
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let client_ip = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string());

    let request_id = request.extensions().get::<RequestId>().map(|r| r.0.clone());

    // Process request
    let response = next.run(request).await;

    let duration_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    // Log in structured format
    info!(
        target: "r8r::access",
        method = %method,
        path = %path,
        query = ?query,
        status = status,
        duration_ms = duration_ms,
        request_id = ?request_id,
        user_agent = ?user_agent,
        client_ip = ?client_ip,
        "request completed"
    );

    response
}

// ============================================================================
// Health Check Authentication Middleware
// ============================================================================

/// Configuration for health check authentication.
#[derive(Clone)]
pub struct HealthAuthConfig {
    /// API key required for health check access. None means no auth required.
    pub api_key: Option<String>,
    /// Paths that require authentication.
    pub protected_paths: Vec<String>,
}

impl Default for HealthAuthConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("R8R_HEALTH_API_KEY").ok(),
            protected_paths: vec!["/api/health".to_string(), "/api/metrics".to_string()],
        }
    }
}

/// Middleware that optionally requires authentication for health/metrics endpoints.
///
/// When R8R_HEALTH_API_KEY is set, requests to /api/health and /api/metrics
/// must include a valid Authorization header:
/// - `Authorization: Bearer <api_key>`
/// - `Authorization: ApiKey <api_key>`
///
/// Environment:
/// - R8R_HEALTH_API_KEY: API key for health endpoints (default: none, no auth required)
pub async fn health_auth_middleware(
    State(config): State<HealthAuthConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // If no API key configured, allow all requests
    let api_key = match &config.api_key {
        Some(key) if !key.is_empty() => key,
        _ => return next.run(request).await,
    };

    let path = request.uri().path();

    // Check if path requires authentication
    let requires_auth = config.protected_paths.iter().any(|p| path.starts_with(p));

    if !requires_auth {
        return next.run(request).await;
    }

    // Extract and validate authorization header
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let is_authorized = match auth_header {
        Some(header) => {
            // Support both "Bearer <key>" and "ApiKey <key>" formats
            let token = header
                .strip_prefix("Bearer ")
                .or_else(|| header.strip_prefix("ApiKey "))
                .unwrap_or("");

            // Use constant-time comparison to prevent timing attacks
            use subtle::ConstantTimeEq;
            token.as_bytes().ct_eq(api_key.as_bytes()).into()
        }
        None => false,
    };

    if is_authorized {
        next.run(request).await
    } else {
        warn!(
            path = %path,
            "Unauthorized access attempt to protected endpoint"
        );
        (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
            "Unauthorized",
        )
            .into_response()
    }
}

// ============================================================================
// API Key Authentication Middleware
// ============================================================================

/// Configuration for API-wide authentication.
#[derive(Clone)]
pub struct ApiAuthConfig {
    /// API key required for API access. None means no auth required.
    pub api_key: Option<String>,
}

impl Default for ApiAuthConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("R8R_API_KEY").ok().filter(|k| !k.is_empty()),
        }
    }
}

/// Public paths that bypass API authentication.
const API_AUTH_PUBLIC_PATHS: &[&str] = &["/api/health", "/api/metrics", "/api/openapi.json"];

/// Middleware that optionally requires authentication for all API endpoints.
///
/// When `R8R_API_KEY` is set, requests to `/api/*` must include a valid
/// Authorization header. Public paths (`/api/health`, `/api/metrics`,
/// `/api/openapi.json`) and non-API paths (webhooks, dashboard) are exempt.
///
/// Supported header formats:
/// - `Authorization: Bearer <api_key>`
/// - `Authorization: ApiKey <api_key>`
///
/// Environment:
/// - R8R_API_KEY: API key for all API endpoints (default: none, no auth required)
pub async fn api_auth_middleware(
    State(config): State<ApiAuthConfig>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // If no API key configured, allow all requests (backwards compatible)
    let api_key = match &config.api_key {
        Some(key) => key,
        None => return next.run(request).await,
    };

    let path = request.uri().path();

    // Skip non-API paths (webhooks, dashboard, static assets)
    if !path.starts_with("/api/") {
        return next.run(request).await;
    }

    // Skip public API paths
    if API_AUTH_PUBLIC_PATHS.iter().any(|p| path.starts_with(p)) {
        return next.run(request).await;
    }

    // Extract and validate authorization header
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let is_authorized = match auth_header {
        Some(header) => {
            let token = header
                .strip_prefix("Bearer ")
                .or_else(|| header.strip_prefix("ApiKey "))
                .unwrap_or("");

            // Use constant-time comparison to prevent timing attacks
            use subtle::ConstantTimeEq;
            token.as_bytes().ct_eq(api_key.as_bytes()).into()
        }
        None => false,
    };

    if is_authorized {
        next.run(request).await
    } else {
        warn!(
            path = %path,
            "Unauthorized API access attempt"
        );
        (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
            "Unauthorized",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get, Router};
    use tower::ServiceExt;

    async fn test_handler() -> &'static str {
        "ok"
    }

    #[tokio::test]
    async fn test_request_id_generated() {
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(axum::middleware::from_fn(request_id_middleware));

        let response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert!(response.headers().contains_key("x-request-id"));
        let request_id = response
            .headers()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(Uuid::parse_str(request_id).is_ok());
    }

    #[tokio::test]
    async fn test_health_auth_no_key_configured() {
        let config = HealthAuthConfig {
            api_key: None,
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config, health_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_auth_valid_bearer_token() {
        let config = HealthAuthConfig {
            api_key: Some("secret123".to_string()),
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config, health_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("Authorization", "Bearer secret123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_auth_valid_apikey_token() {
        let config = HealthAuthConfig {
            api_key: Some("secret123".to_string()),
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config, health_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("Authorization", "ApiKey secret123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_auth_invalid_token() {
        let config = HealthAuthConfig {
            api_key: Some("secret123".to_string()),
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config, health_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("Authorization", "Bearer wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_health_auth_missing_token() {
        let config = HealthAuthConfig {
            api_key: Some("secret123".to_string()),
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config, health_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_health_auth_unprotected_path() {
        let config = HealthAuthConfig {
            api_key: Some("secret123".to_string()),
            protected_paths: vec!["/api/health".to_string()],
        };

        let app = Router::new()
            .route("/api/workflows", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config,
                health_auth_middleware,
            ));

        // No auth required for unprotected paths
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/workflows")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ========================================================================
    // API Auth Middleware Tests
    // ========================================================================

    #[tokio::test]
    async fn test_api_auth_no_key_configured() {
        let config = ApiAuthConfig { api_key: None };

        let app = Router::new()
            .route("/api/workflows", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config,
                api_auth_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/workflows")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_auth_valid_token() {
        let config = ApiAuthConfig {
            api_key: Some("test-api-key".to_string()),
        };

        let app = Router::new()
            .route("/api/workflows", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config,
                api_auth_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/workflows")
                    .header("Authorization", "Bearer test-api-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_api_auth_invalid_token() {
        let config = ApiAuthConfig {
            api_key: Some("test-api-key".to_string()),
        };

        let app = Router::new()
            .route("/api/workflows", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config,
                api_auth_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/workflows")
                    .header("Authorization", "Bearer wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_api_auth_public_endpoints_bypass() {
        let config = ApiAuthConfig {
            api_key: Some("test-api-key".to_string()),
        };

        // Health endpoint should bypass auth
        let app = Router::new().route("/api/health", get(test_handler)).layer(
            axum::middleware::from_fn_with_state(config.clone(), api_auth_middleware),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Metrics endpoint should bypass auth
        let app = Router::new()
            .route("/api/metrics", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config.clone(),
                api_auth_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // OpenAPI spec should bypass auth
        let app = Router::new()
            .route("/api/openapi.json", get(test_handler))
            .layer(axum::middleware::from_fn_with_state(
                config,
                api_auth_middleware,
            ));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
