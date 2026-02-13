//! Trigger implementations.
//!
//! Triggers define what starts a workflow:
//! - Cron: Schedule-based (e.g., every 5 minutes)
//! - Manual: CLI or API invocation
//! - Webhook: HTTP endpoint
//! - Event: Redis pub/sub events

mod event;
mod scheduler;
mod webhook;

pub use event::{EventError, EventMessage, EventPublisher, EventSubscriber};
pub use scheduler::Scheduler;
pub use webhook::create_webhook_routes;
