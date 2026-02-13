//! Dedupe node - remove duplicate items from arrays.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// Dedupe node for removing duplicates from arrays.
pub struct DedupeNode;

impl DedupeNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DedupeNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum DedupeMode {
    /// Keep first occurrence of each duplicate
    #[default]
    KeepFirst,
    /// Keep last occurrence of each duplicate
    KeepLast,
    /// Remove all duplicates (keep only unique items)
    RemoveAll,
}

#[derive(Debug, Deserialize)]
struct DedupeConfig {
    /// Field to use for comparison (for objects). If not set, compares entire values.
    #[serde(default)]
    field: Option<String>,

    /// Fields to use for comparison (for multi-field deduplication)
    #[serde(default)]
    fields: Option<Vec<String>>,

    /// Mode for handling duplicates
    #[serde(default)]
    mode: DedupeMode,

    /// Whether comparison should be case-insensitive (for string values)
    #[serde(default)]
    case_insensitive: bool,

    /// Field path in input containing the array to dedupe
    #[serde(default)]
    input_field: Option<String>,

    /// Output field name (defaults to "items")
    #[serde(default)]
    output_field: Option<String>,
}

#[async_trait]
impl Node for DedupeNode {
    fn node_type(&self) -> &str {
        "dedupe"
    }

    fn description(&self) -> &str {
        "Remove duplicate items from an array"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DedupeConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid dedupe config: {}", e)))?;

        // Get the array to dedupe
        let items = get_input_array(&ctx.input, config.input_field.as_deref())?;
        let original_count = items.len();

        // Perform deduplication
        let (deduped, duplicate_count) = dedupe_items(
            &items,
            config.field.as_deref(),
            config.fields.as_deref(),
            &config.mode,
            config.case_insensitive,
        );

        let output_field = config.output_field.as_deref().unwrap_or("items");

        Ok(NodeResult::with_metadata(
            json!({ output_field: deduped }),
            json!({
                "original_count": original_count,
                "deduped_count": deduped.len(),
                "duplicates_removed": duplicate_count,
            }),
        ))
    }
}

/// Get input array from context.
fn get_input_array(input: &Value, field: Option<&str>) -> Result<Vec<Value>> {
    let value = if let Some(field_path) = field {
        get_nested_field(input, field_path)?
    } else {
        input.clone()
    };

    match value {
        Value::Array(arr) => Ok(arr),
        _ => Err(Error::Node("Input must be an array".to_string())),
    }
}

/// Get nested field from value.
fn get_nested_field(value: &Value, path: &str) -> Result<Value> {
    let mut current = value.clone();
    for part in path.split('.') {
        current = match current {
            Value::Object(obj) => obj
                .get(part)
                .cloned()
                .ok_or_else(|| Error::Node(format!("Field '{}' not found", path)))?,
            Value::Array(arr) => {
                let idx: usize = part
                    .parse()
                    .map_err(|_| Error::Node(format!("Invalid array index: {}", part)))?;
                arr.get(idx)
                    .cloned()
                    .ok_or_else(|| Error::Node(format!("Index {} out of bounds", idx)))?
            }
            _ => return Err(Error::Node(format!("Cannot index into {:?}", current))),
        };
    }
    Ok(current)
}

/// Extract comparison key from a value.
fn extract_key(
    value: &Value,
    field: Option<&str>,
    fields: Option<&[String]>,
    case_insensitive: bool,
) -> String {
    // Multi-field key
    if let Some(field_list) = fields {
        let parts: Vec<String> = field_list
            .iter()
            .map(|f| {
                let v = get_field_value(value, f);
                normalize_value(&v, case_insensitive)
            })
            .collect();
        return parts.join("|");
    }

    // Single field key
    if let Some(field_name) = field {
        let v = get_field_value(value, field_name);
        return normalize_value(&v, case_insensitive);
    }

    // Full value comparison
    normalize_value(value, case_insensitive)
}

