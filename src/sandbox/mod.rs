//! Pluggable code execution sandbox.
//!
//! Provides the `SandboxBackend` trait for running code in isolated
//! environments. Backends include subprocess (default) and Docker.

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;
use tokio::sync::mpsc;

#[cfg(feature = "sandbox-docker")]
pub mod docker;
#[cfg(feature = "sandbox-firecracker")]
pub mod firecracker;
pub mod subprocess;

#[cfg(feature = "sandbox-docker")]
pub use docker::DockerBackend;
#[cfg(feature = "sandbox-firecracker")]
pub use firecracker::FirecrackerBackend;
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

    /// Execute code and stream stdout/stderr events through a channel.
    ///
    /// Default implementation buffers all output and sends a single Done event.
    /// Backends that support real streaming should override this.
    async fn execute_streaming(&self, req: SandboxRequest, tx: mpsc::Sender<SandboxStreamEvent>) {
        match self.execute(req).await {
            Ok(result) => {
                if !result.stdout.is_empty() {
                    let _ = tx.send(SandboxStreamEvent::Stdout(result.stdout)).await;
                }
                if !result.stderr.is_empty() {
                    let _ = tx.send(SandboxStreamEvent::Stderr(result.stderr)).await;
                }
                let _ = tx
                    .send(SandboxStreamEvent::Done {
                        exit_code: result.exit_code,
                        duration_ms: result.duration_ms,
                        artifacts: result.artifacts,
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
            }
        }
    }

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
    /// Collect files written to the output dir as artifacts (default: false).
    /// Code should write output files to the path in `R8R_OUTPUT_DIR` env var.
    pub collect_artifacts: bool,
    /// Max total bytes for all collected artifacts (default: 50 MB).
    pub max_artifact_bytes: u64,
    /// Packages to install before running code (pip for python, npm for node).
    pub packages: Vec<String>,
    /// Docker-only: custom image name to use instead of the backend default.
    pub image: Option<String>,
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
            collect_artifacts: false,
            max_artifact_bytes: 50 * 1024 * 1024, // 50MB
            packages: Vec::new(),
            image: None,
        }
    }
}

/// A file artifact collected from the sandbox output directory.
#[derive(Debug, Clone)]
pub struct SandboxArtifact {
    /// Relative path within the output directory.
    pub path: String,
    /// File contents (raw bytes).
    pub content: Vec<u8>,
    /// Size in bytes.
    pub size: usize,
}

/// Result of sandbox code execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    /// Files collected from the output directory (empty if `collect_artifacts` was false).
    pub artifacts: Vec<SandboxArtifact>,
}

/// A streaming event from a sandbox execution.
#[derive(Debug, Clone)]
pub enum SandboxStreamEvent {
    /// A chunk of stdout.
    Stdout(String),
    /// A chunk of stderr.
    Stderr(String),
    /// Execution completed.
    Done {
        exit_code: i32,
        duration_ms: u64,
        artifacts: Vec<SandboxArtifact>,
    },
    /// Execution failed.
    Error(String),
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
// Shared helpers
// ============================================================================

/// Walk `dir` and collect all files as `SandboxArtifact`s.
///
/// Skips files that would push total size over `max_bytes`.
/// Returns paths relative to `dir`.
pub fn collect_output_artifacts(
    dir: impl AsRef<std::path::Path>,
    max_bytes: u64,
) -> Result<Vec<SandboxArtifact>, SandboxError> {
    let dir = dir.as_ref();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut artifacts = Vec::new();
    let mut total_bytes: u64 = 0;

    collect_dir_recursive(dir, dir, &mut artifacts, &mut total_bytes, max_bytes)?;

    // Sort for deterministic ordering
    artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(artifacts)
}

fn collect_dir_recursive(
    base: &std::path::Path,
    current: &std::path::Path,
    artifacts: &mut Vec<SandboxArtifact>,
    total_bytes: &mut u64,
    max_bytes: u64,
) -> Result<(), SandboxError> {
    let entries = std::fs::read_dir(current)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_recursive(base, &path, artifacts, total_bytes, max_bytes)?;
        } else if path.is_file() {
            let file_size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if *total_bytes + file_size > max_bytes {
                // Skip files that exceed the budget
                continue;
            }
            let content = std::fs::read(&path)?;
            let size = content.len();
            *total_bytes += size as u64;

            // Relative path from the output dir root
            let rel = path.strip_prefix(base).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().to_string();

            artifacts.push(SandboxArtifact {
                path: rel_str,
                content,
                size,
            });
        }
    }
    Ok(())
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
                artifacts: Vec::new(),
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
