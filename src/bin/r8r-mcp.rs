//! R8r MCP Server binary
//!
//! Run with: r8r-mcp [OPTIONS]
//!
//! This binary starts the r8r MCP server, which exposes r8r workflow
//! capabilities via the Model Context Protocol. It can be used with
//! Claude Desktop, Claude Code, or any MCP-compatible client.
//!
//! ## Usage with Claude Desktop
//!
//! Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "r8r": {
//!       "command": "r8r-mcp"
//!     }
//!   }
//! }
//! ```
//!
//! ## Usage with Claude Code
//!
//! Add to `.claude/settings.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "r8r": {
//!       "command": "r8r-mcp",
//!       "args": ["--db", "/path/to/r8r.db"]
//!     }
//!   }
//! }
//! ```

use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use r8r::R8rMcpServer;

#[derive(Parser)]
#[command(name = "r8r-mcp")]
#[command(about = "R8r MCP Server - Expose r8r workflows via Model Context Protocol")]
#[command(version)]
struct Cli {
    /// Database path (default: ~/.r8r/r8r.db or $R8R_DB)
    #[arg(long, short)]
    db: Option<PathBuf>,

    /// Enable debug logging (writes to stderr)
    #[arg(long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging to stderr (stdout is used for MCP transport)
    if cli.debug {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("r8r=debug,rmcp=debug"))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("r8r=info"))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .init();
    }

    // Get database path from:
    // 1. CLI argument
    // 2. R8R_DB environment variable
    // 3. Default: ~/.r8r/r8r.db
    let db_path = cli
        .db
        .or_else(|| std::env::var("R8R_DB").ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            dirs::data_dir()
                .map(|d| d.join("r8r").join("r8r.db"))
                .unwrap_or_else(|| PathBuf::from(".r8r/r8r.db"))
        });

    // Ensure directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    tracing::info!("Starting r8r MCP server with database: {:?}", db_path);

    // Create and run server
    let server = R8rMcpServer::new(&db_path)?;
    server.run_stdio().await?;

    Ok(())
}
