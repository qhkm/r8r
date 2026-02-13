//! Summarize node - AI-powered text summarization.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Summarize node for AI-powered text summarization.
pub struct SummarizeNode;

impl SummarizeNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SummarizeNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SummaryStyle {
    /// Brief one-paragraph summary
    #[default]
    Brief,
    /// Detailed multi-paragraph summary
    Detailed,
    /// Bullet point summary
    Bullets,
    /// Key points extraction
    KeyPoints,
    /// Executive summary style
    Executive,
    /// Custom (uses custom_prompt)
    Custom,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum Provider {
    /// OpenAI API (default)
    #[default]
    Openai,
    /// Anthropic Claude API
    Anthropic,
    /// Custom OpenAI-compatible endpoint
    Custom,
}

#[derive(Debug, Deserialize)]
struct SummarizeConfig {
    /// Text to summarize (if not provided, uses input)
    #[serde(default)]
    text: Option<String>,

    /// Field path in input containing text to summarize
    #[serde(default)]
    text_field: Option<String>,

    /// Summary style
    #[serde(default)]
    style: SummaryStyle,

    /// Custom prompt (for Custom style)
    #[serde(default)]
    custom_prompt: Option<String>,

    /// Target length hint (e.g., "100 words", "3 sentences")
    #[serde(default)]
    max_length: Option<String>,

    /// API provider
    #[serde(default)]
    provider: Provider,

    /// Model to use (e.g., "gpt-4", "claude-3-opus")
    #[serde(default)]
    model: Option<String>,

    /// API endpoint (for custom provider)
    #[serde(default)]
    api_url: Option<String>,

    /// Credential name for API key
    #[serde(default)]
    credential: Option<String>,

    /// Temperature for generation (0.0 - 1.0)
    #[serde(default = "default_temperature")]
    temperature: f32,

    /// Additional context for summarization
    #[serde(default)]
    context: Option<String>,

    /// Language for output
    #[serde(default)]
    language: Option<String>,
}

fn default_temperature() -> f32 {
    0.3
}

#[async_trait]
impl Node for SummarizeNode {
    fn node_type(&self) -> &str {
        "summarize"
    }

    fn description(&self) -> &str {
        "AI-powered text summarization using LLM APIs"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SummarizeConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid summarize config: {}", e)))?;

        // Get text to summarize
        let text = get_text(&config, &ctx.input)?;

        if text.trim().is_empty() {
            return Err(Error::Node(
                "No text provided for summarization".to_string(),
            ));
        }

        info!("Summarizing {} characters of text", text.len());

        // Get API key from credentials
        let api_key = if let Some(cred_name) = &config.credential {
            ctx.get_credential(cred_name)
                .cloned()
                .ok_or_else(|| Error::Node(format!("Credential '{}' not found", cred_name)))?
        } else {
            // Try default credential names
            ctx.get_credential("openai")
                .or_else(|| ctx.get_credential("anthropic"))
                .or_else(|| ctx.get_credential("llm"))
                .cloned()
                .ok_or_else(|| {
                    Error::Node(
                        "No API credential found. Set 'credential' config or add 'openai'/'anthropic'/'llm' credential"
                            .to_string(),
                    )
                })?
        };

        // Build the prompt
        let prompt = build_prompt(&config, &text);
        debug!("Generated prompt: {}", prompt);

        // Call the API
        let (api_url, headers, body) = build_api_request(&config, &api_key, &prompt)?;

        let client = reqwest::Client::new();
        let response = client
            .post(&api_url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Node(format!("API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Node(format!(
                "API returned error {}: {}",
                status, error_text
            )));
        }

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| Error::Node(format!("Failed to parse API response: {}", e)))?;

        // Extract summary from response
        let summary = extract_summary(&config.provider, &response_json)?;

        Ok(NodeResult::with_metadata(
            json!({
                "summary": summary,
                "original_length": text.len(),
                "summary_length": summary.len(),
            }),
            json!({
                "provider": format!("{:?}", config.provider),
                "model": config.model,
                "style": format!("{:?}", config.style),
            }),
        ))
    }
}

