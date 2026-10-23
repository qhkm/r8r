/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! HTTP node - make HTTP requests.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::template::{render_body, render_template, RenderMode};
use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};
use crate::ssrf;

/// Thin wrapper: delegates to the shared `ssrf::check_ssrf` and maps errors to `Error::Node`.
fn validate_url(url: &str) -> Result<()> {
    ssrf::check_ssrf(url).map_err(|msg| {
        if msg.starts_with("Access to") {
            warn!("Blocked SSRF attempt: {}", url);
        }
        Error::Node(msg)
    })
}

use super::circuit_breaker::CircuitBreakerRegistry;

/// Global circuit breaker registry for HTTP endpoints.
static CIRCUIT_BREAKERS: OnceLock<CircuitBreakerRegistry> = OnceLock::new();

fn get_circuit_breakers() -> &'static CircuitBreakerRegistry {
    CIRCUIT_BREAKERS.get_or_init(CircuitBreakerRegistry::new)
}

/// HTTP request node.
pub struct HttpNode {
    client: Client,
}

const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;
const DEFAULT_HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;

impl HttpNode {
    pub fn new() -> Self {
        let client = ssrf::hardened_client_builder()
            .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(DEFAULT_HTTP_CONNECT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|e| {
                warn!("Failed to build HTTP client with timeout defaults: {}", e);
                Client::new()
            });
        Self { client }
    }
}

impl Default for HttpNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct HttpConfig {
    url: String,
    #[serde(default = "default_method")]
    method: String,
    #[serde(default)]
    headers: Option<Value>,
    #[serde(default)]
    body: Option<Value>,
    /// How to encode the body: "json" (default) or "form" (application/x-www-form-urlencoded)
    #[serde(default = "default_body_type")]
    body_type: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Return raw text instead of parsing JSON
    #[serde(default)]
    raw_response: bool,
    /// Credential service name to use for authentication.
    /// The credential value is resolved from the credential store.
    #[serde(default)]
    credential: Option<String>,
    /// How to use the credential for authentication.
    /// - `bearer`: Add `Authorization: Bearer <value>` header
    /// - `basic`: Interpret value as "user:password" and add Basic auth
    /// - `api_key`: Add `X-API-Key: <value>` header
    /// - `header:<name>`: Add custom header with name `<name>`
    #[serde(default = "default_auth_type")]
    auth_type: String,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_body_type() -> String {
    "json".to_string()
}

/// Build a multipart/form-data body from a JSON object (binary-safe, zero deps).
///
/// String values starting with `data:<mime>;base64,` (or `data:<mime>,` raw) become
/// binary file parts with a filename derived from the MIME type; all other values
/// become text parts. Returns (body bytes, Content-Type header value).
fn build_multipart_body(obj: &serde_json::Map<String, Value>) -> Result<(Vec<u8>, String)> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let boundary = format!("r8r-{}-{}", std::process::id(), nanos % 1_000_000_000_000);
    let mut out: Vec<u8> = Vec::new();
    let mut file_idx: u32 = 0;

    for (k, v) in obj {
        let val = match v {
            Value::Null => continue,
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if let Some((mime, bytes)) = parse_data_uri(&val) {
            let ext = mime_extension(&mime);
            let filename = format!("file{file_idx}{ext}");
            file_idx += 1;
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{k}\"; filename=\"{filename}\"\r\n"
                )
                .as_bytes(),
            );
            out.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
            out.extend_from_slice(&bytes);
            out.extend_from_slice(b"\r\n");
        } else {
            out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            out.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{k}\"\r\n\r\n").as_bytes(),
            );
            out.extend_from_slice(val.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let content_type = format!("multipart/form-data; boundary={boundary}");
    Ok((out, content_type))
}

/// Parse a `data:` URI (`data:<mime>;base64,<payload>` or `data:<mime>,<raw>`).
fn parse_data_uri(s: &str) -> Option<(String, Vec<u8>)> {
    use base64::Engine as _;
    let rest = s.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let parts: Vec<&str> = meta.split(';').collect();
    let mime = parts
        .first()
        .filter(|m| !m.is_empty())
        .copied()
        .unwrap_or("application/octet-stream")
        .to_string();
    let is_base64 = parts.iter().any(|p| p.eq_ignore_ascii_case("base64"));
    let bytes = if is_base64 {
        base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?
    } else {
        payload.as_bytes().to_vec()
    };
    Some((mime, bytes))
}

