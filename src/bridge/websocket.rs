/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! WebSocket endpoint for the r8r <-> ZeptoClaw bridge.
//!
//! Provides a single `/api/ws/events` route that upgrades to a WebSocket
//! connection for bidirectional event streaming. Only one client may be
//! connected at a time; a new connection displaces any existing one with
//! close code 4002.

use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::events::{Ack, BridgeEventEnvelope};
use super::handlers::handle_incoming_event;
use super::BridgeState;

/// Sentinel message: tells the sender task to close with code 4002.
const CLOSE_SENTINEL: &str = "__CLOSE_4002__";

/// Sentinel message: tells the sender task to emit a WebSocket Ping frame.
const PING_SENTINEL: &str = "__WS_PING__";

/// Interval between WebSocket keep-alive pings (seconds).
const PING_INTERVAL_SECS: u64 = 30;

/// Channel capacity for the per-client mpsc sender.
const CLIENT_CHANNEL_CAPACITY: usize = 256;

/// Create the Axum router for bridge WebSocket routes.
///
/// Returns a `Router` with one route:
/// - `GET /api/ws/events` — WebSocket upgrade for bridge communication
pub fn create_bridge_routes(bridge: Arc<BridgeState>) -> Router {
    Router::new()
        .route("/api/ws/events", get(ws_handler))
        .with_state(bridge)
}

/// WebSocket upgrade handler.
///
/// Extracts the `Authorization: Bearer <token>` header and validates it
/// against `bridge.token` using constant-time comparison. If no token is
/// configured on the bridge, connections are allowed (dev mode) with a warning.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(bridge): State<Arc<BridgeState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Extract bearer token from Authorization header
    let provided_token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    // Validate token
    if !validate_bridge_token(bridge.token.as_deref(), provided_token.as_deref()) {
        warn!("Bridge WebSocket connection rejected: invalid or missing token");
        return (
            StatusCode::UNAUTHORIZED,
            "Invalid or missing authentication token",
        )
            .into_response();
    }

    info!("Bridge WebSocket upgrade accepted");
    ws.on_upgrade(move |socket| handle_bridge_socket(socket, bridge))
        .into_response()
}

