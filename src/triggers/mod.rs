/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Trigger implementations.
//!
//! Triggers define what starts a workflow:
//! - Cron: Schedule-based (e.g., every 5 minutes)
//! - Manual: CLI or API invocation
//! - Webhook: HTTP endpoint with optional debouncing and header filtering
//! - Event: Pub/sub events with fan-out and routing support (native or Redis)
//! - Delayed: Events scheduled for future processing via SQLite
//! - Approval timeout: Background checker for expired approval requests

mod approval_timeout;
#[cfg(feature = "aws")]
mod aws;
mod delayed;
mod event;
mod event_backend;
#[cfg(feature = "imap")]
mod imap;
#[cfg(feature = "kafka")]
mod kafka;
#[cfg(feature = "redis")]
mod redis_backend;
mod scheduler;
mod webhook;
#[cfg(feature = "whatsapp-inbound")]
mod whatsapp;

pub use approval_timeout::ApprovalTimeoutChecker;
#[cfg(feature = "aws")]
pub use aws::{AwsCredentials, SnsPublisher, SqsConfig, SqsMessage, SqsTrigger};
pub use delayed::{DelayedEvent, DelayedEventProcessor};
pub use event::{EventError, EventFanOut, EventMessage, EventSubscriber};
pub use event_backend::{EventBackend, EventReceiver, NativeEventBackend};
#[cfg(feature = "imap")]
pub use imap::{
    imap_topic, mail_to_event as imap_mail_to_event, run_imap_trigger, ImapTriggerConfig,
};
#[cfg(feature = "kafka")]
pub use kafka::{
    parse_payload as kafka_parse_payload, run_kafka_trigger, KafkaOffset, KafkaTriggerConfig,
};
#[cfg(feature = "redis")]
pub use redis_backend::RedisEventBackend;
pub use scheduler::Scheduler;
pub use webhook::create_webhook_routes;
#[cfg(feature = "whatsapp-inbound")]
pub use whatsapp::{
    parse_inbound_payload as parse_whatsapp_inbound_payload,
    verify_signature as verify_whatsapp_signature, verify_token as verify_whatsapp_token,
    InboundMessage as WhatsAppInboundMessage, WHATSAPP_EVENT, WHATSAPP_STATUS_EVENT,
};
