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

    #[error("Internal error: {0}")]
    Internal(String),

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
            Error::Internal(_) => "INTERNAL_ERROR",
            Error::Http(_) => "HTTP_ERROR",
            Error::Database(_) => "DATABASE_ERROR",
            Error::Yaml(_) => "YAML_ERROR",
            Error::Json(_) => "JSON_ERROR",
            Error::Io(_) => "IO_ERROR",
        }
    }

    /// Get a sanitized error message safe for external consumers.
    ///
    /// This hides internal details like file paths, SQL statements,
    /// and stack traces that could leak sensitive information.
    pub fn external_message(&self) -> String {
        match self {
            // User-facing errors - safe to expose the message
            Error::Workflow(msg) => format!("Workflow error: {}", msg),
            Error::Node(msg) => format!("Node error: {}", msg),
            Error::Execution(msg) => format!("Execution error: {}", msg),
            Error::Config(msg) => format!("Configuration error: {}", msg),
            Error::Parse(msg) => format!("Parse error: {}", msg),
            Error::Validation(msg) => format!("Validation error: {}", msg),

            // Internal errors - sanitize to avoid leaking details
            Error::Storage(_) => "A storage error occurred".to_string(),
            Error::Internal(_) => "An internal error occurred".to_string(),
            Error::Database(_) => "A database error occurred".to_string(),
            Error::Io(_) => "An I/O error occurred".to_string(),

            // HTTP errors - extract status code if available, hide details
            Error::Http(e) => {
                if let Some(status) = e.status() {
                    format!("HTTP request failed with status {}", status.as_u16())
                } else if e.is_timeout() {
                    "HTTP request timed out".to_string()
                } else if e.is_connect() {
                    "Failed to connect to remote server".to_string()
                } else {
                    "HTTP request failed".to_string()
                }
            }

            // Serialization errors - indicate format issue without details
            Error::Yaml(_) => "Invalid YAML format".to_string(),
            Error::Json(_) => "Invalid JSON format".to_string(),
        }
    }

    /// Convert to agent-friendly JSON response with sanitized message.
    pub fn to_external_json(&self) -> serde_json::Value {
        serde_json::json!({
            "success": false,
            "error": {
                "code": self.code(),
                "message": self.external_message(),
            }
        })
    }

    /// Convert to agent-friendly JSON response (includes full error details).
    ///
    /// **Warning**: Only use this for internal/debug purposes, not for external APIs.
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
