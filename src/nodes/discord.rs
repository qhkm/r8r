/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Discord node - send messages to Discord channels via Bot API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Discord node for sending messages.
pub struct DiscordNode;

impl DiscordNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiscordNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct DiscordConfig {
    /// Channel ID to send the message to (required when using bot token)
    #[serde(default)]
    channel_id: Option<String>,

    /// Message content
    text: String,

    /// Message embeds (rich format)
    #[serde(default)]
    embeds: Option<Vec<Value>>,

    /// Bot token (alternative to webhook_url)
    #[serde(default)]
    token: Option<String>,

    /// Credential name for the bot token
    #[serde(default)]
    credential: Option<String>,

    /// Webhook URL (alternative to bot token + channel_id)
    #[serde(default)]
    webhook_url: Option<String>,
}

#[async_trait]
impl Node for DiscordNode {
    fn node_type(&self) -> &str {
        "discord"
    }

    fn description(&self) -> &str {
        "Send messages to Discord channels via Bot API or webhook"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DiscordConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid discord config: {}", e)))?;

        // Resolve token from credential store or inline
        let token = if let Some(cred_name) = &config.credential {
            ctx.credentials.get(cred_name).cloned()
        } else {
            config.token.clone()
        };

        debug!(
            has_channel = config.channel_id.is_some(),
            has_webhook = config.webhook_url.is_some(),
            "Sending Discord message"
        );

        // Prefer webhook if provided (no token required)
        if let Some(webhook_url) = &config.webhook_url {
            send_via_webhook(webhook_url, &config).await
        } else {
            let token = token.ok_or_else(|| {
                Error::Node(
                    "Discord requires 'webhook_url', or 'token'/'credential' + 'channel_id'"
                        .to_string(),
                )
            })?;
            let channel_id = config.channel_id.clone().ok_or_else(|| {
                Error::Node("Discord with bot token requires 'channel_id'".to_string())
            })?;
            send_via_api(&token, &channel_id, &config).await
        }
    }
}

fn build_payload(config: &DiscordConfig) -> Value {
    let mut payload = json!({
        "content": config.text,
    });
    if let Some(embeds) = &config.embeds {
        payload["embeds"] = json!(embeds);
    }
    payload
}

async fn send_via_webhook(webhook_url: &str, config: &DiscordConfig) -> Result<NodeResult> {
    let client = reqwest::Client::new();
    let payload = build_payload(config);

    let response = client
        .post(webhook_url)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Discord webhook request failed: {}", e)))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    // Discord webhooks return 204 No Content on success
    if status.is_success() {
        Ok(NodeResult::new(json!({
            "success": true,
            "method": "webhook",
        })))
    } else {
        Err(Error::Node(format!("Discord webhook error: {}", body)))
    }
}

async fn send_via_api(token: &str, channel_id: &str, config: &DiscordConfig) -> Result<NodeResult> {
    let client = reqwest::Client::new();
    let payload = build_payload(config);

    let url = format!(
        "https://discord.com/api/v10/channels/{}/messages",
        channel_id
    );
    let response = client
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Discord API request failed: {}", e)))?;

    let status = response.status();
    let body: Value = response.json().await.unwrap_or(json!({}));

    if status.is_success() {
        Ok(NodeResult::new(json!({
            "success": true,
            "method": "api",
            "channel_id": body["channel_id"],
            "message_id": body["id"],
        })))
    } else {
        Err(Error::Node(format!(
            "Discord API error: {}",
            body["message"].as_str().unwrap_or("unknown")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discord_config_parse() {
        let config = json!({
            "channel_id": "123456789",
            "text": "Hello, Discord!",
            "token": "MTIzNDU2"
        });

        let parsed: DiscordConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.channel_id, Some("123456789".to_string()));
        assert_eq!(parsed.text, "Hello, Discord!");
    }

    #[tokio::test]
    async fn test_discord_embeds_config() {
        let config = json!({
            "channel_id": "123456789",
            "text": "Check this out",
            "embeds": [
                {
                    "title": "Report",
                    "description": "Daily report ready",
                    "color": 3447003
                }
            ],
            "token": "MTIzNDU2"
        });

        let parsed: DiscordConfig = serde_json::from_value(config).unwrap();
        assert!(parsed.embeds.is_some());
        assert_eq!(parsed.embeds.as_ref().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_discord_missing_auth() {
        let node = DiscordNode::new();
        let config = json!({
            "channel_id": "123456789",
            "text": "Hello"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_discord_webhook_no_token_needed() {
        let node = DiscordNode::new();
        let config = json!({
            "text": "Hello via webhook",
            "webhook_url": "https://discord.com/api/webhooks/xxx/yyy"
        });
        let ctx = NodeContext::new("exec-1", "test");

        // Should not fail on missing token; failing only on network/webhook auth
        let result = node.execute(&config, &ctx).await;
        // Either network error (no internet) or webhook auth error — but NOT "requires token"
        if let Err(e) = &result {
            let msg = e.to_string();
            assert!(!msg.contains("requires"));
        }
    }
}
