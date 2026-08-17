/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! HTTP endpoints for Meta Cloud API WhatsApp webhooks.

use std::collections::HashMap;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use tracing::{error, warn};

use super::AppState;
use crate::triggers::{
    parse_whatsapp_inbound_payload, verify_whatsapp_signature, verify_whatsapp_token,
};

const SIGNATURE_HEADER: &str = "x-hub-signature-256";

/// Build the public Meta verification and inbound webhook routes.
pub fn whatsapp_routes() -> Router<AppState> {
    Router::new().route(
        "/webhooks/whatsapp",
        get(verify_webhook).post(receive_webhook),
    )
}

async fn verify_webhook(Query(query): Query<HashMap<String, String>>) -> Response {
    let expected = match std::env::var("WHATSAPP_VERIFY_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            warn!(
                marker = "whatsapp_verify_token_missing",
                "WHATSAPP_VERIFY_TOKEN is not configured; rejecting webhook verification"
            );
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    let valid = query.get("hub.mode").map(String::as_str) == Some("subscribe")
        && query
            .get("hub.verify_token")
            .is_some_and(|candidate| verify_whatsapp_token(candidate, &expected));

    if valid {
        query
            .get("hub.challenge")
            .cloned()
            .map(IntoResponse::into_response)
            .unwrap_or_else(|| StatusCode::FORBIDDEN.into_response())
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

async fn receive_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Ok(app_secret) = std::env::var("WHATSAPP_APP_SECRET") {
        let signature = headers
            .get(SIGNATURE_HEADER)
            .and_then(|value| value.to_str().ok());
        if !verify_whatsapp_signature(&app_secret, &body, signature) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    } else {
        warn!(
            marker = "whatsapp_app_secret_missing",
            "WHATSAPP_APP_SECRET is not configured; accepting an unsigned WhatsApp webhook"
        );
    }

    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(payload) => payload,
        Err(parse_error) => {
            warn!(error = %parse_error, "Invalid WhatsApp webhook JSON payload");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid JSON payload"})),
            )
                .into_response();
        }
    };
    let (message_events, status_events) = parse_whatsapp_inbound_payload(&payload);

    let Some(backend) = state.event_backend.as_ref() else {
        error!(
            marker = "whatsapp_event_backend_missing",
            "WhatsApp webhook received without an event backend; returning 500 for retry"
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    let mut received = 0usize;
    for event in message_events.into_iter().chain(status_events) {
        if let Err(publish_error) = backend.publish(event).await {
            error!(error = %publish_error, "Failed to publish WhatsApp webhook event");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        received += 1;
    }

    (
        StatusCode::OK,
        Json(json!({"status": "ok", "received": received})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;

    use super::*;
    use crate::{
        engine::PauseRegistry,
        nodes::NodeRegistry,
        shutdown::ShutdownCoordinator,
        storage::SqliteStorage,
        triggers::{EventBackend, NativeEventBackend},
    };

    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct EnvGuard {
        name: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: Option<&str>) -> Self {
            let original = std::env::var(name).ok();
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
            Self { name, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.original.as_deref() {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }

    fn test_state(event_backend: Option<Arc<dyn EventBackend>>) -> AppState {
        AppState {
            storage: Arc::new(SqliteStorage::open_in_memory().unwrap()),
            registry: Arc::new(NodeRegistry::new()),
            monitor: None,
            shutdown: Arc::new(ShutdownCoordinator::new()),
            pause_registry: PauseRegistry::new(),
            event_backend,
            bridge: None,
        }
    }

    #[tokio::test]
    async fn verification_endpoint_returns_challenge_or_forbidden() {
        let _lock = ENV_LOCK.lock().await;
        let _token = EnvGuard::set("WHATSAPP_VERIFY_TOKEN", Some("verify-me"));
        let app = whatsapp_routes().with_state(test_state(None));

        let accepted = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=verify-me&hub.challenge=challenge-42")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(
            accepted.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            to_bytes(accepted.into_body(), usize::MAX).await.unwrap(),
            "challenge-42"
        );

        let rejected = app
            .oneshot(
                Request::builder()
                    .uri("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=wrong&hub.challenge=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn inbound_endpoint_publishes_events_and_retries_without_backend() {
        let _lock = ENV_LOCK.lock().await;
        let _secret = EnvGuard::set("WHATSAPP_APP_SECRET", None);
        let backend = Arc::new(NativeEventBackend::new());
        let mut receiver = backend.subscribe();
        let app = whatsapp_routes().with_state(test_state(Some(backend)));
        let payload = json!({
            "entry": [{"changes": [{"field": "messages", "value": {
                "metadata": {"phone_number_id": "phone-1"},
                "contacts": [{"wa_id": "60123"}],
                "messages": [{"id": "wamid.1", "type": "text", "text": {"body": "hi"}}]
            }}]}]
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/whatsapp")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["received"],
            1
        );
        assert_eq!(receiver.recv().await.unwrap().event, "whatsapp.inbound");

        let response = whatsapp_routes()
            .with_state(test_state(None))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/whatsapp")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
