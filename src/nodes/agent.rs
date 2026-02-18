//! Agent node - AI-powered decisions via multiple LLM providers.
//!
//! This node allows workflows to invoke an AI agent for tasks that require
//! reasoning, classification, or content generation. Supports:
//! - ZeptoClaw (default, backward compatible)
//! - OpenAI / OpenAI-compatible endpoints
//! - Anthropic Claude
//! - Ollama (local models)

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::template::{render_template, RenderMode};
use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};
use crate::validation::SchemaValidator;

/// Agent node - invoke LLM providers for AI decisions.
pub struct AgentNode {
    client: Client,
}

const DEFAULT_AGENT_HTTP_TIMEOUT_SECS: u64 = 30;
const DEFAULT_AGENT_HTTP_CONNECT_TIMEOUT_SECS: u64 = 10;

impl AgentNode {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_AGENT_HTTP_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(DEFAULT_AGENT_HTTP_CONNECT_TIMEOUT_SECS))
            .build()
            .unwrap_or_else(|e| {
                warn!(
                    "Failed to build agent HTTP client with timeout defaults: {}",
                    e
                );
                Client::new()
            });
        Self { client }
    }
}

impl Default for AgentNode {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Provider enum
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AgentProvider {
    #[default]
    Zeptoclaw,
    Openai,
    Anthropic,
    Ollama,
    Custom,
}

// ---------------------------------------------------------------------------
// Usage info (returned in metadata)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct AgentUsageInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_tokens: Option<u32>,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AgentConfig {
    /// The prompt to send to the agent
    prompt: String,

    /// LLM provider (default: zeptoclaw for backward compat)
    #[serde(default)]
    provider: AgentProvider,

    /// Endpoint URL. Meaning varies by provider:
    /// - zeptoclaw: ZeptoClaw chat URL (default from env or localhost:3000)
    /// - openai: OpenAI-compatible base URL (default: https://api.openai.com/v1/chat/completions)
    /// - anthropic: Anthropic messages URL (default: https://api.anthropic.com/v1/messages)
    /// - ollama: Ollama API URL (default: http://localhost:11434/api/chat)
    /// - custom: any OpenAI-compatible endpoint
    #[serde(default)]
    endpoint: Option<String>,

    /// Model to use (optional, provider-specific defaults apply)
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

    /// Credential name for API key resolution
    #[serde(default)]
    credential: Option<String>,

    /// JSON schema to validate the response against (when response_format = "json")
    #[serde(default)]
    json_schema: Option<Value>,

    /// Maximum response tokens (for providers that support it)
    #[serde(default)]
    max_tokens: Option<u32>,

    /// ZeptoClaw agent template name (e.g. "code-reviewer")
    #[serde(default)]
    agent: Option<String>,
}

fn default_response_format() -> String {
    "text".to_string()
}

fn default_timeout() -> u64 {
    30
}

// ---------------------------------------------------------------------------
// ZeptoClaw request/response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ZeptoClawRequest {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZeptoClawResponse {
    content: String,
    #[serde(default)]
    usage: Option<ZeptoClawUsage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ZeptoClawUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// ---------------------------------------------------------------------------
// Default endpoint helpers
// ---------------------------------------------------------------------------

fn default_zeptoclaw_endpoint() -> String {
    std::env::var("R8R_AGENT_ENDPOINT")
        .or_else(|_| std::env::var("ZEPTOCLAW_API_URL"))
        .unwrap_or_else(|_| "http://localhost:3000/api/chat".to_string())
}

fn default_openai_endpoint() -> String {
    "https://api.openai.com/v1/chat/completions".to_string()
}

fn default_anthropic_endpoint() -> String {
    "https://api.anthropic.com/v1/messages".to_string()
}

fn default_ollama_endpoint() -> String {
    "http://localhost:11434/api/chat".to_string()
}

fn resolve_endpoint(config: &AgentConfig) -> String {
    if let Some(ep) = &config.endpoint {
        return ep.clone();
    }
    match config.provider {
        AgentProvider::Zeptoclaw => default_zeptoclaw_endpoint(),
        AgentProvider::Openai => default_openai_endpoint(),
        AgentProvider::Anthropic => default_anthropic_endpoint(),
        AgentProvider::Ollama => default_ollama_endpoint(),
        AgentProvider::Custom => default_openai_endpoint(), // custom uses OpenAI-compatible format
    }
}

