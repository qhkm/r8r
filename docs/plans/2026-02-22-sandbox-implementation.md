# Sandbox Module Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a pluggable `sandbox` node type to r8r that executes arbitrary code (Python, Node, Bash) with configurable isolation backends.

**Architecture:** A `SandboxBackend` trait (mirroring `EventBackend`) with `SubprocessBackend` (default) and `DockerBackend` (via bollard). The `SandboxNode` implements the `Node` trait, holds `Arc<dyn SandboxBackend>`, and is registered in `main.rs` after backend selection. Entire module behind `sandbox` feature flag.

**Tech Stack:** Rust, tokio (async subprocess), bollard (Docker SDK), tempfile (temp dirs), serde (config)

**Design doc:** `docs/plans/2026-02-22-sandbox-module-design.md`

---

### Task 1: Sandbox trait, types, and errors

**Files:**
- Create: `src/sandbox/mod.rs`

**Step 1: Write the code**

Create `src/sandbox/mod.rs` with the `SandboxBackend` trait, request/result/error types, and mock backend for testing.

```rust
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

pub use subprocess::SubprocessBackend;
#[cfg(feature = "sandbox-docker")]
pub use docker::DockerBackend;

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
        let err = SandboxError::Timeout { timeout_seconds: 30 };
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

        let result = backend
            .execute(SandboxRequest::default())
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, r#"{"result": 42}"#);
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo check --features sandbox`
Expected: FAIL — feature `sandbox` doesn't exist yet, and `subprocess` module missing. That's expected; we add those in the next tasks.

**Step 3: Commit**

```bash
git add src/sandbox/mod.rs
git commit -m "feat(sandbox): add SandboxBackend trait and types"
```

---

### Task 2: Feature flags and Cargo.toml

**Files:**
- Modify: `Cargo.toml` (lines 5-8 for features, dependencies section)
- Modify: `src/lib.rs` (line 55, add sandbox module)

**Step 1: Add feature flags to `Cargo.toml`**

After the existing `redis` feature (line ~8), add:

```toml
[features]
default = []
redis = ["dep:redis"]
sandbox = ["dep:tempfile"]
sandbox-docker = ["sandbox", "dep:bollard"]
```

Add to `[dependencies]`:

```toml
bollard = { version = "0.18", optional = true }
```

Note: `tempfile` is already in `[dev-dependencies]`. Move it to `[dependencies]` as optional:

```toml
tempfile = { version = "3", optional = true }
```

And remove `tempfile = "3"` from `[dev-dependencies]`, replacing with:

```toml
[dev-dependencies]
tempfile = "3"
tokio-test = "0.4"
```

(The dev-dep ensures tests still get tempfile even without the sandbox feature.)

**Step 2: Add sandbox module to `src/lib.rs`**

After line 55 (`pub mod workflow;`), add:

```rust
#[cfg(feature = "sandbox")]
pub mod sandbox;
```

**Step 3: Verify it compiles**

Run: `cargo check --features sandbox`
Expected: FAIL — `src/sandbox/subprocess.rs` not found yet. Next task.

**Step 4: Commit**

```bash
git add Cargo.toml src/lib.rs
git commit -m "feat(sandbox): add sandbox and sandbox-docker feature flags"
```

---

### Task 3: SubprocessBackend

**Files:**
- Create: `src/sandbox/subprocess.rs`

**Step 1: Write the failing test (at bottom of file)**

The test should verify that executing `echo hello` via bash returns "hello\n" on stdout.

**Step 2: Write the implementation**

```rust
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
}
```

**Step 3: Run tests**

Run: `cargo test --features sandbox sandbox::subprocess -- --nocapture`
Expected: All tests pass (bash and python3 must be available on dev machine).

**Step 4: Commit**

```bash
git add src/sandbox/subprocess.rs
git commit -m "feat(sandbox): add SubprocessBackend"
```

---

### Task 4: SandboxNode (workflow node)

**Files:**
- Create: `src/nodes/sandbox.rs`
- Modify: `src/nodes/mod.rs` (add conditional module + export)

**Step 1: Write the node implementation**

Create `src/nodes/sandbox.rs`:

