//! HTTP node - make HTTP requests.

use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Check if SSRF protection is enabled (default: true).
/// Set R8R_ALLOW_INTERNAL_URLS=true to disable protection.
fn is_ssrf_protection_enabled() -> bool {
    std::env::var("R8R_ALLOW_INTERNAL_URLS")
        .map(|v| v.to_lowercase() != "true")
        .unwrap_or(true)
}

/// Validate URL to prevent SSRF attacks.
/// Blocks access to localhost, private IP ranges, and non-http(s) schemes.
fn validate_url(url: &str) -> Result<()> {
    if !is_ssrf_protection_enabled() {
        return Ok(());
    }

    let parsed = reqwest::Url::parse(url)
        .map_err(|e| Error::Node(format!("Invalid URL '{}': {}", url, e)))?;

    // Only allow http/https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(Error::Node(format!(
                "Unsupported URL scheme '{}'. Only http and https are allowed.",
                scheme
            )));
        }
    }

    // Check host
    if let Some(host) = parsed.host_str() {
        // Block localhost variants
        let host_lower = host.to_lowercase();
        if host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "[::1]"
            || host_lower == "0.0.0.0"
        {
            warn!("Blocked SSRF attempt to localhost: {}", url);
            return Err(Error::Node(
                "Access to localhost is not allowed for security reasons.".to_string(),
            ));
        }

        // Check if host is an IP address
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_or_special_ip(&ip) {
                warn!("Blocked SSRF attempt to private IP: {}", url);
                return Err(Error::Node(
                    "Access to private or internal IP addresses is not allowed for security reasons.".to_string(),
                ));
            }
        }

        // Block common internal hostnames
        if host_lower.ends_with(".local")
            || host_lower.ends_with(".internal")
            || host_lower.ends_with(".localhost")
            || host_lower == "metadata.google.internal"  // GCP metadata
            || host_lower == "169.254.169.254"
        // Cloud metadata endpoint
        {
            warn!("Blocked SSRF attempt to internal host: {}", url);
            return Err(Error::Node(
                "Access to internal hostnames is not allowed for security reasons.".to_string(),
            ));
        }
    }

    Ok(())
}

/// Check if an IP address is private, loopback, or otherwise special.
fn is_private_or_special_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()              // 127.0.0.0/8
                || ipv4.is_private()         // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || ipv4.is_link_local()      // 169.254.0.0/16
                || ipv4.is_broadcast()       // 255.255.255.255
                || ipv4.is_unspecified()     // 0.0.0.0
                || ipv4.octets()[0] == 100 && (ipv4.octets()[1] & 0xc0) == 64 // 100.64.0.0/10 (CGNAT)
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()              // ::1
                || ipv6.is_unspecified()     // ::
                // Check for IPv4-mapped addresses
                || ipv6.to_ipv4_mapped().map(|v4| is_private_or_special_ip(&IpAddr::V4(v4))).unwrap_or(false)
        }
    }
}

/// HTTP request node.
pub struct HttpNode {
    client: Client,
}

const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;
const DEFAULT_HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;

impl HttpNode {
    pub fn new() -> Self {
        let client = Client::builder()
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

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: HttpConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid HTTP config: {}", e)))?;

        // Render URL with template variables
        let url = render_template(&config.url, ctx);

        // Validate URL to prevent SSRF attacks
        validate_url(&url)?;

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
                        Value::String(s) => render_template(s, ctx),
                        _ => value.to_string(),
                    };
                    request = request.header(key, header_value);
                }
            }
        }

        // Add body
        if let Some(body) = &config.body {
            // Render template variables in body
            let rendered_body = render_body(body, ctx);
            request = request.json(&rendered_body);
        }

        // Set timeout
        if let Some(timeout) = config.timeout_seconds {
            request = request.timeout(std::time::Duration::from_secs(timeout));
        }

        let start = std::time::Instant::now();
        let response = request.send().await?;
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
            return Err(Error::Node(format!(
                "HTTP {} {} -> {}: {}",
                config.method, url, status, body_text
            )));
        }

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

