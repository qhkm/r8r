//! Pluggable code execution sandbox.
//!
//! Provides the `SandboxBackend` trait for running code in isolated
//! environments. Backends include subprocess (default) and Docker.

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

#[cfg(feature = "sandbox-docker")]
pub mod docker;
pub mod subprocess;

#[cfg(feature = "sandbox-docker")]
pub use docker::DockerBackend;
pub use subprocess::SubprocessBackend;

/// Trait for sandbox execution backends.
///
/// Implementations run code in isolated environments with varying
/// levels of security. The default backend uses subprocesses; Docker
/// provides stronger isolation.
#[async_trait]
pub trait SandboxBackend: Send + Sync + 'static {
    /// Execute code in a sandboxed environment.
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError>;

    /// Human-readable backend name for logs and metadata.
    fn name(&self) -> &str;

    /// Runtime check: is the backend usable on this system?
    fn available(&self) -> bool;
}

/// Request to execute code in a sandbox.
#[derive(Debug, Clone)]
pub struct SandboxRequest {
    /// Runtime to use: "python3", "node", "bash"
    pub runtime: String,
    /// Code to execute
    pub code: String,
    /// Environment variables to set
    pub env: HashMap<String, String>,
    /// Maximum execution time
    pub timeout: Duration,
    /// Memory limit in MB (enforced by Docker, advisory for subprocess)
    pub memory_mb: Option<u64>,
    /// Whether to allow network access (default: false)
    pub network: bool,
    /// Max bytes for stdout + stderr combined
    pub max_output_bytes: u64,
}

impl Default for SandboxRequest {
    fn default() -> Self {
        Self {
            runtime: "python3".to_string(),
            code: String::new(),
            env: HashMap::new(),
            timeout: Duration::from_secs(30),
            memory_mb: None,
            network: false,
            max_output_bytes: 1_048_576, // 1MB
        }
    }
}

/// Result of sandbox code execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// Errors from sandbox execution.
#[derive(Debug)]
pub enum SandboxError {
    /// Code execution exceeded the timeout.
    Timeout { timeout_seconds: u64 },
    /// Requested runtime binary not found on system.
    RuntimeNotFound { runtime: String },
    /// Backend is not available on this system.
    BackendUnavailable { backend: String, reason: String },
    /// Resource limit exceeded.
    ResourceLimit { resource: String, limit: String },
    /// IO error during sandbox operation.
    Io(std::io::Error),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout { timeout_seconds } => {
                write!(f, "Sandbox execution timed out after {}s", timeout_seconds)
            }
            Self::RuntimeNotFound { runtime } => {
                write!(f, "Runtime '{}' not found", runtime)
            }
            Self::BackendUnavailable { backend, reason } => {
                write!(f, "Sandbox backend '{}' unavailable: {}", backend, reason)
            }
            Self::ResourceLimit { resource, limit } => {
                write!(f, "Resource limit exceeded: {} ({})", resource, limit)
            }
            Self::Io(e) => write!(f, "Sandbox IO error: {}", e),
        }
    }
}

impl std::error::Error for SandboxError {}

impl From<std::io::Error> for SandboxError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ============================================================================
// Mock backend for testing
// ============================================================================

#[cfg(test)]
pub mod mock {
    use super::*;

    pub struct MockSandboxBackend {
        pub result: SandboxResult,
    }

    impl MockSandboxBackend {
        pub fn new(result: SandboxResult) -> Self {
            Self { result }
        }

        pub fn success(stdout: &str) -> Self {
            Self::new(SandboxResult {
                stdout: stdout.to_string(),
                stderr: String::new(),
                exit_code: 0,
                duration_ms: 10,
            })
        }
    }

    #[async_trait]
    impl SandboxBackend for MockSandboxBackend {
        async fn execute(&self, _req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
            Ok(self.result.clone())
        }

        fn name(&self) -> &str {
            "mock"
        }

        fn available(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_request_defaults() {
        let req = SandboxRequest::default();
        assert_eq!(req.runtime, "python3");
        assert_eq!(req.timeout, Duration::from_secs(30));
        assert!(!req.network);
        assert_eq!(req.max_output_bytes, 1_048_576);
    }

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::Timeout {
            timeout_seconds: 30,
        };
        assert_eq!(err.to_string(), "Sandbox execution timed out after 30s");

        let err = SandboxError::RuntimeNotFound {
            runtime: "ruby".to_string(),
        };
        assert_eq!(err.to_string(), "Runtime 'ruby' not found");
    }

    #[tokio::test]
    async fn test_mock_backend() {
        let backend = mock::MockSandboxBackend::success(r#"{"result": 42}"#);
        assert_eq!(backend.name(), "mock");
        assert!(backend.available());

        let result = backend.execute(SandboxRequest::default()).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, r#"{"result": 42}"#);
    }
}
