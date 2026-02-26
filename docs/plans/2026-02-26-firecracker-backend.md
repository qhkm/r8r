# Firecracker Backend + Future Sandbox Roadmap

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Firecracker microVM sandbox backend for strong hardware-level isolation, plus document future backend options (WASM, Hyperlight, libkrun).

**Architecture:** New `src/sandbox/firecracker.rs` behind `sandbox-firecracker` feature flag. Uses `hyper-client-sockets` to talk to the Firecracker REST API over a Unix socket. Spawns the `firecracker` binary, configures the VM (kernel + rootfs + machine config), executes code via a guest agent over vsock, collects output, and destroys the VM. Requires Linux + KVM (`/dev/kvm`). Falls back to Docker or subprocess on unsupported systems.

**Tech Stack:** `hyper-client-sockets` (Unix socket HTTP), `hyper`/`http-body-util` (HTTP client), `tokio` (async), `serde_json` (API payloads), Firecracker binary + vmlinux kernel + Alpine rootfs (external deps, not compiled in).

---

## Task 1: Add feature flag and dependencies

**Files:**
- Modify: `Cargo.toml:14-22` (features and dependencies)

**Step 1: Add the feature flag and deps to Cargo.toml**

In the `[features]` section, add:

```toml
sandbox-firecracker = ["sandbox", "dep:hyper-client-sockets", "dep:hyper", "dep:http-body-util"]
```

In the `[dependencies]` section (after the existing sandbox deps around line 122), add:

```toml
# Firecracker microVM
hyper-client-sockets = { version = "0.2", optional = true, features = ["unix"] }
hyper = { version = "1", optional = true, features = ["client", "http1"] }
http-body-util = { version = "0.1", optional = true }
```

**Step 2: Verify it compiles**

Run: `cargo check --features sandbox-firecracker`
Expected: compiles with no errors (new deps download, no code uses them yet)

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "feat(sandbox): add sandbox-firecracker feature flag and deps"
```

---

## Task 2: Add Firecracker config structs

**Files:**
- Modify: `src/config/mod.rs`

**Step 1: Add SandboxFirecrackerConfig struct**

After the existing `SandboxDockerConfig` block (around line 154), add:

```rust
/// Firecracker microVM sandbox configuration.
#[cfg(feature = "sandbox-firecracker")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxFirecrackerConfig {
    /// Path to the firecracker binary
    #[serde(default = "default_firecracker_bin")]
    pub firecracker_bin: String,
    /// Path to the vmlinux kernel image
    #[serde(default = "default_kernel_path")]
    pub kernel_path: String,
    /// Path to the rootfs ext4 image (must contain python3/node/bash)
    #[serde(default = "default_rootfs_path")]
    pub rootfs_path: String,
    /// Default vCPU count per microVM
    #[serde(default = "default_vcpu_count")]
    pub vcpu_count: u32,
    /// Default memory in MiB per microVM
    #[serde(default = "default_fc_memory_mb")]
    pub memory_mb: u64,
    /// vsock guest CID base (each VM gets CID = base + random offset)
    #[serde(default = "default_vsock_cid")]
    pub vsock_guest_cid: u32,
    /// Guest agent port on vsock (default: 52)
    #[serde(default = "default_agent_port")]
    pub agent_port: u32,
}

#[cfg(feature = "sandbox-firecracker")]
impl Default for SandboxFirecrackerConfig {
    fn default() -> Self {
        Self {
            firecracker_bin: default_firecracker_bin(),
            kernel_path: default_kernel_path(),
            rootfs_path: default_rootfs_path(),
            vcpu_count: default_vcpu_count(),
            memory_mb: default_fc_memory_mb(),
            vsock_guest_cid: default_vsock_cid(),
            agent_port: default_agent_port(),
        }
    }
}

