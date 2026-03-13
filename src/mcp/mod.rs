/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! R8r MCP Server
//!
//! Exposes r8r workflow capabilities via Model Context Protocol (MCP).
//! This allows AI agents (Claude Desktop, Claude Code, ZeptoClaw, etc.)
//! to directly interact with r8r workflows.
//!
//! ## Capabilities
//!
//! - **Tools**: Execute, list, validate, and debug workflows
//! - **Resources**: Access workflow definitions and execution data
//! - **Prompts**: Built-in prompts for debugging and creating workflows
//!
//! ## Example
//!
//! ```rust,ignore
//! use r8r::mcp::R8rMcpServer;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let server = R8rMcpServer::new("~/.r8r/r8r.db")?;
//!     server.run_stdio().await?;
//!     Ok(())
//! }
//! ```

mod server;
pub(crate) mod tools;

pub use server::R8rMcpServer;
