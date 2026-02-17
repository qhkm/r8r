//! Trigger implementations.
//!
//! Triggers define what starts a workflow:
//! - Cron: Schedule-based (e.g., every 5 minutes)
//! - Manual: CLI or API invocation
//! - Webhook: HTTP endpoint with optional debouncing and header filtering
//! - Event: Redis pub/sub events with fan-out and routing support
//! - Delayed: Events scheduled for future processing

mod delayed;
mod event;
mod scheduler;
mod webhook;

pub use delayed::{DelayedEvent, DelayedEventQueue};
pub use event::{EventError, EventFanOut, EventMessage, EventPublisher, EventSubscriber};
pub use scheduler::Scheduler;
pub use webhook::create_webhook_routes;
