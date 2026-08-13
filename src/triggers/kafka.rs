/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Kafka consumer trigger (feature `kafka`).
//!
//! Uses the pure-Rust `kafka` crate — no librdkafka/C dependency, keeping the
//! binary self-contained. A background blocking task polls the topic and
//! forwards each message onto the r8r event bus as a `kafka.<topic>` event,
//! where existing event-triggered workflows pick it up.
//!
//! The blocking poll loop bridges into async via an mpsc channel instead of
//! nesting runtimes inside `spawn_blocking`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::triggers::event::EventMessage;
use crate::triggers::event_backend::EventBackend;

/// Where the consumer starts when no committed offset exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KafkaOffset {
    /// Start from the oldest available message.
    Earliest,
    /// Start from new messages only (skip history).
    Latest,
}

/// Kafka consumer trigger configuration.
#[derive(Debug, Clone)]
pub struct KafkaTriggerConfig {
    /// Broker addresses, e.g. `["localhost:9092"]`.
    pub brokers: Vec<String>,
    /// Topic to consume.
    pub topic: String,
    /// Client id prefix (used in broker logs; not Kafka consumer groups).
    pub client_id: String,
    /// Offset to start from when no history exists.
    pub fallback_offset: KafkaOffset,
    /// Sleep between poll rounds (ms).
    pub poll_interval_ms: u64,
}

impl KafkaTriggerConfig {
    /// Create a new trigger config with sensible defaults.
    pub fn new(brokers: Vec<String>, topic: impl Into<String>) -> Self {
        Self {
            brokers,
            topic: topic.into(),
            client_id: "r8r".to_string(),
            fallback_offset: KafkaOffset::Latest,
            poll_interval_ms: 1_000,
        }
    }
}

/// Parse a raw Kafka message payload into an event data value.
///
/// Valid JSON payloads are surfaced as structured values; anything else is
/// wrapped as a plain string so no payload is ever lost.
pub fn parse_payload(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => v,
        Err(_) => json!({ "value": text }),
    }
}

/// Run a Kafka consumer loop, publishing `kafka.<topic>` events until aborted.
pub async fn run_kafka_trigger(
    config: KafkaTriggerConfig,
    backend: Arc<dyn EventBackend>,
) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<EventMessage>(256);
    let topic = config.topic.clone();
    debug!(
        "Kafka trigger starting: brokers={:?} topic={} offset={:?}",
        config.brokers, config.topic, config.fallback_offset
    );

    // Publisher task: drain the channel into the event bus.
    let publisher = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Err(e) = backend.publish(event).await {
                warn!("Kafka event publish failed: {e}");
            }
        }
    });

    // Polling task: blocking `kafka` client, bridged to async via the channel.
    tokio::task::spawn_blocking(move || {
        let mut client = kafka::client::KafkaClient::new(config.brokers.clone());
        client.set_client_id(config.client_id.clone());
        let offset = match config.fallback_offset {
            KafkaOffset::Earliest => kafka::consumer::FetchOffset::Earliest,
            KafkaOffset::Latest => kafka::consumer::FetchOffset::Latest,
        };
        let mut consumer = match kafka::consumer::Consumer::from_client(client)
            .with_topic(config.topic.clone())
            .with_fallback_offset(offset)
            .create()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("Kafka consumer creation failed (is the broker up?): {e}");
                return;
            }
        };

        loop {
            match consumer.poll() {
                Ok(message_sets) => {
                    for message_set in message_sets.iter() {
                        for msg in message_set.messages() {
                            if msg.value.is_empty() {
                                continue;
                            }
                            let data = parse_payload(msg.value);
                            let event = EventMessage::new(
                                format!("kafka.{topic}"),
                                json!({
                                    "topic": topic,
                                    "partition": message_set.partition(),
                                    "offset": msg.offset,
                                    "data": data,
                                }),
                            )
                            .with_source("kafka");
                            // Channel closed only if the runtime is shutting down.
                            if tx.blocking_send(event).is_err() {
                                warn!("Kafka event channel closed; stopping consumer");
                                return;
                            }
                        }
                    }
                }
                Err(e) => warn!("Kafka poll error: {e}"),
            }
            std::thread::sleep(Duration::from_millis(config.poll_interval_ms));
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("Kafka polling task panicked: {e}"))?;

    // If we get here the blocking task returned early (e.g. broker down on
    // creation); stop the publisher so the loop doesn't hang.
    publisher.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_payload_as_structured_value() {
        let v = parse_payload(br#"{"order_id": 42, "ok": true}"#);
        assert_eq!(v["order_id"], 42);
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn wraps_non_json_payload_in_value_field() {
        let v = parse_payload(b"hello world");
        assert_eq!(v["value"], "hello world");
    }

    #[test]
    fn handles_utf8_lossy_binary_payload() {
        // Invalid UTF-8 must not panic; the lossy string is preserved.
        let v = parse_payload(&[0xff, 0xfe, b'x']);
        assert!(v["value"].as_str().is_some());
    }

    #[test]
    fn config_defaults_are_sane() {
        let cfg = KafkaTriggerConfig::new(vec!["localhost:9092".to_string()], "orders");
        assert_eq!(cfg.topic, "orders");
        assert_eq!(cfg.fallback_offset, KafkaOffset::Latest);
        assert_eq!(cfg.client_id, "r8r");
        assert!(cfg.poll_interval_ms >= 100);
    }
}
