/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Meta Cloud API WhatsApp webhook verification and payload normalization.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::triggers::EventMessage;

/// Event emitted for each inbound WhatsApp message.
pub const WHATSAPP_EVENT: &str = "whatsapp.inbound";
/// Event emitted for each WhatsApp delivery status update.
pub const WHATSAPP_STATUS_EVENT: &str = "whatsapp.status";

/// Verify a Meta `X-Hub-Signature-256` header against the raw request body.
pub fn verify_signature(app_secret: &str, raw_body: &[u8], signature_header: Option<&str>) -> bool {
    let Some(signature) = signature_header.map(str::trim) else {
        return false;
    };
    let Some(encoded) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(provided) = hex::decode(encoded.trim()) else {
        return false;
    };

    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(raw_body);
    let expected = mac.finalize().into_bytes();

    provided.len() == expected.len() && bool::from(provided.as_slice().ct_eq(expected.as_slice()))
}

/// Compare a webhook verification token in constant time.
pub fn verify_token(token: &str, expected: &str) -> bool {
    bool::from(token.as_bytes().ct_eq(expected.as_bytes()))
}

/// Normalized inbound WhatsApp message delivered to event-triggered workflows.
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct InboundMessage {
    pub wa_id: String,
    pub profile_name: Option<String>,
    pub phone_number_id: Option<String>,
    pub wamid: Option<String>,
    pub timestamp: Option<String>,
    pub message_type: String,
    pub text_body: Option<String>,
    pub media: Option<Value>,
    pub location: Option<Value>,
    pub context: Option<Value>,
    pub caption: Option<String>,
    pub raw: Value,
}

