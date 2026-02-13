//! Switch node - multi-branch routing based on conditions.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Switch node for conditional branching.
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
    /// Expression to evaluate for switching.
    expression: String,

    /// Cases to match against.
    cases: Vec<SwitchCase>,

    /// Default output if no case matches.
    #[serde(default)]
    default: Option<Value>,

    /// Mode: "first" (first match) or "all" (all matches).
    #[serde(default = "default_mode")]
    mode: String,
}

#[derive(Debug, Deserialize)]
struct SwitchCase {
    /// Value to match or condition expression.
    #[serde(alias = "when")]
    value: Value,

    /// Output when this case matches.
    output: Value,

    /// Optional label for the case.
    #[serde(default)]
    label: Option<String>,
}

fn default_mode() -> String {
    "first".to_string()
}

#[async_trait]
impl Node for SwitchNode {
    fn node_type(&self) -> &str {
        "switch"
    }

    fn description(&self) -> &str {
        "Route execution based on conditions"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SwitchConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid switch config: {}", e)))?;

        // Evaluate the switch expression
        let switch_value = evaluate_expression(&config.expression, &ctx.input)?;

        let mut matched_cases: Vec<Value> = Vec::new();
        let mut matched_labels: Vec<String> = Vec::new();

        for (i, case) in config.cases.iter().enumerate() {
            let matches = match &case.value {
                // String starting with "==" is an equality check
                Value::String(s) if s.starts_with("==") => {
                    let compare_val = s.trim_start_matches("==").trim();
                    switch_value.to_string().trim_matches('"') == compare_val
                }
                // String starting with expression operators
                Value::String(s) if s.starts_with('>') || s.starts_with('<') || s.starts_with('!') => {
                    evaluate_condition_expr(s, &switch_value)?
                }
                // String is a Rhai expression returning bool
                Value::String(s) if s.contains("input") || s.contains("value") => {
                    let expr_result = evaluate_bool_expression(s, &ctx.input, &switch_value)?;
                    expr_result
                }
                // Direct value comparison
                _ => switch_value == case.value,
            };

            if matches {
                let label = case.label.clone().unwrap_or_else(|| format!("case_{}", i));
                matched_labels.push(label);
                matched_cases.push(case.output.clone());

                if config.mode == "first" {
                    break;
                }
            }
        }

        // Build output
        let output = if matched_cases.is_empty() {
            json!({
                "matched": false,
                "value": switch_value,
                "output": config.default.unwrap_or(Value::Null),
                "branch": "default",
            })
        } else if config.mode == "first" {
            json!({
                "matched": true,
                "value": switch_value,
                "output": matched_cases[0],
                "branch": matched_labels[0],
            })
        } else {
            json!({
                "matched": true,
                "value": switch_value,
                "outputs": matched_cases,
                "branches": matched_labels,
                "match_count": matched_cases.len(),
            })
        };

        Ok(NodeResult::new(output))
    }
}

fn evaluate_expression(expr: &str, input: &Value) -> Result<Value> {
    let engine = rhai::Engine::new();
    
    // Register input as a variable
    let input_str = serde_json::to_string(input).unwrap_or_default();
    let scope_expr = format!(
        r#"let input = parse_json("{}"); {}"#,
        input_str.replace('\\', "\\\\").replace('"', "\\\""),
        expr
    );

    // Try to evaluate and convert result
    let result: rhai::Dynamic = engine
        .eval(&scope_expr)
        .or_else(|_| {
            // Fallback: try as simple expression
            engine.eval_expression(expr)
        })
        .map_err(|e| Error::Node(format!("Expression error: {}", e)))?;

    // Convert Rhai Dynamic to JSON Value
    dynamic_to_value(result)
}

