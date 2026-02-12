//! Filter node - filter array items based on conditions.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Filter node implementation.
pub struct FilterNode;

impl FilterNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FilterNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct FilterConfig {
    field: String,
    conditions: Vec<Condition>,
    #[serde(default = "default_match_mode")]
    match_mode: String, // all | any
}

#[derive(Debug, Deserialize)]
struct Condition {
    field: String,
    operator: String,
    value: Value,
}

fn default_match_mode() -> String {
    "all".to_string()
}

#[async_trait]
impl Node for FilterNode {
    fn node_type(&self) -> &str {
        "filter"
    }

    fn description(&self) -> &str {
        "Filter an array using conditional rules"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: FilterConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid filter config: {}", e)))?;

        if config.conditions.is_empty() {
            return Err(Error::Node(
                "Filter node requires at least one condition".to_string(),
            ));
        }

        let match_mode = config.match_mode.to_lowercase();
        if match_mode != "all" && match_mode != "any" {
            return Err(Error::Node(format!(
                "Invalid match_mode '{}', expected 'all' or 'any'",
                config.match_mode
            )));
        }

        let items = resolve_field(&config.field, ctx);
        let Value::Array(items) = items else {
            return Err(Error::Node(format!(
                "Filter field '{}' must resolve to an array",
                config.field
            )));
        };

        let original_count = items.len();
        let mut filtered = Vec::new();

        for item in items {
            let mut results = Vec::with_capacity(config.conditions.len());

            for condition in &config.conditions {
                let left = resolve_item_field(&item, &condition.field);
                let passed = evaluate_condition(&left, &condition.operator, &condition.value)?;
                results.push(passed);
            }

            let include = if match_mode == "any" {
                results.iter().any(|r| *r)
            } else {
                results.iter().all(|r| *r)
            };

            if include {
                filtered.push(item);
            }
        }

        Ok(NodeResult::with_metadata(
            Value::Array(filtered.clone()),
            json!({
                "original_count": original_count,
                "filtered_count": filtered.len(),
                "match_mode": match_mode,
            }),
        ))
    }
}

fn evaluate_condition(left: &Value, operator: &str, right: &Value) -> Result<bool> {
    match operator {
        "equals" => Ok(left == right),
        "not_equals" => Ok(left != right),
        "contains" => match left {
            Value::String(s) => Ok(right
                .as_str()
                .map(|needle| s.contains(needle))
                .unwrap_or(false)),
            Value::Array(items) => Ok(items.contains(right)),
            Value::Object(map) => Ok(right.as_str().map(|k| map.contains_key(k)).unwrap_or(false)),
            _ => Ok(false),
        },
        "gt" | "lt" | "gte" | "lte" => {
            let l = as_f64(left).ok_or_else(|| {
                Error::Node(format!(
                    "Operator '{}' requires numeric left operand",
                    operator
                ))
            })?;
            let r = as_f64(right).ok_or_else(|| {
                Error::Node(format!(
                    "Operator '{}' requires numeric right operand",
                    operator
                ))
            })?;
            Ok(match operator {
                "gt" => l > r,
                "lt" => l < r,
                "gte" => l >= r,
                "lte" => l <= r,
                _ => false,
            })
        }
        "regex" => {
            let pattern = right
                .as_str()
                .ok_or_else(|| Error::Node("regex operator requires string pattern".to_string()))?;
            let text = stringify_value(left);
            let regex = regex_lite::Regex::new(pattern)
                .map_err(|e| Error::Node(format!("Invalid regex '{}': {}", pattern, e)))?;
            Ok(regex.is_match(&text))
        }
        "exists" => Ok(!left.is_null()),
        "not_exists" => Ok(left.is_null()),
        _ => Err(Error::Node(format!("Unsupported operator '{}'", operator))),
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => value.to_string(),
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
    async fn test_filter_equals() {
        let node = FilterNode::new();
        let config = json!({
            "field": "{{ input.items }}",
            "conditions": [{
                "field": "status",
                "operator": "equals",
                "value": "active"
            }]
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [
                {"id": 1, "status": "active"},
                {"id": 2, "status": "inactive"},
                {"id": 3, "status": "active"}
            ]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(
            result.data,
            json!([
                {"id": 1, "status": "active"},
                {"id": 3, "status": "active"}
            ])
        );
    }

    #[tokio::test]
    async fn test_filter_any_mode() {
        let node = FilterNode::new();
        let config = json!({
            "field": "input.items",
            "match_mode": "any",
            "conditions": [
                {"field": "score", "operator": "gte", "value": 90},
                {"field": "priority", "operator": "equals", "value": "high"}
            ]
        });
        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "items": [
                {"id": 1, "score": 85, "priority": "high"},
                {"id": 2, "score": 92, "priority": "low"},
                {"id": 3, "score": 70, "priority": "low"}
            ]
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data.as_array().unwrap().len(), 2);
    }
}
