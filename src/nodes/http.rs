//! HTTP node - make HTTP requests.

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// HTTP request node.
pub struct HttpNode {
    client: Client,
}

impl HttpNode {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
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
}

fn default_method() -> String {
    "GET".to_string()
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

    // Replace {{ env.VAR }}
    let env_regex = regex_lite::Regex::new(r"\{\{\s*env\.(\w+)\s*\}\}").unwrap();
    result = env_regex
        .replace_all(&result, |caps: &regex_lite::Captures| {
            let var_name = &caps[1];
            std::env::var(var_name).unwrap_or_default()
        })
        .to_string();

    result
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
    fn test_render_template_env() {
        std::env::set_var("TEST_VAR", "test_value");
        let ctx = NodeContext::new("exec-1", "test");

        let result = render_template("Value: {{ env.TEST_VAR }}", &ctx);
        assert!(result.contains("test_value"));
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
}
