//! Error types for r8r.
//!
//! All errors are designed to be agent-friendly with structured information
//! that AI agents can parse and act upon.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type alias for r8r operations.
pub type Result<T> = std::result::Result<T, Error>;

/// r8r error types.
///
/// Each error variant includes a code that agents can parse programmatically.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Workflow error: {0}")]
    Workflow(String),

    #[error("Node error: {0}")]
    Node(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Get the error code for agent parsing.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Workflow(_) => "WORKFLOW_ERROR",
            Error::Node(_) => "NODE_ERROR",
            Error::Execution(_) => "EXECUTION_ERROR",
            Error::Storage(_) => "STORAGE_ERROR",
            Error::Config(_) => "CONFIG_ERROR",
            Error::Parse(_) => "PARSE_ERROR",
            Error::Validation(_) => "VALIDATION_ERROR",
            Error::Http(_) => "HTTP_ERROR",
            Error::Database(_) => "DATABASE_ERROR",
            Error::Yaml(_) => "YAML_ERROR",
            Error::Json(_) => "JSON_ERROR",
            Error::Io(_) => "IO_ERROR",
        }
    }

    /// Convert to agent-friendly JSON response.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "success": false,
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        })
    }
}

/// Agent-friendly error response structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_results: Option<serde_json::Value>,
}

impl ErrorResponse {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            node_id: None,
            retry_after_ms: None,
            partial_results: None,
        }
    }

    pub fn with_node(mut self, node_id: &str) -> Self {
        self.node_id = Some(node_id.to_string());
        self
    }

    pub fn with_retry(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }
}
