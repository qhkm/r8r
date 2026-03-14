/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Bridge event handlers — dispatches inbound events to r8r subsystems.
//!
//! Incoming events from ZeptoClaw are routed here by the WebSocket receive
//! loop. Each event type maps to a handler that either updates bridge state,
//! forwards to the executor, or is logged as a TODO for future wiring.

use chrono::Utc;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::engine::ExecutionMetadata;
use crate::mcp::tools::inject_approval_decision_into_checkpoint;
use crate::workflow::parse_workflow;

use super::events::{BridgeEvent, BridgeEventEnvelope};
use super::BridgeState;

/// Dispatch an incoming event from ZeptoClaw to the appropriate handler.
///
/// # Arguments
/// - `event_type` — the dotted type string (e.g. `zeptoclaw.health.ping`)
/// - `data` — the event payload as a JSON `Value`
/// - `bridge` — shared bridge state
/// - `client_tx` — sender channel to the WebSocket client (for responses)
pub async fn handle_incoming_event(
    event_type: &str,
    data: &Value,
    correlation_id: Option<&str>,
    bridge: &std::sync::Arc<BridgeState>,
    client_tx: &mpsc::Sender<String>,
) {
    match event_type {
        "zeptoclaw.health.ping" => {
            handle_health_ping(bridge, client_tx).await;
        }
        "zeptoclaw.approval.decision" => {
            handle_approval_decision(data, bridge).await;
        }
        "zeptoclaw.workflow.trigger" => {
            handle_workflow_trigger(data, correlation_id, bridge).await;
        }
        unknown => {
            warn!("Bridge: unknown incoming event type: {}", unknown);
        }
    }
}

/// Handle a health ping from ZeptoClaw.
///
/// Updates `last_ping` and responds with a `HealthStatus` event containing
/// stub values (will be wired to real metrics in Task 4).
async fn handle_health_ping(bridge: &BridgeState, client_tx: &mpsc::Sender<String>) {
    // Update last ping timestamp
    {
        let mut last_ping = bridge.last_ping.lock().await;
        *last_ping = Some(Utc::now());
    }
    info!("Bridge: received health ping, updated last_ping");

    // Respond with health status
    let status = BridgeEvent::HealthStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: 0, // TODO: wire to real uptime in Task 4
        active_executions: 0,
        pending_approvals: 0,
        workflows_loaded: 0,
    };

    let envelope = BridgeEventEnvelope::new(status, None);
    match serde_json::to_string(&envelope) {
        Ok(json) => {
            if let Err(e) = client_tx.send(json).await {
                warn!("Bridge: failed to send health status response: {}", e);
            }
        }
        Err(e) => {
            warn!("Bridge: failed to serialize health status: {}", e);
        }
    }
}

