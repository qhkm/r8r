/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Cross-project integration tests for the bridge approval flow.
//!
//! Validates the full approval roundtrip and event replay-on-reconnect
//! using a real Axum server and a mock WebSocket client (no actual
//! ZeptoClaw required).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};
use r8r::bridge::websocket::create_bridge_routes;
use r8r::bridge::BridgeState;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

const TEST_TOKEN: &str = "test-token-integration-12345";

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

/// Connect to the bridge WebSocket with a valid bearer token.
async fn connect_ws(
    addr: &str,
) -> tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
> {
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
///
/// Panics if a non-text message is received or the timeout expires.
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

/// Full approval roundtrip: send approval event -> client receives -> client acks
/// -> client sends decision -> verify ack removed event from buffer.
#[tokio::test]
async fn test_full_approval_roundtrip() {
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));
    let addr = start_test_server_with_bridge(bridge.clone()).await;

    // Connect as mock ZeptoClaw client
    let mut ws = connect_ws(&addr).await;

    // Small delay to let the server register the client
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Push an approval event through bridge.send_event()
    bridge
        .send_event(
            BridgeEvent::ApprovalRequested {
                approval_id: "appr_test_001".to_string(),
                workflow: "test-workflow".to_string(),
                execution_id: "exec_test_001".to_string(),
                node_id: "approve-step".to_string(),
                message: "Approve this test?".to_string(),
                timeout_secs: 300,
                requester: Some("test-user".to_string()),
                context: json!({"env": "test"}),
            },
            Some("corr_test_001".to_string()),
        )
        .await
        .expect("Failed to send approval event");

    // Client should receive the approval event
    let msg = read_next_text(&mut ws).await;
    let envelope: BridgeEventEnvelope =
        serde_json::from_str(&msg).expect("Failed to parse received event");

    assert_eq!(
        envelope.event_type, "r8r.approval.requested",
        "Expected approval.requested event type"
    );
    assert_eq!(
        envelope.correlation_id,
        Some("corr_test_001".to_string()),
        "Correlation ID should be preserved"
    );
    assert!(
        envelope.id.starts_with("evt_"),
        "Event ID should have evt_ prefix"
    );

    // Verify the payload data
    assert_eq!(envelope.data["approval_id"], "appr_test_001");
    assert_eq!(envelope.data["workflow"], "test-workflow");
    assert_eq!(envelope.data["message"], "Approve this test?");
    assert_eq!(envelope.data["timeout_secs"], 300);

    // Buffer should have 1 event (not yet acked)
    {
        let buf = bridge.buffer.lock().await;
        assert_eq!(buf.len(), 1, "Buffer should have 1 unacked event");
    }

    // Send ack for the received event
    let ack_msg = json!({
        "type": "ack",
        "event_id": envelope.id,
    });
    ws.send(tungstenite::Message::Text(ack_msg.to_string()))
        .await
        .expect("Failed to send ack");

    // Small delay for the server to process the ack
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Buffer should now be empty (ack removed the event)
    {
        let buf = bridge.buffer.lock().await;
        assert_eq!(
            buf.len(),
            0,
            "Buffer should be empty after ack, but has {} events",
            buf.len()
        );
    }

    // Now send an approval decision from the client (ZeptoClaw -> r8r)
    let decision_envelope = BridgeEventEnvelope::new(
        BridgeEvent::ApprovalDecision {
            approval_id: "appr_test_001".to_string(),
            execution_id: "exec_test_001".to_string(),
            node_id: "approve-step".to_string(),
            decision: "approved".to_string(),
            reason: "Looks good".to_string(),
            decided_by: "reviewer@example.com".to_string(),
            channel: "bridge-test".to_string(),
        },
        Some("corr_test_001".to_string()),
    );
    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&decision_envelope).unwrap(),
    ))
    .await
    .expect("Failed to send approval decision");

    // Small delay for the server to process the decision
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The decision handler logs and attempts storage resolution.
    // Without storage configured, the handler logs a warning and returns.
    // Verify the bridge is still operational by confirming client is connected.
    assert!(
        bridge.is_connected().await,
        "Bridge client should still be connected after decision"
    );

    // Clean up
    ws.close(None).await.ok();
}

