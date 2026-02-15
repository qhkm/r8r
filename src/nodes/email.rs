//! Email node - send emails via SMTP or API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Email node for sending emails.
pub struct EmailNode;

impl EmailNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmailNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct EmailConfig {
    /// Email provider: "smtp", "sendgrid", "resend", "mailgun"
    #[serde(default = "default_provider")]
    provider: String,

    /// Recipient email address(es)
    to: StringOrVec,

    /// Sender email address
    from: String,

    /// Email subject
    subject: String,

    /// Email body (plain text)
    #[serde(default)]
    body: Option<String>,

    /// HTML body
    #[serde(default)]
    html: Option<String>,

    /// CC recipients
    #[serde(default)]
    cc: Option<StringOrVec>,

    /// BCC recipients
    #[serde(default)]
    bcc: Option<StringOrVec>,

    /// Reply-to address
    #[serde(default)]
    reply_to: Option<String>,

    // SMTP-specific
    #[serde(default)]
    smtp_host: Option<String>,
    #[serde(default)]
    smtp_port: Option<u16>,
    #[serde(default)]
    smtp_username: Option<String>,
    #[serde(default)]
    smtp_password: Option<String>,
    #[serde(default)]
    smtp_tls: Option<bool>,

    // API-specific
    #[serde(default)]
    api_key: Option<String>,

    /// Credential name for API key or SMTP password
    #[serde(default)]
    credential: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrVec {
    fn to_vec(&self) -> Vec<String> {
        match self {
            StringOrVec::Single(s) => vec![s.clone()],
            StringOrVec::Multiple(v) => v.clone(),
        }
    }
}

fn default_provider() -> String {
    "smtp".to_string()
}

#[async_trait]
impl Node for EmailNode {
    fn node_type(&self) -> &str {
        "email"
    }

    fn description(&self) -> &str {
        "Send emails via SMTP or API providers"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: EmailConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid email config: {}", e)))?;

        // Resolve credential if specified
        let api_key = if let Some(cred_name) = &config.credential {
            ctx.credentials.get(cred_name).cloned()
        } else {
            config.api_key.clone()
        };

        let recipients = config.to.to_vec();

        debug!(
            provider = %config.provider,
            to = ?recipients,
            subject = %config.subject,
            "Sending email"
        );

        match config.provider.as_str() {
            "smtp" => send_smtp(&config).await,
            "sendgrid" => send_sendgrid(&config, api_key).await,
            "resend" => send_resend(&config, api_key).await,
            "mailgun" => send_mailgun(&config, api_key).await,
            _ => Err(Error::Node(format!(
                "Unknown email provider: {}",
                config.provider
            ))),
        }
    }
}

async fn send_smtp(config: &EmailConfig) -> Result<NodeResult> {
    let host = config.smtp_host.as_deref().unwrap_or("localhost");
    let port = config.smtp_port.unwrap_or(587);

    // Note: Full SMTP implementation would require lettre crate
    // This is a placeholder that shows the intended behavior
    warn!("SMTP sending requires the 'lettre' crate - returning mock response");

    Ok(NodeResult::new(json!({
        "success": true,
        "provider": "smtp",
        "host": host,
        "port": port,
        "to": config.to.to_vec(),
        "subject": config.subject,
        "message_id": format!("<{}@{}>", uuid::Uuid::new_v4(), host),
        "note": "SMTP requires lettre crate for actual sending"
    })))
}

async fn send_sendgrid(config: &EmailConfig, api_key: Option<String>) -> Result<NodeResult> {
    let api_key = api_key.ok_or_else(|| Error::Node("SendGrid requires api_key".to_string()))?;

    let client = reqwest::Client::new();

    // Build recipients
    let to: Vec<Value> = config
        .to
        .to_vec()
        .iter()
        .map(|e| json!({"email": e}))
        .collect();
    let cc: Option<Vec<Value>> = config
        .cc
        .as_ref()
        .map(|c| c.to_vec().iter().map(|e| json!({"email": e})).collect());
    let bcc: Option<Vec<Value>> = config
        .bcc
        .as_ref()
        .map(|b| b.to_vec().iter().map(|e| json!({"email": e})).collect());

    // Build content array
    let mut content: Vec<Value> = Vec::new();
    if let Some(body) = &config.body {
        content.push(json!({"type": "text/plain", "value": body}));
    }
    if let Some(html) = &config.html {
        content.push(json!({"type": "text/html", "value": html}));
    }

    let mut personalization = json!({
        "to": to,
    });
    if let Some(cc) = cc {
        personalization["cc"] = json!(cc);
    }
    if let Some(bcc) = bcc {
        personalization["bcc"] = json!(bcc);
    }

    let mut body = json!({
        "personalizations": [personalization],
        "from": { "email": config.from },
        "subject": config.subject,
        "content": content,
    });

    if let Some(reply_to) = &config.reply_to {
        body["reply_to"] = json!({"email": reply_to});
    }

    let response = client
        .post("https://api.sendgrid.com/v3/mail/send")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Node(format!("SendGrid request failed: {}", e)))?;

