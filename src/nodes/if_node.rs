//! IF node - conditional branching output.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// IF node implementation.
pub struct IfNode;

impl IfNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for IfNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct IfConfig {
    conditions: Vec<Condition>,
    true_branch: String,
    false_branch: String,
    #[serde(default = "default_match_mode")]
    match_mode: String, // "all" | "any"
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
impl Node for IfNode {
    fn node_type(&self) -> &str {
        "if"
    }

    fn description(&self) -> &str {
        "Evaluate conditions and choose true_branch or false_branch"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: IfConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid if config: {}", e)))?;

        if config.conditions.is_empty() {
            return Err(Error::Node(
                "IF node requires at least one condition".to_string(),
            ));
        }

        let match_mode = config.match_mode.to_lowercase();
        if match_mode != "all" && match_mode != "any" {
            return Err(Error::Node(format!(
                "Invalid match_mode '{}', expected 'all' or 'any'",
                config.match_mode
            )));
        }

        let mut results = Vec::with_capacity(config.conditions.len());
        for condition in &config.conditions {
            let left = resolve_field(&condition.field, ctx);
            let passed = evaluate_condition(&left, &condition.operator, &condition.value)?;
            results.push(passed);
        }

        let condition_result = if match_mode == "any" {
            results.iter().any(|r| *r)
        } else {
            results.iter().all(|r| *r)
        };

        let branch = if condition_result {
            config.true_branch.clone()
        } else {
            config.false_branch.clone()
        };

        Ok(NodeResult::with_metadata(
            json!({
                "condition_result": condition_result,
                "selected_branch": branch,
                "true_branch": config.true_branch,
                "false_branch": config.false_branch,
            }),
            json!({
                "conditions_evaluated": results.len(),
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
    async fn test_if_node_true_branch() {
        let node = IfNode::new();
        let config = json!({
            "conditions": [{
                "field": "{{ input.priority }}",
                "operator": "equals",
                "value": "high"
            }],
            "true_branch": "urgent-handler",
            "false_branch": "normal-handler"
        });
        let ctx = NodeContext::new("exec-1", "wf").with_input(json!({"priority": "high"}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["condition_result"], true);
        assert_eq!(result.data["selected_branch"], "urgent-handler");
    }

    #[tokio::test]
    async fn test_if_node_false_branch() {
        let node = IfNode::new();
        let config = json!({
            "conditions": [{
                "field": "input.score",
                "operator": "gt",
                "value": 90
            }],
            "true_branch": "pass",
            "false_branch": "fail"
        });
        let ctx = NodeContext::new("exec-1", "wf").with_input(json!({"score": 88}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["selected_branch"], "fail");
    }
}