```rust
//! Sandbox node — execute code in a sandboxed environment.
//!
//! Runs Python, Node, or Bash code via a pluggable `SandboxBackend`.
//! The backend is selected globally at startup (subprocess or Docker).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

use super::template::{render_template, RenderMode};
use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};
use crate::sandbox::{SandboxBackend, SandboxError, SandboxRequest};

/// Sandbox node — execute code via a pluggable backend.
pub struct SandboxNode {
    backend: Arc<dyn SandboxBackend>,
}

impl SandboxNode {
    pub fn new(backend: Arc<dyn SandboxBackend>) -> Self {
        Self { backend }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SandboxNodeConfig {
    /// Runtime: "python3", "node", "bash"
    runtime: String,

    /// Code to execute (supports template variables)
    code: String,

    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,

    /// Memory limit in MB (optional, enforced by Docker backend)
    #[serde(default)]
    memory_mb: Option<u64>,

    /// Allow network access (default: false)
    #[serde(default)]
    network: bool,

    /// Environment variables to pass into the sandbox
    #[serde(default)]
    env: HashMap<String, String>,
}

fn default_timeout() -> u64 {
    30
}

// ---------------------------------------------------------------------------
// Node implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Node for SandboxNode {
    fn node_type(&self) -> &str {
        "sandbox"
    }

    fn description(&self) -> &str {
        "Execute code in a sandboxed environment (Python, Node, Bash)"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SandboxNodeConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid sandbox config: {}", e)))?;

        // Render code template with context
        let code = render_template(&config.code, ctx, RenderMode::Raw);

        // Render env var values
        let mut env = HashMap::new();
        for (k, v) in &config.env {
            env.insert(k.clone(), render_template(v, ctx, RenderMode::Raw));
        }

        info!(
            "Sandbox [{}] runtime={} timeout={}s code={}...",
            self.backend.name(),
            config.runtime,
            config.timeout_seconds,
            &code.chars().take(80).collect::<String>()
        );

        let request = SandboxRequest {
            runtime: config.runtime.clone(),
            code,
            env,
            timeout: Duration::from_secs(config.timeout_seconds),
            memory_mb: config.memory_mb,
            network: config.network,
            max_output_bytes: 1_048_576, // 1MB default
        };

        let result = self.backend.execute(request).await.map_err(|e| match e {
            SandboxError::Timeout { timeout_seconds } => Error::Node(format!(
                "Sandbox execution timed out after {}s",
                timeout_seconds
            )),
            SandboxError::RuntimeNotFound { runtime } => {
                Error::Node(format!("Runtime '{}' not found", runtime))
            }
            SandboxError::BackendUnavailable { backend, reason } => {
                Error::Node(format!("Sandbox backend '{}' unavailable: {}", backend, reason))
            }
            SandboxError::ResourceLimit { resource, limit } => {
                Error::Node(format!("Resource limit exceeded: {} ({})", resource, limit))
            }
            SandboxError::Io(e) => Error::Node(format!("Sandbox IO error: {}", e)),
        })?;

        debug!(
            "Sandbox completed: exit_code={} duration={}ms stdout_len={} stderr_len={}",
            result.exit_code,
            result.duration_ms,
            result.stdout.len(),
            result.stderr.len(),
        );

        // Try to parse stdout as JSON for structured output
        let parsed_output = serde_json::from_str::<Value>(result.stdout.trim()).ok();

        let data = json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
            "output": parsed_output,
        });

        let metadata = json!({
            "backend": self.backend.name(),
            "runtime": config.runtime,
            "duration_ms": result.duration_ms,
            "network": config.network,
        });

        Ok(NodeResult::with_metadata(data, metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::mock::MockSandboxBackend;

    fn make_ctx() -> NodeContext {
        NodeContext::new("exec-1", "test-workflow")
    }

    #[tokio::test]
    async fn test_sandbox_node_basic() {
        let backend = Arc::new(MockSandboxBackend::success(r#"{"total": 42}"#));
        let node = SandboxNode::new(backend);

        let config = json!({
            "runtime": "python3",
            "code": "print('hello')",
        });

        let result = node.execute(&config, &make_ctx()).await.unwrap();

        assert_eq!(result.data["exit_code"], 0);
        assert_eq!(result.data["output"]["total"], 42);
        assert_eq!(result.metadata["backend"], "mock");
        assert_eq!(result.metadata["runtime"], "python3");
    }

    #[tokio::test]
    async fn test_sandbox_node_non_json_stdout() {
        let backend = Arc::new(MockSandboxBackend::success("just plain text"));
        let node = SandboxNode::new(backend);

        let config = json!({
            "runtime": "bash",
            "code": "echo hello",
        });

        let result = node.execute(&config, &make_ctx()).await.unwrap();

        assert_eq!(result.data["stdout"], "just plain text");
        assert!(result.data["output"].is_null());
    }

    #[tokio::test]
    async fn test_sandbox_node_with_env() {
        let backend = Arc::new(MockSandboxBackend::success("ok"));
        let node = SandboxNode::new(backend);

        let config = json!({
            "runtime": "bash",
            "code": "echo $MY_VAR",
            "env": {
                "MY_VAR": "test_value"
            }
        });

        let result = node.execute(&config, &make_ctx()).await.unwrap();
        assert_eq!(result.data["exit_code"], 0);
    }

    #[tokio::test]
    async fn test_sandbox_node_template_rendering() {
        let backend = Arc::new(MockSandboxBackend::success("ok"));
        let node = SandboxNode::new(backend);

        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({"name": "world"}));

        let config = json!({
            "runtime": "python3",
            "code": "print('hello {{ input.name }}')",
        });

        // The node should render the template — mock doesn't care about actual code
        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["exit_code"], 0);
    }

    #[tokio::test]
    async fn test_sandbox_node_invalid_config() {
        let backend = Arc::new(MockSandboxBackend::success("ok"));
        let node = SandboxNode::new(backend);

        let config = json!({
            "not_a_valid_field": true,
        });

        let result = node.execute(&config, &make_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_sandbox_node_custom_timeout() {
        let backend = Arc::new(MockSandboxBackend::success("ok"));
        let node = SandboxNode::new(backend);

        let config = json!({
            "runtime": "python3",
            "code": "print('ok')",
            "timeout_seconds": 60,
            "memory_mb": 512,
            "network": true,
        });

        let result = node.execute(&config, &make_ctx()).await.unwrap();
        assert_eq!(result.metadata["network"], true);
    }
}
```