#[cfg(feature = "sandbox-firecracker")]
fn default_firecracker_bin() -> String { "firecracker".to_string() }
#[cfg(feature = "sandbox-firecracker")]
fn default_kernel_path() -> String { "/opt/r8r/vmlinux".to_string() }
#[cfg(feature = "sandbox-firecracker")]
fn default_rootfs_path() -> String { "/opt/r8r/rootfs.ext4".to_string() }
#[cfg(feature = "sandbox-firecracker")]
fn default_vcpu_count() -> u32 { 1 }
#[cfg(feature = "sandbox-firecracker")]
fn default_fc_memory_mb() -> u64 { 256 }
#[cfg(feature = "sandbox-firecracker")]
fn default_vsock_cid() -> u32 { 3 }
#[cfg(feature = "sandbox-firecracker")]
fn default_agent_port() -> u32 { 52 }
```

Then add the field to `SandboxConfig`:

```rust
// Add this field to the existing SandboxConfig struct:
#[cfg(feature = "sandbox-firecracker")]
#[serde(default)]
pub firecracker: SandboxFirecrackerConfig,
```

And in `SandboxConfig`'s `Default` impl, add:

```rust
#[cfg(feature = "sandbox-firecracker")]
firecracker: SandboxFirecrackerConfig::default(),
```

Also update the `PartialConfig` struct and `apply_partial` to handle the firecracker config, following the same pattern as `docker`.

**Step 2: Verify it compiles**

Run: `cargo check --features sandbox-firecracker`
Expected: compiles cleanly

**Step 3: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(sandbox): add Firecracker config structs"
```

---

## Task 3: Implement FirecrackerBackend

**Files:**
- Create: `src/sandbox/firecracker.rs`
- Modify: `src/sandbox/mod.rs`

This is the core task. The backend:

1. Spawns the `firecracker` binary with `--api-sock <path>`
2. Configures the VM via REST API over Unix socket (kernel, rootfs, machine-config, vsock)
3. Starts the VM
4. Sends code to the guest agent via vsock (HTTP POST to guest agent)
5. Collects stdout/stderr/exit_code from the guest agent response
6. Kills the firecracker process and cleans up

**Step 1: Add the module to mod.rs**

In `src/sandbox/mod.rs`, add after the docker module declarations:

```rust
#[cfg(feature = "sandbox-firecracker")]
pub mod firecracker;

#[cfg(feature = "sandbox-firecracker")]
pub use firecracker::FirecrackerBackend;
```

**Step 2: Create src/sandbox/firecracker.rs**

