//! r8r - Agent-first Rust workflow automation engine
//!
//! r8r is a lightweight, fast workflow automation tool designed for AI agents.
//! It complements ZeptoClaw by providing deterministic, repeatable workflows
//! that agents can invoke reliably.
//!
//! ## Key Features
//!
//! - **Agent-First**: Designed for AI agents to invoke, not humans to click
//! - **LLM-Friendly YAML**: Consistent patterns, easy for agents to generate
//! - **ZeptoClaw Integration**: Bidirectional - r8r can call ZeptoClaw and vice versa
//! - **Structured Responses**: JSON with predictable shapes, parseable error codes
//!
//! ## Example
//!
//! ```yaml
//! name: order-notification
//! description: Send notifications for new orders
//!
//! triggers:
//!   - type: cron
//!     schedule: "*/5 * * * *"
//!
//! nodes:
//!   - id: fetch-orders
//!     type: http
//!     config:
//!       url: https://api.example.com/orders
//!       method: GET
//!
//!   - id: notify
//!     type: agent
//!     config:
//!       prompt: "Summarize these orders: {{ input }}"
//!       response_format: json
//!     depends_on: [fetch-orders]
//! ```

pub mod api;
pub mod config;
pub mod credentials;
pub mod engine;
pub mod error;
pub mod mcp;
pub mod metrics;
pub mod nodes;
pub mod storage;
pub mod triggers;
pub mod validation;
pub mod workflow;

pub use error::{Error, Result};
pub use mcp::R8rMcpServer;
