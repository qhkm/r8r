/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Notification channels for approval requests and other workflow events.
//!
//! Supports: webhook (HTTP POST), Slack incoming webhooks, and email (SMTP).

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::ssrf;

/// A single notification channel config.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotifyChannel {
    /// Generic HTTP POST webhook.
    Webhook(WebhookConfig),
    /// Slack incoming webhook.
    Slack(SlackConfig),
    /// Email via SMTP.
    Email(EmailConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookConfig {
    /// URL to POST to.
    pub url: String,
    /// Optional extra headers (key: value).
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackConfig {
    /// Slack incoming webhook URL.
    pub webhook_url: String,
    /// Optional channel override (e.g. "#approvals").
    #[serde(default)]
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmailConfig {
    /// Comma-separated list of recipient email addresses.
    pub to: String,
    /// Optional subject line override.
    #[serde(default)]
    pub subject: Option<String>,
}

/// Context passed to notification channels.
pub struct NotifyContext<'a> {
    pub approval_id: &'a str,
    pub execution_id: &'a str,
    pub workflow_name: &'a str,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub expires_at: Option<&'a str>,
}

/// Send notifications on all configured channels. Failures are logged but not propagated.
pub async fn send_approval_notifications(channels: &[NotifyChannel], ctx: &NotifyContext<'_>) {
    for channel in channels {
        match channel {
            NotifyChannel::Webhook(cfg) => {
                if let Err(e) = send_webhook(cfg, ctx).await {
                    warn!("Webhook notification failed ({}): {}", cfg.url, e);
                } else {
                    info!("Webhook notification sent for approval {}", ctx.approval_id);
                }
            }
            NotifyChannel::Slack(cfg) => {
                if let Err(e) = send_slack(cfg, ctx).await {
                    warn!("Slack notification failed: {}", e);
                } else {
                    info!("Slack notification sent for approval {}", ctx.approval_id);
                }
            }
            NotifyChannel::Email(cfg) => {
                info!(
                    "Email notification for approval {} → {} (subject: {})",
                    ctx.approval_id,
                    cfg.to,
                    cfg.subject.as_deref().unwrap_or("Approval required")
                );
                if let Err(e) = send_email_smtp(cfg, ctx).await {
                    warn!("Email notification failed: {}", e);
                }
            }
        }
    }
}

/// Build a reqwest client with a 10-second timeout.
fn notify_client() -> anyhow::Result<reqwest::Client> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    Ok(client)
}

async fn send_webhook(cfg: &WebhookConfig, ctx: &NotifyContext<'_>) -> anyhow::Result<()> {
    // SSRF protection: reject private/internal URLs
    ssrf::check_ssrf(&cfg.url)
        .map_err(|e| anyhow::anyhow!("Webhook URL rejected (SSRF): {}", e))?;

    let payload = build_payload(ctx);
    let client = notify_client()?;
    let mut req = client.post(&cfg.url).json(&payload);
    for (k, v) in &cfg.headers {
        req = req.header(k, v);
    }

    let resp = req.send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from webhook", resp.status());
    }
    Ok(())
}

async fn send_slack(cfg: &SlackConfig, ctx: &NotifyContext<'_>) -> anyhow::Result<()> {
    // SSRF protection: reject private/internal URLs
    ssrf::check_ssrf(&cfg.webhook_url)
        .map_err(|e| anyhow::anyhow!("Slack webhook URL rejected (SSRF): {}", e))?;

    let mut blocks = vec![
        json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!("*Approval required:* {}", ctx.title)
            }
        }),
        json!({
            "type": "section",
            "fields": [
                { "type": "mrkdwn", "text": format!("*Workflow:*
        {}", ctx.workflow_name) },
                { "type": "mrkdwn", "text": format!("*Approval ID:*
        `{}`", ctx.approval_id) },
            ]
        }),
    ];

    if let Some(desc) = ctx.description {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": desc }
        }));
    }

    if let Some(exp) = ctx.expires_at {
        blocks.push(json!({
            "type": "context",
            "elements": [{ "type": "mrkdwn", "text": format!("Expires: {}", exp) }]
        }));
    }

    let mut payload = json!({ "blocks": blocks });
    if let Some(ch) = &cfg.channel {
        payload["channel"] = json!(ch);
    }

    let client = notify_client()?;
    let resp = client.post(&cfg.webhook_url).json(&payload).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from Slack", resp.status());
    }
    Ok(())
}

async fn send_email_smtp(cfg: &EmailConfig, ctx: &NotifyContext<'_>) -> anyhow::Result<()> {
    let smtp_url = match std::env::var("R8R_SMTP_RELAY_URL") {
        Ok(u) => u,
        Err(_) => {
            info!(
                "R8R_SMTP_RELAY_URL not set; skipping email to {} for approval {}",
                cfg.to, ctx.approval_id
            );
            return Ok(());
        }
    };

    // SSRF protection: reject private/internal relay URLs
    ssrf::check_ssrf(&smtp_url)
        .map_err(|e| anyhow::anyhow!("SMTP relay URL rejected (SSRF): {}", e))?;

    let default_subject = format!("Approval required: {}", ctx.title);
    let subject = cfg.subject.as_deref().unwrap_or(&default_subject);

    let payload = json!({
        "to": cfg.to,
        "subject": subject,
        "html": build_email_html(ctx),
    });

    let client = notify_client()?;
    let resp = client.post(&smtp_url).json(&payload).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {} from SMTP relay", resp.status());
    }
    Ok(())
}

fn build_payload(ctx: &NotifyContext<'_>) -> Value {
    json!({
        "event": "approval_requested",
        "approval_id": ctx.approval_id,
        "execution_id": ctx.execution_id,
        "workflow_name": ctx.workflow_name,
        "title": ctx.title,
        "description": ctx.description,
        "expires_at": ctx.expires_at,
    })
}

/// Escape HTML special characters to prevent XSS in email bodies.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn build_email_html(ctx: &NotifyContext<'_>) -> String {
    let desc_html = ctx
        .description
        .map(|d| format!("<p>{}</p>", html_escape(d)))
        .unwrap_or_default();
    let expires_html = ctx
        .expires_at
        .map(|e| format!("<p><em>Expires: {}</em></p>", html_escape(e)))
        .unwrap_or_default();

    format!(
        r#"<h2>Approval Required: {title}</h2>
<p><strong>Workflow:</strong> {workflow}</p>
<p><strong>Approval ID:</strong> <code>{approval_id}</code></p>
{desc}
{expires}
<p>Use <code>r8r runs</code> to see pending approvals or the REST API to decide.</p>"#,
        title = html_escape(ctx.title),
        workflow = html_escape(ctx.workflow_name),
        approval_id = html_escape(ctx.approval_id),
        desc = desc_html,
        expires = expires_html,
    )
}