// ---------------------------------------------------------------------------
// Credential resolution
// ---------------------------------------------------------------------------

fn resolve_credential<'a>(config: &AgentConfig, ctx: &'a NodeContext) -> Result<Option<&'a str>> {
    // Explicit credential field takes priority
    if let Some(cred_name) = &config.credential {
        return ctx
            .get_credential(cred_name)
            .map(|s| Some(s.as_str()))
            .ok_or_else(|| Error::Node(format!("Credential '{}' not found", cred_name)));
    }

    // Try provider-specific defaults
    match config.provider {
        AgentProvider::Openai | AgentProvider::Custom => {
            if let Some(key) = ctx.get_credential("openai") {
                return Ok(Some(key.as_str()));
            }
            if let Some(key) = ctx.get_credential("llm") {
                return Ok(Some(key.as_str()));
            }
            Err(Error::Node(
                "No API credential found for OpenAI. Set 'credential' in config or add 'openai' credential".to_string(),
            ))
        }
        AgentProvider::Anthropic => {
            if let Some(key) = ctx.get_credential("anthropic") {
                return Ok(Some(key.as_str()));
            }
            if let Some(key) = ctx.get_credential("llm") {
                return Ok(Some(key.as_str()));
            }
            Err(Error::Node(
                "No API credential found for Anthropic. Set 'credential' in config or add 'anthropic' credential".to_string(),
            ))
        }
        AgentProvider::Zeptoclaw | AgentProvider::Ollama => Ok(None), // no credential required
    }
}

// ---------------------------------------------------------------------------
// Per-provider request builders
// ---------------------------------------------------------------------------

fn build_openai_request(
    config: &AgentConfig,
    prompt: &str,
    api_key: &str,
) -> Result<(String, reqwest::header::HeaderMap, Value)> {
    let url = resolve_endpoint(config);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", api_key)
            .parse()
            .map_err(|_| Error::Node("Invalid API key format".to_string()))?,
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|_| Error::Node("Invalid content type".to_string()))?,
    );

    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "gpt-4o".to_string());

    let mut messages = Vec::new();
    if let Some(system) = &config.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": prompt}));

    let mut body = json!({
        "model": model,
        "messages": messages,
    });

    if let Some(temp) = config.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(max_tok) = config.max_tokens {
        body["max_tokens"] = json!(max_tok);
    }

    // OpenAI structured output via response_format
    if config.response_format == "json" {
        if let Some(schema) = &config.json_schema {
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "agent_response",
                    "strict": true,
                    "schema": schema
                }
            });
        } else {
            body["response_format"] = json!({"type": "json_object"});
        }
    }

    Ok((url, headers, body))
}

fn build_anthropic_request(
    config: &AgentConfig,
    prompt: &str,
    api_key: &str,
) -> Result<(String, reqwest::header::HeaderMap, Value)> {
    let url = resolve_endpoint(config);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "x-api-key",
        api_key
            .parse()
            .map_err(|_| Error::Node("Invalid API key format".to_string()))?,
    );
    headers.insert(
        "anthropic-version",
        "2023-06-01"
            .parse()
            .map_err(|_| Error::Node("Invalid version format".to_string()))?,
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|_| Error::Node("Invalid content type".to_string()))?,
    );

    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "claude-3-haiku-20240307".to_string());

    let max_tokens = config.max_tokens.unwrap_or(4096);

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [
            {"role": "user", "content": prompt}
        ],
    });

    if let Some(system) = &config.system {
        body["system"] = json!(system);
    }
    if let Some(temp) = config.temperature {
        body["temperature"] = json!(temp);
    }

    Ok((url, headers, body))
}

fn build_ollama_request(config: &AgentConfig, prompt: &str) -> Result<(String, Value)> {
    let url = resolve_endpoint(config);
    let model = config
        .model
        .clone()
        .unwrap_or_else(|| "llama3".to_string());

    let mut messages = Vec::new();
    if let Some(system) = &config.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    messages.push(json!({"role": "user", "content": prompt}));

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    if let Some(temp) = config.temperature {
        body["options"] = json!({"temperature": temp});
    }

    if config.response_format == "json" {
        body["format"] = json!("json");
    }

    Ok((url, body))
}

// ---------------------------------------------------------------------------
// Per-provider response extractors
// ---------------------------------------------------------------------------