/// Check if an environment variable is safe to expose in templates.
///
/// By default, only R8R_* prefixed variables are allowed.
/// Additional variables can be whitelisted via R8R_ALLOWED_ENV_VARS
/// (comma-separated list).
fn is_safe_env_var(var_name: &str) -> bool {
    // Always allow R8R_* variables (application configuration)
    if var_name.starts_with("R8R_") {
        return true;
    }

    // Check user-defined allowlist
    if let Ok(allowed) = std::env::var("R8R_ALLOWED_ENV_VARS") {
        let allowed_vars: Vec<&str> = allowed.split(',').map(|s| s.trim()).collect();
        if allowed_vars.contains(&var_name) {
            return true;
        }
    }

    false
}

/// Simple template rendering for HTTP configs.
fn render_template(template: &str, ctx: &NodeContext) -> String {
    let mut result = template.to_string();

    // Replace {{ input }} with full input
    result = result.replace("{{ input }}", &ctx.input.to_string());
    result = result.replace("{{input}}", &ctx.input.to_string());

    // Replace {{ input.field }}
    if let Some(obj) = ctx.input.as_object() {
        for (key, value) in obj {
            let pattern1 = format!("{{{{ input.{} }}}}", key);
            let pattern2 = format!("{{{{input.{}}}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                _ => value.to_string(),
            };
            result = result.replace(&pattern1, &replacement);
            result = result.replace(&pattern2, &replacement);
        }
    }

    // Replace {{ nodes.node_id.output... }}
    for (node_id, output) in &ctx.node_outputs {
        let pattern = format!("{{{{ nodes.{}.output }}}}", node_id);
        result = result.replace(&pattern, &output.to_string());

        if let Some(obj) = output.as_object() {
            for (key, value) in obj {
                let pattern = format!("{{{{ nodes.{}.output.{} }}}}", node_id, key);
                let replacement = match value {
                    Value::String(s) => s.clone(),
                    _ => value.to_string(),
                };
                result = result.replace(&pattern, &replacement);
            }
        }
    }

    // Replace {{ env.VAR }} - only allow safe environment variables
    // By default, only R8R_* variables are allowed. Additional variables can be
    // whitelisted via R8R_ALLOWED_ENV_VARS (comma-separated list).
    let env_regex = env_template_regex();
    result = env_regex
        .replace_all(&result, |caps: &regex_lite::Captures| {
            let var_name = &caps[1];
            if is_safe_env_var(var_name) {
                std::env::var(var_name).unwrap_or_default()
            } else {
                tracing::warn!(
                    "Blocked access to environment variable '{}' in template (not in allowlist)",
                    var_name
                );
                String::new()
            }
        })
        .to_string();

    result
}

fn env_template_regex() -> &'static regex_lite::Regex {
    static ENV_TEMPLATE_REGEX: OnceLock<regex_lite::Regex> = OnceLock::new();
    ENV_TEMPLATE_REGEX
        .get_or_init(|| regex_lite::Regex::new(r"\{\{\s*env\.(\w+)\s*\}\}").expect("valid regex"))
}

/// Render template variables in body recursively.
fn render_body(body: &Value, ctx: &NodeContext) -> Value {
    match body {
        Value::String(s) => Value::String(render_template(s, ctx)),
        Value::Object(obj) => {
            let mut new_obj = serde_json::Map::new();
            for (k, v) in obj {
                new_obj.insert(k.clone(), render_body(v, ctx));
            }
            Value::Object(new_obj)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| render_body(v, ctx)).collect()),
        _ => body.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_input() {
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "John", "email": "john@example.com"}));

        let result = render_template("Hello {{ input.name }}", &ctx);
        assert!(result.contains("John"));
    }

    #[test]
    fn test_render_template_env_allowed() {
        // R8R_* prefixed variables are always allowed
        std::env::set_var("R8R_TEST_VAR", "test_value");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.R8R_TEST_VAR }}", &ctx);
        assert!(result.contains("test_value"));
    }

    #[test]
    fn test_render_template_env_blocked() {
        // Non-R8R_* variables are blocked by default
        std::env::set_var("SECRET_KEY", "super_secret");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.SECRET_KEY }}", &ctx);
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

        let result = render_template("Value: {{ env.CUSTOM_VAR }}", &ctx);
        assert!(result.contains("custom_value"));

        // Clean up
        std::env::remove_var("R8R_ALLOWED_ENV_VARS");
    }

    #[tokio::test]
    async fn test_http_get() {
        let node = HttpNode::new();
        let config = json!({
            "url": "https://httpbin.org/get",
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
}
