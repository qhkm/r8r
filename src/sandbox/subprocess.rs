//! Subprocess sandbox backend.
//!
//! Executes code by spawning a child process. Provides no real isolation
//! beyond process boundaries — suitable for development and trusted code.

use async_trait::async_trait;
use std::time::Instant;
use tokio::process::Command;
use tracing::warn;

use super::{SandboxBackend, SandboxError, SandboxRequest, SandboxResult};

/// Subprocess-based sandbox backend.
///
/// Spawns a child process for each execution. Code is written to a temp
/// file and executed via the runtime binary (python3, node, bash).
///
/// **WARNING:** This backend provides no isolation. The child process
/// inherits the host's filesystem, network, and environment. Use the
/// Docker backend for production workloads with untrusted code.
pub struct SubprocessBackend;

impl SubprocessBackend {
    pub fn new() -> Self {
        warn!(
            "Subprocess sandbox provides no isolation. \
             Use Docker backend (--features sandbox-docker) for production."
        );
        Self
    }
}

impl Default for SubprocessBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve runtime name to binary path and execution args.
///
/// Returns (binary, flag) where code is passed via flag:
/// - python3 -c <code>
/// - node -e <code>
/// - bash -c <code>
fn resolve_runtime(runtime: &str) -> Result<(&str, &str), SandboxError> {
    match runtime {
        "python3" | "python" => Ok(("python3", "-c")),
        "node" | "nodejs" | "javascript" => Ok(("node", "-e")),
        "bash" | "sh" => Ok(("bash", "-c")),
        other => Err(SandboxError::RuntimeNotFound {
            runtime: other.to_string(),
        }),
    }
}

/// Truncate a string to at most `max_bytes` bytes, at a valid UTF-8 boundary.
fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Find the last valid char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n... [output truncated]");
    truncated
}

#[async_trait]
impl SandboxBackend for SubprocessBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        let (binary, flag) = resolve_runtime(&req.runtime)?;
        let start = Instant::now();

        let mut cmd = Command::new(binary);
        cmd.arg(flag).arg(&req.code).kill_on_drop(true);

        // Set env vars
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        // Run with timeout
        let output = match tokio::time::timeout(req.timeout, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                // IO error (binary not found, permission denied, etc.)
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Err(SandboxError::RuntimeNotFound {
                        runtime: req.runtime,
                    });
                }
                return Err(SandboxError::Io(e));
            }
            Err(_elapsed) => {
                return Err(SandboxError::Timeout {
                    timeout_seconds: req.timeout.as_secs(),
                });
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;

        let max_per_stream = req.max_output_bytes as usize / 2;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Ok(SandboxResult {
            stdout: truncate_output(&stdout, max_per_stream),
            stderr: truncate_output(&stderr, max_per_stream),
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms,
        })
    }

    fn name(&self) -> &str {
        "subprocess"
    }

    fn available(&self) -> bool {
        true // subprocesses always work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_request(runtime: &str, code: &str) -> SandboxRequest {
        SandboxRequest {
            runtime: runtime.to_string(),
            code: code.to_string(),
            timeout: Duration::from_secs(10),
            ..SandboxRequest::default()
        }
    }

    #[test]
    fn test_resolve_runtime_python() {
        let (bin, flag) = resolve_runtime("python3").unwrap();
        assert_eq!(bin, "python3");
        assert_eq!(flag, "-c");
    }

    #[test]
    fn test_resolve_runtime_node() {
        let (bin, flag) = resolve_runtime("node").unwrap();
        assert_eq!(bin, "node");
        assert_eq!(flag, "-e");
    }

    #[test]
    fn test_resolve_runtime_bash() {
        let (bin, flag) = resolve_runtime("bash").unwrap();
        assert_eq!(bin, "bash");
        assert_eq!(flag, "-c");
    }

    #[test]
    fn test_resolve_runtime_unknown() {
        assert!(resolve_runtime("ruby").is_err());
    }

    #[test]
    fn test_truncate_output_short() {
        assert_eq!(truncate_output("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_output_long() {
        let long = "a".repeat(200);
        let truncated = truncate_output(&long, 100);
        assert!(truncated.len() < 200);
        assert!(truncated.contains("[output truncated]"));
    }

    #[tokio::test]
    async fn test_subprocess_bash_echo() {
        let backend = SubprocessBackend;
        let req = make_request("bash", "echo hello");
        let result = backend.execute(req).await.unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello");
        assert!(result.stderr.is_empty());
    }

    #[tokio::test]
    async fn test_subprocess_bash_stderr() {
        let backend = SubprocessBackend;
        let req = make_request("bash", "echo error >&2; exit 1");
        let result = backend.execute(req).await.unwrap();

        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stderr.trim(), "error");
    }

    #[tokio::test]
    async fn test_subprocess_python_json() {
        let backend = SubprocessBackend;
        let req = make_request("python3", "import json; print(json.dumps({'result': 42}))");
        let result = backend.execute(req).await.unwrap();

        assert_eq!(result.exit_code, 0);
        let parsed: serde_json::Value = serde_json::from_str(result.stdout.trim()).unwrap();
        assert_eq!(parsed["result"], 42);
    }

    #[tokio::test]
    async fn test_subprocess_timeout() {
        let backend = SubprocessBackend;
        let req = SandboxRequest {
            runtime: "bash".to_string(),
            code: "sleep 60".to_string(),
            timeout: Duration::from_millis(100),
            ..SandboxRequest::default()
        };
        let result = backend.execute(req).await;

        assert!(matches!(result, Err(SandboxError::Timeout { .. })));
    }

    #[tokio::test]
    async fn test_subprocess_env_vars() {
        let backend = SubprocessBackend;
        let mut req = make_request("bash", "echo $MY_VAR");
        req.env
            .insert("MY_VAR".to_string(), "test_value".to_string());

        let result = backend.execute(req).await.unwrap();
        assert_eq!(result.stdout.trim(), "test_value");
    }

    #[tokio::test]
    async fn test_subprocess_runtime_not_found() {
        let backend = SubprocessBackend;
        let req = make_request("nonexistent_runtime_xyz", "hello");
        let result = backend.execute(req).await;

        assert!(matches!(result, Err(SandboxError::RuntimeNotFound { .. })));
    }
}
