//! Aggregate node - combine many items into a single value.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Aggregate node implementation.
pub struct AggregateNode;

impl AggregateNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AggregateNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct AggregateConfig {
    #[serde(default = "default_mode")]
    mode: String, // array | object | first | last
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    key_field: Option<String>,
    #[serde(default)]
    value_field: Option<String>,
}

fn default_mode() -> String {
    "array".to_string()
}

#[async_trait]
impl Node for AggregateNode {
    fn node_type(&self) -> &str {
        "aggregate"
    }

    fn description(&self) -> &str {
        "Aggregate an array as array/object/first/last"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: AggregateConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid aggregate config: {}", e)))?;

        let source = if let Some(field) = &config.field {
            resolve_field(field, ctx)
        } else {
            ctx.input.clone()
        };

        let Value::Array(items) = source else {
            return Err(Error::Node(
                "Aggregate node expects array input (or array field)".to_string(),
            ));
        };

        let mode = config.mode.to_lowercase();
        let output = match mode.as_str() {
            "array" => Value::Array(items.clone()),
            "first" => items.first().cloned().unwrap_or(Value::Null),
            "last" => items.last().cloned().unwrap_or(Value::Null),
            "object" => aggregate_object(&items, &config.key_field, &config.value_field)?,
            _ => {
                return Err(Error::Node(format!(
                    "Invalid aggregate mode '{}', expected array/object/first/last",
                    config.mode
                )))
            }
        };

        Ok(NodeResult::with_metadata(
            output,
            json!({
                "mode": mode,
                "items_count": items.len(),
            }),
        ))
    }
}

fn aggregate_object(
    items: &[Value],
    key_field: &Option<String>,
    value_field: &Option<String>,
) -> Result<Value> {
    let key_field = key_field
        .as_ref()
        .ok_or_else(|| Error::Node("aggregate object mode requires key_field".to_string()))?;

    let mut out = Map::new();
    for item in items {
        let key_value = resolve_item_field(item, key_field);
        let key = value_to_key(&key_value)?;

        let value = if let Some(field) = value_field {
            resolve_item_field(item, field)
        } else {
            item.clone()
        };

        out.insert(key, value);
    }

    Ok(Value::Object(out))
}

fn value_to_key(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Null => Err(Error::Node(
            "aggregate key_field resolved to null, which is not allowed".to_string(),
        )),
        _ => Err(Error::Node(
            "aggregate key_field must resolve to string/number/bool".to_string(),
        )),
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

fn resolve_item_field(item: &Value, field: &str) -> Value {
    let expr = normalize_template(field.trim());

    if expr == "item" || expr == "input" {
        return item.clone();
    }

    if let Some(path) = expr
        .strip_prefix("item.")
        .or_else(|| expr.strip_prefix("input."))
    {
        return get_path_value(item, path).unwrap_or(Value::Null);
    }

    get_path_value(item, expr).unwrap_or(Value::Null)
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
    async fn test_aggregate_object() {
        let node = AggregateNode::new();
        let config = json!({
            "mode": "object",
            "field": "input.items",
            "key_field": "id",
            "value_field": "value"
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [
                {"id": "A", "value": 1},
                {"id": "B", "value": 2}
            ]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data, json!({"A": 1, "B": 2}));
    }

    #[tokio::test]
    async fn test_aggregate_first_last() {
        let node = AggregateNode::new();
        let ctx = NodeContext::new("exec", "wf").with_input(json!([1, 2, 3]));

        let first = node.execute(&json!({"mode": "first"}), &ctx).await.unwrap();
        let last = node.execute(&json!({"mode": "last"}), &ctx).await.unwrap();

        assert_eq!(first.data, json!(1));
        assert_eq!(last.data, json!(3));
    }
}
