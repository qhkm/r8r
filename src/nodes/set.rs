//! Set node - add or update fields in object data.

use chrono::Utc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Set node implementation.
pub struct SetNode;

impl SetNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SetNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SetConfig {
    fields: Vec<SetField>,
}

#[derive(Debug, Deserialize)]
struct SetField {
    name: String,
    value: Value,
}

#[async_trait]
impl Node for SetNode {
    fn node_type(&self) -> &str {
        "set"
    }

    fn description(&self) -> &str {
        "Set or update fields in object data"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SetConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid set config: {}", e)))?;

        if config.fields.is_empty() {
            return Err(Error::Node(
                "Set node requires at least one field assignment".to_string(),
            ));
        }

        let mut output = match &ctx.input {
            Value::Object(obj) => obj.clone(),
            _ => Map::new(),
        };

        for assignment in &config.fields {
            if assignment.name.trim().is_empty() {
                return Err(Error::Node(
                    "Set node field name cannot be empty".to_string(),
                ));
            }

            let rendered = render_value(&assignment.value, ctx);
            set_path_value(&mut output, &assignment.name, rendered);
        }

        Ok(NodeResult::with_metadata(
            Value::Object(output),
            json!({
                "fields_set": config.fields.len(),
            }),
        ))
    }
}

fn render_value(value: &Value, ctx: &NodeContext) -> Value {
    match value {
        Value::String(s) => render_string_value(s, ctx),
        Value::Array(arr) => Value::Array(arr.iter().map(|v| render_value(v, ctx)).collect()),
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(k.clone(), render_value(v, ctx));
            }
            Value::Object(out)
        }
        _ => value.clone(),
    }
}

fn render_string_value(template: &str, ctx: &NodeContext) -> Value {
    let full_template = regex_lite::Regex::new(r"^\s*\{\{\s*([^{}]+?)\s*\}\}\s*$").unwrap();
    if let Some(captures) = full_template.captures(template) {
        let expr = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        return resolve_expression(expr, ctx);
    }

    let template_re = regex_lite::Regex::new(r"\{\{\s*(.+?)\s*\}\}").unwrap();
    let rendered = template_re
        .replace_all(template, |caps: &regex_lite::Captures| {
            let expr = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let value = resolve_expression(expr, ctx);
            match value {
                Value::String(s) => s,
                other => other.to_string(),
            }
        })
        .to_string();

    Value::String(rendered)
}

fn resolve_expression(expr: &str, ctx: &NodeContext) -> Value {
    let expr = expr.trim();

    if expr == "now()" {
        return Value::String(Utc::now().to_rfc3339());
    }

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

    Value::Null
}

fn set_path_value(root: &mut Map<String, Value>, path: &str, value: Value) {
    let segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return;
    }

    let mut current = root;
    for segment in &segments[..segments.len() - 1] {
        let entry = current
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));

        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }

        if let Some(map) = entry.as_object_mut() {
            current = map;
        } else {
            return;
        }
    }

    current.insert(segments[segments.len() - 1].to_string(), value);
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
    async fn test_set_add_fields() {
        let node = SetNode::new();
        let config = json!({
            "fields": [
                {"name": "status", "value": "processed"},
                {"name": "full_name", "value": "{{ input.first_name }} {{ input.last_name }}"}
            ]
        });

        let ctx = NodeContext::new("exec", "wf").with_input(json!({
            "first_name": "Nur",
            "last_name": "Alya"
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["status"], "processed");
        assert_eq!(result.data["full_name"], "Nur Alya");
    }

    #[tokio::test]
    async fn test_set_nested_field() {
        let node = SetNode::new();
        let config = json!({
            "fields": [
                {"name": "meta.source", "value": "r8r"},
                {"name": "meta.raw", "value": "{{ input }}"}
            ]
        });

        let ctx = NodeContext::new("exec", "wf").with_input(json!({"order_id": "A-1"}));
        let result = node.execute(&config, &ctx).await.unwrap();

        assert_eq!(result.data["meta"]["source"], "r8r");
        assert_eq!(result.data["meta"]["raw"], json!({"order_id": "A-1"}));
    }
}