**Step 2: Add to `src/nodes/mod.rs`**

After line 33 (`mod wait;`), add:

```rust
#[cfg(feature = "sandbox")]
mod sandbox;
```

After line 60 (`pub use wait::WaitNode;`), add:

```rust
#[cfg(feature = "sandbox")]
pub use sandbox::SandboxNode;
```

**Step 3: Run tests**

Run: `cargo test --features sandbox nodes::sandbox -- --nocapture`
Expected: All tests pass.

**Step 4: Also run all existing tests to verify no breakage**

Run: `cargo test`
Expected: All existing tests still pass.

**Step 5: Commit**

```bash
git add src/nodes/sandbox.rs src/nodes/mod.rs
git commit -m "feat(sandbox): add SandboxNode workflow node"
```

---

### Task 5: Config integration

**Files:**
- Modify: `src/config/mod.rs`

**Step 1: Add sandbox config structs**

After `AgentConfig` (line 83), add:

```rust
/// Sandbox execution configuration.
#[cfg(feature = "sandbox")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Backend to use: "subprocess" or "docker"
    #[serde(default = "default_sandbox_backend")]
    pub backend: String,

    /// Docker-specific settings
    #[serde(default)]
    pub docker: SandboxDockerConfig,

    /// Security constraints
    #[serde(default)]
    pub security: SandboxSecurityConfig,
}

#[cfg(feature = "sandbox")]
impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: default_sandbox_backend(),
            docker: SandboxDockerConfig::default(),
            security: SandboxSecurityConfig::default(),
        }
    }
}

#[cfg(feature = "sandbox")]
fn default_sandbox_backend() -> String {
    std::env::var("R8R_SANDBOX_BACKEND").unwrap_or_else(|_| "subprocess".to_string())
}

/// Docker sandbox configuration.
#[cfg(feature = "sandbox")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxDockerConfig {
    #[serde(default = "default_python_image")]
    pub python_image: String,
    #[serde(default = "default_node_image")]
    pub node_image: String,
    #[serde(default = "default_bash_image")]
    pub bash_image: String,
}

#[cfg(feature = "sandbox")]
impl Default for SandboxDockerConfig {
    fn default() -> Self {
        Self {
            python_image: default_python_image(),
            node_image: default_node_image(),
            bash_image: default_bash_image(),
        }
    }
}

#[cfg(feature = "sandbox")]
fn default_python_image() -> String {
    "python:3.12-slim".to_string()
}

#[cfg(feature = "sandbox")]
fn default_node_image() -> String {
    "node:22-slim".to_string()
}

#[cfg(feature = "sandbox")]
fn default_bash_image() -> String {
    "alpine:3.19".to_string()
}

/// Sandbox security constraints.
#[cfg(feature = "sandbox")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSecurityConfig {
    /// Global kill switch for network access
    #[serde(default)]
    pub allow_network: bool,
    /// Maximum memory in MB any sandbox can use
    #[serde(default = "default_max_memory")]
    pub max_memory_mb: u64,
    /// Maximum execution time in seconds
    #[serde(default = "default_max_timeout")]
    pub max_timeout_seconds: u64,
    /// Maximum output bytes (stdout + stderr)
    #[serde(default = "default_max_output")]
    pub max_output_bytes: u64,
    /// Allowed runtimes
    #[serde(default = "default_allowed_runtimes")]
    pub allowed_runtimes: Vec<String>,
}

#[cfg(feature = "sandbox")]
impl Default for SandboxSecurityConfig {
    fn default() -> Self {
        Self {
            allow_network: false,
            max_memory_mb: default_max_memory(),
            max_timeout_seconds: default_max_timeout(),
            max_output_bytes: default_max_output(),
            allowed_runtimes: default_allowed_runtimes(),
        }
    }
}

#[cfg(feature = "sandbox")]
fn default_max_memory() -> u64 { 512 }
#[cfg(feature = "sandbox")]
fn default_max_timeout() -> u64 { 300 }
#[cfg(feature = "sandbox")]
fn default_max_output() -> u64 { 1_048_576 }
#[cfg(feature = "sandbox")]
fn default_allowed_runtimes() -> Vec<String> {
    vec!["python3".to_string(), "node".to_string(), "bash".to_string()]
}
```