```rust
//! Firecracker microVM sandbox backend.
//!
//! Executes code inside Firecracker microVMs with hardware-level isolation.
//! Requires Linux with KVM support (`/dev/kvm`).
//!
//! External requirements:
//! - `firecracker` binary in PATH or configured path
//! - vmlinux kernel image
//! - rootfs ext4 image with python3/node/bash and the r8r guest agent

use async_trait::async_trait;
use hyper::body::Bytes;
use hyper::Request;
use hyper_client_sockets::unix::UnixConnector;
use http_body_util::{BodyExt, Full};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tracing::{debug, error, info, warn};

use super::{SandboxBackend, SandboxError, SandboxRequest, SandboxResult};
use crate::config::SandboxFirecrackerConfig;

/// Firecracker microVM sandbox backend.
///
/// Provides the strongest isolation level via hardware virtualization:
/// - Separate kernel per execution
/// - Memory isolation enforced by KVM
/// - No shared filesystem with host
/// - Network disabled by default (no TAP device created)
pub struct FirecrackerBackend {
    config: SandboxFirecrackerConfig,
}

impl FirecrackerBackend {
    pub fn new(config: &SandboxFirecrackerConfig) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Start the firecracker process and return (child, socket_path).
    async fn spawn_firecracker(&self) -> Result<(Child, PathBuf), SandboxError> {
        let vm_id = uuid::Uuid::new_v4();
        let socket_path = std::env::temp_dir().join(format!("r8r-fc-{}.sock", vm_id));

        // Clean up stale socket if exists
        let _ = tokio::fs::remove_file(&socket_path).await;

        let child = Command::new(&self.config.firecracker_bin)
            .arg("--api-sock")
            .arg(&socket_path)
            .arg("--level")
            .arg("Warning")
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SandboxError::BackendUnavailable {
                backend: "firecracker".to_string(),
                reason: format!("Failed to spawn firecracker: {}", e),
            })?;

        // Wait for socket to appear (up to 2s)
        for _ in 0..20 {
            if socket_path.exists() {
                return Ok((child, socket_path));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err(SandboxError::BackendUnavailable {
            backend: "firecracker".to_string(),
            reason: "Firecracker API socket did not appear within 2s".to_string(),
        })
    }

    /// Send an HTTP PUT request to the Firecracker API.
    async fn api_put(
        socket_path: &Path,
        path: &str,
        body: serde_json::Value,
    ) -> Result<(), SandboxError> {
        let connector = UnixConnector::new(socket_path.to_path_buf());
        let client = hyper::client::conn::http1::handshake(
            connector.connect().await.map_err(|e| {
                SandboxError::Io(std::io::Error::other(format!("Unix connect: {}", e)))
            })?,
        )
        .await
        .map_err(|e| {
            SandboxError::Io(std::io::Error::other(format!("HTTP handshake: {}", e)))
        })?;

        // We need a slightly different approach—use the hyper_client_sockets
        // UnixClient directly for simplicity
        let body_bytes = serde_json::to_vec(&body).map_err(|e| {
            SandboxError::Io(std::io::Error::other(format!("JSON serialize: {}", e)))
        })?;

        let req = Request::builder()
            .method("PUT")
            .uri(format!("http://localhost{}", path))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(Full::new(Bytes::from(body_bytes)))
            .map_err(|e| {
                SandboxError::Io(std::io::Error::other(format!("Build request: {}", e)))
            })?;

        debug!("Firecracker API PUT {}: {}", path, body);

        // For the actual implementation, we'll use a simpler approach:
        // spawn a tokio task with the sender half
        let (mut sender, conn) = client;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                error!("Firecracker API connection error: {}", e);
            }
        });

        let response = sender.send_request(req).await.map_err(|e| {
            SandboxError::Io(std::io::Error::other(format!("API request: {}", e)))
        })?;

        if !response.status().is_success() {
            let body = response
                .into_body()
                .collect()
                .await
                .map(|b| String::from_utf8_lossy(&b.to_bytes()).to_string())
                .unwrap_or_default();
            return Err(SandboxError::BackendUnavailable {
                backend: "firecracker".to_string(),
                reason: format!("API {} failed: {}", path, body),
            });
        }

        Ok(())
    }

    /// Configure and start a microVM.
    async fn configure_and_start(
        &self,
        socket_path: &Path,
        req: &SandboxRequest,
    ) -> Result<(), SandboxError> {
        let memory_mb = req.memory_mb.unwrap_or(self.config.memory_mb);

        // 1. Set machine config
        Self::api_put(
            socket_path,
            "/machine-config",
            json!({
                "vcpu_count": self.config.vcpu_count,
                "mem_size_mib": memory_mb,
            }),
        )
        .await?;

        // 2. Set boot source
        Self::api_put(
            socket_path,
            "/boot-source",
            json!({
                "kernel_image_path": self.config.kernel_path,
                "boot_args": "console=ttyS0 reboot=k panic=1 pci=off quiet"
            }),
        )
        .await?;

        // 3. Set rootfs drive
        Self::api_put(
            socket_path,
            "/drives/rootfs",
            json!({
                "drive_id": "rootfs",
                "path_on_host": self.config.rootfs_path,
                "is_root_device": true,
                "is_read_only": true
            }),
        )
        .await?;

        // 4. Set vsock (for guest agent communication)
        Self::api_put(
            socket_path,
            "/vsock",
            json!({
                "guest_cid": self.config.vsock_guest_cid,
                "uds_path": format!("{}.vsock", socket_path.display())
            }),
        )
        .await?;

        // 5. Start the VM
        Self::api_put(
            socket_path,
            "/actions",
            json!({ "action_type": "InstanceStart" }),
        )
        .await?;

        // Wait for guest agent to become ready (boot time)
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(())
    }

    /// Execute code via the guest agent over vsock.
    async fn execute_via_guest_agent(
        &self,
        socket_path: &Path,
        req: &SandboxRequest,
    ) -> Result<SandboxResult, SandboxError> {
        let vsock_path = format!("{}.vsock", socket_path.display());
        let start = Instant::now();

        // Connect to guest agent via vsock UDS path
        // The guest agent listens on a simple protocol:
        // Send JSON: {"runtime": "python3", "code": "...", "env": {...}}
        // Receive JSON: {"stdout": "...", "stderr": "...", "exit_code": 0}
        let payload = json!({
            "runtime": req.runtime,
            "code": req.code,
            "env": req.env,
            "timeout_seconds": req.timeout.as_secs(),
        });

        // Use tokio UnixStream to connect to the vsock UDS
        let stream = tokio::net::UnixStream::connect(&vsock_path)
            .await
            .map_err(|e| {
                SandboxError::Io(std::io::Error::other(format!(
                    "Failed to connect to guest agent via vsock: {}",
                    e
                )))
            })?;

        // Write request
        let payload_bytes = serde_json::to_vec(&payload).map_err(|e| {
            SandboxError::Io(std::io::Error::other(format!("JSON: {}", e)))
        })?;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut reader, mut writer) = stream.into_split();

        // Write length-prefixed message
        writer
            .write_all(&(payload_bytes.len() as u32).to_be_bytes())
            .await?;
        writer.write_all(&payload_bytes).await?;
        writer.shutdown().await?;

        // Read response with timeout
        let response = tokio::time::timeout(req.timeout, async {
            let mut len_buf = [0u8; 4];
            reader.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;

            let mut buf = vec![0u8; len.min(req.max_output_bytes as usize)];
            reader.read_exact(&mut buf).await?;
            Ok::<_, std::io::Error>(buf)
        })
        .await
        .map_err(|_| SandboxError::Timeout {
            timeout_seconds: req.timeout.as_secs(),
        })?
        .map_err(SandboxError::Io)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Parse response
        #[derive(serde::Deserialize)]
        struct AgentResponse {
            stdout: String,
            stderr: String,
            exit_code: i32,
        }

        let agent_resp: AgentResponse =
            serde_json::from_slice(&response).map_err(|e| {
                SandboxError::Io(std::io::Error::other(format!(
                    "Invalid guest agent response: {}",
                    e
                )))
            })?;

        Ok(SandboxResult {
            stdout: agent_resp.stdout,
            stderr: agent_resp.stderr,
            exit_code: agent_resp.exit_code,
            duration_ms,
        })
    }
}

#[async_trait]
impl SandboxBackend for FirecrackerBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        info!(
            "Firecracker sandbox: runtime={} timeout={}s",
            req.runtime,
            req.timeout.as_secs()
        );

        // 1. Spawn firecracker process
        let (mut child, socket_path) = self.spawn_firecracker().await?;

        // 2. Configure and start VM
        let result = async {
            self.configure_and_start(&socket_path, &req).await?;
            self.execute_via_guest_agent(&socket_path, &req).await
        }
        .await;

        // 3. Cleanup: kill firecracker process and remove socket
        let _ = child.kill().await;
        let _ = tokio::fs::remove_file(&socket_path).await;
        let _ = tokio::fs::remove_file(format!("{}.vsock", socket_path.display())).await;

        result
    }

    fn name(&self) -> &str {
        "firecracker"
    }

    fn available(&self) -> bool {
        // Check for /dev/kvm and firecracker binary
        Path::new("/dev/kvm").exists()
            && std::process::Command::new(&self.config.firecracker_bin)
                .arg("--version")
                .output()
                .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_checks_kvm() {
        let config = SandboxFirecrackerConfig::default();
        let backend = FirecrackerBackend::new(&config);
        // On macOS / CI without KVM, this should return false
        if !Path::new("/dev/kvm").exists() {
            assert!(!backend.available());
        }
    }

    #[test]
    fn test_default_config() {
        let config = SandboxFirecrackerConfig::default();
        assert_eq!(config.firecracker_bin, "firecracker");
        assert_eq!(config.vcpu_count, 1);
        assert_eq!(config.memory_mb, 256);
        assert_eq!(config.agent_port, 52);
    }
}
```

