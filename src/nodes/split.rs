//! Split node - split arrays into chunks.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Split node implementation.
pub struct SplitNode;

impl SplitNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SplitNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SplitConfig {
    field: String,
    #[serde(default)]
    batch_size: Option<usize>,
}

#[async_trait]
impl Node for SplitNode {
    fn node_type(&self) -> &str {
        "split"
    }

    fn description(&self) -> &str {
        "Split arrays into batches"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SplitConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid split config: {}", e)))?;

        let items = resolve_field(&config.field, ctx);
        let Value::Array(items) = items else {
            return Err(Error::Node(format!(
                "Split field '{}' must resolve to an array",
                config.field
            )));
        };

        let output = if let Some(batch_size) = config.batch_size {
            if batch_size == 0 {
                return Err(Error::Node("split batch_size must be >= 1".to_string()));
            }
            let mut batches = Vec::new();
            for chunk in items.chunks(batch_size) {
                batches.push(Value::Array(chunk.to_vec()));
            }
            Value::Array(batches)
        } else {
            Value::Array(items)
        };

        let batches = output.as_array().map(|a| a.len()).unwrap_or(0);
        Ok(NodeResult::with_metadata(
            output,
            json!({
                "batch_size": config.batch_size,
                "batches": batches,
            }),
        ))
    }
}

fn resolve_field(field: &str, ctx: &NodeContext) -> Value {
    let expr = normalize_template(field.trim());
    if expr == "input" {
        return ctx.input.clone();
    }

    if let Some(path) = expr.strip_prefix("input.") {
        return get_path_value(&ctx.input, path).unwrap_or(Value::Null);
    }

    if let Some(rest) = expr.strip_prefix("nodes.") {
        if let Some((node_id, path)) = rest.split_once(".output") {
            let base = ctx
                .node_outputs
                .get(node_id)
                .cloned()
                .unwrap_or(Value::Null);
            let path = path.strip_prefix('.').unwrap_or(path);
            if path.is_empty() {
                return base;
            }
            return get_path_value(&base, path).unwrap_or(Value::Null);
        }
    }

    Value::String(field.to_string())
}

fn normalize_template(expr: &str) -> &str {
    let trimmed = expr.trim();
    if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
        trimmed[2..trimmed.len() - 2].trim()
    } else {
        trimmed
    }
}

fn get_path_value(root: &Value, path: &str) -> Option<Value> {
    let mut current = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        match current {
            Value::Object(map) => current = map.get(segment)?,
            Value::Array(items) => {
                let index = segment.parse::<usize>().ok()?;
                current = items.get(index)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_split_batches() {
        let node = SplitNode::new();
        let config = json!({
            "field": "input.items",
            "batch_size": 2
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [1, 2, 3, 4, 5]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!([[1, 2], [3, 4], [5]]));
    }

    #[tokio::test]
    async fn test_split_passthrough() {
        let node = SplitNode::new();
        let config = json!({
            "field": "{{ input.items }}"
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": ["a", "b"]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!(["a", "b"]));
    }
}
