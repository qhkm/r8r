//! Sort node - sort array items by one or more fields.

use std::cmp::Ordering;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Sort node implementation.
pub struct SortNode;

impl SortNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SortNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SortConfig {
    field: String,
    by: Vec<SortBy>,
}

#[derive(Debug, Deserialize)]
struct SortBy {
    field: String,
    #[serde(default = "default_order")]
    order: String, // asc | desc
}

fn default_order() -> String {
    "asc".to_string()
}

#[async_trait]
impl Node for SortNode {
    fn node_type(&self) -> &str {
        "sort"
    }

    fn description(&self) -> &str {
        "Sort an array by one or more fields"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SortConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid sort config: {}", e)))?;

        if config.by.is_empty() {
            return Err(Error::Node(
                "Sort node requires at least one sort field in 'by'".to_string(),
            ));
        }

        for rule in &config.by {
            let order = rule.order.to_lowercase();
            if order != "asc" && order != "desc" {
                return Err(Error::Node(format!(
                    "Invalid sort order '{}' for field '{}', expected 'asc' or 'desc'",
                    rule.order, rule.field
                )));
            }
        }

        let items = resolve_field(&config.field, ctx);
        let Value::Array(mut items) = items else {
            return Err(Error::Node(format!(
                "Sort field '{}' must resolve to an array",
                config.field
            )));
        };

        items.sort_by(|a, b| {
            for rule in &config.by {
                let left = resolve_item_field(a, &rule.field);
                let right = resolve_item_field(b, &rule.field);
                let mut ord = compare_values(&left, &right);
                if rule.order.eq_ignore_ascii_case("desc") {
                    ord = ord.reverse();
                }

                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });

        Ok(NodeResult::with_metadata(
            Value::Array(items.clone()),
            json!({
                "sorted_count": items.len(),
                "sort_fields": config.by.len(),
            }),
        ))
    }
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Number(l), Value::Number(r)) => {
            let l = l.as_f64().unwrap_or(0.0);
            let r = r.as_f64().unwrap_or(0.0);
            l.partial_cmp(&r).unwrap_or(Ordering::Equal)
        }
        (Value::Bool(l), Value::Bool(r)) => l.cmp(r),
        (Value::String(l), Value::String(r)) => l.cmp(r),
        _ => left.to_string().cmp(&right.to_string()),
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
    async fn test_sort_desc_then_asc() {
        let node = SortNode::new();
        let config = json!({
            "field": "input.items",
            "by": [
                {"field": "priority", "order": "desc"},
                {"field": "created_at", "order": "asc"}
            ]
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [
                {"id": 1, "priority": 1, "created_at": "2026-01-03"},
                {"id": 2, "priority": 3, "created_at": "2026-01-02"},
                {"id": 3, "priority": 3, "created_at": "2026-01-01"}
            ]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data[0]["id"], 3);
        assert_eq!(result.data[1]["id"], 2);
        assert_eq!(result.data[2]["id"], 1);
    }
}
