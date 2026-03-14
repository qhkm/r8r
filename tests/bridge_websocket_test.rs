/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Integration tests for the bridge WebSocket endpoint.
//!
//! Starts a minimal Axum server with just the bridge routes and connects
//! via `tokio-tungstenite` to verify auth, health ping/response, and
//! invalid-token rejection.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use r8r::bridge::websocket::create_bridge_routes;
use r8r::bridge::BridgeState;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

/// Start a test server with bridge routes and return the bound address.
async fn start_test_server(token: Option<String>) -> String {
    let bridge = Arc::new(BridgeState::new(token));
    let app = create_bridge_routes(bridge);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    addr.to_string()
}

#[tokio::test]
async fn test_bridge_connection_and_health_ping() {
    let token = "test-secret-token-12345";
    let addr = start_test_server(Some(token.to_string())).await;

    let url = format!("ws://{}/api/ws/events", addr);

    // Connect with valid token
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Host", &addr)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();

    let (mut ws_stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect to bridge WebSocket");

    // Send a health ping
    let ping_envelope = json!({
        "id": "evt_test_ping_001",
        "type": "zeptoclaw.health.ping",
        "timestamp": "2026-03-14T12:00:00Z",
        "data": {}
    });
    ws_stream
        .send(tungstenite::Message::Text(ping_envelope.to_string()))
        .await
        .expect("Failed to send health ping");

    // Read the response — should be a HealthStatus event
    let response = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        ws_stream.next(),
    )
    .await
    .expect("Timed out waiting for health status response")
    .expect("Stream ended unexpectedly")
    .expect("WebSocket error reading response");

    if let tungstenite::Message::Text(text) = response {
        let parsed: serde_json::Value = serde_json::from_str(&text)
            .expect("Failed to parse health status response as JSON");

        assert_eq!(
            parsed["type"], "r8r.health.status",
            "Expected r8r.health.status event, got: {}",
            parsed["type"]
        );
        assert!(
            parsed["id"].as_str().unwrap().starts_with("evt_"),
            "Event ID should start with evt_"
        );
        assert!(
            parsed["data"]["version"].as_str().is_some(),
            "Health status should include version"
        );
    } else {
        panic!("Expected Text message, got: {:?}", response);
    }

    // Clean up
    ws_stream
        .close(None)
        .await
        .ok();
}

#[tokio::test]
async fn test_bridge_rejects_invalid_token() {
    let token = "correct-secret-token";
    let addr = start_test_server(Some(token.to_string())).await;

    let url = format!("ws://{}/api/ws/events", addr);

    // Attempt to connect with wrong token
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Authorization", "Bearer wrong-token-value")
        .header("Host", &addr)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap();

    let result = tokio_tungstenite::connect_async(request).await;

    // Connection should fail with 401
    match result {
        Err(tungstenite::Error::Http(response)) => {
            assert_eq!(
                response.status(),
                401,
                "Expected 401 Unauthorized, got: {}",
                response.status()
            );
        }
        Err(other) => {
            // Some tungstenite versions wrap the error differently
            let err_str = format!("{}", other);
            assert!(
                err_str.contains("401") || err_str.contains("Unauthorized"),
                "Expected 401 error, got: {}",
                err_str
            );
        }
        Ok(_) => {
            panic!("Expected connection to be rejected with invalid token, but it succeeded");
        }
    }
}