fn extract_content(
    provider: &AgentProvider,
    response: &Value,
) -> Result<(String, Option<AgentUsageInfo>)> {
    match provider {
        AgentProvider::Openai | AgentProvider::Custom => {
            let content = response["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| {
                    Error::Node("Could not extract content from OpenAI response".to_string())
                })?
                .to_string();

            let usage = if response.get("usage").is_some() {
                Some(AgentUsageInfo {
                    prompt_tokens: response["usage"]["prompt_tokens"].as_u64().map(|v| v as u32),
                    completion_tokens: response["usage"]["completion_tokens"]
                        .as_u64()
                        .map(|v| v as u32),
                    total_tokens: response["usage"]["total_tokens"].as_u64().map(|v| v as u32),
                })
            } else {
                None
            };

            Ok((content, usage))
        }
        AgentProvider::Anthropic => {
            let content = response["content"][0]["text"]
                .as_str()
                .ok_or_else(|| {
                    Error::Node("Could not extract content from Anthropic response".to_string())
                })?
                .to_string();

            let usage = if response.get("usage").is_some() {
                Some(AgentUsageInfo {
                    prompt_tokens: response["usage"]["input_tokens"].as_u64().map(|v| v as u32),
                    completion_tokens: response["usage"]["output_tokens"]
                        .as_u64()
                        .map(|v| v as u32),
                    total_tokens: None, // Anthropic doesn't return total
                })
            } else {
                None
            };

            Ok((content, usage))
        }
        AgentProvider::Ollama => {
            let content = response["message"]["content"]
                .as_str()
                .ok_or_else(|| {
                    Error::Node("Could not extract content from Ollama response".to_string())
                })?
                .to_string();

            let usage = if response.get("prompt_eval_count").is_some() {
                Some(AgentUsageInfo {
                    prompt_tokens: response["prompt_eval_count"].as_u64().map(|v| v as u32),
                    completion_tokens: response["eval_count"].as_u64().map(|v| v as u32),
                    total_tokens: None,
                })
            } else {
                None
            };

            Ok((content, usage))
        }
        AgentProvider::Zeptoclaw => {
            // ZeptoClaw is handled separately via call_zeptoclaw
            Err(Error::Node(
                "extract_content should not be called for ZeptoClaw".to_string(),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// ZeptoClaw caller (extracted from original execute)
// ---------------------------------------------------------------------------

async fn call_zeptoclaw(
    client: &Client,
    config: &AgentConfig,
    prompt: &str,
) -> Result<(String, Option<AgentUsageInfo>)> {
    let endpoint = resolve_endpoint(config);
    debug!("Agent node calling ZeptoClaw: {}", endpoint);

    let request = ZeptoClawRequest {
        message: prompt.to_string(),
        model: config.model.clone(),
        system: config.system.clone(),
        temperature: config.temperature,
        agent: config.agent.clone(),
    };

    let response = client
        .post(&endpoint)
        .timeout(Duration::from_secs(config.timeout_seconds))
        .json(&request)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(Error::Node(format!(
            "Agent API error ({}): {}",
            status, error_text
        )));
    }

    let zc_response: ZeptoClawResponse = response
        .json()
        .await
        .map_err(|e| Error::Node(format!("Failed to parse agent response: {}", e)))?;

    let usage = zc_response.usage.map(|u| AgentUsageInfo {
        prompt_tokens: Some(u.prompt_tokens),
        completion_tokens: Some(u.completion_tokens),
        total_tokens: Some(u.prompt_tokens + u.completion_tokens),
    });

    Ok((zc_response.content, usage))
}

// ---------------------------------------------------------------------------
// Node implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Node for AgentNode {
    fn node_type(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "AI agent node \u{2014} call OpenAI, Anthropic, Ollama, or ZeptoClaw for classification, generation, and reasoning"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: AgentConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid agent config: {}", e)))?;

        // Render prompt with template variables (pretty mode for better LLM readability)
        let prompt = render_template(&config.prompt, ctx, RenderMode::Pretty);

        info!(
            "Agent prompt ({:?}): {}...",
            config.provider,
            &prompt.chars().take(100).collect::<String>()
        );

        let start = std::time::Instant::now();

        // Dispatch to provider
        let (content, usage) = match config.provider {
            AgentProvider::Zeptoclaw => call_zeptoclaw(&self.client, &config, &prompt).await?,

            AgentProvider::Openai | AgentProvider::Custom => {
                let api_key = resolve_credential(&config, ctx)?
                    .ok_or_else(|| Error::Node("API key required for OpenAI".to_string()))?;
                let (url, headers, body) = build_openai_request(&config, &prompt, api_key)?;

                debug!("Agent calling OpenAI-compatible: {}", url);
                let response = self
                    .client
                    .post(&url)
                    .timeout(Duration::from_secs(config.timeout_seconds))
                    .headers(headers)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(Error::Node(format!(
                        "Agent API error ({}): {}",
                        status, error_text
                    )));
                }

                let response_json: Value = response
                    .json()
                    .await
                    .map_err(|e| Error::Node(format!("Failed to parse response: {}", e)))?;

                extract_content(&config.provider, &response_json)?
            }

            AgentProvider::Anthropic => {
                let api_key = resolve_credential(&config, ctx)?
                    .ok_or_else(|| Error::Node("API key required for Anthropic".to_string()))?;
                let (url, headers, body) = build_anthropic_request(&config, &prompt, api_key)?;

                debug!("Agent calling Anthropic: {}", url);
                let response = self
                    .client
                    .post(&url)
                    .timeout(Duration::from_secs(config.timeout_seconds))
                    .headers(headers)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(Error::Node(format!(
                        "Agent API error ({}): {}",
                        status, error_text
                    )));
                }

                let response_json: Value = response
                    .json()
                    .await
                    .map_err(|e| Error::Node(format!("Failed to parse response: {}", e)))?;

                extract_content(&AgentProvider::Anthropic, &response_json)?
            }

            AgentProvider::Ollama => {
                let (url, body) = build_ollama_request(&config, &prompt)?;

                debug!("Agent calling Ollama: {}", url);
                let response = self
                    .client
                    .post(&url)
                    .timeout(Duration::from_secs(config.timeout_seconds))
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| Error::Node(format!("Agent request failed: {}", e)))?;

                if !response.status().is_success() {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(Error::Node(format!(
                        "Agent API error ({}): {}",
                        status, error_text
                    )));
                }

                let response_json: Value = response
                    .json()
                    .await
                    .map_err(|e| Error::Node(format!("Failed to parse response: {}", e)))?;

                extract_content(&AgentProvider::Ollama, &response_json)?
            }
        };

        let duration = start.elapsed();
        info!("Agent response received ({}ms)", duration.as_millis());

        // Parse response based on expected format
        let output = if config.response_format == "json" {
            match serde_json::from_str(&content) {
                Ok(parsed) => {
                    // Validate against json_schema if provided
                    if let Some(schema_value) = &config.json_schema {
                        let validator = SchemaValidator::new(schema_value.clone())
                            .map_err(|e| Error::Node(format!("Invalid json_schema: {}", e)))?;
                        validator.validate(&parsed).map_err(|e| {
                            Error::Node(format!("Agent response failed schema validation: {}", e))
                        })?;
                    }
                    parsed
                }
                Err(e) => {
                    warn!("Failed to parse agent response as JSON: {}", e);
                    // Try to extract JSON from the response (LLMs sometimes add extra text)
                    let extracted = extract_json(&content)
                        .unwrap_or_else(|| Value::String(content.clone()));

                    // Validate extracted JSON if schema is provided
                    if let Some(schema_value) = &config.json_schema {
                        if !extracted.is_string() {
                            let validator =
                                SchemaValidator::new(schema_value.clone()).map_err(|e| {
                                    Error::Node(format!("Invalid json_schema: {}", e))
                                })?;
                            validator.validate(&extracted).map_err(|e| {
                                Error::Node(format!(
                                    "Agent response failed schema validation: {}",
                                    e
                                ))
                            })?;
                        }
                    }

                    extracted
                }
            }
        } else {
            Value::String(content.clone())
        };

        let metadata = json!({
            "provider": format!("{:?}", config.provider),
            "model": config.model,
            "agent": config.agent,
            "response_format": config.response_format,
            "has_json_schema": config.json_schema.is_some(),
            "usage": usage,
            "duration_ms": duration.as_millis() as u64,
            "endpoint": resolve_endpoint(&config),
        });

        Ok(NodeResult::with_metadata(output, metadata))
    }
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

    // -----------------------------------------------------------------------
    // Backward compatibility
    // -----------------------------------------------------------------------

    #[test]
    fn test_agent_config_defaults_backward_compat() {
        // Old configs with just "prompt" must still work
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "Hello"
        }))
        .unwrap();

        assert_eq!(config.response_format, "text");
        assert_eq!(config.timeout_seconds, 30);
        assert!(matches!(config.provider, AgentProvider::Zeptoclaw));
        assert!(config.credential.is_none());
        assert!(config.json_schema.is_none());
        assert!(config.max_tokens.is_none());
        assert!(config.agent.is_none());
        assert!(config.endpoint.is_none());
    }

    // -----------------------------------------------------------------------
    // Provider deserialization
    // -----------------------------------------------------------------------

    #[test]
    fn test_provider_deserialize_openai() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "test",
            "provider": "openai",
            "model": "gpt-4o",
            "credential": "my_key"
        }))
        .unwrap();

        assert!(matches!(config.provider, AgentProvider::Openai));
        assert_eq!(config.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn test_provider_deserialize_anthropic() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "test",
            "provider": "anthropic",
            "model": "claude-sonnet-4-5-20250929"
        }))
        .unwrap();

        assert!(matches!(config.provider, AgentProvider::Anthropic));
    }

    #[test]
    fn test_provider_deserialize_ollama() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "test",
            "provider": "ollama",
            "model": "llama3"
        }))
        .unwrap();

        assert!(matches!(config.provider, AgentProvider::Ollama));
    }

    #[test]
    fn test_provider_deserialize_custom() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "test",
            "provider": "custom",
            "endpoint": "https://my-vllm.internal/v1/chat/completions"
        }))
        .unwrap();

        assert!(matches!(config.provider, AgentProvider::Custom));
        assert_eq!(
            config.endpoint.as_deref(),
            Some("https://my-vllm.internal/v1/chat/completions")
        );
    }

    // -----------------------------------------------------------------------
    // JSON schema validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_schema_config_roundtrip() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "classify",
            "provider": "openai",
            "response_format": "json",
            "json_schema": {
                "type": "object",
                "required": ["severity"],
                "properties": {
                    "severity": {
                        "type": "string",
                        "enum": ["low", "medium", "high"]
                    }
                }
            }
        }))
        .unwrap();

        assert!(config.json_schema.is_some());
        let schema = config.json_schema.unwrap();
        assert_eq!(schema["required"][0], "severity");
    }

    #[test]
    fn test_schema_validation_pass() {
        let schema = json!({
            "type": "object",
            "required": ["severity"],
            "properties": {
                "severity": {
                    "type": "string",
                    "enum": ["low", "medium", "high"]
                }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();
        let valid = json!({"severity": "high"});
        assert!(validator.validate(&valid).is_ok());
    }

    #[test]
    fn test_schema_validation_fail() {
        let schema = json!({
            "type": "object",
            "required": ["severity"],
            "properties": {
                "severity": {
                    "type": "string",
                    "enum": ["low", "medium", "high"]
                }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();
        let invalid = json!({"severity": "critical"});
        assert!(validator.validate(&invalid).is_err());
    }

    #[test]
    fn test_schema_validation_missing_field() {
        let schema = json!({
            "type": "object",
            "required": ["severity"],
            "properties": {
                "severity": { "type": "string" }
            }
        });

        let validator = SchemaValidator::new(schema).unwrap();
        let missing = json!({});
        assert!(validator.validate(&missing).is_err());
    }

    // -----------------------------------------------------------------------
    // Request builders
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_openai_request_shape() {
        let config = AgentConfig {
            prompt: "test".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: Some("gpt-4o".to_string()),
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: Some("You are helpful.".to_string()),
            temperature: Some(0.7),
            credential: None,
            json_schema: None,
            max_tokens: Some(1000),
            agent: None,
        };

        let (url, headers, body) = build_openai_request(&config, "Hello", "sk-test").unwrap();

        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert!(headers
            .get(reqwest::header::AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Bearer "));
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are helpful.");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Hello");
        assert!((body["temperature"].as_f64().unwrap() - 0.7).abs() < 0.01);
        assert_eq!(body["max_tokens"], 1000);
    }

    #[test]
    fn test_build_openai_request_json_schema() {
        let config = AgentConfig {
            prompt: "classify".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "json".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: Some(json!({"type": "object", "properties": {"a": {"type": "string"}}})),
            max_tokens: None,
            agent: None,
        };

        let (_url, _headers, body) = build_openai_request(&config, "test", "sk-test").unwrap();

        assert_eq!(body["response_format"]["type"], "json_schema");
        assert!(body["response_format"]["json_schema"]["schema"].is_object());
    }

    #[test]
    fn test_build_openai_request_json_no_schema() {
        let config = AgentConfig {
            prompt: "classify".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "json".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let (_url, _headers, body) = build_openai_request(&config, "test", "sk-test").unwrap();

        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn test_build_anthropic_request_shape() {
        let config = AgentConfig {
            prompt: "test".to_string(),
            provider: AgentProvider::Anthropic,
            endpoint: None,
            model: Some("claude-sonnet-4-5-20250929".to_string()),
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: Some("Be concise.".to_string()),
            temperature: Some(0.5),
            credential: None,
            json_schema: None,
            max_tokens: Some(2048),
            agent: None,
        };

        let (url, headers, body) = build_anthropic_request(&config, "Hello", "sk-ant-test").unwrap();

        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(headers.get("x-api-key").unwrap().to_str().unwrap(), "sk-ant-test");
        assert_eq!(headers.get("anthropic-version").unwrap().to_str().unwrap(), "2023-06-01");
        assert_eq!(body["model"], "claude-sonnet-4-5-20250929");
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["system"], "Be concise.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn test_build_ollama_request_shape() {
        let config = AgentConfig {
            prompt: "test".to_string(),
            provider: AgentProvider::Ollama,
            endpoint: None,
            model: Some("llama3".to_string()),
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: Some("You are a classifier.".to_string()),
            temperature: Some(0.3),
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let (url, body) = build_ollama_request(&config, "Classify this").unwrap();

        assert_eq!(url, "http://localhost:11434/api/chat");
        assert_eq!(body["model"], "llama3");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are a classifier.");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Classify this");
        assert!((body["options"]["temperature"].as_f64().unwrap() - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_build_ollama_request_json_format() {
        let config = AgentConfig {
            prompt: "test".to_string(),
            provider: AgentProvider::Ollama,
            endpoint: None,
            model: None,
            response_format: "json".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let (_url, body) = build_ollama_request(&config, "test").unwrap();
        assert_eq!(body["format"], "json");
    }

    // -----------------------------------------------------------------------
    // Response extractors
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_content_openai() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Hello from GPT"
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let (content, usage) = extract_content(&AgentProvider::Openai, &response).unwrap();
        assert_eq!(content, "Hello from GPT");
        let usage = usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(10));
        assert_eq!(usage.completion_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(15));
    }

    #[test]
    fn test_extract_content_anthropic() {
        let response = json!({
            "content": [{
                "type": "text",
                "text": "Hello from Claude"
            }],
            "usage": {
                "input_tokens": 8,
                "output_tokens": 4
            }
        });

        let (content, usage) = extract_content(&AgentProvider::Anthropic, &response).unwrap();
        assert_eq!(content, "Hello from Claude");
        let usage = usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(8));
        assert_eq!(usage.completion_tokens, Some(4));
        assert!(usage.total_tokens.is_none());
    }

    #[test]
    fn test_extract_content_ollama() {
        let response = json!({
            "message": {
                "role": "assistant",
                "content": "Hello from Llama"
            },
            "prompt_eval_count": 12,
            "eval_count": 6
        });

        let (content, usage) = extract_content(&AgentProvider::Ollama, &response).unwrap();
        assert_eq!(content, "Hello from Llama");
        let usage = usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(12));
        assert_eq!(usage.completion_tokens, Some(6));
    }

    #[test]
    fn test_extract_content_custom() {
        // Custom uses same format as OpenAI
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Hello from custom"
                }
            }]
        });

        let (content, usage) = extract_content(&AgentProvider::Custom, &response).unwrap();
        assert_eq!(content, "Hello from custom");
        assert!(usage.is_none());
    }

    // -----------------------------------------------------------------------
    // Credential resolution
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_credential_explicit() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: Some("my_key".to_string()),
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let mut creds = std::collections::HashMap::new();
        creds.insert("my_key".to_string(), "sk-explicit".to_string());
        let ctx = NodeContext::new("exec-1", "test").with_credentials(creds);

        let result = resolve_credential(&config, &ctx).unwrap();
        assert_eq!(result, Some("sk-explicit"));
    }

    #[test]
    fn test_resolve_credential_fallback_openai() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let mut creds = std::collections::HashMap::new();
        creds.insert("openai".to_string(), "sk-fallback".to_string());
        let ctx = NodeContext::new("exec-1", "test").with_credentials(creds);

        let result = resolve_credential(&config, &ctx).unwrap();
        assert_eq!(result, Some("sk-fallback"));
    }

    #[test]
    fn test_resolve_credential_fallback_anthropic() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Anthropic,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let mut creds = std::collections::HashMap::new();
        creds.insert("anthropic".to_string(), "sk-ant-fallback".to_string());
        let ctx = NodeContext::new("exec-1", "test").with_credentials(creds);

        let result = resolve_credential(&config, &ctx).unwrap();
        assert_eq!(result, Some("sk-ant-fallback"));
    }

    #[test]
    fn test_resolve_credential_zeptoclaw_none() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Zeptoclaw,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let ctx = NodeContext::new("exec-1", "test");
        let result = resolve_credential(&config, &ctx).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_credential_ollama_none() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Ollama,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let ctx = NodeContext::new("exec-1", "test");
        let result = resolve_credential(&config, &ctx).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_credential_missing_explicit() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: Some("nonexistent".to_string()),
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let ctx = NodeContext::new("exec-1", "test");
        let result = resolve_credential(&config, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn test_resolve_credential_llm_fallback() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Openai,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        let mut creds = std::collections::HashMap::new();
        creds.insert("llm".to_string(), "sk-llm-generic".to_string());
        let ctx = NodeContext::new("exec-1", "test").with_credentials(creds);

        let result = resolve_credential(&config, &ctx).unwrap();
        assert_eq!(result, Some("sk-llm-generic"));
    }

    // -----------------------------------------------------------------------
    // Endpoint resolution
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_endpoint_custom() {
        let config = AgentConfig {
            prompt: "t".to_string(),
            provider: AgentProvider::Custom,
            endpoint: Some("https://my-vllm.internal/v1/chat/completions".to_string()),
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        assert_eq!(
            resolve_endpoint(&config),
            "https://my-vllm.internal/v1/chat/completions"
        );
    }

    #[test]
    fn test_resolve_endpoint_defaults() {
        let make_config = |provider: AgentProvider| AgentConfig {
            prompt: "t".to_string(),
            provider,
            endpoint: None,
            model: None,
            response_format: "text".to_string(),
            timeout_seconds: 30,
            system: None,
            temperature: None,
            credential: None,
            json_schema: None,
            max_tokens: None,
            agent: None,
        };

        assert!(resolve_endpoint(&make_config(AgentProvider::Openai))
            .contains("api.openai.com"));
        assert!(resolve_endpoint(&make_config(AgentProvider::Anthropic))
            .contains("api.anthropic.com"));
        assert!(resolve_endpoint(&make_config(AgentProvider::Ollama))
            .contains("localhost:11434"));
    }

    // -----------------------------------------------------------------------
    // Existing tests (preserved)
    // -----------------------------------------------------------------------

    #[test]
    fn test_render_template_simple() {
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "John", "email": "john@example.com"}));

        let template = "Hello {{ input.name }}, your email is {{ input.email }}";
        let result = render_template(template, &ctx, RenderMode::Pretty);

        assert!(result.contains("John"));
        assert!(result.contains("john@example.com"));
    }

    #[test]
    fn test_render_template_with_node_output() {
        let mut ctx = NodeContext::new("exec-1", "test");
        ctx.add_output("fetch", json!({"status": 200, "data": "hello"}));

        let template = "Previous status: {{ nodes.fetch.output.status }}";
        let result = render_template(template, &ctx, RenderMode::Pretty);

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

    // -----------------------------------------------------------------------
    // Agent field (ZeptoClaw template)
    // -----------------------------------------------------------------------

    #[test]
    fn test_agent_field_deserialize() {
        let config: AgentConfig = serde_json::from_value(json!({
            "prompt": "Review this code",
            "agent": "code-reviewer"
        }))
        .unwrap();

        assert_eq!(config.agent.as_deref(), Some("code-reviewer"));
        assert!(matches!(config.provider, AgentProvider::Zeptoclaw));
    }
}