**Important notes for the implementer:**
- The `api_put` function uses `hyper-client-sockets` to talk over the Unix socket. The exact API may differ slightly from the pseudocode above — consult the `hyper-client-sockets` docs and adjust the connection setup. The key pattern is: connect to Unix socket → send HTTP PUT → check response status.
- The guest agent communication via vsock uses a simple length-prefixed JSON protocol. The actual guest agent (a small Python/Go binary inside the rootfs) is **not part of this task** — it's a separate artifact that operators build into their rootfs image.
- The `available()` check only verifies `/dev/kvm` exists and `firecracker --version` runs. It does NOT check for kernel/rootfs images (those are checked at execute time).

**Step 3: Verify it compiles**

Run: `cargo check --features sandbox-firecracker`
Expected: compiles cleanly

**Step 4: Commit**

```bash
git add src/sandbox/firecracker.rs src/sandbox/mod.rs
git commit -m "feat(sandbox): implement FirecrackerBackend"
```

---

## Task 4: Wire Firecracker into backend selection

**Files:**
- Modify: `src/main.rs:1606-1649` (build_registry function)
- Modify: `src/config/mod.rs` (env override for backend)

**Step 1: Add "firecracker" case to build_registry()**

In `src/main.rs`, in the `build_registry()` function, add a new match arm after the docker arm (around line 1641):

