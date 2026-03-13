/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Firecracker microVM sandbox backend.
//!
//! Executes code inside lightweight Firecracker microVMs for strong isolation.
//! Each execution boots a fresh microVM, runs the code via a guest agent
//! communicating over vsock, then destroys the VM.
//!
//! Requires:
//! - `/dev/kvm` device (Linux KVM support)
//! - `firecracker` binary in PATH (or configured path)
//! - vmlinux kernel image at configured path
//! - rootfs ext4 image containing runtimes + guest agent at configured path

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tracing::{debug, info, warn};

use super::{SandboxBackend, SandboxError, SandboxRequest, SandboxResult};
use crate::config::SandboxFirecrackerConfig;

/// Firecracker microVM sandbox backend.
///
/// Provides strong isolation via microVMs with:
/// - Hardware-level VM isolation (KVM)
/// - Per-execution ephemeral rootfs
/// - vsock-based guest agent communication
/// - Sub-second boot times
pub struct FirecrackerBackend {
    config: SandboxFirecrackerConfig,
}

impl FirecrackerBackend {
    pub fn new(config: &SandboxFirecrackerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }
}

// ============================================================================
// Firecracker API helpers (raw HTTP over Unix socket)
// ============================================================================

/// Send a PUT request to the Firecracker management API using raw HTTP/1.1
/// over a Unix socket. Returns an error if the response is not 2xx.
async fn api_put(
    socket_path: &Path,
    path: &str,
    body: &serde_json::Value,
) -> Result<(), SandboxError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = UnixStream::connect(socket_path).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Connect to Firecracker API socket {}: {}",
            socket_path.display(),
            e
        )))
    })?;

    let body_str = serde_json::to_string(body).unwrap_or_default();
    let request = format!(
        "PUT {} HTTP/1.1
Host: localhost
Content-Type: application/json
Accept: application/json
Content-Length: {}

{}",
        path,
        body_str.len(),
        body_str
    );

    stream.write_all(request.as_bytes()).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Write to Firecracker API: {}",
            e
        )))
    })?;

    // Read up to 8 KiB for the response — enough for any FC API response
    let mut response = vec![0u8; 8192];
    let n = stream.read(&mut response).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Read Firecracker API response: {}",
            e
        )))
    })?;

    let response_str = String::from_utf8_lossy(&response[..n]);

    // Firecracker returns 200 or 204 for successful PUT operations
    if !response_str.starts_with("HTTP/1.1 2") {
        // Extract body from response for error details
        let error_body = response_str
            .split(
                "

",
            )
            .nth(1)
            .unwrap_or("")
            .to_string();
        return Err(SandboxError::BackendUnavailable {
            backend: "firecracker".to_string(),
            reason: format!("API PUT {} failed: {}", path, error_body),
        });
    }

    Ok(())
}

/// Poll for the Firecracker API socket to appear (up to `timeout`).
async fn wait_for_socket(socket_path: &Path, timeout: Duration) -> Result<(), SandboxError> {
    let deadline = Instant::now() + timeout;
    loop {
        if socket_path.exists() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(SandboxError::BackendUnavailable {
                backend: "firecracker".to_string(),
                reason: format!(
                    "API socket {} did not appear within {}ms",
                    socket_path.display(),
                    timeout.as_millis()
                ),
            });
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// ============================================================================
// Guest agent protocol
// ============================================================================

/// Request sent to the in-VM guest agent.
#[derive(serde::Serialize)]
struct AgentRequest {
    runtime: String,
    code: String,
    env: std::collections::HashMap<String, String>,
    timeout_ms: u64,
}

/// Response received from the in-VM guest agent.
#[derive(serde::Deserialize)]
struct AgentResponse {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

/// Send a length-prefixed JSON request to the guest agent and receive the response.
///
/// Protocol: `[4-byte big-endian length][JSON payload]` in both directions.
async fn call_guest_agent(
    vsock_path: &Path,
    req: &AgentRequest,
) -> Result<AgentResponse, SandboxError> {
    let mut stream = UnixStream::connect(vsock_path).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Connect to vsock agent {}: {}",
            vsock_path.display(),
            e
        )))
    })?;

    // Encode request as length-prefixed JSON
    let payload = serde_json::to_vec(req).map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Serialize agent request: {}",
            e
        )))
    })?;
    let len = payload.len() as u32;
    stream.write_all(&len.to_be_bytes()).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Write agent request length: {}",
            e
        )))
    })?;
    stream.write_all(&payload).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!("Write agent request: {}", e)))
    })?;

    // Read response: 4-byte length then payload
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Read agent response length: {}",
            e
        )))
    })?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    // Guard against unreasonably large responses (16 MiB)
    const MAX_RESPONSE: usize = 16 * 1024 * 1024;
    if resp_len > MAX_RESPONSE {
        return Err(SandboxError::ResourceLimit {
            resource: "agent_response".to_string(),
            limit: format!("{} bytes", MAX_RESPONSE),
        });
    }

    let mut resp_buf = vec![0u8; resp_len];
    stream.read_exact(&mut resp_buf).await.map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Read agent response payload: {}",
            e
        )))
    })?;

    serde_json::from_slice(&resp_buf).map_err(|e| {
        SandboxError::Io(std::io::Error::other(format!(
            "Deserialize agent response: {}",
            e
        )))
    })
}

