/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Background processor for expired approval requests.
//!
//! Polls every 30 seconds for pending approvals past their `expires_at` deadline,
//! applies the configured `default_action`, and resumes the paused execution.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::engine::executor::emit_audit;
use crate::engine::Executor;
use crate::storage::{AuditEventType, Storage};

/// Default poll interval: 30 seconds.
const POLL_INTERVAL_MS: u64 = 30_000;

/// Background task that resolves expired approval requests.
pub struct ApprovalTimeoutChecker {
    storage: Arc<dyn Storage>,
    executor: Arc<Executor>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    handle: Option<JoinHandle<()>>,
    poll_interval_ms: u64,
}

impl ApprovalTimeoutChecker {
    pub fn new(storage: Arc<dyn Storage>, executor: Arc<Executor>) -> Self {
        Self {
            storage,
            executor,
            shutdown_tx: None,
            handle: None,
            poll_interval_ms: POLL_INTERVAL_MS,
        }
    }

    /// Set custom poll interval (useful for testing).
    #[allow(dead_code)]
    pub fn with_poll_interval(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }

    /// Start the background polling task.
    pub async fn start(&mut self) {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);

        let storage = self.storage.clone();
        let executor = self.executor.clone();
        let poll_interval = self.poll_interval_ms;

        let handle = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(poll_interval));

            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Approval timeout checker received shutdown signal");
                        break;
                    }
                    _ = ticker.tick() => {
                        if let Err(e) = process_expired_approvals(&storage, &executor).await {
                            error!("Error processing expired approvals: {}", e);
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
        info!(
            "Approval timeout checker started with {}ms poll interval",
            self.poll_interval_ms
        );
    }

    /// Stop the background polling task.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        if let Some(handle) = self.handle.take() {
            if let Err(e) = handle.await {
                error!("Approval timeout checker task panicked: {}", e);
            }
        }

        info!("Approval timeout checker stopped");
    }
}

/// Process all expired approval requests.
async fn process_expired_approvals(
    storage: &dyn Storage,
    executor: &Executor,
) -> crate::Result<()> {
    let expired = storage.list_expired_approvals().await?;
    if expired.is_empty() {
        return Ok(());
    }

    info!("Found {} expired approval(s) to process", expired.len());

    for mut approval in expired {
        let default_action = approval
            .context_data
            .as_ref()
            .and_then(|ctx| ctx.get("default_action"))
            .and_then(Value::as_str)
            .unwrap_or("reject")
            .to_string();

        info!(
            "Processing expired approval {} (workflow '{}', default_action='{}')",
            approval.id, approval.workflow_name, default_action
        );

        // Load checkpoint for this execution
        let mut checkpoint = match storage.get_latest_checkpoint(&approval.execution_id).await {
            Ok(Some(cp)) => cp,
            Ok(None) => {
                warn!(
                    "No checkpoint found for expired approval {} (execution {}), marking expired",
                    approval.id, approval.execution_id
                );
                approval.status = "expired".to_string();
                approval.decided_by = Some("timeout".to_string());
                approval.decided_at = Some(Utc::now());
                storage.save_approval_request(&approval).await?;
                continue;
            }
            Err(e) => {
                error!(
                    "Failed to load checkpoint for expired approval {}: {}",
                    approval.id, e
                );
                continue;
            }
        };

        // Inject the timeout decision into checkpoint outputs
        let mut outputs = checkpoint
            .node_outputs
            .as_object()
            .cloned()
            .unwrap_or_default();

        let node_id_hint = if approval.node_id.is_empty() {
            None
        } else {
            Some(approval.node_id.as_str())
        };

        let resolved_node =
            inject_timeout_decision(&mut outputs, &approval.id, node_id_hint, &default_action);

        let Some(node_id) = resolved_node else {
            warn!(
                "Could not find approval {} in checkpoint outputs, marking expired without resume",
                approval.id
            );
            approval.status = "expired".to_string();
            approval.decided_by = Some("timeout".to_string());
            approval.decided_at = Some(Utc::now());
            storage.save_approval_request(&approval).await?;
            continue;
        };

        checkpoint.node_outputs = Value::Object(outputs);
        if let Err(e) = storage.save_checkpoint(&checkpoint).await {
            error!(
                "Failed to save checkpoint for expired approval {}: {}",
                approval.id, e
            );
            continue;
        }

        // Update approval request
        let decided_status = if default_action == "approve" {
            "approved"
        } else {
            "rejected"
        };
        approval.status = decided_status.to_string();
        approval.node_id = node_id;
        approval.decided_by = Some("timeout".to_string());
        approval.decided_at = Some(Utc::now());
        if let Err(e) = storage.save_approval_request(&approval).await {
            error!("Failed to update expired approval {}: {}", approval.id, e);
            continue;
        }

        emit_audit(
            storage,
            AuditEventType::ApprovalTimedOut,
            Some(&approval.execution_id),
            Some(&approval.workflow_name),
            None,
            "system",
            &format!(
                "Approval timed out (auto-{}): {}",
                decided_status, approval.id
            ),
            Some(json!({
                "approval_id": approval.id,
                "decision": decided_status,
                "default_action": default_action,
            })),
        )
        .await;

        // Resume the paused execution
        match executor
            .resume_from_checkpoint(&approval.execution_id)
            .await
        {
            Ok(execution) => {
                info!(
                    "Expired approval {} resolved as '{}', execution {} resumed (status: {})",
                    approval.id, decided_status, execution.id, execution.status
                );
            }
            Err(e) => {
                error!(
                    "Expired approval {} resolved but failed to resume execution {}: {}",
                    approval.id, approval.execution_id, e
                );
            }
        }
    }

    Ok(())
}

