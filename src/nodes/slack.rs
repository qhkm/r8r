//! Slack node - send messages to Slack.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Slack node for sending messages.
pub struct SlackNode;

impl SlackNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SlackNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SlackConfig {
    /// Slack channel (e.g., "#general" or "C01234567")
    channel: String,

    /// Message text
    #[serde(default)]
    text: Option<String>,

    /// Block Kit blocks for rich formatting
    #[serde(default)]
    blocks: Option<Vec<Value>>,

    /// Attachments (legacy)
    #[serde(default)]
    attachments: Option<Vec<Value>>,

    /// Bot username override
    #[serde(default)]
    username: Option<String>,

    /// Bot icon emoji (e.g., ":robot_face:")
    #[serde(default)]
    icon_emoji: Option<String>,

    /// Bot icon URL
    #[serde(default)]
    icon_url: Option<String>,

    /// Thread timestamp (for replies)
    #[serde(default)]
    thread_ts: Option<String>,

    /// Whether to reply in thread and also post to channel
    #[serde(default)]
    reply_broadcast: Option<bool>,

    /// OAuth token or webhook URL
    #[serde(default)]
    token: Option<String>,

    /// Webhook URL (alternative to token)
    #[serde(default)]
    webhook_url: Option<String>,

    /// Credential name for token
    #[serde(default)]
    credential: Option<String>,

    /// Unfurl links
    #[serde(default = "default_true")]
    unfurl_links: bool,

    /// Unfurl media
    #[serde(default = "default_true")]
    unfurl_media: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Node for SlackNode {
    fn node_type(&self) -> &str {
        "slack"
    }

    fn description(&self) -> &str {
        "Send messages to Slack channels"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SlackConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid slack config: {}", e)))?;

        // Validate we have either text or blocks
        if config.text.is_none() && config.blocks.is_none() {
            return Err(Error::Node("Slack message requires 'text' or 'blocks'".to_string()));
        }

        // Resolve credential
        let token = if let Some(cred_name) = &config.credential {
            ctx.credentials.get(cred_name).cloned()
        } else {
            config.token.clone()
        };

        debug!(
            channel = %config.channel,
            has_blocks = config.blocks.is_some(),
            "Sending Slack message"
        );

        // Use webhook URL if provided, otherwise use API with token
        if let Some(webhook_url) = &config.webhook_url {
            send_via_webhook(webhook_url, &config).await
        } else {
            let token = token.ok_or_else(|| {
                Error::Node("Slack requires 'token', 'webhook_url', or 'credential'".to_string())
            })?;
            send_via_api(&token, &config).await
        }
    }
}

async fn send_via_webhook(webhook_url: &str, config: &SlackConfig) -> Result<NodeResult> {
    let client = reqwest::Client::new();

    let mut payload = json!({});
    
    if let Some(text) = &config.text {
        payload["text"] = json!(text);
    }
    if let Some(blocks) = &config.blocks {
        payload["blocks"] = json!(blocks);
    }
    if let Some(attachments) = &config.attachments {
        payload["attachments"] = json!(attachments);
    }
    if let Some(username) = &config.username {
        payload["username"] = json!(username);
    }
    if let Some(icon_emoji) = &config.icon_emoji {
        payload["icon_emoji"] = json!(icon_emoji);
    }
    if let Some(icon_url) = &config.icon_url {
        payload["icon_url"] = json!(icon_url);
    }

    let response = client
        .post(webhook_url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Slack webhook request failed: {}", e)))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if status.is_success() && body == "ok" {
        Ok(NodeResult::new(json!({
            "success": true,
            "method": "webhook",
            "channel": config.channel,
        })))
    } else {
        Err(Error::Node(format!("Slack webhook error: {}", body)))
    }
}

async fn send_via_api(token: &str, config: &SlackConfig) -> Result<NodeResult> {
    let client = reqwest::Client::new();

    let mut payload = json!({
        "channel": config.channel,
        "unfurl_links": config.unfurl_links,
        "unfurl_media": config.unfurl_media,
    });

    if let Some(text) = &config.text {
        payload["text"] = json!(text);
    }
    if let Some(blocks) = &config.blocks {
        payload["blocks"] = json!(blocks);
    }
    if let Some(attachments) = &config.attachments {
        payload["attachments"] = json!(attachments);
    }
    if let Some(username) = &config.username {
        payload["username"] = json!(username);
    }
    if let Some(icon_emoji) = &config.icon_emoji {
        payload["icon_emoji"] = json!(icon_emoji);
    }
    if let Some(icon_url) = &config.icon_url {
        payload["icon_url"] = json!(icon_url);
    }
    if let Some(thread_ts) = &config.thread_ts {
        payload["thread_ts"] = json!(thread_ts);
    }
    if let Some(reply_broadcast) = config.reply_broadcast {
        payload["reply_broadcast"] = json!(reply_broadcast);
    }

    let response = client
        .post("https://slack.com/api/chat.postMessage")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Slack API request failed: {}", e)))?;

    let status = response.status();
    let body: Value = response.json().await.unwrap_or(json!({}));

    if status.is_success() && body["ok"].as_bool() == Some(true) {
        Ok(NodeResult::new(json!({
            "success": true,
            "method": "api",
            "channel": body["channel"],
            "ts": body["ts"],
            "message": body["message"],
        })))
    } else {
        Err(Error::Node(format!(
            "Slack API error: {}",
            body["error"].as_str().unwrap_or("unknown")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_slack_config_parse() {
        let config = json!({
            "channel": "#general",
            "text": "Hello, Slack!",
            "token": "xoxb-test-token"
        });

        let parsed: SlackConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.channel, "#general");
        assert_eq!(parsed.text, Some("Hello, Slack!".to_string()));
    }

    #[tokio::test]
    async fn test_slack_blocks_config() {
        let config = json!({
            "channel": "#general",
            "blocks": [
                {
                    "type": "section",
                    "text": {
                        "type": "mrkdwn",
                        "text": "*Bold* and _italic_"
                    }
                }
            ],
            "webhook_url": "https://hooks.slack.com/services/xxx"
        });

        let parsed: SlackConfig = serde_json::from_value(config).unwrap();
        assert!(parsed.blocks.is_some());
        assert_eq!(parsed.blocks.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_slack_missing_text_and_blocks() {
        let node = SlackNode::new();
        let config = json!({
            "channel": "#general",
            "token": "xoxb-test"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