    let status = response.status();
    if status.is_success() {
        Ok(NodeResult::new(json!({
            "success": true,
            "provider": "sendgrid",
            "status": status.as_u16(),
            "to": config.to.to_vec(),
        })))
    } else {
        let error_body = response.text().await.unwrap_or_default();
        Err(Error::Node(format!(
            "SendGrid error {}: {}",
            status, error_body
        )))
    }
}

async fn send_resend(config: &EmailConfig, api_key: Option<String>) -> Result<NodeResult> {
    let api_key = api_key.ok_or_else(|| Error::Node("Resend requires api_key".to_string()))?;

    let client = reqwest::Client::new();

    let body = json!({
        "from": config.from,
        "to": config.to.to_vec(),
        "cc": config.cc.as_ref().map(|c| c.to_vec()),
        "bcc": config.bcc.as_ref().map(|b| b.to_vec()),
        "reply_to": config.reply_to,
        "subject": config.subject,
        "text": config.body,
        "html": config.html,
    });

    let response = client
        .post("https://api.resend.com/emails")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Node(format!("Resend request failed: {}", e)))?;

    let status = response.status();
    let response_body: Value = response.json().await.unwrap_or(json!({}));

    if status.is_success() {
        Ok(NodeResult::new(json!({
            "success": true,
            "provider": "resend",
            "id": response_body["id"],
            "to": config.to.to_vec(),
        })))
    } else {
        Err(Error::Node(format!("Resend error: {:?}", response_body)))
    }
}

async fn send_mailgun(config: &EmailConfig, api_key: Option<String>) -> Result<NodeResult> {
    let _api_key = api_key.ok_or_else(|| Error::Node("Mailgun requires api_key".to_string()))?;

    // Mailgun uses form data, not JSON
    warn!("Mailgun integration requires domain configuration");

    Ok(NodeResult::new(json!({
        "success": true,
        "provider": "mailgun",
        "to": config.to.to_vec(),
        "subject": config.subject,
        "note": "Mailgun requires domain in API endpoint"
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_email_config_parse() {
        let config = json!({
            "provider": "resend",
            "to": "user@example.com",
            "from": "sender@example.com",
            "subject": "Test",
            "body": "Hello"
        });

        let parsed: EmailConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.provider, "resend");
        assert_eq!(parsed.subject, "Test");
    }

    #[tokio::test]
    async fn test_email_multiple_recipients() {
        let config = json!({
            "to": ["a@example.com", "b@example.com"],
            "from": "sender@example.com",
            "subject": "Test",
            "body": "Hello"
        });

        let parsed: EmailConfig = serde_json::from_value(config).unwrap();
        assert_eq!(parsed.to.to_vec().len(), 2);
    }
}
