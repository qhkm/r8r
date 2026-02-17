//! Trigger implementations.
//!
//! Triggers define what starts a workflow:
//! - Cron: Schedule-based (e.g., every 5 minutes)
//! - Manual: CLI or API invocation
//! - Webhook: HTTP endpoint with optional debouncing and header filtering
//! - Event: Pub/sub events with fan-out and routing support (native or Redis)
//! - Delayed: Events scheduled for future processing via SQLite

mod delayed;
mod event;
mod event_backend;
#[cfg(feature = "redis")]
mod redis_backend;
mod scheduler;
mod webhook;

pub use delayed::{DelayedEvent, DelayedEventProcessor};
pub use event::{EventError, EventFanOut, EventMessage, EventSubscriber};
pub use event_backend::{EventBackend, EventReceiver, NativeEventBackend};
#[cfg(feature = "redis")]
pub use redis_backend::RedisEventBackend;
pub use scheduler::Scheduler;
pub use webhook::create_webhook_routes;