**Step 2: Add sandbox field to `Config` struct (line 13)**

Add after line 24 (`pub agent: AgentConfig,`):

```rust
    /// Sandbox execution configuration
    #[cfg(feature = "sandbox")]
    #[serde(default)]
    pub sandbox: SandboxConfig,
```

**Step 3: Add to `PartialConfig` (line 183)**

Add after `agent: Option<AgentConfig>,`:

```rust
    #[cfg(feature = "sandbox")]
    sandbox: Option<SandboxConfig>,
```

**Step 4: Add to `apply_partial` (line 169)**

Add after the agent block:

```rust
        #[cfg(feature = "sandbox")]
        if let Some(sandbox) = partial.sandbox {
            self.sandbox = sandbox;
        }
```

**Step 5: Add env overrides to `apply_env_overrides` (line 132)**

Add at the end of the method:

```rust
        #[cfg(feature = "sandbox")]
        {
            if let Ok(backend) = std::env::var("R8R_SANDBOX_BACKEND") {
                self.sandbox.backend = backend;
            }
            if let Ok(timeout) = std::env::var("R8R_SANDBOX_MAX_TIMEOUT_SECONDS") {
                if let Ok(parsed) = timeout.parse::<u64>() {
                    self.sandbox.security.max_timeout_seconds = parsed;
                }
            }
        }
```

**Step 6: Run tests**

Run: `cargo test --features sandbox`
Expected: All tests pass.

**Step 7: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(sandbox): add sandbox config with security constraints"
```

---

### Task 6: Wire sandbox into main.rs

**Files:**
- Modify: `src/main.rs`

**Step 1: Add a helper function to build a registry with sandbox**

Add a new function near the bottom of `main.rs`, before `get_storage()` (around line 1609):

```rust
/// Build a NodeRegistry, optionally with sandbox backend if feature is enabled.
fn build_registry() -> r8r::nodes::NodeRegistry {
    let mut registry = r8r::nodes::NodeRegistry::new();

    #[cfg(feature = "sandbox")]
    {
        use r8r::config::Config;
        use r8r::sandbox::SubprocessBackend;
        use std::sync::Arc;

        let config = Config::load();
        let sandbox_backend: Arc<dyn r8r::sandbox::SandboxBackend> = match config.sandbox.backend.as_str() {
            #[cfg(feature = "sandbox-docker")]
            "docker" => {
                match r8r::sandbox::DockerBackend::new(&config.sandbox.docker) {
                    Ok(db) if db.available() => {
                        tracing::info!("Using Docker sandbox backend");
                        Arc::new(db)
                    }
                    Ok(_) => {
                        tracing::warn!("Docker unavailable, falling back to subprocess sandbox");
                        Arc::new(SubprocessBackend::new())
                    }
                    Err(e) => {
                        tracing::warn!("Docker sandbox init failed: {}. Falling back to subprocess.", e);
                        Arc::new(SubprocessBackend::new())
                    }
                }
            }
            _ => {
                Arc::new(SubprocessBackend::new())
            }
        };

        registry.register(Arc::new(r8r::nodes::SandboxNode::new(sandbox_backend)));
    }

    registry
}
```

**Step 2: Replace `NodeRegistry::new()` / `NodeRegistry::default()` calls**

Replace these four locations:

1. `cmd_server` line 1039: `let registry = Arc::new(NodeRegistry::default());`
   → `let registry = Arc::new(build_registry());`

2. `cmd_workflows_run` line 532: `let registry = NodeRegistry::new();`
   → `let registry = build_registry();`

3. `cmd_workflows_replay` line 673: `let registry = NodeRegistry::new();`
   → `let registry = build_registry();`

4. `cmd_workflows_resume` line 699: `let registry = NodeRegistry::new();`
   → `let registry = build_registry();`

**Step 3: Verify it compiles**

Run: `cargo check --features sandbox`
Expected: Compiles successfully.

Run: `cargo check` (without sandbox feature)
Expected: Also compiles — the `#[cfg]` guards ensure no breakage.

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(sandbox): wire sandbox backend into main.rs registry"
```

---

### Task 7: Integration test + example workflow

**Files:**
- Create: `examples/sandbox-python.yaml`
- Create: `tests/sandbox_integration.rs`

**Step 1: Create example workflow**

```yaml
name: sandbox-python-example
description: Execute Python code in a sandboxed environment
version: 1

