//! Transform node - data transformation using Rhai expressions.

use async_trait::async_trait;
use rhai::{Engine, Scope};
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Transform node using Rhai expressions.
pub struct TransformNode;

impl TransformNode {
    pub fn new() -> Self {
        Self
    }

    /// Create a configured Rhai engine.
    fn create_engine() -> Engine {
        let mut engine = Engine::new();

        // Add some useful functions
        engine.register_fn("to_json", |v: rhai::Dynamic| -> String {
            // Convert Rhai Dynamic to JSON string
            dynamic_to_json_string(v)
        });

        engine.register_fn("from_json", |s: &str| -> rhai::Dynamic {
            // Parse JSON string and convert to Rhai Dynamic
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
                json_to_dynamic(value)
            } else {
                rhai::Dynamic::UNIT
            }
        });

        engine
    }
}

impl Default for TransformNode {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert rhai::Dynamic to JSON string
fn dynamic_to_json_string(v: rhai::Dynamic) -> String {
    let json_value = dynamic_to_json(v);
    serde_json::to_string(&json_value).unwrap_or_default()
}

/// Convert rhai::Dynamic to serde_json::Value
fn dynamic_to_json(v: rhai::Dynamic) -> serde_json::Value {
    if v.is_unit() {
        serde_json::Value::Null
    } else if v.is_bool() {
        serde_json::Value::Bool(v.as_bool().unwrap_or(false))
    } else if v.is_int() {
        serde_json::json!(v.as_int().unwrap_or(0))
    } else if v.is_float() {
        serde_json::json!(v.as_float().unwrap_or(0.0))
    } else if v.is_string() {
        serde_json::Value::String(v.into_string().unwrap_or_default())
    } else if v.is_array() {
        if let Ok(arr) = v.into_array() {
            serde_json::Value::Array(arr.into_iter().map(dynamic_to_json).collect())
        } else {
            serde_json::Value::Null
        }
    } else if v.is_map() {
        if let Some(map) = v.try_cast::<rhai::Map>() {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .map(|(k, v)| (k.to_string(), dynamic_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        } else {
            serde_json::Value::Null
        }
    } else {
        // Fallback: convert to string
        serde_json::Value::String(v.to_string())
    }
}

/// Convert serde_json::Value to rhai::Dynamic
fn json_to_dynamic(value: serde_json::Value) -> rhai::Dynamic {
    match value {
        serde_json::Value::Null => rhai::Dynamic::UNIT,
        serde_json::Value::Bool(b) => rhai::Dynamic::from(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                rhai::Dynamic::from(i)
            } else if let Some(f) = n.as_f64() {
                rhai::Dynamic::from(f)
            } else {
                rhai::Dynamic::UNIT
            }
        }
        serde_json::Value::String(s) => rhai::Dynamic::from(s),
        serde_json::Value::Array(arr) => {
            let vec: Vec<rhai::Dynamic> = arr.into_iter().map(json_to_dynamic).collect();
            rhai::Dynamic::from(vec)
        }
        serde_json::Value::Object(obj) => {
            let mut map = rhai::Map::new();
            for (k, v) in obj {
                map.insert(k.into(), json_to_dynamic(v));
            }
            rhai::Dynamic::from(map)
        }
    }
}

#[derive(Debug, Deserialize)]
struct TransformConfig {
    /// Rhai expression to evaluate
    expression: String,
}

#[async_trait]
impl Node for TransformNode {
    fn node_type(&self) -> &str {
        "transform"
    }

    fn description(&self) -> &str {
        "Transform data using Rhai expressions"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: TransformConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid transform config: {}", e)))?;

        debug!("Transform: {}", config.expression);

        let engine = Self::create_engine();
        let mut scope = Scope::new();

        // Add input as JSON string (Rhai can parse it)
        let input_json = serde_json::to_string(&ctx.input).unwrap_or_default();
        scope.push("input", input_json.clone());
        scope.push("data", input_json);

        // Add previous node outputs
        for (node_id, output) in ctx.node_outputs.iter() {
            let output_json = serde_json::to_string(output).unwrap_or_default();
            scope.push(node_id.replace('-', "_"), output_json);
        }

        // Evaluate expression
        let result: rhai::Dynamic = engine
            .eval_with_scope(&mut scope, &config.expression)
            .map_err(|e| Error::Node(format!("Transform error: {}", e)))?;

        // Convert result to JSON Value
        let output: Value = if result.is_string() {
            let s = result.into_string().unwrap_or_default();
            // Try to parse JSON payloads encoded as strings.
            serde_json::from_str(&s).unwrap_or(Value::String(s))
        } else {
            dynamic_to_json(result)
        };

        Ok(NodeResult::new(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_transform_simple() {
        let node = TransformNode::new();
        let config = json!({
            "expression": r#""hello world""#
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, "hello world");
    }

    #[tokio::test]
    async fn test_transform_math() {
        let node = TransformNode::new();
        let config = json!({
            "expression": "2 + 2"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, 4);
    }

    #[tokio::test]
    async fn test_transform_with_input() {
        let node = TransformNode::new();
        let config = json!({
            "expression": r#"input"#
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({"name": "John"}));

        let result = node.execute(&config, &ctx).await.unwrap();
        // Input is passed as JSON string
        assert!(result.data.is_string() || result.data.is_object());
    }

    #[tokio::test]
    async fn test_transform_array_keeps_types() {
        let node = TransformNode::new();
        let config = json!({
            "expression": "[1, 2, 3]"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn test_transform_map_keeps_types() {
        let node = TransformNode::new();
        let config = json!({
            "expression": "from_json(\"{\\\"kind\\\":\\\"ok\\\",\\\"count\\\":2}\")"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!({"kind": "ok", "count": 2}));
    }
}