fn evaluate_condition_expr(expr: &str, value: &Value) -> Result<bool> {
    let num_value = match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    };

    if let Some(rest) = expr.strip_prefix(">=") {
        let threshold: f64 = rest.trim().parse().map_err(|_| Error::Node("Invalid number".to_string()))?;
        Ok(num_value >= threshold)
    } else if let Some(rest) = expr.strip_prefix("<=") {
        let threshold: f64 = rest.trim().parse().map_err(|_| Error::Node("Invalid number".to_string()))?;
        Ok(num_value <= threshold)
    } else if let Some(rest) = expr.strip_prefix('>') {
        let threshold: f64 = rest.trim().parse().map_err(|_| Error::Node("Invalid number".to_string()))?;
        Ok(num_value > threshold)
    } else if let Some(rest) = expr.strip_prefix('<') {
        let threshold: f64 = rest.trim().parse().map_err(|_| Error::Node("Invalid number".to_string()))?;
        Ok(num_value < threshold)
    } else if let Some(rest) = expr.strip_prefix("!=") {
        let compare = rest.trim();
        Ok(value.to_string().trim_matches('"') != compare)
    } else {
        Ok(false)
    }
}

fn evaluate_bool_expression(expr: &str, input: &Value, switch_value: &Value) -> Result<bool> {
    let engine = rhai::Engine::new();
    
    let input_str = serde_json::to_string(input).unwrap_or_default();
    let value_str = serde_json::to_string(switch_value).unwrap_or_default();
    
    let full_expr = format!(
        r#"let input = parse_json("{}"); let value = parse_json("{}"); {}"#,
        input_str.replace('\\', "\\\\").replace('"', "\\\""),
        value_str.replace('\\', "\\\\").replace('"', "\\\""),
        expr
    );

    engine
        .eval::<bool>(&full_expr)
        .map_err(|e| Error::Node(format!("Condition error: {}", e)))
}

fn dynamic_to_value(d: rhai::Dynamic) -> Result<Value> {
    if d.is::<i64>() {
        Ok(json!(d.as_int().unwrap()))
    } else if d.is::<f64>() {
        Ok(json!(d.as_float().unwrap()))
    } else if d.is::<bool>() {
        Ok(json!(d.as_bool().unwrap()))
    } else if d.is::<String>() || d.is::<rhai::ImmutableString>() {
        Ok(json!(d.into_string().unwrap()))
    } else {
        Ok(json!(d.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_switch_exact_match() {
        let node = SwitchNode::new();
        let config = json!({
            "expression": "input.status",
            "cases": [
                { "value": "active", "output": { "action": "process" }, "label": "active" },
                { "value": "pending", "output": { "action": "wait" }, "label": "pending" },
            ],
            "default": { "action": "skip" }
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({ "status": "active" }));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["matched"], true);
        assert_eq!(result.data["branch"], "active");
        assert_eq!(result.data["output"]["action"], "process");
    }

    #[tokio::test]
    async fn test_switch_default() {
        let node = SwitchNode::new();
        let config = json!({
            "expression": "input.status",
            "cases": [
                { "value": "active", "output": "active_output" },
            ],
            "default": "default_output"
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({ "status": "unknown" }));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["matched"], false);
        assert_eq!(result.data["branch"], "default");
        assert_eq!(result.data["output"], "default_output");
    }

    #[tokio::test]
    async fn test_switch_numeric() {
        let node = SwitchNode::new();
        let config = json!({
            "expression": "input.count",
            "cases": [
                { "value": 0, "output": "zero", "label": "zero" },
                { "value": 1, "output": "one", "label": "one" },
                { "value": 2, "output": "two", "label": "two" },
            ]
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({ "count": 1 }));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["matched"], true);
        assert_eq!(result.data["branch"], "one");
    }

    #[tokio::test]
    async fn test_switch_comparison() {
        let node = SwitchNode::new();
        let config = json!({
            "expression": "input.score",
            "cases": [
                { "value": ">=90", "output": "A", "label": "grade_a" },
                { "value": ">=80", "output": "B", "label": "grade_b" },
                { "value": ">=70", "output": "C", "label": "grade_c" },
            ],
            "default": "F"
        });
        let ctx = NodeContext::new("exec-1", "test")
            .with_input(json!({ "score": 85 }));

        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["matched"], true);
        assert_eq!(result.data["output"], "B");
    }
}