/// Parse a Meta webhook payload into inbound-message and status events.
pub fn parse_inbound_payload(payload: &Value) -> (Vec<EventMessage>, Vec<EventMessage>) {
    let mut message_events = Vec::new();
    let mut status_events = Vec::new();

    let Some(entries) = payload.get("entry").and_then(Value::as_array) else {
        return (message_events, status_events);
    };

    for change in entries
        .iter()
        .filter_map(|entry| entry.get("changes").and_then(Value::as_array))
        .flatten()
        .filter(|change| change.get("field").and_then(Value::as_str) == Some("messages"))
    {
        let Some(value) = change.get("value").and_then(Value::as_object) else {
            continue;
        };
        let phone_number_id = value
            .get("metadata")
            .and_then(|metadata| metadata.get("phone_number_id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let contact = value
            .get("contacts")
            .and_then(Value::as_array)
            .and_then(|contacts| contacts.first());
        let wa_id = contact
            .and_then(|contact| contact.get("wa_id"))
            .and_then(Value::as_str);
        let profile_name = contact
            .and_then(|contact| contact.get("profile"))
            .and_then(|profile| profile.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        if let (Some(wa_id), Some(messages)) =
            (wa_id, value.get("messages").and_then(Value::as_array))
        {
            for message in messages.iter().filter(|message| message.is_object()) {
                let message_type = message
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|kind| {
                        matches!(
                            *kind,
                            "text"
                                | "image"
                                | "audio"
                                | "video"
                                | "document"
                                | "location"
                                | "contact"
                                | "sticker"
                                | "button"
                                | "interactive"
                        )
                    })
                    .unwrap_or("unknown");
                let media = match message_type {
                    "image" | "audio" | "video" | "document" | "sticker" => {
                        message.get(message_type).cloned()
                    }
                    _ => None,
                };
                let wamid = message.get("id").and_then(Value::as_str).map(str::to_owned);
                let normalized = InboundMessage {
                    wa_id: wa_id.to_owned(),
                    profile_name: profile_name.clone(),
                    phone_number_id: phone_number_id.clone(),
                    wamid: wamid.clone(),
                    timestamp: message
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    message_type: message_type.to_owned(),
                    text_body: message
                        .get("text")
                        .and_then(|text| text.get("body"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    caption: media
                        .as_ref()
                        .and_then(|media| media.get("caption"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    media,
                    location: message.get("location").cloned(),
                    context: message.get("context").cloned(),
                    raw: message.clone(),
                };

                let Ok(data) = serde_json::to_value(normalized) else {
                    continue;
                };
                let mut event = EventMessage::new(WHATSAPP_EVENT, data);
                if let Some(source) = phone_number_id.as_deref() {
                    event = event.with_source(source);
                }
                if let Some(correlation_id) = wamid.as_deref() {
                    event = event.with_correlation_id(correlation_id);
                }
                message_events.push(event);
            }
        }

        if let Some(statuses) = value.get("statuses").and_then(Value::as_array) {
            for status in statuses.iter().filter(|status| status.is_object()) {
                let message_id = status.get("id").and_then(Value::as_str);
                let data = json!({
                    "status": status.get("status").cloned().unwrap_or(Value::Null),
                    "message_id": message_id,
                    "recipient_id": status.get("recipient_id").cloned().unwrap_or(Value::Null),
                    "timestamp": status.get("timestamp").cloned().unwrap_or(Value::Null),
                });
                let mut event = EventMessage::new(WHATSAPP_STATUS_EVENT, data);
                if let Some(source) = phone_number_id.as_deref() {
                    event = event.with_source(source);
                }
                if let Some(correlation_id) = message_id {
                    event = event.with_correlation_id(correlation_id);
                }
                status_events.push(event);
            }
        }
    }

    (message_events, status_events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with(message: Value) -> Value {
        json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "changes": [{
                    "field": "messages",
                    "value": {
                        "metadata": {"phone_number_id": "phone-123"},
                        "contacts": [{"wa_id": "60123456789", "profile": {"name": "Ada"}}],
                        "messages": [message]
                    }
                }]
            }]
        })
    }

    #[test]
    fn parses_text_message_event_shape() {
        let payload = payload_with(json!({
            "from": "60123456789",
            "id": "wamid.text-1",
            "timestamp": "1710000000",
            "type": "text",
            "text": {"body": "hello"},
            "context": {"id": "wamid.parent"}
        }));

        let (messages, statuses) = parse_inbound_payload(&payload);
        assert!(statuses.is_empty());
        assert_eq!(messages.len(), 1);
        let event = &messages[0];
        assert_eq!(event.event, WHATSAPP_EVENT);
        assert_eq!(event.source.as_deref(), Some("phone-123"));
        assert_eq!(event.correlation_id.as_deref(), Some("wamid.text-1"));
        assert_eq!(event.data["wa_id"], "60123456789");
        assert_eq!(event.data["profile_name"], "Ada");
        assert_eq!(event.data["message_type"], "text");
        assert_eq!(event.data["text_body"], "hello");
        assert_eq!(event.data["timestamp"], "1710000000");
        assert_eq!(event.data["context"]["id"], "wamid.parent");
    }

    #[test]
    fn parses_image_media_and_caption() {
        let payload = payload_with(json!({
            "id": "wamid.image-1",
            "type": "image",
            "image": {"id": "media-1", "mime_type": "image/jpeg", "caption": "receipt"}
        }));

        let (messages, _) = parse_inbound_payload(&payload);
        assert_eq!(messages[0].data["message_type"], "image");
        assert_eq!(messages[0].data["media"]["id"], "media-1");
        assert_eq!(messages[0].data["caption"], "receipt");
        assert_eq!(messages[0].data["raw"]["image"]["mime_type"], "image/jpeg");
    }

    #[test]
    fn parses_status_events() {
        let payload = json!({
            "entry": [{"changes": [{"field": "messages", "value": {
                "metadata": {"phone_number_id": "phone-456"},
                "statuses": [{
                    "id": "wamid.status-1",
                    "status": "delivered",
                    "recipient_id": "60111111111",
                    "timestamp": "1710000001"
                }]
            }}]}]
        });

        let (messages, statuses) = parse_inbound_payload(&payload);
        assert!(messages.is_empty());
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].event, WHATSAPP_STATUS_EVENT);
        assert_eq!(statuses[0].source.as_deref(), Some("phone-456"));
        assert_eq!(
            statuses[0].correlation_id.as_deref(),
            Some("wamid.status-1")
        );
        assert_eq!(statuses[0].data["status"], "delivered");
        assert_eq!(statuses[0].data["message_id"], "wamid.status-1");
        assert_eq!(statuses[0].data["recipient_id"], "60111111111");
    }

    #[test]
    fn empty_payload_returns_no_events() {
        let (messages, statuses) = parse_inbound_payload(&json!({}));
        assert!(messages.is_empty());
        assert!(statuses.is_empty());
    }

    #[test]
    fn account_payload_without_changes_returns_no_events() {
        let payload = json!({"object": "whatsapp_business_account", "entry": [{}]});
        let (messages, statuses) = parse_inbound_payload(&payload);
        assert!(messages.is_empty());
        assert!(statuses.is_empty());
    }

    #[test]
    fn malformed_values_do_not_panic() {
        let payload = json!({
            "entry": [
                {"changes": "not-an-array"},
                {"changes": [null, {"field": "messages", "value": {
                    "contacts": [{"wa_id": "1"}], "messages": "not-an-array", "statuses": 7
                }}]}
            ]
        });
        let (messages, statuses) = parse_inbound_payload(&payload);
        assert!(messages.is_empty());
        assert!(statuses.is_empty());
    }

    #[test]
    fn token_verification_accepts_only_exact_match() {
        assert!(verify_token("shared-secret", "shared-secret"));
        assert!(!verify_token("shared-secreu", "shared-secret"));
        assert!(!verify_token("shared-secret-extra", "shared-secret"));
    }

    #[test]
    fn signature_verification_covers_valid_and_invalid_headers() {
        let secret = "app-secret";
        let body = br#"{"object":"whatsapp_business_account"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        assert!(verify_signature(secret, body, Some(&signature)));
        assert!(verify_signature(
            secret,
            body,
            Some(&format!("  {signature}  "))
        ));
        assert!(!verify_signature(secret, b"different", Some(&signature)));
        assert!(!verify_signature(secret, body, Some("sha1=deadbeef")));
        assert!(!verify_signature(secret, body, None));
    }
}