/// Constant-time token comparison.
///
/// - If `expected` is `None`, allow (dev mode) with a warning.
/// - If `expected` is `Some` but `provided` is `None`, reject.
/// - Otherwise, XOR-compare every byte to avoid timing side-channels.
fn validate_bridge_token(expected: Option<&str>, provided: Option<&str>) -> bool {
    match (expected, provided) {
        (None, _) => {
            warn!("R8R_BRIDGE_TOKEN not set — bridge WebSocket is unprotected!");
            true
        }
        (Some(_), None) => false,
        (Some(exp), Some(prov)) => {
            if exp.len() != prov.len() {
                return false;
            }
            exp.bytes()
                .zip(prov.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
    }
}

/// Handle an upgraded bridge WebSocket connection.
///
/// 1. Displace any existing client (send close-4002 sentinel).
/// 2. Create a new mpsc channel and store the sender in `bridge.client_tx`.
/// 3. Spawn a sender task that forwards channel messages to the WebSocket.
/// 4. Spawn a ping task that sends keep-alive pings every 30 seconds.
/// 5. Replay unacked events from the buffer.
/// 6. Enter the receive loop: parse acks and dispatch incoming events.
/// 7. On disconnect, clear `client_tx`.
async fn handle_bridge_socket(socket: WebSocket, bridge: Arc<BridgeState>) {
    // Step 1: Displace any existing client
    {
        let mut tx_guard = bridge.client_tx.lock().await;
        if let Some(old_tx) = tx_guard.take() {
            info!("Bridge: displacing existing client (close 4002)");
            let _ = old_tx.send(CLOSE_SENTINEL.to_string()).await;
            // Brief pause to let the old sender task process the close
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }

    // Step 2: Create new channel
    let (client_tx, mut client_rx) = mpsc::channel::<String>(CLIENT_CHANNEL_CAPACITY);
    {
        let mut tx_guard = bridge.client_tx.lock().await;
        *tx_guard = Some(client_tx.clone());
    }

    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Step 3: Spawn sender task
    let sender_task = tokio::spawn(async move {
        while let Some(msg) = client_rx.recv().await {
            if msg == CLOSE_SENTINEL {
                let close_frame = CloseFrame {
                    code: 4002,
                    reason: "Displaced by new connection".into(),
                };
                let _ = ws_sender.send(Message::Close(Some(close_frame))).await;
                break;
            } else if msg == PING_SENTINEL {
                if let Err(e) = ws_sender.send(Message::Ping(vec![].into())).await {
                    debug!("Bridge: failed to send ping: {}", e);
                    break;
                }
            } else {
                if let Err(e) = ws_sender.send(Message::Text(msg.into())).await {
                    error!("Bridge: failed to send message: {}", e);
                    break;
                }
            }
        }
    });

    // Step 4: Spawn ping task
    let ping_tx = client_tx.clone();
    let ping_task = tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(PING_INTERVAL_SECS));
        // Skip the first immediate tick
        interval.tick().await;
        loop {
            interval.tick().await;
            if ping_tx.send(PING_SENTINEL.to_string()).await.is_err() {
                break;
            }
        }
    });

    // Step 5: Replay unacked events
    {
        let buf = bridge.buffer.lock().await;
        let unacked = buf.unacked_events();
        if !unacked.is_empty() {
            info!("Bridge: replaying {} unacked events", unacked.len());
            for envelope in unacked {
                if let Ok(json) = serde_json::to_string(&envelope) {
                    if client_tx.send(json).await.is_err() {
                        warn!("Bridge: client disconnected during replay");
                        break;
                    }
                }
            }
        }
    }

    // Step 6: Receive loop
    info!("Bridge: client connected, entering receive loop");
    while let Some(msg_result) = ws_receiver.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                let text_str: &str = &text;
                debug!("Bridge: received text message: {}", text_str);

                // Try to parse as Ack first
                if let Ok(ack) = serde_json::from_str::<Ack>(text_str) {
                    debug!("Bridge: acknowledged event {}", ack.event_id);
                    let mut buf = bridge.buffer.lock().await;
                    buf.ack(&ack.event_id);
                    continue;
                }

                // Try to parse as BridgeEventEnvelope
                if let Ok(envelope) = serde_json::from_str::<BridgeEventEnvelope>(text_str) {
                    handle_incoming_event(
                        &envelope.event_type,
                        &envelope.data,
                        &bridge,
                        &client_tx,
                    )
                    .await;
                    continue;
                }

                warn!("Bridge: unrecognized message: {}", text_str);
            }
            Ok(Message::Ping(data)) => {
                // Respond with pong via the sender channel — but sending
                // raw pong frames through the mpsc would require a different
                // message type. Axum auto-responds to pings, so this is fine.
                debug!("Bridge: received ping ({} bytes)", data.len());
            }
            Ok(Message::Pong(_)) => {
                debug!("Bridge: received pong");
            }
            Ok(Message::Close(_)) => {
                info!("Bridge: client sent close frame");
                break;
            }
            Ok(Message::Binary(_)) => {
                warn!("Bridge: ignoring binary message");
            }
            Err(e) => {
                error!("Bridge: WebSocket error: {}", e);
                break;
            }
        }
    }

    // Step 7: Cleanup
    info!("Bridge: client disconnected");
    ping_task.abort();
    sender_task.abort();

    // Clear client_tx
    {
        let mut tx_guard = bridge.client_tx.lock().await;
        *tx_guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_bridge_token_no_expected() {
        // No token configured = dev mode, always allow
        assert!(validate_bridge_token(None, None));
        assert!(validate_bridge_token(None, Some("anything")));
    }

    #[test]
    fn test_validate_bridge_token_matching() {
        assert!(validate_bridge_token(Some("secret123"), Some("secret123")));
    }

    #[test]
    fn test_validate_bridge_token_mismatched() {
        assert!(!validate_bridge_token(Some("secret123"), Some("wrong456")));
    }

    #[test]
    fn test_validate_bridge_token_missing_when_required() {
        assert!(!validate_bridge_token(Some("secret123"), None));
    }

    #[test]
    fn test_validate_bridge_token_length_mismatch() {
        assert!(!validate_bridge_token(Some("short"), Some("longer_token")));
    }
}
