//! Merge node - combine outputs from multiple upstream nodes.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Merge node implementation.
pub struct MergeNode;

impl MergeNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MergeNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct MergeConfig {
    #[serde(default = "default_mode")]
    mode: String, // append | combine | zip
    #[serde(default)]
    wait_for: Vec<String>,
}

fn default_mode() -> String {
    "append".to_string()
}

#[async_trait]
impl Node for MergeNode {
    fn node_type(&self) -> &str {
        "merge"
    }

    fn description(&self) -> &str {
        "Merge multiple inputs by append, combine, or zip mode"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: MergeConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid merge config: {}", e)))?;
        let mode = config.mode.to_lowercase();
        let sources = collect_sources(ctx, &config.wait_for);

        let merged = match mode.as_str() {
            "append" => merge_append(sources),
            "combine" => merge_combine(sources)?,
            "zip" => merge_zip(sources)?,
            _ => {
                return Err(Error::Node(format!(
                    "Invalid merge mode '{}', expected append/combine/zip",
                    config.mode
                )))
            }
        };

        Ok(NodeResult::with_metadata(
            merged,
            json!({
                "mode": mode,
                "inputs_count": if config.wait_for.is_empty() { ctx.node_outputs.len() } else { config.wait_for.len() },
            }),
        ))
    }
}

fn collect_sources(ctx: &NodeContext, wait_for: &[String]) -> Vec<Value> {
    if !wait_for.is_empty() {
        return wait_for
            .iter()
            .map(|node_id| ctx.get_output(node_id).cloned().unwrap_or(Value::Null))
            .collect();
    }

    if let Some(obj) = ctx.input.as_object() {
        let mut keys: Vec<_> = obj.keys().cloned().collect();
        keys.sort();
        return keys
            .into_iter()
            .map(|k| obj.get(&k).cloned().unwrap_or(Value::Null))
            .collect();
    }

    vec![ctx.input.clone()]
}

fn merge_append(sources: Vec<Value>) -> Value {
    let mut out = Vec::new();
    for source in sources {
        if let Value::Array(items) = source {
            out.extend(items);
        } else {
            out.push(source);
        }
    }
    Value::Array(out)
}

fn merge_combine(sources: Vec<Value>) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for source in sources {
        match source {
            Value::Object(map) => {
                for (k, v) in map {
                    out.insert(k, v);
                }
            }
            other => {
                return Err(Error::Node(format!(
                    "merge combine mode requires object inputs, found {}",
                    type_name(&other)
                )))
            }
        }
    }
    Ok(Value::Object(out))
}

fn merge_zip(sources: Vec<Value>) -> Result<Value> {
    let arrays: Result<Vec<Vec<Value>>> = sources
        .into_iter()
        .map(|v| match v {
            Value::Array(items) => Ok(items),
            other => Err(Error::Node(format!(
                "merge zip mode requires array inputs, found {}",
                type_name(&other)
            ))),
        })
        .collect();
    let arrays = arrays?;

    let min_len = arrays.iter().map(|a| a.len()).min().unwrap_or(0);
    let mut zipped = Vec::with_capacity(min_len);
    for idx in 0..min_len {
        let row = arrays
            .iter()
            .map(|arr| arr[idx].clone())
            .collect::<Vec<_>>();
        zipped.push(Value::Array(row));
    }

    Ok(Value::Array(zipped))
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_merge_append() {
        let node = MergeNode::new();
        let config = json!({"mode": "append", "wait_for": ["a", "b"]});
        let mut ctx = NodeContext::new("exec-1", "wf");
        ctx.add_output("a", json!([1, 2]));
        ctx.add_output("b", json!([3, 4]));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!([1, 2, 3, 4]));
    }

    #[tokio::test]
    async fn test_merge_combine() {
        let node = MergeNode::new();
        let config = json!({"mode": "combine", "wait_for": ["a", "b"]});
        let mut ctx = NodeContext::new("exec-1", "wf");
        ctx.add_output("a", json!({"x": 1}));
        ctx.add_output("b", json!({"y": 2}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!({"x": 1, "y": 2}));
    }

    #[tokio::test]
    async fn test_merge_zip() {
        let node = MergeNode::new();
        let config = json!({"mode": "zip", "wait_for": ["a", "b"]});
        let mut ctx = NodeContext::new("exec-1", "wf");
        ctx.add_output("a", json!([1, 2, 3]));
        ctx.add_output("b", json!(["a", "b"]));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!([[1, "a"], [2, "b"]]));
    }
}
