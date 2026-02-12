//! R8r MCP Server implementation.
//!
//! This module provides the main MCP server that exposes r8r capabilities.

use std::path::Path;
use std::sync::Arc;

use rmcp::transport::stdio;
use rmcp::ServiceExt;
use tracing::info;

use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;

use super::tools::R8rService;

/// R8r MCP Server
///
/// Exposes r8r workflow capabilities via Model Context Protocol.
/// Can run with stdio transport (for Claude Desktop) or HTTP (for web clients).
pub struct R8rMcpServer {
    service: R8rService,
}

impl R8rMcpServer {
    /// Create a new MCP server with the given database path.
    pub fn new(db_path: &Path) -> crate::error::Result<Self> {
        let storage = SqliteStorage::open(db_path)?;
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, storage.clone()));

        Ok(Self {
            service: R8rService { storage, executor },
        })
    }

    /// Create a new MCP server with in-memory storage.
    ///
    /// Useful for testing and development.
    pub fn in_memory() -> crate::error::Result<Self> {
        let storage = SqliteStorage::open_in_memory()?;
        let registry = NodeRegistry::new();
        let executor = Arc::new(Executor::new(registry, storage.clone()));

        Ok(Self {
            service: R8rService { storage, executor },
        })
    }

    /// Run the MCP server with stdio transport.
    pub async fn run_stdio(self) -> crate::error::Result<()> {
        info!("Starting r8r MCP server (stdio transport)");

        let service = self
            .service
            .serve(stdio())
            .await
            .map_err(|e| crate::error::Error::Execution(format!("MCP server error: {}", e)))?;

        // Wait for shutdown
        let quit_reason = service
            .waiting()
            .await
            .map_err(|e| crate::error::Error::Execution(format!("MCP server error: {}", e)))?;

        info!("r8r MCP server stopped: {:?}", quit_reason);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation_in_memory() {
        let server = R8rMcpServer::in_memory();
        assert!(server.is_ok());
    }
}
