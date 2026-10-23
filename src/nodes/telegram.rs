/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Telegram node - send messages via Telegram Bot API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Telegram node for sending bot messages.
pub struct TelegramNode;

impl TelegramNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TelegramNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct TelegramConfig {
    /// Target chat ID (numeric or @username)
    chat_id: Value,

    /// Message text
    text: String,

    /// Parse mode: "HTML" or "MarkdownV2"
    #[serde(default)]
    parse_mode: Option<String>,

    /// Disable notification for the message
    #[serde(default)]
    disable_notification: Option<bool>,

    /// Disable link previews
    #[serde(default)]
    disable_web_page_preview: Option<bool>,

    /// Reply to a specific message ID
    #[serde(default)]
    reply_to_message_id: Option<i64>,

    /// Bot token (alternative to credential)
    #[serde(default)]
    token: Option<String>,

    /// Credential name for the bot token
    #[serde(default)]
    credential: Option<String>,
}

#[async_trait]
impl Node for TelegramNode {
    fn node_type(&self) -> &str {
        "telegram"
    }

    fn description(&self) -> &str {
        "Send messages to Telegram chats via Bot API"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: TelegramConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid telegram config: {}", e)))?;

        // Resolve bot token: credential store first, then inline token
        let token = if let Some(cred_name) = &config.credential {
            ctx.credentials.get(cred_name).cloned()
        } else {
            config.token.clone()
        };
        let token = token
            .ok_or_else(|| Error::Node("Telegram requires 'token' or 'credential'".to_string()))?;

        debug!(
            chat_id = %config.chat_id,
            has_parse_mode = config.parse_mode.is_some(),
            "Sending Telegram message"
        );

        let client = reqwest::Client::new();

        let mut payload = json!({
            "chat_id": config.chat_id,
            "text": config.text,
        });
        if let Some(parse_mode) = &config.parse_mode {
            payload["parse_mode"] = json!(parse_mode);
        }
        if let Some(v) = config.disable_notification {
            payload["disable_notification"] = json!(v);
        }
        if let Some(v) = config.disable_web_page_preview {
            payload["disable_web_page_preview"] = json!(v);
        }
        if let Some(v) = config.reply_to_message_id {
            payload["reply_to_message_id"] = json!(v);
        }

        let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Node(format!("Telegram API request failed: {}", e)))?;

        let status = response.status();
        let body: Value = response.json().await.unwrap_or(json!({}));

        if status.is_success() && body["ok"].as_bool() == Some(true) {
            Ok(NodeResult::new(json!({
                "success": true,
                "message_id": body["result"]["message_id"],
                "chat": body["result"]["chat"],
                "date": body["result"]["date"],
            })))
        } else {
            Err(Error::Node(format!(
                "Telegram API error: {}",
                body["description"].as_str().unwrap_or("unknown")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telegram_config_parse() {
        let config = json!({
            "chat_id": "@my_channel",
            "text": "Hello, Telegram!",
            "token": "123456:ABC-DEF"
        });

        let parsed: TelegramConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.chat_id, json!("@my_channel"));
        assert_eq!(parsed.text, "Hello, Telegram!");
        assert_eq!(parsed.parse_mode, None);
    }

    #[tokio::test]
    async fn test_telegram_config_parse_mode() {
        let config = json!({
            "chat_id": 123456789,
            "text": "<b>Bold</b>",
            "parse_mode": "HTML",
            "disable_notification": true
        });

        let parsed: TelegramConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.parse_mode, Some("HTML".to_string()));
        assert_eq!(parsed.disable_notification, Some(true));
    }

    #[tokio::test]
    async fn test_telegram_missing_token_and_credential() {
        let node = TelegramNode::new();
        let config = json!({
            "chat_id": 123456789,
            "text": "Hello"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