/// Inject timeout decision into checkpoint outputs.
///
/// Mirrors `inject_approval_decision_into_checkpoint` in `mcp/tools.rs` but
/// uses "timeout" semantics (no comment field, decided_by = "timeout").
fn inject_timeout_decision(
    checkpoint_outputs: &mut serde_json::Map<String, Value>,
    approval_id: &str,
    node_id_hint: Option<&str>,
    decision: &str,
) -> Option<String> {
    let status = if decision == "approve" {
        "approved"
    } else {
        "rejected"
    };

    // Try hint first, then scan outputs for matching approval_id
    let mut resolved_node_id = node_id_hint
        .filter(|id| checkpoint_outputs.contains_key(*id))
        .map(ToOwned::to_owned);

    if resolved_node_id.is_none() {
        resolved_node_id = checkpoint_outputs.iter().find_map(|(node_id, output)| {
            output
                .get("approval_id")
                .and_then(Value::as_str)
                .filter(|id| *id == approval_id)
                .map(|_| node_id.clone())
        });
    }

    let node_id = resolved_node_id?;
    let existing = checkpoint_outputs
        .get(&node_id)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let mut output_obj = existing.as_object().cloned().unwrap_or_default();
    output_obj.insert("approval_id".to_string(), serde_json::json!(approval_id));
    output_obj.insert("status".to_string(), serde_json::json!(status));
    output_obj.insert("decision".to_string(), serde_json::json!(decision));
    output_obj.insert("decided_by".to_string(), serde_json::json!("timeout"));
    output_obj.insert("comment".to_string(), Value::Null);
    checkpoint_outputs.insert(node_id.clone(), Value::Object(output_obj));
    Some(node_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_inject_timeout_decision_with_hint() {
        let mut outputs = serde_json::Map::new();
        outputs.insert(
            "approval-node".to_string(),
            json!({"approval_id": "abc-123", "status": "pending"}),
        );

        let result =
            inject_timeout_decision(&mut outputs, "abc-123", Some("approval-node"), "reject");

        assert_eq!(result, Some("approval-node".to_string()));
        let node_output = &outputs["approval-node"];
        assert_eq!(node_output["status"], "rejected");
        assert_eq!(node_output["decision"], "reject");
        assert_eq!(node_output["decided_by"], "timeout");
    }

    #[test]
    fn test_inject_timeout_decision_scan_by_approval_id() {
        let mut outputs = serde_json::Map::new();
        outputs.insert(
            "some-node".to_string(),
            json!({"approval_id": "xyz-789", "status": "pending"}),
        );

        let result = inject_timeout_decision(&mut outputs, "xyz-789", None, "approve");

        assert_eq!(result, Some("some-node".to_string()));
        let node_output = &outputs["some-node"];
        assert_eq!(node_output["status"], "approved");
        assert_eq!(node_output["decision"], "approve");
        assert_eq!(node_output["decided_by"], "timeout");
    }

    #[test]
    fn test_inject_timeout_decision_not_found() {
        let mut outputs = serde_json::Map::new();
        outputs.insert("other-node".to_string(), json!({"result": "hello"}));

        let result = inject_timeout_decision(&mut outputs, "missing-id", None, "reject");
        assert!(result.is_none());
    }
}