triggers:
  - type: manual

nodes:
  - id: compute
    type: sandbox
    config:
      runtime: python3
      timeout_seconds: 10
      code: |
        import json
        numbers = [1, 2, 3, 4, 5]
        result = {
            "sum": sum(numbers),
            "count": len(numbers),
            "average": sum(numbers) / len(numbers)
        }
        print(json.dumps(result))

  - id: show-result
    type: debug
    config:
      message: "Computation result: {{ nodes.compute.output.output }}"
    depends_on: [compute]
```

**Step 2: Create integration test**

Create `tests/sandbox_integration.rs`:

```rust
//! Integration tests for sandbox node.
//!
//! These tests require real runtimes (python3, bash) to be available.

#![cfg(feature = "sandbox")]

use r8r::nodes::{Node, NodeContext, SandboxNode};
use r8r::sandbox::{SubprocessBackend, SandboxBackend};
use serde_json::json;
use std::sync::Arc;

fn make_sandbox_node() -> SandboxNode {
    let backend: Arc<dyn SandboxBackend> = Arc::new(SubprocessBackend);
    SandboxNode::new(backend)
}

#[tokio::test]
async fn test_sandbox_python_json_output() {
    let node = make_sandbox_node();
    let ctx = NodeContext::new("test-exec", "test-wf");

    let config = json!({
        "runtime": "python3",
        "code": "import json; print(json.dumps({'sum': 15, 'count': 5}))",
    });

    let result = node.execute(&config, &ctx).await.unwrap();

    assert_eq!(result.data["exit_code"], 0);
    assert_eq!(result.data["output"]["sum"], 15);
    assert_eq!(result.data["output"]["count"], 5);
    assert_eq!(result.metadata["backend"], "subprocess");
}

#[tokio::test]
async fn test_sandbox_bash_with_env() {
    let node = make_sandbox_node();
    let ctx = NodeContext::new("test-exec", "test-wf");

    let config = json!({
        "runtime": "bash",
        "code": "echo \"Hello $NAME\"",
        "env": { "NAME": "r8r" },
    });

    let result = node.execute(&config, &ctx).await.unwrap();

    assert_eq!(result.data["exit_code"], 0);
    assert_eq!(result.data["stdout"].as_str().unwrap().trim(), "Hello r8r");
}

#[tokio::test]
async fn test_sandbox_timeout_kills_process() {
    let node = make_sandbox_node();
    let ctx = NodeContext::new("test-exec", "test-wf");

    let config = json!({
        "runtime": "bash",
        "code": "sleep 60",
        "timeout_seconds": 1,
    });

    let result = node.execute(&config, &ctx).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("timed out"));
}

#[tokio::test]
async fn test_sandbox_nonzero_exit() {
    let node = make_sandbox_node();
    let ctx = NodeContext::new("test-exec", "test-wf");

    let config = json!({
        "runtime": "bash",
        "code": "exit 42",
    });

    let result = node.execute(&config, &ctx).await.unwrap();
    assert_eq!(result.data["exit_code"], 42);
}

