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

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use r8r::bridge::events::{BridgeEvent, BridgeEventEnvelope};
use r8r::bridge::websocket::create_bridge_routes;
use r8r::bridge::BridgeState;
use r8r::engine::PauseRegistry;
use r8r::nodes::NodeRegistry;
use r8r::storage::{Execution, Storage, StoredWorkflow};
use r8r::workflow::parse_workflow;
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

async fn build_runtime_bridge() -> (Arc<BridgeState>, Arc<dyn Storage>) {
    let storage: Arc<dyn Storage> =
        Arc::new(r8r::storage::SqliteStorage::open_in_memory().unwrap());
    let registry = Arc::new(NodeRegistry::new());
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));

    {
        let mut storage_guard = bridge.storage.lock().await;
        *storage_guard = Some(storage.clone());
    }

    bridge
        .configure_execution_context(registry, None, PauseRegistry::new())
        .await;

    (bridge, storage)
}

async fn save_workflow(storage: &Arc<dyn Storage>, id: &str, name: &str, definition: &str) {
    let now = Utc::now();
    storage
        .save_workflow(&StoredWorkflow {
            id: id.to_string(),
            name: name.to_string(),
            definition: definition.to_string(),
            enabled: true,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

async fn wait_for_execution_status_by_workflow(
    storage: &Arc<dyn Storage>,
    workflow_name: &str,
    expected_status: &str,
) -> Execution {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        let executions = storage.list_executions(workflow_name, 10).await.unwrap();
        let last_statuses = executions
            .iter()
            .map(|execution| execution.status.to_string())
            .collect::<Vec<_>>();
        if let Some(execution) = executions
            .into_iter()
            .find(|execution| execution.status.to_string() == expected_status)
        {
            return execution;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for workflow {} to reach status {} (seen statuses: {:?})",
            workflow_name,
            expected_status,
            last_statuses
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_approval_status(
    storage: &Arc<dyn Storage>,
    approval_id: &str,
    expected_status: &str,
) -> r8r::storage::ApprovalRequest {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);

    loop {
        let approval = storage
            .get_approval_request(approval_id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("approval {} disappeared", approval_id));
        if approval.status == expected_status {
            return approval;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for approval {} to reach status {} (last status: {})",
            approval_id,
            expected_status,
            approval.status
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
    let (bridge, storage) = build_runtime_bridge().await;
    let definition = r#"
name: bridge-approval
nodes:
  - id: gate
    type: approval
    config:
      title: Approve bridge action
  - id: finalize
    type: set
    depends_on: [gate]
    config:
      fields:
        - name: approved_via
          value: bridge
"#;
    save_workflow(&storage, "wf-approval", "bridge-approval", definition).await;

    let addr = start_test_server_with_bridge(bridge.clone()).await;

    // Connect as mock ZeptoClaw client
    let mut ws = connect_ws(&addr).await;

    // Small delay to let the server register the client
    tokio::time::sleep(Duration::from_millis(100)).await;

    let workflow = parse_workflow(definition).unwrap();
    let executor = bridge.create_executor().await.unwrap();
    let execution = executor
        .execute(
            &workflow,
            "wf-approval",
            "manual",
            json!({"source": "bridge-test"}),
        )
        .await
        .expect("Failed to execute approval workflow");
    assert_eq!(execution.status.to_string(), "waiting_for_approval");

    // Client should receive the emitted approval request event.
    let msg = read_next_text(&mut ws).await;
    let envelope: BridgeEventEnvelope =
        serde_json::from_str(&msg).expect("Failed to parse received event");

    assert_eq!(
        envelope.event_type, "r8r.approval.requested",
        "Expected approval.requested event type"
    );
    assert_eq!(
        envelope.correlation_id.as_deref(),
        Some(execution.id.as_str()),
        "Correlation ID should use the execution ID"
    );
    assert!(
        envelope.id.starts_with("evt_"),
        "Event ID should have evt_ prefix"
    );

    // Verify the payload data
    let approval_id = envelope.data["approval_id"].as_str().unwrap().to_string();
    assert_eq!(envelope.data["workflow"], "bridge-approval");
    assert_eq!(envelope.data["message"], "Approve bridge action");

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
            approval_id: approval_id.clone(),
            execution_id: execution.id.clone(),
            node_id: "gate".to_string(),
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

    let approval = wait_for_approval_status(&storage, &approval_id, "approved").await;
    assert_eq!(approval.decided_by.as_deref(), Some("reviewer@example.com"));

    let completed =
        wait_for_execution_status_by_workflow(&storage, "bridge-approval", "completed").await;
    assert_ne!(completed.id, execution.id);
    assert_eq!(completed.trigger_type, "resume");
    assert_eq!(completed.output.unwrap()["approved_via"], "bridge");

    let original = storage.get_execution(&execution.id).await.unwrap().unwrap();
    assert_eq!(original.status.to_string(), "waiting_for_approval");

    let checkpoint = storage
        .get_latest_checkpoint(&execution.id)
        .await
        .unwrap()
        .expect("checkpoint should exist");
    assert_eq!(checkpoint.node_outputs["gate"]["status"], "approved");
    assert_eq!(checkpoint.node_outputs["gate"]["decision"], "approve");

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
    assert!(
        !bridge.is_connected().await,
        "No client should be connected yet"
    );
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

#[tokio::test]
async fn test_workflow_trigger_executes_workflow() {
    let (bridge, storage) = build_runtime_bridge().await;
    let definition = r#"
name: bridge-trigger
nodes:
  - id: finish
    type: set
    config:
      fields:
        - name: triggered_by_bridge
          value: true
"#;
    save_workflow(&storage, "wf-trigger", "bridge-trigger", definition).await;

    let addr = start_test_server_with_bridge(bridge.clone()).await;
    let mut ws = connect_ws(&addr).await;

    let trigger = BridgeEventEnvelope::new(
        BridgeEvent::WorkflowTrigger {
            workflow: "bridge-trigger".to_string(),
            params: json!({"payload": "ok"}),
            triggered_by: "zeptoclaw".to_string(),
            channel: "bridge-test".to_string(),
        },
        Some("corr_bridge_trigger".to_string()),
    );
    ws.send(tungstenite::Message::Text(
        serde_json::to_string(&trigger).unwrap(),
    ))
    .await
    .expect("failed to send workflow trigger");

    let execution =
        wait_for_execution_status_by_workflow(&storage, "bridge-trigger", "completed").await;
    assert_eq!(execution.output.unwrap()["triggered_by_bridge"], true);

    ws.close(None).await.ok();
}

#[tokio::test]
async fn test_new_connection_does_not_clear_current_client() {
    let bridge = Arc::new(BridgeState::new(Some(TEST_TOKEN.to_string())));
    let addr = start_test_server_with_bridge(bridge.clone()).await;

    let mut ws1 = connect_ws(&addr).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut ws2 = connect_ws(&addr).await;

    match tokio::time::timeout(Duration::from_secs(2), ws1.next()).await {
        Ok(Some(Ok(tungstenite::Message::Close(_)))) | Ok(None) => {}
        other => panic!("expected first client to be displaced, got {:?}", other),
    }

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        bridge.is_connected().await,
        "second client should remain registered after displacing the first"
    );

    bridge
        .send_event(
            BridgeEvent::ExecutionCompleted {
                workflow: "wf-race".to_string(),
                execution_id: "exec-race-001".to_string(),
                status: "completed".to_string(),
                duration_ms: 42,
                node_count: 1,
            },
            Some("corr_race".to_string()),
        )
        .await
        .unwrap();

    let msg = read_next_text(&mut ws2).await;
    let envelope: BridgeEventEnvelope = serde_json::from_str(&msg).unwrap();
    assert_eq!(envelope.event_type, "r8r.execution.completed");

    ws2.close(None).await.ok();
}