/// Get text from config or input.
fn get_text(config: &SummarizeConfig, input: &Value) -> Result<String> {
    if let Some(text) = &config.text {
        return Ok(text.clone());
    }

    if let Some(field) = &config.text_field {
        return get_field_as_string(input, field);
    }

    // Try to extract from input directly
    match input {
        Value::String(s) => Ok(s.clone()),
        Value::Object(obj) => {
            // Try common field names
            for field in &["text", "content", "body", "message", "data"] {
                if let Some(Value::String(s)) = obj.get(*field) {
                    return Ok(s.clone());
                }
            }
            // Convert entire object to string
            Ok(serde_json::to_string_pretty(input).unwrap_or_default())
        }
        Value::Array(arr) => {
            // Join array elements
            let texts: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    _ => v.to_string(),
                })
                .collect();
            Ok(texts.join("\n\n"))
        }
        _ => Ok(input.to_string()),
    }
}

/// Get nested field as string.
fn get_field_as_string(value: &Value, path: &str) -> Result<String> {
    let mut current = value.clone();
    for part in path.split('.') {
        current = match current {
            Value::Object(obj) => obj
                .get(part)
                .cloned()
                .ok_or_else(|| Error::Node(format!("Field '{}' not found", path)))?,
            _ => return Err(Error::Node(format!("Cannot index into {:?}", current))),
        };
    }

    match current {
        Value::String(s) => Ok(s),
        _ => Ok(current.to_string()),
    }
}

/// Build the summarization prompt.
fn build_prompt(config: &SummarizeConfig, text: &str) -> String {
    let style_instruction = match &config.style {
        SummaryStyle::Brief => "Provide a brief, one-paragraph summary.",
        SummaryStyle::Detailed => {
            "Provide a detailed, comprehensive summary with multiple paragraphs."
        }
        SummaryStyle::Bullets => "Summarize the key points as a bulleted list.",
        SummaryStyle::KeyPoints => "Extract and list the most important key points.",
        SummaryStyle::Executive => {
            "Write an executive summary suitable for quick review by decision-makers."
        }
        SummaryStyle::Custom => config
            .custom_prompt
            .as_deref()
            .unwrap_or("Summarize the following text:"),
    };

    let length_instruction = config
        .max_length
        .as_ref()
        .map(|l| format!(" Keep the summary to approximately {}.", l))
        .unwrap_or_default();

    let language_instruction = config
        .language
        .as_ref()
        .map(|l| format!(" Write the summary in {}.", l))
        .unwrap_or_default();

    let context_instruction = config
        .context
        .as_ref()
        .map(|c| format!(" Context: {}", c))
        .unwrap_or_default();

    format!(
        "{}{}{}{}\n\nText to summarize:\n{}",
        style_instruction, length_instruction, language_instruction, context_instruction, text
    )
}

/// Build the API request based on provider.
fn build_api_request(
    config: &SummarizeConfig,
    api_key: &str,
    prompt: &str,
) -> Result<(String, reqwest::header::HeaderMap, Value)> {
    let mut headers = reqwest::header::HeaderMap::new();

    match config.provider {
        Provider::Openai | Provider::Custom => {
            let url = config
                .api_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1/chat/completions".to_string());

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
                .unwrap_or_else(|| "gpt-3.5-turbo".to_string());

            let body = json!({
                "model": model,
                "messages": [
                    {
                        "role": "system",
                        "content": "You are a helpful assistant that summarizes text accurately and concisely."
                    },
                    {
                        "role": "user",
                        "content": prompt
                    }
                ],
                "temperature": config.temperature,
            });

            Ok((url, headers, body))
        }
        Provider::Anthropic => {
            let url = config
                .api_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com/v1/messages".to_string());

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

            let body = json!({
                "model": model,
                "max_tokens": 4096,
                "messages": [
                    {
                        "role": "user",
                        "content": prompt
                    }
                ],
                "system": "You are a helpful assistant that summarizes text accurately and concisely.",
                "temperature": config.temperature,
            });

            Ok((url, headers, body))
        }
    }
}

