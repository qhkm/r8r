/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Approval node - pauses workflow for human/agent approval.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use crate::engine::executor::emit_audit;
use crate::error::{Error, Result};
use crate::notifications::{send_approval_notifications, NotifyChannel, NotifyContext};
use crate::storage::{ApprovalRequest, AuditEventType};

use super::types::{Node, NodeContext, NodeResult};

/// Node that pauses execution until approved/rejected externally.
pub struct ApprovalNode;

impl ApprovalNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ApprovalNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ApprovalConfig {
    /// Short title shown in approval queues.
    title: String,
    /// Optional detail for reviewers.
    #[serde(default)]
    description: Option<String>,
    /// Timeout in seconds before auto decision.
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Action on timeout: "approve" or "reject".
    #[serde(default)]
    default_action: Option<String>,
    /// Notification channels to fire when approval is requested.
    /// Each entry must have a `type` field: `webhook`, `slack`, or `email`.
    #[serde(default)]
    notify: Vec<NotifyChannel>,
    /// Route this approval to a specific user identity (e.g. email or username).
    #[serde(default)]
    assign_to: Option<String>,
    /// Route this approval to one or more groups. Reviewers in any listed group can act.
    #[serde(default)]
    assign_groups: Vec<String>,
}

#[async_trait]
impl Node for ApprovalNode {
    fn node_type(&self) -> &str {
        "approval"
    }

    fn description(&self) -> &str {
        "Pause execution for human or agent approval"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: ApprovalConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid approval config: {}", e)))?;

        if config.timeout_seconds.is_some() && config.default_action.is_none() {
            return Err(Error::Node(
                "timeout_seconds requires default_action (\"approve\" or \"reject\")".to_string(),
            ));
        }

        if let Some(action) = &config.default_action {
            if action != "approve" && action != "reject" {
                return Err(Error::Node(format!(
                    "default_action must be \"approve\" or \"reject\", got \"{}\"",
                    action
                )));
            }
        }

        let approval_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = config
            .timeout_seconds
            .map(|seconds| now + Duration::seconds(seconds as i64));

        let request = ApprovalRequest {
            id: approval_id.clone(),
            execution_id: ctx.execution_id.to_string(),
            node_id: String::new(),
            workflow_name: ctx.workflow_name.to_string(),
            title: config.title.clone(),
            description: config.description.clone(),
            status: "pending".to_string(),
            decision_comment: None,
            decided_by: None,
            context_data: Some(json!({
                "input": ctx.input.clone(),
                "default_action": config.default_action.clone(),
            })),
            created_at: now,
            expires_at,
            decided_at: None,
            assignee: config.assign_to.clone(),
            groups: config.assign_groups.clone(),
        };

        if let Some(storage) = &ctx.storage {
            storage.save_approval_request(&request).await?;
            info!(
                "Created approval request {} for workflow '{}'",
                approval_id, ctx.workflow_name
            );
            emit_audit(
                storage,
                AuditEventType::ApprovalRequested,
                Some(&ctx.execution_id),
                Some(&ctx.workflow_name),
                None,
                "system",
                &format!("Approval requested: {}", config.title),
                Some(json!({ "approval_id": approval_id })),
            )
            .await;
        }

        // Fire notification channels (non-blocking failures)
        if !config.notify.is_empty() {
            let expires_str = expires_at.map(|t| t.to_rfc3339());
            let notify_ctx = NotifyContext {
                approval_id: &approval_id,
                execution_id: &ctx.execution_id,
                workflow_name: &ctx.workflow_name,
                title: &config.title,
                description: config.description.as_deref(),
                expires_at: expires_str.as_deref(),
            };
            send_approval_notifications(&config.notify, &notify_ctx).await;
        }

        Ok(NodeResult::new(json!({
            "approval_id": approval_id,
            "status": "pending",
            "title": config.title,
            "description": config.description,
            "expires_at": expires_at.map(|t| t.to_rfc3339()),
            "default_action": config.default_action,
            "assignee": config.assign_to,
            "groups": config.assign_groups,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_approval_node_basic() {
        let node = ApprovalNode::new();
        assert_eq!(node.node_type(), "approval");

        let config = json!({
            "title": "Test approval",
            "description": "Please approve this"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await.unwrap();
        let data = result.data;
        assert_eq!(data["status"], "pending");
        assert!(data["approval_id"].is_string());
        assert_eq!(data["title"], "Test approval");
    }

    #[tokio::test]
    async fn test_approval_node_with_timeout() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Timed approval",
            "timeout_seconds": 3600,
            "default_action": "reject"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await.unwrap();
        let data = result.data;
        assert_eq!(data["default_action"], "reject");
        assert!(data["expires_at"].is_string());
    }

    #[tokio::test]
    async fn test_approval_node_timeout_requires_default_action() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Bad config",
            "timeout_seconds": 3600
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_approval_node_invalid_default_action() {
        let node = ApprovalNode::new();
        let config = json!({
            "title": "Bad action",
            "timeout_seconds": 3600,
            "default_action": "maybe"
        });
        let ctx = NodeContext::new("test-exec", "test-workflow");
        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