/// Handle an approval decision from ZeptoClaw.
///
/// Resolves the pending approval in storage by calling `decide_approval_request`.
/// Validates that the approval exists and is still pending before updating.
async fn handle_approval_decision(data: &Value, bridge: &std::sync::Arc<BridgeState>) {
    let approval_id = data
        .get("approval_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let decision = data
        .get("decision")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let decided_by = data
        .get("decided_by")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let comment = data.get("reason").and_then(|v| v.as_str());
    info!(
        "Bridge: approval decision received — id={}, decision={}, by={}",
        approval_id, decision, decided_by
    );

    let Some(storage) = bridge.storage_backend().await else {
        warn!(
            "Bridge: no storage backend configured, cannot resolve approval {}",
            approval_id
        );
        return;
    };

    match storage.get_approval_request(approval_id).await {
        Ok(Some(approval)) => {
            if approval.status != "pending" {
                warn!(
                    "Bridge: approval {} is no longer pending (status={}), ignoring decision",
                    approval_id, approval.status
                );
                return;
            }

            // Check if expired
            if let Some(expires_at) = approval.expires_at {
                if Utc::now() > expires_at {
                    warn!(
                        "Bridge: approval {} has expired, ignoring decision",
                        approval_id
                    );
                    return;
                }
            }

            let normalized_decision = match decision {
                "approved" | "approve" => "approve",
                "rejected" | "reject" | "denied" | "deny" => "reject",
                other => {
                    warn!(
                        "Bridge: unknown decision '{}' for approval {}",
                        other, approval_id
                    );
                    return;
                }
            };
            let new_status = if normalized_decision == "approve" {
                "approved"
            } else {
                "rejected"
            };

            let mut checkpoint = match storage.get_latest_checkpoint(&approval.execution_id).await {
                Ok(Some(checkpoint)) => checkpoint,
                Ok(None) => {
                    warn!(
                        "Bridge: no checkpoint found for execution {}, cannot resume approval {}",
                        approval.execution_id, approval_id
                    );
                    return;
                }
                Err(e) => {
                    warn!(
                        "Bridge: error loading checkpoint for execution {}: {}",
                        approval.execution_id, e
                    );
                    return;
                }
            };

            let mut outputs = checkpoint
                .node_outputs
                .as_object()
                .cloned()
                .unwrap_or_default();
            let resolved_node = inject_approval_decision_into_checkpoint(
                &mut outputs,
                &approval.id,
                if approval.node_id.is_empty() {
                    None
                } else {
                    Some(approval.node_id.as_str())
                },
                normalized_decision,
                comment,
            );
            let Some(node_id) = resolved_node else {
                warn!(
                    "Bridge: approval {} not found in latest checkpoint outputs",
                    approval.id
                );
                return;
            };

            checkpoint.node_outputs = Value::Object(outputs);
            if let Err(e) = storage.save_checkpoint(&checkpoint).await {
                warn!(
                    "Bridge: failed to persist checkpoint for approval {}: {}",
                    approval_id, e
                );
                return;
            }

            match storage
                .decide_approval_request(
                    approval_id,
                    new_status,
                    decided_by,
                    comment,
                    Utc::now(),
                    &node_id,
                )
                .await
            {
                Ok(true) => {
                    info!(
                        "Bridge: approval {} resolved as '{}' by {}",
                        approval_id, new_status, decided_by
                    );
                    let bridge = bridge.clone();
                    let execution_id = approval.execution_id.clone();
                    tokio::spawn(async move {
                        let Some(executor) = bridge.create_executor().await else {
                            warn!(
                                "Bridge: executor context not configured, cannot resume execution {}",
                                execution_id
                            );
                            return;
                        };

                        if let Err(e) = executor.resume_from_checkpoint(&execution_id).await {
                            error!(
                                "Bridge: approval resolved but failed to resume execution {}: {}",
                                execution_id, e
                            );
                        }
                    });
                }
                Ok(false) => {
                    warn!(
                        "Bridge: failed to resolve approval {} — may have been decided concurrently",
                        approval_id
                    );
                }
                Err(e) => {
                    warn!("Bridge: error resolving approval {}: {}", approval_id, e);
                }
            }
        }
        Ok(None) => {
            warn!("Bridge: approval {} not found in storage", approval_id);
        }
        Err(e) => {
            warn!("Bridge: error loading approval {}: {}", approval_id, e);
        }
    }
}

/// Handle a workflow trigger request from ZeptoClaw.
///
/// Loads the referenced workflow and executes it asynchronously.
async fn handle_workflow_trigger(
    data: &Value,
    correlation_id: Option<&str>,
    bridge: &std::sync::Arc<BridgeState>,
) {
    let workflow = data
        .get("workflow")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let triggered_by = data
        .get("triggered_by")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let channel = data
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    info!(
        "Bridge: workflow trigger received — workflow={}, by={}, channel={}",
        workflow, triggered_by, channel
    );
    let workflow_name = workflow.to_string();
    let triggered_by = triggered_by.to_string();
    let channel = channel.to_string();
    let params = data
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let correlation_id = correlation_id.map(str::to_string);
    let bridge = bridge.clone();

    tokio::spawn(async move {
        let Some(storage) = bridge.storage_backend().await else {
            warn!(
                "Bridge: no storage backend configured, cannot trigger workflow {}",
                workflow_name
            );
            return;
        };
        let Some(executor) = bridge.create_executor().await else {
            warn!(
                "Bridge: executor context not configured, cannot trigger workflow {}",
                workflow_name
            );
            return;
        };

        let stored = match storage.get_workflow(&workflow_name).await {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                warn!("Bridge: workflow '{}' not found", workflow_name);
                return;
            }
            Err(e) => {
                warn!("Bridge: failed to load workflow '{}': {}", workflow_name, e);
                return;
            }
        };

        let workflow = match parse_workflow(&stored.definition) {
            Ok(workflow) => workflow,
            Err(e) => {
                warn!(
                    "Bridge: failed to parse workflow '{}': {}",
                    workflow_name, e
                );
                return;
            }
        };

        let metadata = ExecutionMetadata {
            correlation_id,
            idempotency_key: None,
            origin: Some(format!("bridge:{}", channel)),
        };

        match executor
            .execute_with_metadata(&workflow, &stored.id, "bridge", params, metadata)
            .await
        {
            Ok(execution) => {
                info!(
                    "Bridge: triggered workflow '{}' as execution {} by {}",
                    workflow_name, execution.id, triggered_by
                );
            }
            Err(e) => {
                error!(
                    "Bridge: failed to trigger workflow '{}' by {}: {}",
                    workflow_name, triggered_by, e
                );
            }
        }
    });
}