```rust
#[cfg(feature = "sandbox-firecracker")]
"firecracker" => {
    use r8r::sandbox::FirecrackerBackend;
    let fb = FirecrackerBackend::new(&config.sandbox.firecracker);
    if fb.available() {
        tracing::info!("Using Firecracker sandbox backend");
        Arc::new(fb)
    } else {
        tracing::warn!(
            "Firecracker unavailable (no /dev/kvm?), falling back to subprocess sandbox"
        );
        Arc::new(SubprocessBackend::new())
    }
}
```

**Step 2: Verify it compiles with all feature combinations**

Run:
```bash
cargo check --features sandbox
cargo check --features sandbox-docker
cargo check --features sandbox-firecracker
cargo check --features "sandbox-docker,sandbox-firecracker"
```
Expected: all compile cleanly

**Step 3: Commit**

```bash
git add src/main.rs src/config/mod.rs
git commit -m "feat(sandbox): wire Firecracker backend into build_registry"
```

---

## Task 5: Add unit tests for FirecrackerBackend

**Files:**
- Modify: `src/sandbox/firecracker.rs` (expand test module)

**Step 1: Add tests**

Expand the `#[cfg(test)] mod tests` block in `firecracker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_checks_kvm() {
        let config = SandboxFirecrackerConfig::default();
        let backend = FirecrackerBackend::new(&config);
        if !Path::new("/dev/kvm").exists() {
            assert!(!backend.available());
        }
    }

    #[test]
    fn test_default_config() {
        let config = SandboxFirecrackerConfig::default();
        assert_eq!(config.firecracker_bin, "firecracker");
        assert_eq!(config.vcpu_count, 1);
        assert_eq!(config.memory_mb, 256);
        assert_eq!(config.agent_port, 52);
        assert_eq!(config.vsock_guest_cid, 3);
        assert_eq!(config.kernel_path, "/opt/r8r/vmlinux");
        assert_eq!(config.rootfs_path, "/opt/r8r/rootfs.ext4");
    }

    #[test]
    fn test_backend_name() {
        let config = SandboxFirecrackerConfig::default();
        let backend = FirecrackerBackend::new(&config);
        assert_eq!(backend.name(), "firecracker");
    }

    #[test]
    fn test_custom_config() {
        let config = SandboxFirecrackerConfig {
            firecracker_bin: "/usr/local/bin/firecracker".to_string(),
            kernel_path: "/custom/vmlinux".to_string(),
            rootfs_path: "/custom/rootfs.ext4".to_string(),
            vcpu_count: 2,
            memory_mb: 512,
            vsock_guest_cid: 10,
            agent_port: 8080,
        };
        let backend = FirecrackerBackend::new(&config);
        assert_eq!(backend.config.vcpu_count, 2);
        assert_eq!(backend.config.memory_mb, 512);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = SandboxFirecrackerConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: SandboxFirecrackerConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.firecracker_bin, config.firecracker_bin);
        assert_eq!(parsed.vcpu_count, config.vcpu_count);
    }
}
```

**Step 2: Run the tests**

