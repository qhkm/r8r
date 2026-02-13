//! Debug node - log and inspect data during workflow execution.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Debug node for logging and inspecting data.
pub struct DebugNode;

impl DebugNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DebugNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum DebugLevel {
    #[default]
    Info,
    Debug,
    Warn,
    Trace,
}

#[derive(Debug, Deserialize)]
struct DebugConfig {
    /// Message to log
    #[serde(default)]
    message: Option<String>,

    /// Log level
    #[serde(default)]
    level: DebugLevel,

    /// Whether to log the full input
    #[serde(default = "default_true")]
    log_input: bool,

    /// Whether to log the context variables
    #[serde(default)]
    log_variables: bool,

    /// Whether to log node outputs so far
    #[serde(default)]
    log_outputs: bool,

    /// Specific fields to extract and log from input
    #[serde(default)]
    fields: Option<Vec<String>>,

    /// Whether to pause for inspection (for testing)
    #[serde(default)]
    breakpoint: bool,

    /// Add custom data to output
    #[serde(default)]
    custom_data: Option<Value>,

    /// Label for this debug point
    #[serde(default)]
    label: Option<String>,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Node for DebugNode {
    fn node_type(&self) -> &str {
        "debug"
    }

    fn description(&self) -> &str {
        "Log and inspect data during workflow execution"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DebugConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid debug config: {}", e)))?;

        let label = config.label.as_deref().unwrap_or("debug");
        let mut debug_info = serde_json::Map::new();

        // Add label
        debug_info.insert("label".to_string(), json!(label));

        // Add execution context
        debug_info.insert("execution_id".to_string(), json!(ctx.execution_id));
        debug_info.insert("workflow_name".to_string(), json!(ctx.workflow_name));
        debug_info.insert("item_index".to_string(), json!(ctx.item_index));

        // Log custom message
        if let Some(message) = &config.message {
            debug_info.insert("message".to_string(), json!(message));
            log_message(&config.level, label, message);
        }

        // Log input
        if config.log_input {
            debug_info.insert("input".to_string(), ctx.input.clone());
            log_value(&config.level, label, "input", &ctx.input);
        }

        // Log specific fields
        if let Some(fields) = &config.fields {
            let mut extracted = serde_json::Map::new();
            for field in fields {
                let value = get_field_value(&ctx.input, field);
                extracted.insert(field.clone(), value.clone());
                log_value(&config.level, label, &format!("field.{}", field), &value);
            }
            debug_info.insert("fields".to_string(), Value::Object(extracted));
        }

        // Log variables
        if config.log_variables {
            debug_info.insert("variables".to_string(), (*ctx.variables).clone());
            log_value(&config.level, label, "variables", &ctx.variables);
        }

        // Log node outputs
        if config.log_outputs {
            let outputs: Value = ctx
                .node_outputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            debug_info.insert("node_outputs".to_string(), outputs.clone());
            log_value(&config.level, label, "node_outputs", &outputs);
        }

        // Add custom data
        if let Some(custom) = &config.custom_data {
            debug_info.insert("custom".to_string(), custom.clone());
        }

        // Breakpoint (just logs a marker for now - actual breakpoints would need runtime support)
        if config.breakpoint {
            warn!(
                "[{}] BREAKPOINT hit - execution_id: {}, item_index: {:?}",
                label, ctx.execution_id, ctx.item_index
            );
            debug_info.insert("breakpoint".to_string(), json!(true));
        }

        // Pass through input unchanged, but add debug info to metadata
        Ok(NodeResult::with_metadata(
            ctx.input.clone(),
            Value::Object(debug_info),
        ))
    }
}

/// Get a nested field value from JSON.
fn get_field_value(value: &Value, path: &str) -> Value {
    let mut current = value.clone();
    for part in path.split('.') {
        current = match current {
            Value::Object(obj) => obj.get(part).cloned().unwrap_or(Value::Null),
            Value::Array(arr) => {
                if let Ok(idx) = part.parse::<usize>() {
                    arr.get(idx).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        };
    }
    current
}

/// Log a message at the specified level.
fn log_message(level: &DebugLevel, label: &str, message: &str) {
    match level {
        DebugLevel::Info => info!("[{}] {}", label, message),
        DebugLevel::Debug => debug!("[{}] {}", label, message),
        DebugLevel::Warn => warn!("[{}] {}", label, message),
        DebugLevel::Trace => tracing::trace!("[{}] {}", label, message),
    }
}

/// Log a value at the specified level.
fn log_value(level: &DebugLevel, label: &str, name: &str, value: &Value) {
    let formatted = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    match level {
        DebugLevel::Info => info!("[{}] {} = {}", label, name, formatted),
        DebugLevel::Debug => debug!("[{}] {} = {}", label, name, formatted),
        DebugLevel::Warn => warn!("[{}] {} = {}", label, name, formatted),
        DebugLevel::Trace => tracing::trace!("[{}] {} = {}", label, name, formatted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_field_value_simple() {
        let value = json!({"name": "test", "count": 42});
        assert_eq!(get_field_value(&value, "name"), json!("test"));
        assert_eq!(get_field_value(&value, "count"), json!(42));
    }

    #[test]
    fn test_get_field_value_nested() {
        let value = json!({"user": {"profile": {"name": "Alice"}}});
        assert_eq!(get_field_value(&value, "user.profile.name"), json!("Alice"));
    }

    #[test]
    fn test_get_field_value_array() {
        let value = json!({"items": ["a", "b", "c"]});
        assert_eq!(get_field_value(&value, "items.1"), json!("b"));
    }

    #[test]
    fn test_get_field_value_missing() {
        let value = json!({"name": "test"});
        assert_eq!(get_field_value(&value, "missing"), Value::Null);
        assert_eq!(get_field_value(&value, "name.nested"), Value::Null);
    }

    #[tokio::test]
    async fn test_debug_node_passthrough() {
        let node = DebugNode::new();
        let config = json!({
            "message": "Test debug",
            "log_input": false
        });
        let ctx = NodeContext::new("exec-1", "test-workflow").with_input(json!({"data": "value"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        // Should pass through input unchanged
        assert_eq!(result.data, json!({"data": "value"}));
        // Should have debug metadata
        assert_eq!(result.metadata["message"], "Test debug");
    }

    #[tokio::test]
    async fn test_debug_node_with_label() {
        let node = DebugNode::new();
        let config = json!({
            "label": "checkpoint-1",
            "message": "Reached checkpoint"
        });
        let ctx = NodeContext::new("exec-1", "test-workflow");

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["label"], "checkpoint-1");
    }

    #[tokio::test]
    async fn test_debug_node_log_variables() {
        let node = DebugNode::new();
        let config = json!({
            "log_input": false,
            "log_variables": true
        });
        let ctx = NodeContext::new("exec-1", "test-workflow")
            .with_variables(json!({"api_key": "secret", "debug": true}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["variables"]["debug"], true);
    }

    #[tokio::test]
    async fn test_debug_node_extract_fields() {
        let node = DebugNode::new();
        let config = json!({
            "log_input": false,
            "fields": ["user.name", "user.id"]
        });
        let ctx = NodeContext::new("exec-1", "test-workflow")
            .with_input(json!({"user": {"name": "Alice", "id": 123, "email": "alice@test.com"}}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["fields"]["user.name"], "Alice");
        assert_eq!(result.metadata["fields"]["user.id"], 123);
    }

    #[tokio::test]
    async fn test_debug_node_breakpoint() {
        let node = DebugNode::new();
        let config = json!({
            "breakpoint": true,
            "log_input": false
        });
        let ctx = NodeContext::new("exec-1", "test-workflow");

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["breakpoint"], true);
    }

    #[tokio::test]
    async fn test_debug_node_custom_data() {
        let node = DebugNode::new();
        let config = json!({
            "log_input": false,
            "custom_data": {"marker": "test", "step": 5}
        });
        let ctx = NodeContext::new("exec-1", "test-workflow");

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["custom"]["marker"], "test");
        assert_eq!(result.metadata["custom"]["step"], 5);
    }

    #[tokio::test]
    async fn test_debug_node_item_index() {
        let node = DebugNode::new();
        let config = json!({
            "log_input": false
        });
        let base_ctx = NodeContext::new("exec-1", "test-workflow");
        let ctx = base_ctx.for_item(json!("item-data"), 3);

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.metadata["item_index"], 3);
    }
}
