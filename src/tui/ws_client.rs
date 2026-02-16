//! WebSocket client for connecting to the r8r monitor endpoint.

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

use crate::api::MonitorEvent;

use super::event::TuiEvent;

/// Connect to the WebSocket monitor endpoint and send events via the channel.
///
/// Auto-reconnects with exponential backoff on disconnection.
pub async fn connect(base_url: &str, token: Option<&str>, tx: mpsc::Sender<TuiEvent>) {
    let ws_url = build_ws_url(base_url, token);
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        info!("Connecting to WebSocket: {}", ws_url);

        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((ws_stream, _)) => {
                backoff = Duration::from_secs(1); // Reset backoff on successful connect
                if tx.send(TuiEvent::WsConnected).await.is_err() {
                    return; // Channel closed, TUI exited
                }

                let (_write, mut read) = ws_stream.split();

                loop {
                    match read.next().await {
                        Some(Ok(msg)) => {
                            if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                                match serde_json::from_str::<MonitorEvent>(&text) {
                                    Ok(event) => {
                                        if tx.send(TuiEvent::Monitor(event)).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        debug!("Failed to parse monitor event: {}", e);
                                    }
                                }
                            }
                        }
                        Some(Err(e)) => {
                            warn!("WebSocket error: {}", e);
                            let _ = tx.send(TuiEvent::WsDisconnected(Some(e.to_string()))).await;
                            break;
                        }
                        None => {
                            info!("WebSocket stream ended");
                            let _ = tx.send(TuiEvent::WsDisconnected(None)).await;
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                error!("WebSocket connection failed: {}", e);
                let _ = tx.send(TuiEvent::WsDisconnected(Some(e.to_string()))).await;
            }
        }

        // Exponential backoff before reconnecting
        info!("Reconnecting in {:?}...", backoff);
        sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Build the WebSocket URL from a base HTTP URL.
///
/// Converts `http://` to `ws://` and `https://` to `wss://`,
/// appends `/api/monitor`, and adds token query parameter if provided.
pub fn build_ws_url(base_url: &str, token: Option<&str>) -> String {
    let ws_base = if base_url.starts_with("https://") {
        base_url.replacen("https://", "wss://", 1)
    } else if base_url.starts_with("http://") {
        base_url.replacen("http://", "ws://", 1)
    } else {
        format!("ws://{}", base_url)
    };

    let ws_base = ws_base.trim_end_matches('/');

    match token {
        Some(t) => format!("{}/api/monitor?token={}", ws_base, t),
        None => format!("{}/api/monitor", ws_base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ws_url_http() {
        assert_eq!(
            build_ws_url("http://localhost:8080", None),
            "ws://localhost:8080/api/monitor"
        );
    }

    #[test]
    fn test_build_ws_url_https() {
        assert_eq!(
            build_ws_url("https://example.com", None),
            "wss://example.com/api/monitor"
        );
    }

    #[test]
    fn test_build_ws_url_with_token() {
        assert_eq!(
            build_ws_url("http://localhost:8080", Some("mytoken")),
            "ws://localhost:8080/api/monitor?token=mytoken"
        );
    }

    #[test]
    fn test_build_ws_url_trailing_slash() {
        assert_eq!(
            build_ws_url("http://localhost:8080/", None),
            "ws://localhost:8080/api/monitor"
        );
    }

    #[test]
    fn test_build_ws_url_no_protocol() {
        assert_eq!(
            build_ws_url("localhost:8080", None),
            "ws://localhost:8080/api/monitor"
        );
    }
}