#[tokio::test]
async fn test_sandbox_template_rendering() {
    let node = make_sandbox_node();
    let ctx = NodeContext::new("test-exec", "test-wf")
        .with_input(json!({"value": 100}));

    let config = json!({
        "runtime": "python3",
        "code": "import json; val = {{ input.value }}; print(json.dumps({'doubled': val * 2}))",
    });

    let result = node.execute(&config, &ctx).await.unwrap();

    assert_eq!(result.data["exit_code"], 0);
    assert_eq!(result.data["output"]["doubled"], 200);
}
```

**Step 3: Run integration tests**

Run: `cargo test --features sandbox --test sandbox_integration -- --nocapture`
Expected: All tests pass.

**Step 4: Run full test suite**

Run: `cargo test --features sandbox`
Expected: All tests pass (existing + new).

Run: `cargo test` (without sandbox feature)
Expected: All existing tests still pass.

**Step 5: Commit**

```bash
git add examples/sandbox-python.yaml tests/sandbox_integration.rs
git commit -m "feat(sandbox): add integration tests and example workflow"
```

---

### Task 8: DockerBackend (feature: sandbox-docker)

**Files:**
- Create: `src/sandbox/docker.rs`

**Step 1: Write the Docker backend**

This task uses the `bollard` crate to communicate with the Docker daemon. The implementation creates a container, copies code into it, runs it, collects output, and removes the container.

```rust
//! Docker sandbox backend.
//!
//! Executes code inside Docker containers with resource limits and
//! network isolation. Requires Docker daemon to be running.

use async_trait::async_trait;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, LogOutput, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::Docker;
use futures_util::StreamExt;
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
        let mut host_config = bollard::models::HostConfig::default();

        if let Some(mem_mb) = req.memory_mb {
            host_config.memory = Some((mem_mb * 1024 * 1024) as i64);
        }

        if !req.network {
            host_config.network_mode = Some("none".to_string());
        }

        // Prevent fork bombs
        host_config.pids_limit = Some(256);

        // Read-only root filesystem with writable /tmp
        host_config.read_only_rootfs = Some(true);
        host_config.tmpfs = Some(
            std::collections::HashMap::from([("/tmp".to_string(), "rw,noexec,nosuid,size=64m".to_string())]),
        );

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

        // Wait for container with timeout
        let wait_result = tokio::time::timeout(req.timeout, async {
            let mut stream = self
                .docker
                .wait_container(&container_name, None::<WaitContainerOptions<String>>);
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
                // Timeout — kill container
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

        let log_opts = bollard::container::LogsOptions::<String> {
            stdout: true,
            stderr: true,
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

        // Remove container
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
        // Try to ping Docker daemon synchronously via a quick check
        // In practice, we verified connectivity in new()
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

    // Docker integration tests are in tests/sandbox_docker_integration.rs
    // and require Docker daemon to be running.
}
```

**Step 2: Verify it compiles**

Run: `cargo check --features sandbox-docker`
Expected: Compiles successfully.

**Step 3: Run unit tests**

Run: `cargo test --features sandbox-docker sandbox::docker -- --nocapture`
Expected: Unit tests pass (Docker integration tests are separate).

**Step 4: Commit**

```bash
git add src/sandbox/docker.rs
git commit -m "feat(sandbox): add DockerBackend via bollard SDK"
```

---

### Task 9: Update NODE_TYPES.md documentation

**Files:**
- Modify: `docs/NODE_TYPES.md`

**Step 1: Add sandbox node documentation**

Add a new section for the sandbox node type with YAML examples, config reference, and security notes. Follow the existing documentation style.

**Step 2: Commit**

```bash
git add docs/NODE_TYPES.md
git commit -m "docs: add sandbox node type documentation"
```

---

### Task 10: Final verification

**Step 1: Run full test suite without sandbox feature**

Run: `cargo test`
Expected: All existing tests pass. No regressions.

**Step 2: Run full test suite with sandbox feature**

Run: `cargo test --features sandbox`
Expected: All tests pass (existing + sandbox unit + integration).

**Step 3: Run clippy**

Run: `cargo clippy --features sandbox -- -D warnings`
Expected: No warnings.

**Step 4: Run fmt check**

Run: `cargo fmt --all -- --check`
Expected: No formatting issues.

**Step 5: Run full test suite with all features**

Run: `cargo test --all-features`
Expected: All tests pass.

**Step 6: Test the example workflow manually**

Run:
```bash
cargo run --features sandbox -- workflows validate examples/sandbox-python.yaml
```
Expected: "Workflow 'sandbox-python-example' is valid"

**Step 7: Final commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: address clippy/fmt issues in sandbox module"
```
