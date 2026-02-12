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
mod tools;

pub use server::R8rMcpServer;
