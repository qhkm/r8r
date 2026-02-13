//! Wait/delay node - pause execution for a duration.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Wait/delay node that pauses execution.
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
    /// Duration to wait in seconds (can be fractional, e.g., 0.5 for 500ms)
    #[serde(default)]
    seconds: Option<f64>,

    /// Duration to wait in milliseconds
    #[serde(default)]
    milliseconds: Option<u64>,

    /// Duration to wait in minutes
    #[serde(default)]
    minutes: Option<f64>,

    /// Wait until a specific ISO 8601 timestamp (e.g., "2024-01-15T10:30:00Z")
    #[serde(default)]
    until: Option<String>,
}

#[async_trait]
impl Node for WaitNode {
    fn node_type(&self) -> &str {
        "wait"
    }

    fn description(&self) -> &str {
        "Pause execution for a specified duration or until a specific time"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: WaitConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid wait config: {}", e)))?;

        let wait_ms = calculate_wait_duration(&config)?;

        if wait_ms > 0 {
            info!(
                "Wait node pausing for {}ms (execution: {})",
                wait_ms, ctx.execution_id
            );

            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        }

        Ok(NodeResult::with_metadata(
            ctx.input.clone(), // Pass through input unchanged
            json!({
                "waited_ms": wait_ms,
            }),
        ))
    }
}

/// Calculate the wait duration in milliseconds from config.
fn calculate_wait_duration(config: &WaitConfig) -> Result<u64> {
    // Check for "until" timestamp first
    if let Some(until) = &config.until {
        let target = chrono::DateTime::parse_from_rfc3339(until)
            .map_err(|e| Error::Node(format!("Invalid 'until' timestamp '{}': {}", until, e)))?;

        let now = chrono::Utc::now();
        let target_utc = target.with_timezone(&chrono::Utc);

        if target_utc <= now {
            // Target time is in the past, don't wait
            return Ok(0);
        }

        let duration = target_utc - now;
        return Ok(duration.num_milliseconds() as u64);
    }

    // Calculate from duration fields
    let mut total_ms: u64 = 0;

    if let Some(minutes) = config.minutes {
        if minutes < 0.0 {
            return Err(Error::Node("Wait duration cannot be negative".to_string()));
        }
        total_ms += (minutes * 60.0 * 1000.0) as u64;
    }

    if let Some(seconds) = config.seconds {
        if seconds < 0.0 {
            return Err(Error::Node("Wait duration cannot be negative".to_string()));
        }
        total_ms += (seconds * 1000.0) as u64;
    }

    if let Some(ms) = config.milliseconds {
        total_ms += ms;
    }

    // Cap at 1 hour to prevent accidental long waits
    const MAX_WAIT_MS: u64 = 60 * 60 * 1000; // 1 hour
    if total_ms > MAX_WAIT_MS {
        return Err(Error::Node(format!(
            "Wait duration {}ms exceeds maximum of {}ms (1 hour)",
            total_ms, MAX_WAIT_MS
        )));
    }

    Ok(total_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_seconds() {
        let config = WaitConfig {
            seconds: Some(2.5),
            milliseconds: None,
            minutes: None,
            until: None,
        };
        assert_eq!(calculate_wait_duration(&config).unwrap(), 2500);
    }

    #[test]
    fn test_calculate_milliseconds() {
        let config = WaitConfig {
            seconds: None,
            milliseconds: Some(500),
            minutes: None,
            until: None,
        };
        assert_eq!(calculate_wait_duration(&config).unwrap(), 500);
    }

    #[test]
    fn test_calculate_minutes() {
        let config = WaitConfig {
            seconds: None,
            milliseconds: None,
            minutes: Some(1.5),
            until: None,
        };
        assert_eq!(calculate_wait_duration(&config).unwrap(), 90000);
    }

    #[test]
    fn test_calculate_combined() {
        let config = WaitConfig {
            seconds: Some(30.0),
            milliseconds: Some(500),
            minutes: Some(1.0),
            until: None,
        };
        // 1 min = 60000ms, 30s = 30000ms, 500ms
        assert_eq!(calculate_wait_duration(&config).unwrap(), 90500);
    }

    #[test]
    fn test_negative_duration_fails() {
        let config = WaitConfig {
            seconds: Some(-5.0),
            milliseconds: None,
            minutes: None,
            until: None,
        };
        assert!(calculate_wait_duration(&config).is_err());
    }

    #[test]
    fn test_exceeds_max_fails() {
        let config = WaitConfig {
            seconds: None,
            milliseconds: None,
            minutes: Some(120.0), // 2 hours
            until: None,
        };
        assert!(calculate_wait_duration(&config).is_err());
    }

    #[test]
    fn test_until_past_returns_zero() {
        let config = WaitConfig {
            seconds: None,
            milliseconds: None,
            minutes: None,
            until: Some("2020-01-01T00:00:00Z".to_string()),
        };
        assert_eq!(calculate_wait_duration(&config).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_wait_node_passthrough() {
        let node = WaitNode::new();
        let config = json!({
            "milliseconds": 10
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"data": "value"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        // Should pass through input
        assert_eq!(result.data, json!({"data": "value"}));
        // Should have metadata
        assert!(result.metadata.get("waited_ms").is_some());
    }
}
