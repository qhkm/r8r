//! Limit node - slice an array by offset and limit.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Limit node implementation.
pub struct LimitNode;

impl LimitNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LimitNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct LimitConfig {
    field: String,
    limit: usize,
    #[serde(default)]
    offset: usize,
}

#[async_trait]
impl Node for LimitNode {
    fn node_type(&self) -> &str {
        "limit"
    }

    fn description(&self) -> &str {
        "Limit the number of items from an array"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: LimitConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid limit config: {}", e)))?;

        let items = resolve_field(&config.field, ctx);
        let Value::Array(items) = items else {
            return Err(Error::Node(format!(
                "Limit field '{}' must resolve to an array",
                config.field
            )));
        };

        let original_count = items.len();
        let limited: Vec<Value> = items
            .into_iter()
            .skip(config.offset)
            .take(config.limit)
            .collect();

        Ok(NodeResult::with_metadata(
            Value::Array(limited.clone()),
            json!({
                "original_count": original_count,
                "offset": config.offset,
                "limit": config.limit,
                "result_count": limited.len(),
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
    async fn test_limit_with_offset() {
        let node = LimitNode::new();
        let config = json!({
            "field": "{{ input.items }}",
            "limit": 2,
            "offset": 1
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [1, 2, 3, 4]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!([2, 3]));
    }
}
