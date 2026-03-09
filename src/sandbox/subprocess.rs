//! Subprocess sandbox backend.
//!
//! Executes code by spawning a child process. Provides no real isolation
//! beyond process boundaries — suitable for development and trusted code.

use async_trait::async_trait;
use std::time::Instant;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::warn;

use super::{
    collect_output_artifacts, SandboxArtifact, SandboxBackend, SandboxError, SandboxRequest,
    SandboxResult, SandboxStreamEvent,
};

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

/// Resolve runtime name to binary path.
fn runtime_binary(runtime: &str) -> Result<&str, SandboxError> {
    match runtime {
        "python3" | "python" => Ok("python3"),
        "node" | "nodejs" | "javascript" => Ok("node"),
        "bash" | "sh" => Ok("bash"),
        other => Err(SandboxError::RuntimeNotFound {
            runtime: other.to_string(),
        }),
    }
}

/// Build the full shell command string for a subprocess execution.
///
/// Uses a temp file for the code (avoids shell-quoting issues and CLI arg
/// length limits). Returns the bash one-liner to execute.
fn build_shell_cmd(
    runtime: &str,
    code_path: &str,
    packages: &[String],
) -> Result<String, SandboxError> {
    let install_prefix = if packages.is_empty() {
        String::new()
    } else {
        match runtime {
            "python3" | "python" => format!(
                "pip install -q {} 2>/dev/null && ",
                packages.join(" ")
            ),
            "node" | "nodejs" | "javascript" => format!(
                "npm install -g {} 2>/dev/null && ",
                packages.join(" ")
            ),
            _ => String::new(),
        }
    };

    let run_cmd = match runtime {
        "python3" | "python" => format!("python3 {}", code_path),
        "node" | "nodejs" | "javascript" => format!("node {}", code_path),
        "bash" | "sh" => format!("bash {}", code_path),
        other => {
            return Err(SandboxError::RuntimeNotFound {
                runtime: other.to_string(),
            })
        }
    };

    Ok(format!("{}{}", install_prefix, run_cmd))
}

/// Truncate a string to at most `max_bytes` bytes, at a valid UTF-8 boundary.
fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n... [output truncated]");
    truncated
}

/// Write code to a temp file and return the (TempDir, file path string).
fn write_code_to_tempfile(
    runtime: &str,
    code: &str,
) -> Result<(tempfile::TempDir, String), SandboxError> {
    let dir = tempfile::TempDir::new()?;
    let ext = match runtime {
        "python3" | "python" => "py",
        "node" | "nodejs" | "javascript" => "js",
        _ => "sh",
    };
    let code_path = dir.path().join(format!("_r8r_code.{}", ext));
    std::fs::write(&code_path, code)?;
    let path_str = code_path.to_string_lossy().to_string();
    Ok((dir, path_str))
}