// ============================================================================
// SandboxBackend implementation
// ============================================================================

#[async_trait]
impl SandboxBackend for FirecrackerBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        let start = Instant::now();

        // Generate unique IDs for socket paths so parallel executions don't collide
        let exec_id = uuid::Uuid::new_v4().to_string();
        let api_socket = PathBuf::from(format!("/tmp/r8r-fc-{}.sock", exec_id));
        let vsock_uds = PathBuf::from(format!("/tmp/r8r-fc-vsock-{}.sock", exec_id));

        debug!("Starting Firecracker microVM: exec_id={}", exec_id);

        // Determine memory: prefer request's memory_mb, fall back to config default
        let memory_mb = req.memory_mb.unwrap_or(self.config.memory_mb);

        // ----------------------------------------------------------------
        // 1. Spawn Firecracker process
        // ----------------------------------------------------------------
        let mut child = Command::new(&self.config.firecracker_bin)
            .arg("--api-sock")
            .arg(&api_socket)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SandboxError::BackendUnavailable {
                        backend: "firecracker".to_string(),
                        reason: format!(
                            "Firecracker binary '{}' not found",
                            self.config.firecracker_bin
                        ),
                    }
                } else {
                    SandboxError::Io(std::io::Error::other(format!(
                        "Failed to spawn Firecracker: {}",
                        e
                    )))
                }
            })?;

        // Helper: kill child and clean up sockets on error paths
        let cleanup = |path1: &Path, path2: &Path| {
            let _ = std::fs::remove_file(path1);
            let _ = std::fs::remove_file(path2);
        };

        // ----------------------------------------------------------------
        // 2. Wait for the API socket to appear (up to 2 seconds)
        // ----------------------------------------------------------------
        if let Err(e) = wait_for_socket(&api_socket, Duration::from_secs(2)).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        debug!("Firecracker API socket ready: exec_id={}", exec_id);

        // ----------------------------------------------------------------
        // 3. Configure VM via Firecracker API
        // ----------------------------------------------------------------

        // Machine config: vCPUs and memory
        let machine_cfg = serde_json::json!({
            "vcpu_count": self.config.vcpu_count,
            "mem_size_mib": memory_mb
        });
        if let Err(e) = api_put(&api_socket, "/machine-config", &machine_cfg).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        // Boot source: kernel image and boot arguments
        let boot_source = serde_json::json!({
            "kernel_image_path": self.config.kernel_path,
            "boot_args": "console=ttyS0 reboot=k panic=1 pci=off nomodules"
        });
        if let Err(e) = api_put(&api_socket, "/boot-source", &boot_source).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        // Root drive: rootfs image (read-only so each VM gets a clean state)
        let drive = serde_json::json!({
            "drive_id": "rootfs",
            "path_on_host": self.config.rootfs_path,
            "is_root_device": true,
            "is_read_only": true
        });
        if let Err(e) = api_put(&api_socket, "/drives/rootfs", &drive).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        // vsock: guest CID + host-side Unix socket path
        let vsock = serde_json::json!({
            "guest_cid": self.config.vsock_guest_cid,
            "uds_path": vsock_uds.to_string_lossy()
        });
        if let Err(e) = api_put(&api_socket, "/vsock", &vsock).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        // Start the VM
        let start_action = serde_json::json!({ "action_type": "InstanceStart" });
        if let Err(e) = api_put(&api_socket, "/actions", &start_action).await {
            let _ = child.kill().await;
            cleanup(&api_socket, &vsock_uds);
            return Err(e);
        }

        debug!("Firecracker VM started: exec_id={}", exec_id);

        // ----------------------------------------------------------------
        // 4. Wait for boot (~500ms) then connect to guest agent
        // ----------------------------------------------------------------
        tokio::time::sleep(Duration::from_millis(500)).await;

        let agent_req = AgentRequest {
            runtime: req.runtime.clone(),
            code: req.code.clone(),
            env: req.env.clone(),
            timeout_ms: req.timeout.as_millis() as u64,
        };

        // Connect to agent via vsock with per-request timeout
        let agent_result =
            tokio::time::timeout(req.timeout, call_guest_agent(&vsock_uds, &agent_req)).await;

        // ----------------------------------------------------------------
        // 5. Kill VM and clean up sockets regardless of outcome
        // ----------------------------------------------------------------
        let _ = child.kill().await;
        cleanup(&api_socket, &vsock_uds);

        let duration_ms = start.elapsed().as_millis() as u64;

        // ----------------------------------------------------------------
        // 6. Map results
        // ----------------------------------------------------------------
        match agent_result {
            Ok(Ok(resp)) => {
                info!(
                    "Firecracker sandbox completed: exit_code={} duration={}ms",
                    resp.exit_code, duration_ms
                );

                let max_per_stream = req.max_output_bytes as usize / 2;
                let stdout = if resp.stdout.len() > max_per_stream {
                    let mut s = resp.stdout[..max_per_stream].to_string();
                    s.push_str(
                        "
... [output truncated]",
                    );
                    s
                } else {
                    resp.stdout
                };
                let stderr = if resp.stderr.len() > max_per_stream {
                    let mut s = resp.stderr[..max_per_stream].to_string();
                    s.push_str(
                        "
... [output truncated]",
                    );
                    s
                } else {
                    resp.stderr
                };

                Ok(SandboxResult {
                    stdout,
                    stderr,
                    exit_code: resp.exit_code,
                    duration_ms,
                    artifacts: Vec::new(),
                })
            }
            Ok(Err(e)) => Err(e),
            Err(_elapsed) => {
                warn!("Firecracker execution timed out: exec_id={}", exec_id);
                Err(SandboxError::Timeout {
                    timeout_seconds: req.timeout.as_secs(),
                })
            }
        }
    }

    fn name(&self) -> &str {
        "firecracker"
    }

    fn available(&self) -> bool {
        // Requires KVM device and the Firecracker binary
        Path::new("/dev/kvm").exists()
            && std::process::Command::new(&self.config.firecracker_bin)
                .arg("--version")
                .output()
                .is_ok()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SandboxFirecrackerConfig;

    #[test]
    fn test_backend_name() {
        let backend = FirecrackerBackend::new(&SandboxFirecrackerConfig::default());
        assert_eq!(backend.name(), "firecracker");
    }

    #[test]
    fn test_default_config() {
        let cfg = SandboxFirecrackerConfig::default();
        assert_eq!(cfg.firecracker_bin, "firecracker");
        assert_eq!(cfg.kernel_path, "/opt/r8r/vmlinux");
        assert_eq!(cfg.rootfs_path, "/opt/r8r/rootfs.ext4");
        assert_eq!(cfg.vcpu_count, 1);
        assert_eq!(cfg.memory_mb, 256);
        assert_eq!(cfg.vsock_guest_cid, 3);
        assert_eq!(cfg.agent_port, 52);
    }

    #[test]
    fn test_available_checks_kvm() {
        // On macOS and CI (no /dev/kvm), available() must return false.
        // On a Linux KVM host with firecracker installed it may return true — that's fine.
        let backend = FirecrackerBackend::new(&SandboxFirecrackerConfig::default());
        let result = backend.available();

        // On macOS, /dev/kvm never exists, so we must be false.
        #[cfg(target_os = "macos")]
        assert!(
            !result,
            "available() should be false on macOS (no /dev/kvm)"
        );

        // On Linux without firecracker or KVM, also false; with both, true — just verify it's bool.
        #[cfg(target_os = "linux")]
        {
            // Just assert it returned without panicking.
            let _ = result;
        }
    }

    #[test]
    fn test_new_with_custom_config() {
        let cfg = SandboxFirecrackerConfig {
            firecracker_bin: "/usr/local/bin/firecracker".to_string(),
            kernel_path: "/data/vmlinux".to_string(),
            rootfs_path: "/data/rootfs.ext4".to_string(),
            vcpu_count: 2,
            memory_mb: 512,
            vsock_guest_cid: 5,
            agent_port: 1234,
        };
        let backend = FirecrackerBackend::new(&cfg);
        assert_eq!(backend.name(), "firecracker");
        assert_eq!(backend.config.vcpu_count, 2);
        assert_eq!(backend.config.memory_mb, 512);
    }
}