/// Extract summary from API response.
fn extract_summary(provider: &Provider, response: &Value) -> Result<String> {
    match provider {
        Provider::Openai | Provider::Custom => response["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                Error::Node("Could not extract summary from OpenAI response".to_string())
            }),
        Provider::Anthropic => response["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                Error::Node("Could not extract summary from Anthropic response".to_string())
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_text_from_string() {
        let config = SummarizeConfig {
            text: Some("Hello world".to_string()),
            text_field: None,
            style: SummaryStyle::Brief,
            custom_prompt: None,
            max_length: None,
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: None,
            language: None,
        };
        let input = json!({});
        let result = get_text(&config, &input).unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_get_text_from_input() {
        let config = SummarizeConfig {
            text: None,
            text_field: None,
            style: SummaryStyle::Brief,
            custom_prompt: None,
            max_length: None,
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: None,
            language: None,
        };
        let input = json!({"text": "Input text here"});
        let result = get_text(&config, &input).unwrap();
        assert_eq!(result, "Input text here");
    }

    #[test]
    fn test_get_text_from_field() {
        let config = SummarizeConfig {
            text: None,
            text_field: Some("data.content".to_string()),
            style: SummaryStyle::Brief,
            custom_prompt: None,
            max_length: None,
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: None,
            language: None,
        };
        let input = json!({"data": {"content": "Nested content"}});
        let result = get_text(&config, &input).unwrap();
        assert_eq!(result, "Nested content");
    }

    #[test]
    fn test_build_prompt_brief() {
        let config = SummarizeConfig {
            text: None,
            text_field: None,
            style: SummaryStyle::Brief,
            custom_prompt: None,
            max_length: None,
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: None,
            language: None,
        };
        let prompt = build_prompt(&config, "Test text");
        assert!(prompt.contains("brief"));
        assert!(prompt.contains("Test text"));
    }

    #[test]
    fn test_build_prompt_bullets() {
        let config = SummarizeConfig {
            text: None,
            text_field: None,
            style: SummaryStyle::Bullets,
            custom_prompt: None,
            max_length: None,
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: None,
            language: None,
        };
        let prompt = build_prompt(&config, "Test text");
        assert!(prompt.contains("bulleted list"));
    }

    #[test]
    fn test_build_prompt_with_options() {
        let config = SummarizeConfig {
            text: None,
            text_field: None,
            style: SummaryStyle::Brief,
            custom_prompt: None,
            max_length: Some("100 words".to_string()),
            provider: Provider::Openai,
            model: None,
            api_url: None,
            credential: None,
            temperature: 0.3,
            context: Some("Technical document".to_string()),
            language: Some("Spanish".to_string()),
        };
        let prompt = build_prompt(&config, "Test text");
        assert!(prompt.contains("100 words"));
        assert!(prompt.contains("Spanish"));
        assert!(prompt.contains("Technical document"));
    }

    #[test]
    fn test_extract_summary_openai() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": "This is the summary."
                }
            }]
        });
        let summary = extract_summary(&Provider::Openai, &response).unwrap();
        assert_eq!(summary, "This is the summary.");
    }

    #[test]
    fn test_extract_summary_anthropic() {
        let response = json!({
            "content": [{
                "type": "text",
                "text": "Anthropic summary here."
            }]
        });
        let summary = extract_summary(&Provider::Anthropic, &response).unwrap();
        assert_eq!(summary, "Anthropic summary here.");
    }

    #[tokio::test]
    async fn test_summarize_node_no_credentials() {
        let node = SummarizeNode::new();
        let config = json!({
            "text": "Some text to summarize"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("credential"));
    }

    #[tokio::test]
    async fn test_summarize_node_empty_text() {
        let node = SummarizeNode::new();
        let config = json!({
            "text": ""
        });
        let mut credentials = std::collections::HashMap::new();
        credentials.insert("openai".to_string(), "test-key".to_string());
        let ctx = NodeContext::new("exec-1", "test").with_credentials(credentials);

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No text"));
    }
}
