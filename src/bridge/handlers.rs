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
use tracing::{info, warn};

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
    bridge: &BridgeState,
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
            handle_workflow_trigger(data).await;
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
/// Currently logs the decision. Will be wired to the approval storage
/// backend in Task 4 to resolve pending approval gates.
async fn handle_approval_decision(data: &Value, _bridge: &BridgeState) {
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

    info!(
        "Bridge: approval decision received — id={}, decision={}, by={}",
        approval_id, decision, decided_by
    );

    // TODO (Task 4): resolve pending approval via storage backend
    // let storage_guard = bridge.storage.lock().await;
    // if let Some(storage) = storage_guard.as_ref() {
    //     storage.resolve_approval(approval_id, decision, decided_by).await;
    // }
}

/// Handle a workflow trigger request from ZeptoClaw.
///
/// Currently logs the trigger. Will be wired to the workflow executor
/// in Task 4 to actually start the workflow.
async fn handle_workflow_trigger(data: &Value) {
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

    // TODO (Task 4): wire to executor to start the workflow
    // executor.trigger_workflow(workflow, data["params"].clone(), triggered_by).await;
}
