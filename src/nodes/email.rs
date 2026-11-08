/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Email node - send emails via SMTP or API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

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

    /// Mailgun sending domain (default: the domain of `from`)
    #[serde(default)]
    mailgun_domain: Option<String>,

    /// Mailgun API base URL (default `https://api.mailgun.net`; EU accounts
    /// use `https://api.eu.mailgun.net`)
    #[serde(default)]
    mailgun_base_url: Option<String>,

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

/// Providers that are documented but not implemented. Returning an error
/// beats reporting a message as sent when nothing went out.
fn unsupported_provider(provider: &str) -> Option<String> {
    match provider {
        "smtp" => Some(
            "SMTP sending is not implemented yet. Set `provider` to resend, sendgrid or mailgun"
                .to_string(),
        ),
        _ => None,
    }
}

#[async_trait]
impl Node for EmailNode {
    fn node_type(&self) -> &str {
        "email"
    }

    fn description(&self) -> &str {
        "Send emails via SMTP or API providers"
    }

    fn validate_config(&self, config: &Value) -> Result<()> {
        let provider = config
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(default_provider);
        match unsupported_provider(&provider) {
            Some(reason) => Err(Error::Validation(reason)),
            None => Ok(()),
        }
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
            "smtp" => Err(Error::Node(
                unsupported_provider("smtp").unwrap_or_default(),
            )),
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
    let api_key = api_key.ok_or_else(|| Error::Node("Mailgun requires api_key".to_string()))?;
    let domain = match &config.mailgun_domain {
        Some(d) => d.clone(),
        None => address_domain(&config.from).ok_or_else(|| {
            Error::Node("Mailgun needs mailgun_domain (could not read a domain from `from`)".into())
        })?,
    };
    let base = config
        .mailgun_base_url
        .as_deref()
        .unwrap_or("https://api.mailgun.net")
        .trim_end_matches('/');

    let response = reqwest::Client::new()
        .post(format!("{}/v3/{}/messages", base, domain))
        .basic_auth("api", Some(&api_key))
        .form(&mailgun_form(config))
        .send()
        .await
        .map_err(|e| Error::Node(format!("Mailgun request failed: {}", e)))?;

    let status = response.status();
    let response_body: Value = response.json().await.unwrap_or(json!({}));
    if status.is_success() {
        Ok(NodeResult::new(json!({
            "success": true,
            "provider": "mailgun",
            "id": response_body["id"],
            "to": config.to.to_vec(),
        })))
    } else {
        Err(Error::Node(format!(
            "Mailgun error ({}): {}",
            status, response_body
        )))
    }
}

/// Form fields for Mailgun's `POST /v3/{domain}/messages`.
fn mailgun_form(config: &EmailConfig) -> Vec<(&'static str, String)> {
    let mut form = vec![
        ("from", config.from.clone()),
        ("subject", config.subject.clone()),
    ];
    form.extend(config.to.to_vec().into_iter().map(|a| ("to", a)));
    if let Some(cc) = &config.cc {
        form.extend(cc.to_vec().into_iter().map(|a| ("cc", a)));
    }
    if let Some(bcc) = &config.bcc {
        form.extend(bcc.to_vec().into_iter().map(|a| ("bcc", a)));
    }
    if let Some(text) = &config.body {
        form.push(("text", text.clone()));
    }
    if let Some(html) = &config.html {
        form.push(("html", html.clone()));
    }
    if let Some(reply_to) = &config.reply_to {
        form.push(("h:Reply-To", reply_to.clone()));
    }
    form
}

/// `Acme <hi@mail.acme.com>` or `hi@mail.acme.com` -> `mail.acme.com`.
fn address_domain(address: &str) -> Option<String> {
    let addr = match (address.rfind('<'), address.rfind('>')) {
        (Some(open), Some(close)) if open < close => &address[open + 1..close],
        _ => address,
    };
    let domain = addr.trim().rsplit_once('@')?.1.trim();
    (!domain.is_empty()).then(|| domain.to_string())
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

    #[tokio::test]
    async fn smtp_is_refused_not_faked() {
        let node = EmailNode::new();
        let config = json!({"provider": "smtp", "from": "a@x.test", "to": "b@x.test", "subject": "hi", "body": "x"});
        let err = node.validate_config(&config).unwrap_err().to_string();
        assert!(err.contains("SMTP sending is not implemented"), "{err}");
        let ctx = NodeContext::new("exec-1", "test");
        assert!(node.execute(&config, &ctx).await.is_err());
    }

    #[test]
    fn default_provider_is_refused_too() {
        let node = EmailNode::new();
        let config = json!({"from": "a@x.test", "to": "b@x.test", "subject": "hi"});
        assert!(node.validate_config(&config).is_err());
        let config =
            json!({"provider": "resend", "from": "a@x.test", "to": "b@x.test", "subject": "hi"});
        assert!(node.validate_config(&config).is_ok());
    }

    #[test]
    fn mailgun_domain_from_address() {
        assert_eq!(
            address_domain("Acme <hi@mail.acme.com>").as_deref(),
            Some("mail.acme.com")
        );
        assert_eq!(address_domain("hi@acme.com").as_deref(), Some("acme.com"));
        assert_eq!(address_domain("no-at-sign"), None);
    }

    #[test]
    fn mailgun_form_fields() {
        let config: EmailConfig = serde_json::from_value(json!({
            "provider": "mailgun",
            "from": "a@x.test",
            "to": ["b@x.test", "c@x.test"],
            "bcc": "d@x.test",
            "subject": "hi",
            "body": "text",
            "reply_to": "r@x.test"
        }))
        .unwrap();
        let form = mailgun_form(&config);
        let get = |k: &str| {
            form.iter()
                .filter(|(f, _)| *f == k)
                .map(|(_, v)| v.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(get("to"), ["b@x.test", "c@x.test"]);
        assert_eq!(get("bcc"), ["d@x.test"]);
        assert_eq!(get("text"), ["text"]);
        assert_eq!(get("h:Reply-To"), ["r@x.test"]);
        assert!(get("html").is_empty());
    }

    #[tokio::test]
    async fn mailgun_reports_http_errors() {
        // Nothing listens on port 9: the send must fail, not claim success.
        let node = EmailNode::new();
        let config = json!({
            "provider": "mailgun", "api_key": "k", "from": "a@x.test", "to": "b@x.test",
            "subject": "hi", "body": "x", "mailgun_base_url": "http://127.0.0.1:9"
        });
        let ctx = NodeContext::new("exec-1", "test");
        assert!(node.execute(&config, &ctx).await.is_err());
    }
}