/// Get field value from an object.
fn get_field_value(value: &Value, field: &str) -> Value {
    match value {
        Value::Object(obj) => {
            let mut current: Value = Value::Object(obj.clone());
            for part in field.split('.') {
                current = match current {
                    Value::Object(o) => o.get(part).cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
            }
            current
        }
        _ => Value::Null,
    }
}

/// Normalize a value for comparison.
fn normalize_value(value: &Value, case_insensitive: bool) -> String {
    match value {
        Value::String(s) if case_insensitive => s.to_lowercase(),
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

/// Deduplicate items based on config.
fn dedupe_items(
    items: &[Value],
    field: Option<&str>,
    fields: Option<&[String]>,
    mode: &DedupeMode,
    case_insensitive: bool,
) -> (Vec<Value>, usize) {
    match mode {
        DedupeMode::KeepFirst => dedupe_keep_first(items, field, fields, case_insensitive),
        DedupeMode::KeepLast => dedupe_keep_last(items, field, fields, case_insensitive),
        DedupeMode::RemoveAll => dedupe_remove_all(items, field, fields, case_insensitive),
    }
}

/// Keep first occurrence of each duplicate.
fn dedupe_keep_first(
    items: &[Value],
    field: Option<&str>,
    fields: Option<&[String]>,
    case_insensitive: bool,
) -> (Vec<Value>, usize) {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut duplicates = 0;

    for item in items {
        let key = extract_key(item, field, fields, case_insensitive);
        if seen.insert(key) {
            result.push(item.clone());
        } else {
            duplicates += 1;
        }
    }

    (result, duplicates)
}

/// Keep last occurrence of each duplicate.
fn dedupe_keep_last(
    items: &[Value],
    field: Option<&str>,
    fields: Option<&[String]>,
    case_insensitive: bool,
) -> (Vec<Value>, usize) {
    use std::collections::HashMap;

    // First pass: find last index of each key
    let mut last_index: HashMap<String, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let key = extract_key(item, field, fields, case_insensitive);
        last_index.insert(key, i);
    }

    // Second pass: keep only items at their last index
    let mut result = Vec::new();
    let mut duplicates = 0;

    for (i, item) in items.iter().enumerate() {
        let key = extract_key(item, field, fields, case_insensitive);
        if last_index.get(&key) == Some(&i) {
            result.push(item.clone());
        } else {
            duplicates += 1;
        }
    }

    (result, duplicates)
}

/// Remove all items that have duplicates.
fn dedupe_remove_all(
    items: &[Value],
    field: Option<&str>,
    fields: Option<&[String]>,
    case_insensitive: bool,
) -> (Vec<Value>, usize) {
    use std::collections::HashMap;

    // Count occurrences
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        let key = extract_key(item, field, fields, case_insensitive);
        *counts.entry(key).or_insert(0) += 1;
    }

    // Keep only items that appear exactly once
    let mut result = Vec::new();
    let mut removed = 0;

    for item in items {
        let key = extract_key(item, field, fields, case_insensitive);
        if counts.get(&key) == Some(&1) {
            result.push(item.clone());
        } else {
            removed += 1;
        }
    }

    (result, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedupe_simple_array() {
        let items = vec![json!(1), json!(2), json!(2), json!(3), json!(1)];
        let (result, duplicates) = dedupe_keep_first(&items, None, None, false);
        assert_eq!(result, vec![json!(1), json!(2), json!(3)]);
        assert_eq!(duplicates, 2);
    }

    #[test]
    fn test_dedupe_strings() {
        let items = vec![
            json!("apple"),
            json!("banana"),
            json!("apple"),
            json!("cherry"),
        ];
        let (result, duplicates) = dedupe_keep_first(&items, None, None, false);
        assert_eq!(
            result,
            vec![json!("apple"), json!("banana"), json!("cherry")]
        );
        assert_eq!(duplicates, 1);
    }

    #[test]
    fn test_dedupe_case_insensitive() {
        let items = vec![
            json!("Apple"),
            json!("APPLE"),
            json!("apple"),
            json!("Banana"),
        ];
        let (result, duplicates) = dedupe_keep_first(&items, None, None, true);
        assert_eq!(result.len(), 2);
        assert_eq!(duplicates, 2);
    }

    #[test]
    fn test_dedupe_by_field() {
        let items = vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 2, "name": "Bob"}),
            json!({"id": 1, "name": "Alice Updated"}),
            json!({"id": 3, "name": "Charlie"}),
        ];
        let (result, duplicates) = dedupe_keep_first(&items, Some("id"), None, false);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["name"], "Alice");
        assert_eq!(duplicates, 1);
    }

    #[test]
    fn test_dedupe_keep_last() {
        let items = vec![
            json!({"id": 1, "name": "Alice"}),
            json!({"id": 2, "name": "Bob"}),
            json!({"id": 1, "name": "Alice Updated"}),
        ];
        let (result, duplicates) = dedupe_keep_last(&items, Some("id"), None, false);
        assert_eq!(result.len(), 2);
        // Bob comes first (position 1), then Alice Updated (position 2)
        assert_eq!(result[0]["name"], "Bob");
        assert_eq!(result[1]["name"], "Alice Updated");
        assert_eq!(duplicates, 1);
    }

    #[test]
    fn test_dedupe_remove_all() {
        let items = vec![json!(1), json!(2), json!(2), json!(3), json!(1), json!(4)];
        let (result, removed) = dedupe_remove_all(&items, None, None, false);
        assert_eq!(result, vec![json!(3), json!(4)]);
        assert_eq!(removed, 4);
    }

    #[test]
    fn test_dedupe_by_multiple_fields() {
        let items = vec![
            json!({"first": "John", "last": "Doe"}),
            json!({"first": "Jane", "last": "Doe"}),
            json!({"first": "John", "last": "Doe"}),
            json!({"first": "John", "last": "Smith"}),
        ];
        let fields = vec!["first".to_string(), "last".to_string()];
        let (result, duplicates) = dedupe_keep_first(&items, None, Some(&fields), false);
        assert_eq!(result.len(), 3);
        assert_eq!(duplicates, 1);
    }

    #[test]
    fn test_dedupe_nested_field() {
        let items = vec![
            json!({"user": {"id": 1}, "name": "Alice"}),
            json!({"user": {"id": 2}, "name": "Bob"}),
            json!({"user": {"id": 1}, "name": "Alice Clone"}),
        ];
        let (result, duplicates) = dedupe_keep_first(&items, Some("user.id"), None, false);
        assert_eq!(result.len(), 2);
        assert_eq!(duplicates, 1);
    }

    #[test]
    fn test_extract_key_simple() {
        let value = json!("hello");
        let key = extract_key(&value, None, None, false);
        assert_eq!(key, "hello");
    }

    #[test]
    fn test_extract_key_field() {
        let value = json!({"id": 123, "name": "test"});
        let key = extract_key(&value, Some("id"), None, false);
        assert_eq!(key, "123");
    }

    #[test]
    fn test_extract_key_multiple_fields() {
        let value = json!({"first": "John", "last": "Doe", "age": 30});
        let fields = vec!["first".to_string(), "last".to_string()];
        let key = extract_key(&value, None, Some(&fields), false);
        assert_eq!(key, "John|Doe");
    }

    #[tokio::test]
    async fn test_dedupe_node_basic() {
        let node = DedupeNode::new();
        let config = json!({});
        let ctx = NodeContext::new("exec-1", "test").with_input(json!([1, 2, 2, 3, 1]));

        let result = node.execute(&config, &ctx).await.unwrap();
        let items = result.data["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(result.metadata["duplicates_removed"], 2);
    }

    #[tokio::test]
    async fn test_dedupe_node_by_field() {
        let node = DedupeNode::new();
        let config = json!({
            "field": "email"
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!([
            {"email": "a@example.com", "name": "Alice"},
            {"email": "b@example.com", "name": "Bob"},
            {"email": "a@example.com", "name": "Alice 2"},
        ]));

        let result = node.execute(&config, &ctx).await.unwrap();
        let items = result.data["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["name"], "Alice");
    }

    #[tokio::test]
    async fn test_dedupe_node_keep_last() {
        let node = DedupeNode::new();
        let config = json!({
            "field": "id",
            "mode": "keep_last"
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!([
            {"id": 1, "version": "old"},
            {"id": 1, "version": "new"},
        ]));

        let result = node.execute(&config, &ctx).await.unwrap();
        let items = result.data["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["version"], "new");
    }

    #[tokio::test]
    async fn test_dedupe_node_case_insensitive() {
        let node = DedupeNode::new();
        let config = json!({
            "case_insensitive": true
        });
        let ctx =
            NodeContext::new("exec-1", "test").with_input(json!(["Apple", "APPLE", "Banana"]));

        let result = node.execute(&config, &ctx).await.unwrap();
        let items = result.data["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_dedupe_node_from_field() {
        let node = DedupeNode::new();
        let config = json!({
            "input_field": "data.users",
            "field": "id"
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!({
            "data": {
                "users": [
                    {"id": 1, "name": "Alice"},
                    {"id": 2, "name": "Bob"},
                    {"id": 1, "name": "Alice Dup"},
                ]
            }
        }));

        let result = node.execute(&config, &ctx).await.unwrap();
        let items = result.data["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_dedupe_node_custom_output() {
        let node = DedupeNode::new();
        let config = json!({
            "output_field": "unique_items"
        });
        let ctx = NodeContext::new("exec-1", "test").with_input(json!([1, 1, 2]));

        let result = node.execute(&config, &ctx).await.unwrap();
        assert!(result.data.get("unique_items").is_some());
    }
}
