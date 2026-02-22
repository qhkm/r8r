//! Docker sandbox backend.
//!
//! Executes code inside Docker containers with resource limits and
//! network isolation. Requires Docker daemon to be running.

use async_trait::async_trait;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogOutput, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, WaitContainerOptions,
};
use bollard::models::HostConfig;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info, warn};

use super::{SandboxBackend, SandboxError, SandboxRequest, SandboxResult};
use crate::config::SandboxDockerConfig;

/// Docker-based sandbox backend.
///
/// Provides strong isolation via containers with:
/// - Memory limits
/// - Network isolation (--network none)
/// - Read-only root filesystem
/// - PID limits (prevent fork bombs)
pub struct DockerBackend {
    docker: Docker,
    config: SandboxDockerConfig,
}

impl DockerBackend {
    pub fn new(config: &SandboxDockerConfig) -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_local_defaults().map_err(|e| {
            SandboxError::BackendUnavailable {
                backend: "docker".to_string(),
                reason: format!("Failed to connect to Docker: {}", e),
            }
        })?;
        Ok(Self {
            docker,
            config: config.clone(),
        })
    }

    fn image_for_runtime(&self, runtime: &str) -> Result<&str, SandboxError> {
        match runtime {
            "python3" | "python" => Ok(&self.config.python_image),
            "node" | "nodejs" | "javascript" => Ok(&self.config.node_image),
            "bash" | "sh" => Ok(&self.config.bash_image),
            other => Err(SandboxError::RuntimeNotFound {
                runtime: other.to_string(),
            }),
        }
    }

    fn cmd_for_runtime(runtime: &str, code: &str) -> Vec<String> {
        match runtime {
            "python3" | "python" => vec!["python3".to_string(), "-c".to_string(), code.to_string()],
            "node" | "nodejs" | "javascript" => {
                vec!["node".to_string(), "-e".to_string(), code.to_string()]
            }
            _ => vec!["sh".to_string(), "-c".to_string(), code.to_string()],
        }
    }
}

#[async_trait]
impl SandboxBackend for DockerBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        let image = self.image_for_runtime(&req.runtime)?;
        let cmd = Self::cmd_for_runtime(&req.runtime, &req.code);
        let start = Instant::now();

        let container_name = format!("r8r-sandbox-{}", uuid::Uuid::new_v4());

        // Build env vars as Vec<String> "KEY=VALUE"
        let env_vars: Vec<String> = req.env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();

        // Build host config with resource limits
        let mut host_config = HostConfig::default();

        if let Some(mem_mb) = req.memory_mb {
            host_config.memory = Some((mem_mb * 1024 * 1024) as i64);
        }

        if !req.network {
            host_config.network_mode = Some("none".to_string());
        }

        // Prevent fork bombs
        host_config.pids_limit = Some(256);

        // Read-only root filesystem with writable /tmp via tmpfs
        host_config.readonly_rootfs = Some(true);
        host_config.tmpfs = Some(HashMap::from([(
            "/tmp".to_string(),
            "rw,noexec,nosuid,size=64m".to_string(),
        )]));

        let container_config = ContainerConfig {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            env: Some(env_vars),
            host_config: Some(host_config),
            ..Default::default()
        };

        debug!("Creating Docker container: {}", container_name);

        // Create container
        let create_opts = CreateContainerOptions {
            name: &container_name,
            platform: None,
        };
        self.docker
            .create_container(Some(create_opts), container_config)
            .await
            .map_err(|e| SandboxError::BackendUnavailable {
                backend: "docker".to_string(),
                reason: format!("Failed to create container: {}", e),
            })?;

        // Start container
        self.docker
            .start_container(&container_name, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| {
                SandboxError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to start container: {}", e),
                ))
            })?;

        // Wait for container with timeout; WaitContainerOptions requires a condition field
        let wait_result = tokio::time::timeout(req.timeout, async {
            let mut stream = self.docker.wait_container(
                &container_name,
                Some(WaitContainerOptions {
                    condition: "not-running",
                }),
            );
            stream.next().await
        })
        .await;

        let exit_code = match wait_result {
            Ok(Some(Ok(response))) => response.status_code as i32,
            Ok(Some(Err(e))) => {
                warn!("Container wait error: {}", e);
                -1
            }
            Ok(None) => -1,
            Err(_) => {
                // Timeout — force-remove container
                let _ = self
                    .docker
                    .remove_container(
                        &container_name,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
                return Err(SandboxError::Timeout {
                    timeout_seconds: req.timeout.as_secs(),
                });
            }
        };

        // Collect logs
        let mut stdout = String::new();
        let mut stderr = String::new();

        let log_opts = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            tail: "all".to_string(),
            ..Default::default()
        };

        let mut logs = self.docker.logs(&container_name, Some(log_opts));
        let max_per_stream = req.max_output_bytes as usize / 2;

        while let Some(Ok(log)) = logs.next().await {
            match log {
                LogOutput::StdOut { message } => {
                    if stdout.len() < max_per_stream {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                }
                LogOutput::StdErr { message } => {
                    if stderr.len() < max_per_stream {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                }
                _ => {}
            }
        }

        // Remove container (best-effort; ignore errors)
        let _ = self
            .docker
            .remove_container(
                &container_name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        let duration_ms = start.elapsed().as_millis() as u64;
        info!(
            "Docker sandbox completed: exit_code={} duration={}ms",
            exit_code, duration_ms
        );

        Ok(SandboxResult {
            stdout,
            stderr,
            exit_code,
            duration_ms,
        })
    }

    fn name(&self) -> &str {
        "docker"
    }

    fn available(&self) -> bool {
        // We verified connectivity in new()
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_for_runtime_python() {
        let cmd = DockerBackend::cmd_for_runtime("python3", "print('hi')");
        assert_eq!(cmd, vec!["python3", "-c", "print('hi')"]);
    }

    #[test]
    fn test_cmd_for_runtime_node() {
        let cmd = DockerBackend::cmd_for_runtime("node", "console.log('hi')");
        assert_eq!(cmd, vec!["node", "-e", "console.log('hi')"]);
    }

    #[test]
    fn test_cmd_for_runtime_bash() {
        let cmd = DockerBackend::cmd_for_runtime("bash", "echo hi");
        assert_eq!(cmd, vec!["sh", "-c", "echo hi"]);
    }

    // Docker integration tests require Docker daemon and are not run in CI.
    // They belong in tests/sandbox_docker_integration.rs
}