/// Filename extension for a MIME type (used for multipart file parts).
fn mime_extension(mime: &str) -> &'static str {
    match mime {
        "audio/ogg" => ".ogg",
        "audio/mpeg" | "audio/mp3" => ".mp3",
        "audio/mp4" | "audio/x-m4a" => ".m4a",
        "audio/wav" | "audio/x-wav" => ".wav",
        "audio/webm" => ".webm",
        "video/mp4" => ".mp4",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "application/pdf" => ".pdf",
        _ => ".bin",
    }
}

fn default_auth_type() -> String {
    "bearer".to_string()
}

/// Apply authentication to a request based on auth_type.
fn apply_authentication(
    request: reqwest::RequestBuilder,
    credential: &str,
    auth_type: &str,
) -> Result<reqwest::RequestBuilder> {
    match auth_type {
        "bearer" => Ok(request.bearer_auth(credential)),
        "basic" => {
            // Expect credential in format "username:password"
            let parts: Vec<&str> = credential.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::Node(
                    "Basic auth credential must be in 'username:password' format".to_string(),
                ));
            }
            Ok(request.basic_auth(parts[0], Some(parts[1])))
        }
        "api_key" => Ok(request.header("X-API-Key", credential)),
        other if other.starts_with("header:") => {
            let header_name = &other[7..]; // Skip "header:"
            if header_name.is_empty() {
                return Err(Error::Node(
                    "header: auth_type must specify a header name".to_string(),
                ));
            }
            Ok(request.header(header_name, credential))
        }
        _ => Err(Error::Node(format!(
            "Unknown auth_type '{}'. Use: bearer, basic, api_key, or header:<name>",
            auth_type
        ))),
    }
}

#[async_trait]
impl Node for HttpNode {
    fn node_type(&self) -> &str {
        "http"
    }

    fn description(&self) -> &str {
        "Make HTTP requests (GET, POST, PUT, DELETE, PATCH)"
    }

