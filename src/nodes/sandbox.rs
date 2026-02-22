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
        let code = render_template(&config.code, ctx, RenderMode::Compact);

        // Render env var values
        let mut env = HashMap::new();
        for (k, v) in &config.env {
            env.insert(k.clone(), render_template(v, ctx, RenderMode::Compact));
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
            SandboxError::BackendUnavailable { backend, reason } => Error::Node(format!(
                "Sandbox backend '{}' unavailable: {}",
                backend, reason
            )),
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

        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"name": "world"}));

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
