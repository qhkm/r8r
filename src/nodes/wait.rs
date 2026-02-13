//! Wait node - delay/sleep for a specified duration.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Wait node for introducing delays in workflows.
pub struct WaitNode;

impl WaitNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WaitNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct WaitConfig {
    /// Duration in seconds to wait.
    #[serde(default)]
    seconds: Option<f64>,

    /// Duration in milliseconds to wait.
    #[serde(default)]
    milliseconds: Option<u64>,

    /// Whether to pass through input unchanged (default: true).
    #[serde(default = "default_true")]
    passthrough: bool,
}

fn default_true() -> bool {
    true
}

/// Maximum wait duration (1 hour).
const MAX_WAIT_SECONDS: f64 = 3600.0;

#[async_trait]
impl Node for WaitNode {
    fn node_type(&self) -> &str {
        "wait"
    }

    fn description(&self) -> &str {
        "Pause execution for a specified duration"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: WaitConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid wait config: {}", e)))?;

        // Determine wait duration
        let duration_ms = if let Some(ms) = config.milliseconds {
            ms
        } else if let Some(secs) = config.seconds {
            (secs * 1000.0) as u64
        } else {
            return Err(Error::Node(
                "Wait node requires 'seconds' or 'milliseconds'".to_string(),
            ));
        };

        // Validate duration
        let max_ms = (MAX_WAIT_SECONDS * 1000.0) as u64;
        if duration_ms > max_ms {
            return Err(Error::Node(format!(
                "Wait duration {}ms exceeds maximum {}ms",
                duration_ms, max_ms
            )));
        }

        debug!(
            duration_ms = duration_ms,
            execution_id = %ctx.execution_id,
            "Waiting"
        );

        // Perform the wait
        sleep(Duration::from_millis(duration_ms)).await;

        // Build output
        let output = if config.passthrough {
            json!({
                "waited_ms": duration_ms,
                "input": ctx.input.clone(),
            })
        } else {
            json!({
                "waited_ms": duration_ms,
            })
        };

        Ok(NodeResult::new(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn test_wait_seconds() {
        let node = WaitNode::new();
        let config = json!({ "seconds": 0.1 });
        let ctx = NodeContext::new("exec-1", "test");

        let start = Instant::now();
        let result = node.execute(&config, &ctx).await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed.as_millis() >= 100);
        assert_eq!(result.data["waited_ms"], 100);
    }

    #[tokio::test]
    async fn test_wait_milliseconds() {
        let node = WaitNode::new();
        let config = json!({ "milliseconds": 50 });
        let ctx = NodeContext::new("exec-1", "test");

        let start = Instant::now();
        let result = node.execute(&config, &ctx).await.unwrap();
        let elapsed = start.elapsed();

        assert!(elapsed.as_millis() >= 50);
        assert_eq!(result.data["waited_ms"], 50);
    }

    #[tokio::test]
    async fn test_wait_passthrough() {
        let node = WaitNode::new();
        let config = json!({ "milliseconds": 10, "passthrough": true });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"key": "value"}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["input"]["key"], "value");
    }

    #[tokio::test]
    async fn test_wait_exceeds_max() {
        let node = WaitNode::new();
        let config = json!({ "seconds": 7200 });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
    }
}