    fn validate_config(&self, config: &Value) -> Result<()> {
        let _: HttpConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid http config: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: HttpConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid HTTP config: {}", e)))?;

        // Render URL with template variables
        let url = render_template(&config.url, ctx, RenderMode::Compact);

        // Validate URL to prevent SSRF attacks
        validate_url(&url)?;

        // Extract host for circuit breaker
        let cb_key = reqwest::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.to_string()))
            .unwrap_or_else(|| url.clone());

        // Check circuit breaker
        if !get_circuit_breakers().allow_request(&cb_key) {
            return Err(Error::Node(format!(
                "Circuit breaker open for host '{}' - too many recent failures",
                cb_key
            )));
        }

        debug!("HTTP {} {}", config.method, url);

        let mut request = match config.method.to_uppercase().as_str() {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            "DELETE" => self.client.delete(&url),
            "PATCH" => self.client.patch(&url),
            "HEAD" => self.client.head(&url),
            _ => {
                return Err(Error::Node(format!(
                    "Unknown HTTP method: {}",
                    config.method
                )))
            }
        };

        // Add authentication if credential is specified
        if let Some(credential_name) = &config.credential {
            if let Some(credential_value) = ctx.get_credential(credential_name) {
                request = apply_authentication(request, credential_value, &config.auth_type)?;
            } else {
                return Err(Error::Node(format!(
                    "Credential '{}' not found. Add it with: r8r credentials set {}",
                    credential_name, credential_name
                )));
            }
        }

        // Add headers
        if let Some(headers) = &config.headers {
            if let Some(headers_obj) = headers.as_object() {
                for (key, value) in headers_obj {
                    let header_value = match value {
                        Value::String(s) => render_template(s, ctx, RenderMode::Compact),
                        _ => value.to_string(),
                    };
                    request = request.header(key, header_value);
                }
            }
        }

        // Add body
        if let Some(body) = &config.body {
            // Render template variables in body
            let rendered_body = render_body(body, ctx, RenderMode::Compact);
            match config.body_type.as_str() {
                "form" => {
                    // Convert JSON object to form key-value pairs
                    if let Some(obj) = rendered_body.as_object() {
                        let form_data: HashMap<String, String> = obj
                            .iter()
                            .filter_map(|(k, v)| {
                                let val = match v {
                                    Value::String(s) => s.clone(),
                                    Value::Null => return None,
                                    other => other.to_string().trim_matches('"').to_string(),
                                };
                                Some((k.clone(), val))
                            })
                            .collect();
                        request = request.form(&form_data);
                    } else {
                        request = request.json(&rendered_body);
                    }
                }
                "multipart" => {
                    // Hand-rolled multipart/form-data (no extra deps, binary-safe).
                    // String values starting with `data:<mime>;base64,` become binary
                    // file parts; all other values become text parts.
                    let obj = rendered_body.as_object().ok_or_else(|| {
                        Error::Node("multipart body must be a JSON object".to_string())
                    })?;
                    let (body_bytes, content_type) = build_multipart_body(obj)?;
                    request = request
                        .header(reqwest::header::CONTENT_TYPE, content_type)
                        .body(body_bytes);
                }
                _ => {
                    request = request.json(&rendered_body);
                }
            }
        }

        // Set timeout
        if let Some(timeout) = config.timeout_seconds {
            request = request.timeout(std::time::Duration::from_secs(timeout));
        }

        let start = std::time::Instant::now();
        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                get_circuit_breakers().record_failure(&cb_key);
                return Err(e.into());
            }
        };
        let duration = start.elapsed();

        let status = response.status().as_u16();
        let headers: Value = {
            let mut map = serde_json::Map::new();
            for (k, v) in response.headers().iter() {
                map.insert(
                    k.to_string(),
                    Value::String(v.to_str().unwrap_or("").to_string()),
                );
            }
            Value::Object(map)
        };

        let body_text = response.text().await.map_err(|e| {
            Error::Node(format!(
                "Failed to read HTTP response body from {}: {}",
                url, e
            ))
        })?;

        if status >= 400 {
            // Record failure for server errors (5xx)
            if status >= 500 {
                get_circuit_breakers().record_failure(&cb_key);
            }
            return Err(Error::Node(format!(
                "HTTP {} {} -> {}: {}",
                config.method, url, status, body_text
            )));
        }

        // Record success
        get_circuit_breakers().record_success(&cb_key);

        let body: Value = if config.raw_response {
            Value::String(body_text)
        } else {
            serde_json::from_str(&body_text).map_err(|e| {
                Error::Node(format!(
                    "HTTP {} {} returned non-JSON body: {}",
                    config.method, url, e
                ))
            })?
        };

        info!(
            "HTTP {} {} -> {} ({}ms)",
            config.method,
            url,
            status,
            duration.as_millis()
        );

        Ok(NodeResult::with_metadata(
            json!({
                "status": status,
                "headers": headers,
                "body": body,
            }),
            json!({
                "duration_ms": duration.as_millis() as u64,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_input() {
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "John", "email": "john@example.com"}));

        let result = render_template("Hello {{ input.name }}", &ctx, RenderMode::Compact);
        assert!(result.contains("John"));
    }

    #[test]
    fn test_render_template_env_allowed() {
        // R8R_* prefixed variables are always allowed
        std::env::set_var("R8R_TEST_VAR", "test_value");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.R8R_TEST_VAR }}", &ctx, RenderMode::Compact);
        assert!(result.contains("test_value"));
    }

    #[test]
    fn test_render_template_env_blocked() {
        // Non-R8R_* variables are blocked by default
        std::env::set_var("SECRET_KEY", "super_secret");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.SECRET_KEY }}", &ctx, RenderMode::Compact);
        // Should NOT contain the secret - blocked for security
        assert!(!result.contains("super_secret"));
        assert!(result.contains("Value: "));
    }

    #[test]
    fn test_render_template_env_allowlist() {
        // Variables in R8R_ALLOWED_ENV_VARS are permitted
        std::env::set_var("R8R_ALLOWED_ENV_VARS", "CUSTOM_VAR, ANOTHER_VAR");
        std::env::set_var("CUSTOM_VAR", "custom_value");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.CUSTOM_VAR }}", &ctx, RenderMode::Compact);
        assert!(result.contains("custom_value"));

        // Clean up
        std::env::remove_var("R8R_ALLOWED_ENV_VARS");
    }

    #[tokio::test]
    async fn test_http_get() {
        let node = HttpNode::new();
        let config = json!({
            // httpbin.org frequently returns 503 (rate-limited/down), making
            // this test flaky. jsonplaceholder.typicode.com is a stable JSON
            // test endpoint that always answers 200 with a JSON body.
            "url": "https://jsonplaceholder.typicode.com/todos/1",
            "method": "GET"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        let status = result.data["status"].as_u64().unwrap();
        assert_eq!(status, 200);
    }

    #[test]
    fn test_ssrf_protection_localhost() {
        let result = validate_url("http://localhost:8080/admin");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("localhost"));
    }

    #[test]
    fn test_ssrf_protection_127_0_0_1() {
        let result = validate_url("http://127.0.0.1:6379");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("localhost"));
    }

    #[test]
    fn test_ssrf_protection_private_ip() {
        // 10.0.0.0/8
        let result = validate_url("http://10.0.0.1/internal");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("private"));

        // 192.168.0.0/16
        let result = validate_url("http://192.168.1.1/admin");
        assert!(result.is_err());

        // 172.16.0.0/12
        let result = validate_url("http://172.16.0.1/secret");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_protection_metadata_endpoint() {
        let result = validate_url("http://169.254.169.254/latest/meta-data/");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_protection_internal_hostnames() {
        let result = validate_url("http://db.internal/query");
        assert!(result.is_err());

        let result = validate_url("http://redis.local/");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_protection_invalid_scheme() {
        let result = validate_url("file:///etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("scheme"));

        let result = validate_url("ftp://example.com/file");
        assert!(result.is_err());
    }

    #[test]
    fn test_ssrf_protection_allows_external() {
        let result = validate_url("https://api.example.com/v1/users");
        assert!(result.is_ok());

        let result = validate_url("https://httpbin.org/get");
        assert!(result.is_ok());
    }

    #[test]
    fn test_http_config_body_type_default() {
        let config: HttpConfig = serde_json::from_value(json!({
            "url": "https://example.com",
            "method": "POST",
            "body": {"key": "value"}
        }))
        .unwrap();
        assert_eq!(config.body_type, "json");
    }

    #[test]
    fn test_http_config_body_type_form() {
        let config: HttpConfig = serde_json::from_value(json!({
            "url": "https://example.com",
            "method": "POST",
            "body": {"key": "value"},
            "body_type": "form"
        }))
        .unwrap();
        assert_eq!(config.body_type, "form");
    }

    #[test]
    fn test_parse_data_uri_base64() {
        let (mime, bytes) = parse_data_uri("data:audio/ogg;base64,U29tZSBhdWRpbw==").unwrap();
        assert_eq!(mime, "audio/ogg");
        assert_eq!(bytes, b"Some audio");
    }

    #[test]
    fn test_parse_data_uri_raw() {
        let (mime, bytes) = parse_data_uri("data:text/plain,hello").unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn test_parse_data_uri_invalid() {
        assert!(parse_data_uri("not a uri").is_none());
        assert!(parse_data_uri("data:audio/ogg;base64,@@@not-base64@@@").is_none());
        assert!(parse_data_uri("data:,no-comma-payload").is_some()); // empty mime → octet-stream
    }

    #[test]
    fn test_build_multipart_body_text_and_file() {
        let obj: serde_json::Map<String, Value> = serde_json::from_str(
            r#"{
              "model": "whisper-1",
              "file": "data:audio/ogg;base64,U29tZSBhdWRpbw==",
              "skip_null": null,
              "temperature": 0.2
            }"#,
        )
        .unwrap();
        let (body, content_type) = build_multipart_body(&obj).unwrap();

        assert!(content_type.starts_with("multipart/form-data; boundary=r8r-"));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("name=\"model\""));
        assert!(text.contains("whisper-1"));
        assert!(text.contains("name=\"temperature\""));
        assert!(text.contains("0.2"));
        // null fields are skipped
        assert!(!text.contains("skip_null"));
        // binary file part: filename from audio/ogg mime + raw decoded bytes present
        assert!(text.contains("name=\"file\"; filename=\"file0.ogg\""));
        assert!(text.contains("Content-Type: audio/ogg"));
        assert!(body.windows(10).any(|w| w == b"Some audio"));
        // closing boundary present
        assert!(text.trim_end().ends_with("--"));
        // content type header boundary matches body boundary
        let boundary = content_type.split_once("boundary=").unwrap().1;
        assert!(text.contains(&format!("--{boundary}--")));
    }
}
