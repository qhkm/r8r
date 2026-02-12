//! Switch node - multi-way branch selection.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Switch node implementation.
pub struct SwitchNode;

impl SwitchNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SwitchNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SwitchConfig {
    field: String,
    cases: Vec<SwitchCase>,
    #[serde(default)]
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SwitchCase {
    value: Value,
    branch: String,
    #[serde(default = "default_operator")]
    operator: String,
}

fn default_operator() -> String {
    "equals".to_string()
}

#[async_trait]
impl Node for SwitchNode {
    fn node_type(&self) -> &str {
        "switch"
    }

    fn description(&self) -> &str {
        "Select branch by matching field value against cases"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SwitchConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid switch config: {}", e)))?;

        if config.cases.is_empty() && config.default_branch.is_none() {
            return Err(Error::Node(
                "Switch node requires at least one case or a default_branch".to_string(),
            ));
        }

        let field_value = resolve_field(&config.field, ctx);
        let mut selected_branch = None;
        let mut matched_case_index = None;

        for (idx, case) in config.cases.iter().enumerate() {
            if evaluate_case(&field_value, &case.operator, &case.value)? {
                selected_branch = Some(case.branch.clone());
                matched_case_index = Some(idx);
                break;
            }
        }

        let selected_branch = selected_branch
            .or_else(|| config.default_branch.clone())
            .ok_or_else(|| {
                Error::Node("No switch case matched and no default_branch set".to_string())
            })?;

        Ok(NodeResult::with_metadata(
            json!({
                "selected_branch": selected_branch,
                "matched_case_index": matched_case_index,
                "field_value": field_value,
            }),
            json!({
                "cases_checked": config.cases.len(),
            }),
        ))
    }
}

fn evaluate_case(left: &Value, operator: &str, right: &Value) -> Result<bool> {
    match operator {
        "equals" => Ok(left == right),
        "not_equals" => Ok(left != right),
        "contains" => match left {
            Value::String(s) => Ok(right
                .as_str()
                .map(|needle| s.contains(needle))
                .unwrap_or(false)),
            Value::Array(items) => Ok(items.contains(right)),
            _ => Ok(false),
        },
        _ => Err(Error::Node(format!(
            "Unsupported switch operator '{}'",
            operator
        ))),
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
    async fn test_switch_matches_case() {
        let node = SwitchNode::new();
        let config = json!({
            "field": "{{ input.type }}",
            "cases": [
                {"value": "order", "branch": "process-order"},
                {"value": "refund", "branch": "process-refund"}
            ],
            "default_branch": "unknown-type"
        });
        let ctx = NodeContext::new("exec-1", "wf").with_input(json!({"type": "refund"}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["selected_branch"], "process-refund");
        assert_eq!(result.data["matched_case_index"], 1);
    }

    #[tokio::test]
    async fn test_switch_falls_back_to_default() {
        let node = SwitchNode::new();
        let config = json!({
            "field": "input.type",
            "cases": [
                {"value": "order", "branch": "process-order"}
            ],
            "default_branch": "unknown-type"
        });
        let ctx = NodeContext::new("exec-1", "wf").with_input(json!({"type": "inquiry"}));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["selected_branch"], "unknown-type");
        assert!(result.data["matched_case_index"].is_null());
    }
}
