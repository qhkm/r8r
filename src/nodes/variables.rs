//! Variables node - set and manipulate workflow variables.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Variables node for setting workflow variables.
pub struct VariablesNode;

impl VariablesNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for VariablesNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum VariablesOperation {
    /// Set one or more variables
    #[default]
    Set,
    /// Get variable values
    Get,
    /// Merge variables (deep merge for objects)
    Merge,
    /// Delete variables
    Delete,
    /// List all variables
    List,
}

#[derive(Debug, Deserialize)]
struct VariablesConfig {
    /// Operation to perform
    #[serde(default)]
    operation: VariablesOperation,

    /// Variables to set (for Set operation)
    #[serde(default)]
    values: Option<Value>,

    /// Variable names to get or delete
    #[serde(default)]
    names: Option<Vec<String>>,

    /// Whether to pass through input (default: true)
    #[serde(default = "default_true")]
    passthrough: bool,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Node for VariablesNode {
    fn node_type(&self) -> &str {
        "variables"
    }

    fn description(&self) -> &str {
        "Set, get, or manipulate workflow variables"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: VariablesConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid variables config: {}", e)))?;

        let result = match config.operation {
            VariablesOperation::Set => {
                let values = config.values.ok_or_else(|| {
                    Error::Node("Set operation requires 'values' field".to_string())
                })?;

                // Merge new values with existing variables
                let mut new_vars = ctx.variables.clone();
                if let (Value::Object(existing), Value::Object(new_values)) =
                    (&mut new_vars, &values)
                {
                    for (key, value) in new_values {
                        existing.insert(key.clone(), value.clone());
                    }
                }

                json!({
                    "variables": new_vars,
                    "set": values,
                })
            }
            VariablesOperation::Get => {
                let names = config.names.ok_or_else(|| {
                    Error::Node("Get operation requires 'names' field".to_string())
                })?;

                let mut result = serde_json::Map::new();
                if let Value::Object(vars) = &ctx.variables {
                    for name in &names {
                        if let Some(value) = vars.get(name) {
                            result.insert(name.clone(), value.clone());
                        } else {
                            result.insert(name.clone(), Value::Null);
                        }
                    }
                }

                json!({
                    "variables": Value::Object(result),
                })
            }
            VariablesOperation::Merge => {
                let values = config.values.ok_or_else(|| {
                    Error::Node("Merge operation requires 'values' field".to_string())
                })?;

                let merged = deep_merge(&ctx.variables, &values);

                json!({
                    "variables": merged,
                })
            }
            VariablesOperation::Delete => {
                let names = config.names.ok_or_else(|| {
                    Error::Node("Delete operation requires 'names' field".to_string())
                })?;

                let mut new_vars = ctx.variables.clone();
                if let Value::Object(vars) = &mut new_vars {
                    for name in &names {
                        vars.remove(name);
                    }
                }

                json!({
                    "variables": new_vars,
                    "deleted": names,
                })
            }
            VariablesOperation::List => {
                let names: Vec<String> = if let Value::Object(vars) = &ctx.variables {
                    vars.keys().cloned().collect()
                } else {
                    vec![]
                };

                json!({
                    "variables": ctx.variables.clone(),
                    "names": names,
                    "count": names.len(),
                })
            }
        };

        // Include input in output if passthrough is enabled
        let output = if config.passthrough {
            let mut output = result.clone();
            if let Value::Object(out) = &mut output {
                out.insert("input".to_string(), ctx.input.clone());
            }
            output
        } else {
            result.clone()
        };

        Ok(NodeResult::with_metadata(
            output,
            json!({
                "operation": format!("{:?}", config.operation),
            }),
        ))
    }
}

/// Deep merge two JSON values.
fn deep_merge(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            let mut result = base_obj.clone();
            for (key, value) in overlay_obj {
                if let Some(base_value) = result.get(key) {
                    result.insert(key.clone(), deep_merge(base_value, value));
                } else {
                    result.insert(key.clone(), value.clone());
                }
            }
            Value::Object(result)
        }
        (_, overlay) => overlay.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deep_merge_objects() {
        let base = json!({"a": 1, "b": {"c": 2}});
        let overlay = json!({"b": {"d": 3}, "e": 4});
        let result = deep_merge(&base, &overlay);

        assert_eq!(result["a"], 1);
        assert_eq!(result["b"]["c"], 2);
        assert_eq!(result["b"]["d"], 3);
        assert_eq!(result["e"], 4);
    }

    #[test]
    fn test_deep_merge_replace() {
        let base = json!({"a": 1});
        let overlay = json!({"a": 2});
        let result = deep_merge(&base, &overlay);

        assert_eq!(result["a"], 2);
    }

    #[test]
    fn test_deep_merge_nested() {
        let base = json!({"config": {"debug": false, "timeout": 30}});
        let overlay = json!({"config": {"debug": true}});
        let result = deep_merge(&base, &overlay);

        assert_eq!(result["config"]["debug"], true);
        assert_eq!(result["config"]["timeout"], 30);
    }

    #[tokio::test]
    async fn test_variables_node_set() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "set",
            "values": {
                "api_key": "secret123",
                "max_retries": 3
            }
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["variables"]["api_key"], "secret123");
        assert_eq!(result.data["variables"]["max_retries"], 3);
    }

    #[tokio::test]
    async fn test_variables_node_get() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "get",
            "names": ["api_key", "missing"]
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_variables(json!({"api_key": "secret", "other": "value"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["variables"]["api_key"], "secret");
        assert_eq!(result.data["variables"]["missing"], Value::Null);
    }

    #[tokio::test]
    async fn test_variables_node_merge() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "merge",
            "values": {
                "config": {"new_setting": true}
            }
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_variables(json!({"config": {"old_setting": false}}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["variables"]["config"]["old_setting"], false);
        assert_eq!(result.data["variables"]["config"]["new_setting"], true);
    }

    #[tokio::test]
    async fn test_variables_node_delete() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "delete",
            "names": ["to_delete"]
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_variables(json!({"to_delete": "bye", "keep": "stay"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert!(result.data["variables"].get("to_delete").is_none());
        assert_eq!(result.data["variables"]["keep"], "stay");
    }

    #[tokio::test]
    async fn test_variables_node_list() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "list"
        });
        let ctx =
            NodeContext::new("exec-1", "test").with_variables(json!({"a": 1, "b": 2, "c": 3}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["count"], 3);
        let names = result.data["names"].as_array().unwrap();
        assert_eq!(names.len(), 3);
    }

    #[tokio::test]
    async fn test_variables_node_passthrough() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "set",
            "values": {"new_var": "value"},
            "passthrough": true
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"original": "data"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["input"]["original"], "data");
        assert_eq!(result.data["variables"]["new_var"], "value");
    }

    #[tokio::test]
    async fn test_variables_node_no_passthrough() {
        let node = VariablesNode::new();
        let config = json!({
            "operation": "set",
            "values": {"new_var": "value"},
            "passthrough": false
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"original": "data"}));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert!(result.data.get("input").is_none());
    }
}