#[async_trait]
impl SandboxBackend for SubprocessBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        let start = Instant::now();

        // Write code to temp file (handles quoting + size limits)
        let (_code_dir, code_path) = write_code_to_tempfile(&req.runtime, &req.code)?;

        // Optionally create output artifact directory
        let output_dir = if req.collect_artifacts {
            let dir = tempfile::TempDir::new()?;
            Some(dir)
        } else {
            None
        };

        // Build shell command (with optional package install prefix)
        let shell_cmd = build_shell_cmd(&req.runtime, &code_path, &req.packages)?;

        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(&shell_cmd).kill_on_drop(true);

        // Propagate env vars
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        // Expose output dir path to code via env var
        if let Some(ref dir) = output_dir {
            std::fs::create_dir_all(dir.path().join("output"))?;
            cmd.env("R8R_OUTPUT_DIR", dir.path().join("output"));
        }

        let output = match tokio::time::timeout(req.timeout, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Err(SandboxError::RuntimeNotFound {
                        runtime: req.runtime,
                    });
                }
                return Err(SandboxError::Io(e));
            }
            Err(_) => {
                return Err(SandboxError::Timeout {
                    timeout_seconds: req.timeout.as_secs(),
                });
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let max_per_stream = req.max_output_bytes as usize / 2;
        let stdout = truncate_output(&String::from_utf8_lossy(&output.stdout), max_per_stream);
        let stderr = truncate_output(&String::from_utf8_lossy(&output.stderr), max_per_stream);

        // Collect file artifacts
        let artifacts = if let Some(ref dir) = output_dir {
            collect_output_artifacts(dir.path().join("output"), req.max_artifact_bytes)?
        } else {
            Vec::new()
        };

        Ok(SandboxResult {
            stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms,
            artifacts,
        })
    }

    /// Streaming variant: spawns child process and forwards stdout/stderr line-by-line.
    async fn execute_streaming(&self, req: SandboxRequest, tx: mpsc::Sender<SandboxStreamEvent>) {
        use tokio::io::BufReader;
        use tokio::io::AsyncBufReadExt;
        use tokio::process::Command;

        let start = Instant::now();

        let (_code_dir, code_path) = match write_code_to_tempfile(&req.runtime, &req.code) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
                return;
            }
        };

        let output_dir = if req.collect_artifacts {
            match tempfile::TempDir::new() {
                Ok(d) => {
                    let _ = std::fs::create_dir_all(d.path().join("output"));
                    Some(d)
                }
                Err(e) => {
                    let _ = tx
                        .send(SandboxStreamEvent::Error(format!(
                            "Failed to create output dir: {}",
                            e
                        )))
                        .await;
                    return;
                }
            }
        } else {
            None
        };

        let shell_cmd = match build_shell_cmd(&req.runtime, &code_path, &req.packages) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
                return;
            }
        };

        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg(&shell_cmd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        if let Some(ref dir) = output_dir {
            cmd.env("R8R_OUTPUT_DIR", dir.path().join("output"));
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
                return;
            }
        };

        // Stream stdout and stderr concurrently
        let stdout_pipe = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();

        let tx_out = tx.clone();
        let tx_err = tx.clone();

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout_pipe).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_out
                    .send(SandboxStreamEvent::Stdout(format!("{}\n", line)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx_err
                    .send(SandboxStreamEvent::Stderr(format!("{}\n", line)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        // Wait for child with timeout
        let exit_code = match tokio::time::timeout(req.timeout, child.wait()).await {
            Ok(Ok(status)) => status.code().unwrap_or(-1),
            Ok(Err(e)) => {
                let _ = tx.send(SandboxStreamEvent::Error(e.to_string())).await;
                return;
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = tx
                    .send(SandboxStreamEvent::Error(format!(
                        "Sandbox execution timed out after {}s",
                        req.timeout.as_secs()
                    )))
                    .await;
                return;
            }
        };

        // Wait for IO tasks to flush
        let _ = tokio::join!(stdout_task, stderr_task);

        let duration_ms = start.elapsed().as_millis() as u64;

        let artifacts = if let Some(ref dir) = output_dir {
            collect_output_artifacts(dir.path().join("output"), req.max_artifact_bytes)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let _ = tx
            .send(SandboxStreamEvent::Done {
                exit_code,
                duration_ms,
                artifacts,
            })
            .await;
    }

    fn name(&self) -> &str {
        "subprocess"
    }

    fn available(&self) -> bool {
        true
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
    fn test_runtime_binary_python() {
        assert_eq!(runtime_binary("python3").unwrap(), "python3");
        assert_eq!(runtime_binary("python").unwrap(), "python3");
    }

    #[test]
    fn test_runtime_binary_node() {
        assert_eq!(runtime_binary("node").unwrap(), "node");
    }

    #[test]
    fn test_runtime_binary_bash() {
        assert_eq!(runtime_binary("bash").unwrap(), "bash");
    }

    #[test]
    fn test_runtime_binary_unknown() {
        assert!(runtime_binary("ruby").is_err());
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

    #[test]
    fn test_build_shell_cmd_no_packages() {
        let cmd = build_shell_cmd("python3", "/tmp/code.py", &[]).unwrap();
        assert_eq!(cmd, "python3 /tmp/code.py");
    }

    #[test]
    fn test_build_shell_cmd_with_packages() {
        let pkgs = vec!["requests".to_string(), "numpy".to_string()];
        let cmd = build_shell_cmd("python3", "/tmp/code.py", &pkgs).unwrap();
        assert!(cmd.contains("pip install -q requests numpy"));
        assert!(cmd.contains("python3 /tmp/code.py"));
    }

    #[tokio::test]
    async fn test_subprocess_bash_echo() {
        let backend = SubprocessBackend;
        let req = make_request("bash", "echo hello");
        let result = backend.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hello");
        assert!(result.stderr.is_empty());
        assert!(result.artifacts.is_empty());
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
            timeout: Duration::from_millis(200),
            ..SandboxRequest::default()
        };
        let result = backend.execute(req).await;
        assert!(matches!(result, Err(SandboxError::Timeout { .. })));
    }

    #[tokio::test]
    async fn test_subprocess_env_vars() {
        let backend = SubprocessBackend;
        let mut req = make_request("bash", "echo $MY_VAR");
        req.env.insert("MY_VAR".to_string(), "test_value".to_string());
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

    #[tokio::test]
    async fn test_subprocess_file_artifacts() {
        let backend = SubprocessBackend;
        let req = SandboxRequest {
            runtime: "bash".to_string(),
            code: r#"
echo "hello world" > "$R8R_OUTPUT_DIR/greeting.txt"
echo '{"key": "value"}' > "$R8R_OUTPUT_DIR/data.json"
"#
            .to_string(),
            collect_artifacts: true,
            timeout: Duration::from_secs(10),
            ..SandboxRequest::default()
        };
        let result = backend.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.artifacts.len(), 2);
        let names: Vec<&str> = result.artifacts.iter().map(|a| a.path.as_str()).collect();
        assert!(names.contains(&"greeting.txt") || names.iter().any(|n| n.ends_with("greeting.txt")));
    }

    #[tokio::test]
    async fn test_subprocess_no_artifacts_when_disabled() {
        let backend = SubprocessBackend;
        let req = make_request("bash", "echo hello");
        let result = backend.execute(req).await.unwrap();
        assert!(result.artifacts.is_empty());
    }

    #[tokio::test]
    async fn test_subprocess_streaming() {
        let backend = SubprocessBackend;
        let req = SandboxRequest {
            runtime: "bash".to_string(),
            code: "echo line1; echo line2; echo line3".to_string(),
            timeout: Duration::from_secs(10),
            ..SandboxRequest::default()
        };

        let (tx, mut rx) = mpsc::channel(32);
        backend.execute_streaming(req, tx).await;

        let mut stdout_lines = Vec::new();
        let mut done = false;
        while let Some(event) = rx.recv().await {
            match event {
                SandboxStreamEvent::Stdout(s) => stdout_lines.push(s),
                SandboxStreamEvent::Done { exit_code, .. } => {
                    assert_eq!(exit_code, 0);
                    done = true;
                }
                _ => {}
            }
        }
        assert!(done);
        assert!(!stdout_lines.is_empty());
    }
}
