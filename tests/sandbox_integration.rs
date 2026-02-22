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
