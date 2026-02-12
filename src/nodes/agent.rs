//! Agent node - calls ZeptoClaw for AI-powered decisions.
//!
//! This node allows workflows to invoke an AI agent (ZeptoClaw) for tasks
//! that require reasoning, classification, or content generation.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Agent node - invoke ZeptoClaw for AI decisions.
pub struct AgentNode {
    client: Client,
}

impl AgentNode {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for AgentNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct AgentConfig {
    /// The prompt to send to the agent
    prompt: String,

    /// ZeptoClaw endpoint (default: http://localhost:3000/api/chat)
    #[serde(default = "default_endpoint")]
    endpoint: String,

    /// Model to use (optional, uses ZeptoClaw default)
    #[serde(default)]
    model: Option<String>,

    /// Expected response format: "text" or "json"
    #[serde(default = "default_response_format")]
    response_format: String,

    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,

    /// System prompt (optional)
    #[serde(default)]
    system: Option<String>,

    /// Temperature for generation (optional)
    #[serde(default)]
    temperature: Option<f32>,
}

fn default_endpoint() -> String {
    std::env::var("R8R_AGENT_ENDPOINT")
        .or_else(|_| std::env::var("ZEPTOCLAW_API_URL"))
        .unwrap_or_else(|_| "http://localhost:3000/api/chat".to_string())
}

fn default_response_format() -> String {
    "text".to_string()
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Serialize)]
struct AgentRequest {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct AgentResponse {
    content: String,
    #[serde(default)]
    usage: Option<AgentUsage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AgentUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[async_trait]
impl Node for AgentNode {
    fn node_type(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Call ZeptoClaw for AI-powered decisions (classification, generation, reasoning)"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: AgentConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid agent config: {}", e)))?;

        // Render prompt with template variables
        let prompt = render_template(&config.prompt, ctx);

        debug!("Agent node calling ZeptoClaw: {}", config.endpoint);
        info!(
            "Agent prompt: {}...",
            &prompt.chars().take(100).collect::<String>()
        );

        let request = AgentRequest {
            message: prompt.clone(),
            model: config.model,
            system: config.system,
            temperature: config.temperature,
        };

        let start = std::time::Instant::now();

        let response = self
            .client
            .post(&config.endpoint)
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

        let duration = start.elapsed();

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Node(format!(
                "Agent API error ({}): {}",
                status, error_text
            )));
        }

        let agent_response: AgentResponse = response
            .json()
            .await
            .map_err(|e| Error::Node(format!("Failed to parse agent response: {}", e)))?;

        info!("Agent response received ({}ms)", duration.as_millis());

        // Parse response based on expected format
        let output = if config.response_format == "json" {
            match serde_json::from_str(&agent_response.content) {
                Ok(parsed) => parsed,
                Err(e) => {
                    warn!("Failed to parse agent response as JSON: {}", e);
                    // Try to extract JSON from the response (LLMs sometimes add extra text)
                    extract_json(&agent_response.content)
                        .unwrap_or_else(|| Value::String(agent_response.content.clone()))
                }
            }
        } else {
            Value::String(agent_response.content.clone())
        };

        let metadata = json!({
            "response_format": config.response_format,
            "usage": agent_response.usage,
            "duration_ms": duration.as_millis() as u64,
            "endpoint": config.endpoint,
        });

        Ok(NodeResult::with_metadata(output, metadata))
    }
}

/// Render template variables in the prompt.
fn render_template(template: &str, ctx: &NodeContext) -> String {
    let mut result = template.to_string();

    // Replace {{ input }} with full input
    let input_str = match &ctx.input {
        Value::String(s) => s.clone(),
        _ => serde_json::to_string_pretty(&ctx.input).unwrap_or_default(),
    };
    result = result.replace("{{ input }}", &input_str);
    result = result.replace("{{input}}", &input_str);

    // Replace {{ input.field }}
    if let Some(obj) = ctx.input.as_object() {
        for (key, value) in obj {
            let pattern1 = format!("{{{{ input.{} }}}}", key);
            let pattern2 = format!("{{{{input.{}}}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                _ => serde_json::to_string_pretty(value).unwrap_or_default(),
            };
            result = result.replace(&pattern1, &replacement);
            result = result.replace(&pattern2, &replacement);
        }
    }

    // Replace {{ nodes.node_id.output... }}
    for (node_id, output) in &ctx.node_outputs {
        let output_str = match output {
            Value::String(s) => s.clone(),
            _ => serde_json::to_string_pretty(output).unwrap_or_default(),
        };

        let pattern = format!("{{{{ nodes.{}.output }}}}", node_id);
        result = result.replace(&pattern, &output_str);

        if let Some(obj) = output.as_object() {
            for (key, value) in obj {
                let pattern = format!("{{{{ nodes.{}.output.{} }}}}", node_id, key);
                let replacement = match value {
                    Value::String(s) => s.clone(),
                    _ => serde_json::to_string_pretty(value).unwrap_or_default(),
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

/// Try to extract JSON from a string that may contain extra text.
fn extract_json(s: &str) -> Option<Value> {
    // Look for JSON object
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            if start < end {
                let json_str = &s[start..=end];
                if let Ok(parsed) = serde_json::from_str(json_str) {
                    return Some(parsed);
                }
            }
        }
    }

    // Look for JSON array
    if let Some(start) = s.find('[') {
        if let Some(end) = s.rfind(']') {
            if start < end {
                let json_str = &s[start..=end];
                if let Ok(parsed) = serde_json::from_str(json_str) {
                    return Some(parsed);
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_template_simple() {
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "John", "email": "john@example.com"}));

        let template = "Hello {{ input.name }}, your email is {{ input.email }}";
        let result = render_template(template, &ctx);

        assert!(result.contains("John"));
        assert!(result.contains("john@example.com"));
    }

    #[test]
    fn test_render_template_with_node_output() {
        let mut ctx = NodeContext::new("exec-1", "test");
        ctx.add_output("fetch", json!({"status": 200, "data": "hello"}));

        let template = "Previous status: {{ nodes.fetch.output.status }}";
        let result = render_template(template, &ctx);

        assert!(result.contains("200"));
    }

    #[test]
    fn test_extract_json() {
        let s = "Here is the result: {\"urgency\": \"high\", \"reason\": \"blocking\"}";
        let json = extract_json(s).unwrap();
        assert_eq!(json["urgency"], "high");
    }

    #[test]
    fn test_extract_json_array() {
        let s = "Results: [1, 2, 3]";
        let json = extract_json(s).unwrap();
        assert!(json.is_array());
    }

    #[test]
    fn test_agent_config_defaults() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "Hello"
        }))
        .unwrap();

        assert_eq!(config.response_format, "text");
        assert_eq!(config.timeout_seconds, 30);
    }
}
