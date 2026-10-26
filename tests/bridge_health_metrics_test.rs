/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Integration tests for bridge health status reporting.
//!
//! Verifies that a `zeptoclaw.health.ping` receives a `r8r.health.status`
//! response carrying *real* runtime metrics (not stub zeros): loaded
//! workflows, pending approvals, active executions and uptime.
//!
//! This file lives in its own test binary on purpose: the Prometheus-backed
//! active-execution gauge is process-global, so any workflow execution test
//! running in the same process would race our assertions on
//! `data.active_executions`. With only this file in the process, the counter
//! is guaranteed to be 0.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use r8r::bridge::BridgeState;
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};
use r8r::bridge::websocket::create_bridge_routes;
use r8r::engine::PauseRegistry;
use r8r::nodes::NodeRegistry;
use r8r::storage::{ApprovalRequest, Execution, ExecutionStatus, Storage, StoredWorkflow};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

const TEST_TOKEN: &str = "test-token-health-metrics-77777";

/// Start a test server with the given bridge state and return the bound address.
async fn start_test_server_with_bridge(bridge: Arc<BridgeState>) -> String {
    let app = create_bridge_routes(bridge);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    addr.to_string()
}

/// Build a bridge wired to an in-memory storage backend.
async fn build_bridge() -> Arc<BridgeState> {
    let storage: Arc<dyn Storage> =
        Arc::new(r8r::storage::SqliteStorage::open_in_memory().unwrap());
    let registry = Arc::new(NodeRegistry::new());
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));

    {
        let mut storage_guard = bridge.storage.lock().await;
        *storage_guard = Some(storage);
    }

    bridge
        .configure_execution_context(registry, None, PauseRegistry::new())
        .await;

    bridge
}

async fn save_workflow(bridge: &Arc<BridgeState>, id: &str, name: &str) {
    let storage = bridge.storage_backend().await.unwrap();
    let now = Utc::now();
    storage
        .save_workflow(&StoredWorkflow {
            id: id.to_string(),
            name: name.to_string(),
            definition: format!("name: {name}\nnodes:\n  - id: n1\n    type: set\n    config:\n      fields:\n        - name: ok\n          value: true\n"),
            enabled: true,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

async fn save_pending_approval(bridge: &Arc<BridgeState>, id: &str) {
    let storage = bridge.storage_backend().await.unwrap();
    // The approval row has a FK to the executions table, so seed the
    // execution it belongs to first.
    storage
        .save_execution(&Execution {
            id: "exec_health_1".to_string(),
            workflow_id: "wf-health".to_string(),
            workflow_name: "health-wf".to_string(),
            workflow_version: Some(1),
            status: ExecutionStatus::WaitingForApproval,
            trigger_type: "manual".to_string(),
            input: serde_json::json!({}),
            output: None,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
            correlation_id: None,
            idempotency_key: None,
            origin: Some("manual".to_string()),
        })
        .await
        .unwrap();
    storage
        .save_approval_request(&ApprovalRequest {
            id: id.to_string(),
            execution_id: "exec_health_1".to_string(),
            node_id: "gate".to_string(),
            workflow_name: "health-wf".to_string(),
            title: "Approve health check".to_string(),
            description: None,
            status: "pending".to_string(),
            decision_comment: None,
            decided_by: None,
            context_data: None,
            created_at: Utc::now(),
            expires_at: None,
            decided_at: None,
            assignee: None,
            groups: Vec::new(),
        })
        .await
        .unwrap();
}

/// Connect to the bridge WebSocket with a valid bearer token.
async fn connect_ws(
    addr: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{}/api/ws/events", addr);
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", TEST_TOKEN))
        .header("Host", addr)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();

    let (ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect to bridge WebSocket");

    ws
}

/// Read the next text message from the WebSocket, with a timeout.
async fn read_next_text(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> String {
    match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
        Ok(Some(Ok(tungstenite::Message::Text(text)))) => text.to_string(),
        Ok(Some(Ok(other))) => panic!("Expected text message, got: {:?}", other),
        Ok(Some(Err(e))) => panic!("WebSocket error: {}", e),
        Ok(None) => panic!("WebSocket stream ended unexpectedly"),
        Err(_) => panic!("Timed out waiting for WebSocket message"),
    }
}

/// A health ping is answered with a status report carrying real metrics:
/// the workflows and pending approvals saved to storage are reflected in the
/// response, and the correlation id is echoed for ping/response matching.
#[tokio::test]
async fn test_health_ping_reports_real_metrics() {
    r8r::metrics::init_metrics();

    let bridge = build_bridge().await;

    // One loaded workflow and one pending approval.
    save_workflow(&bridge, "wf-health", "health-wf").await;
    save_pending_approval(&bridge, "apr_health_1").await;

    let addr = start_test_server_with_bridge(bridge).await;

    // Connect as mock ZeptoClaw client.
    let mut ws = connect_ws(&addr).await;

    // Small delay to let the server register the client.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send a health ping with a correlation id.
    let ping = BridgeEventEnvelope::new(BridgeEvent::HealthPing, Some("corr-health-1".to_string()));
    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&ping).unwrap(),
    ))
    .await
    .expect("Failed to send health ping");

    // Receive the HealthStatus response.
    let msg = read_next_text(&mut ws).await;
    let envelope: BridgeEventEnvelope =
        serde_json::from_str(&msg).expect("Failed to parse health status");

    assert_eq!(envelope.event_type, "r8r.health.status");
    assert_eq!(
        envelope.correlation_id.as_deref(),
        Some("corr-health-1"),
        "Health status should echo the ping's correlation id"
    );

    // Real metrics: exactly the workflow and approval we saved.
    assert_eq!(
        envelope.data["workflows_loaded"], 1,
        "Health status should report the loaded workflow"
    );
    assert_eq!(
        envelope.data["pending_approvals"], 1,
        "Health status should report the pending approval"
    );

    // Deterministic in this process: no workflow ever executed, and the
    // uptime clock was initialized moments ago.
    assert_eq!(
        envelope.data["active_executions"], 0,
        "No executions ran in this test process, so active executions must be 0"
    );
    let uptime = envelope.data["uptime_secs"]
        .as_u64()
        .expect("uptime_secs must be a u64");
    assert!(
        uptime < 3600,
        "uptime_secs should be a small value, got {uptime}"
    );

    // Version should be the crate version, not empty.
    let version = envelope.data["version"].as_str().unwrap_or("");
    assert!(
        !version.is_empty(),
        "Health status should report the r8r version"
    );
}