/// Verify that events buffered while no client is connected are replayed
/// when a client connects.
#[tokio::test]
async fn test_replay_on_reconnect() {
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));

    // Push events while no client is connected — they go to the buffer
    bridge
        .send_event(
            BridgeEvent::ExecutionCompleted {
                workflow: "wf-alpha".to_string(),
                execution_id: "exec_001".to_string(),
                status: "success".to_string(),
                duration_ms: 1200,
                node_count: 5,
            },
            Some("corr_replay_001".to_string()),
        )
        .await
        .unwrap();

    bridge
        .send_event(
            BridgeEvent::ApprovalRequested {
                approval_id: "appr_replay_001".to_string(),
                workflow: "wf-beta".to_string(),
                execution_id: "exec_002".to_string(),
                node_id: "gate-1".to_string(),
                message: "Please approve deployment".to_string(),
                timeout_secs: 600,
                requester: Some("deployer".to_string()),
                context: json!({}),
            },
            Some("corr_replay_002".to_string()),
        )
        .await
        .unwrap();

    // Verify buffer has 2 events and no client is connected
    assert!(!bridge.is_connected().await, "No client should be connected yet");
    {
        let buf = bridge.buffer.lock().await;
        assert_eq!(buf.len(), 2, "Buffer should have 2 events before connect");
    }

    // Now start the server and connect — should receive both buffered events
    let addr = start_test_server_with_bridge(bridge.clone()).await;
    let mut ws = connect_ws(&addr).await;

    // Read both replayed events
    let msg1 = read_next_text(&mut ws).await;
    let msg2 = read_next_text(&mut ws).await;

    let env1: BridgeEventEnvelope =
        serde_json::from_str(&msg1).expect("Failed to parse replayed event 1");
    let env2: BridgeEventEnvelope =
        serde_json::from_str(&msg2).expect("Failed to parse replayed event 2");

    // Events should arrive in insertion order (FIFO)
    assert_eq!(
        env1.event_type, "r8r.execution.completed",
        "First replayed event should be execution.completed"
    );
    assert_eq!(
        env1.correlation_id,
        Some("corr_replay_001".to_string()),
        "First event correlation ID mismatch"
    );

    assert_eq!(
        env2.event_type, "r8r.approval.requested",
        "Second replayed event should be approval.requested"
    );
    assert_eq!(
        env2.correlation_id,
        Some("corr_replay_002".to_string()),
        "Second event correlation ID mismatch"
    );

    // Verify data payloads survived the roundtrip
    assert_eq!(env1.data["workflow"], "wf-alpha");
    assert_eq!(env1.data["duration_ms"], 1200);
    assert_eq!(env2.data["approval_id"], "appr_replay_001");
    assert_eq!(env2.data["message"], "Please approve deployment");

    // Ack both events
    for env in [&env1, &env2] {
        let ack_msg = json!({
            "type": "ack",
            "event_id": env.id,
        });
        ws.send(tungstenite::Message::Text(ack_msg.to_string()))
            .await
            .expect("Failed to send ack");
    }

    // Wait for acks to be processed
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Buffer should now be empty
    {
        let buf = bridge.buffer.lock().await;
        assert_eq!(
            buf.len(),
            0,
            "Buffer should be empty after acking both events"
        );
    }

    // Clean up
    ws.close(None).await.ok();
}

/// Verify that a new event sent while the client is connected arrives
/// in real-time (not just via replay).
#[tokio::test]
async fn test_live_event_delivery() {
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));
    let addr = start_test_server_with_bridge(bridge.clone()).await;

    let mut ws = connect_ws(&addr).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send an execution-failed event while client is connected
    bridge
        .send_event(
            BridgeEvent::ExecutionFailed {
                workflow: "wf-gamma".to_string(),
                execution_id: "exec_fail_001".to_string(),
                status: "failed".to_string(),
                error_code: "NODE_TIMEOUT".to_string(),
                error_message: "HTTP node timed out after 30s".to_string(),
                failed_node: "fetch-api".to_string(),
                duration_ms: 30500,
            },
            None,
        )
        .await
        .unwrap();

    // Client should receive it immediately
    let msg = read_next_text(&mut ws).await;
    let envelope: BridgeEventEnvelope =
        serde_json::from_str(&msg).expect("Failed to parse live event");

    assert_eq!(envelope.event_type, "r8r.execution.failed");
    assert_eq!(envelope.data["error_code"], "NODE_TIMEOUT");
    assert_eq!(envelope.data["failed_node"], "fetch-api");
    assert!(
        envelope.correlation_id.is_none(),
        "No correlation ID was set"
    );

    // Clean up
    ws.close(None).await.ok();
}

/// Verify that sending multiple events in quick succession all arrive and
/// can be acked, leaving the buffer empty.
#[tokio::test]
async fn test_multiple_events_ack_drains_buffer() {
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));
    let addr = start_test_server_with_bridge(bridge.clone()).await;

    let mut ws = connect_ws(&addr).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Send 5 events in quick succession
    for i in 0..5 {
        bridge
            .send_event(BridgeEvent::HealthPing, Some(format!("corr_multi_{}", i)))
            .await
            .unwrap();
    }

    // Note: HealthPing events sent via send_event are buffered like any other event.
    // They are outbound r8r->client events (different from inbound health pings).

    // Receive and ack all 5
    for _ in 0..5 {
        let msg = read_next_text(&mut ws).await;
        let envelope: BridgeEventEnvelope =
            serde_json::from_str(&msg).expect("Failed to parse event");

        // Send ack
        let ack_msg = json!({
            "type": "ack",
            "event_id": envelope.id,
        });
        ws.send(tungstenite::Message::Text(ack_msg.to_string()))
            .await
            .expect("Failed to send ack");
    }

    // Wait for all acks to process
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Buffer should be completely empty
    {
        let buf = bridge.buffer.lock().await;
        assert_eq!(
            buf.len(),
            0,
            "Buffer should be empty after acking all 5 events, has {}",
            buf.len()
        );
    }

    // Clean up
    ws.close(None).await.ok();
}