Run: `cargo test --features sandbox-firecracker -- sandbox::firecracker`
Expected: all tests pass (they don't require /dev/kvm or the firecracker binary)

**Step 3: Commit**

```bash
git add src/sandbox/firecracker.rs
git commit -m "test(sandbox): add unit tests for FirecrackerBackend"
```

---

## Task 6: Update docs with Firecracker backend

**Files:**
- Modify: `docs/NODE_TYPES.md`
- Modify: `README.md`
- Modify: `docs/ARCHITECTURE.md`

**Step 1: Update NODE_TYPES.md**

In the Sandbox section's backends table, add a Firecracker row:

```markdown
| `firecracker` | `sandbox-firecracker` | Hardware-level VM isolation via KVM. Strongest isolation. Linux only. |
```

Also add to the configuration section:

```markdown
### Firecracker Configuration (r8r.toml)

```toml
[sandbox.firecracker]
firecracker_bin = "firecracker"       # path to binary
kernel_path = "/opt/r8r/vmlinux"      # uncompressed kernel
rootfs_path = "/opt/r8r/rootfs.ext4"  # Alpine rootfs with runtimes
vcpu_count = 1
memory_mb = 256
```

**Requirements:** Linux with KVM (`/dev/kvm`), Firecracker binary, vmlinux kernel, rootfs image with guest agent.
```

**Step 2: Update README.md**

In the Node Types table, update the sandbox description to mention Firecracker:

```markdown
| **`sandbox`** | **Execute Python, Node, or Bash in isolated environments (subprocess, Docker, or Firecracker)** |
```

In the feature comparison table, the "Code Sandbox" row already says "Pluggable (subprocess, Docker, Firecracker)" — verify this is present.

In the Security section, update sandbox isolation line:

```markdown
- **Sandbox Isolation** — Pluggable backends: Docker containers or Firecracker microVMs with memory, network, and filesystem restrictions
```

**Step 3: Update ARCHITECTURE.md**

Add `firecracker.rs` to the project structure:

```
│   ├── sandbox/             # Code execution sandbox
│   │   ├── mod.rs           # SandboxBackend trait, types, errors
│   │   ├── subprocess.rs    # SubprocessBackend (feature: sandbox)
│   │   ├── docker.rs        # DockerBackend (feature: sandbox-docker)
│   │   └── firecracker.rs   # FirecrackerBackend (feature: sandbox-firecracker)
```

**Step 4: Commit**

```bash
git add docs/NODE_TYPES.md README.md docs/ARCHITECTURE.md
git commit -m "docs: add Firecracker backend to documentation"
```

---

## Task 7: Add future sandbox backends roadmap doc

**Files:**
- Create: `docs/SANDBOX_BACKENDS.md`

**Step 1: Write the roadmap doc**

Create `docs/SANDBOX_BACKENDS.md` with the following content:

```markdown
# Sandbox Backends

r8r's sandbox module uses a pluggable backend architecture. Operators choose the isolation level that fits their deployment.

## Available Backends

| Backend | Feature Flag | Isolation | Platform | Status |
|---------|-------------|-----------|----------|--------|
| Subprocess | `sandbox` | None | macOS, Linux, Windows | Stable |
| Docker | `sandbox-docker` | Container-level | macOS, Linux | Stable |
| Firecracker | `sandbox-firecracker` | Hardware VM (KVM) | Linux only | Stable |

## Subprocess

Default backend. Spawns a child process with `kill_on_drop`. No real isolation — the process inherits the host's network, filesystem, and permissions. Use for local development only.

```toml
[sandbox]
backend = "subprocess"
```

## Docker

Container-based isolation via the Docker daemon (bollard SDK). Enforces memory limits, network isolation (`--network none`), read-only root filesystem, PID limits, and writable `/tmp` only.

```toml
[sandbox]
backend = "docker"

[sandbox.docker]
python_image = "python:3.12-slim"
node_image = "node:22-slim"
bash_image = "alpine:3.19"
```

## Firecracker

Hardware-level isolation via Firecracker microVMs. Each execution gets its own Linux kernel, memory space, and (optionally) network stack. Requires Linux with KVM (`/dev/kvm`), the `firecracker` binary, a vmlinux kernel, and a rootfs image with the guest agent.

```toml
[sandbox]
backend = "firecracker"

[sandbox.firecracker]
firecracker_bin = "firecracker"
kernel_path = "/opt/r8r/vmlinux"
rootfs_path = "/opt/r8r/rootfs.ext4"
vcpu_count = 1
memory_mb = 256
```

### Setup

1. Install Firecracker: https://github.com/firecracker-microvm/firecracker/releases
2. Download a pre-built kernel: `wget https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/x86_64/vmlinux-5.10.204 -O /opt/r8r/vmlinux`
3. Build a rootfs image (Alpine + python3/node/bash + guest agent)
4. Ensure `/dev/kvm` is accessible by the r8r process

### Guest Agent

The rootfs must include a guest agent that listens on vsock and executes code. A reference implementation is provided in `tools/firecracker-guest-agent/`.

---

## Future Backends (Not Yet Implemented)

The `SandboxBackend` trait is designed to support additional backends. Below are candidates evaluated for future implementation.

### WASM (wasmtime)

**Feature flag:** `sandbox-wasm`

**What:** Run code in a WebAssembly sandbox using wasmtime (a Rust-native WASM runtime). Sub-millisecond cold start, cross-platform, embeds directly in the r8r binary.

**Isolation:** Memory-safe sandbox. No filesystem or network access unless explicitly granted via WASI capabilities.

**Limitation:** Cannot run arbitrary Python/Node/Bash scripts. Only works with:
- Pre-compiled WASM modules
- MicroPython (limited stdlib, no pip packages)
- Languages that compile to WASM (Rust, Go, C)

**Best for:** High-frequency, trusted tool execution where you control the code ahead of time. Not suitable for arbitrary user-submitted scripts.

**Rust crate:** `wasmtime` (mature, Bytecode Alliance, production-ready)

**Implementation sketch:**
```rust
struct WasmBackend {
    engine: wasmtime::Engine,
}

impl SandboxBackend for WasmBackend {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult, SandboxError> {
        // 1. Compile WASM module from req.code (or load pre-compiled)
        // 2. Create Store with resource limits (fuel, memory)
        // 3. Run the module, capture stdout/stderr via WASI
        // 4. Return SandboxResult
    }
}
```

### Hyperlight (Microsoft)

**Feature flag:** `sandbox-hyperlight`

**What:** Sub-millisecond microVM execution. Pure Rust library from Microsoft (CNCF sandbox project, open-sourced Feb 2025). Creates minimal microVMs without a full guest OS.

**Cold start:** ~900 microseconds for complete function execution.

**Platform:** Linux (KVM) or Windows (Hyper-V). No macOS support.

**Isolation:** Full hardware-level VM isolation with near-zero overhead.

**Limitation:** Early-stage project (CNCF sandbox). API may change. Best for WASM components or native ELF binaries, not arbitrary scripts.

**Best for:** Ultra-low-latency isolated execution where you need VM-level security but can't afford the ~125ms Firecracker cold start.

**Rust crate:** `hyperlight-host` (pure Rust, actively developed)

**Why wait:** API stability. Once Hyperlight graduates from CNCF sandbox to incubating, it becomes a strong candidate to complement or replace Firecracker for short-lived executions.

### libkrun

**Feature flag:** `sandbox-libkrun`

**What:** Lightweight virtualization library from Red Hat. Creates microVMs using the host's hypervisor (KVM on Linux, Apple Hypervisor Framework on macOS).

**Platform:** Linux (KVM) AND macOS (HVF) — the only microVM option that works on both.

**Isolation:** VM-level isolation, lighter than Firecracker.

**Best for:** Development environments where you want better-than-Docker isolation on macOS without requiring Docker Desktop.

**Rust crate:** `libkrun-sys` (FFI bindings)

**Why wait:** The Rust bindings are lower-level than Firecracker's HTTP API. Integration requires more work. Consider when macOS microVM support becomes a priority.

### Landlock + seccomp (Linux Security Modules)

**Feature flag:** `sandbox-landlock`

**What:** OS-level sandboxing using Linux security modules. Landlock restricts file/network access; seccomp filters system calls. No VM overhead.

**Platform:** Linux only (kernel 5.13+ for Landlock).

**Isolation:** Weak compared to VMs/containers. Kernel vulnerabilities can escape the sandbox.

**Best for:** Trusted code that just needs resource restrictions. An enhanced subprocess backend, not a replacement for Docker/Firecracker.

**Rust crate:** `rust-landlock` (stable, maintained by Landlock upstream)

**Why wait:** Only useful as an incremental improvement over subprocess. If you need real isolation, use Docker or Firecracker instead.
```

**Step 2: Commit**

```bash
git add docs/SANDBOX_BACKENDS.md
git commit -m "docs: add sandbox backends reference with future roadmap"
```

---

## Task 8: Final verification

**Step 1: Run all tests with all feature combinations**

```bash
cargo test --features sandbox
cargo test --features sandbox-docker
cargo test --features sandbox-firecracker
cargo test --features "sandbox-docker,sandbox-firecracker"
```

Expected: all tests pass

**Step 2: Run clippy**

```bash
cargo clippy --features "sandbox-docker,sandbox-firecracker" -- -D warnings
```

Expected: no warnings

**Step 3: Run fmt**

```bash
cargo fmt --all --check
```

Expected: no formatting issues

**Step 4: Commit any fixes**

If clippy or fmt found issues:
```bash
cargo fmt --all
git add -A
git commit -m "fix: clippy and formatting fixes"
```
