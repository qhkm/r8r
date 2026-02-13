//! Workflow parameter validation and processing.

use serde_json::Value;

use crate::error::{Error, Result};
use super::types::{InputDefinition, Workflow};

/// Validate and process workflow input against parameter definitions.
pub fn validate_parameters(workflow: &Workflow, input: &Value) -> Result<Value> {
    if workflow.inputs.is_empty() {
        // No parameters defined, pass through input as-is
        return Ok(input.clone());
    }

    let input_obj = input.as_object();
    let mut result = serde_json::Map::new();
    let mut errors = Vec::new();

    for (name, def) in &workflow.inputs {
        let value = input_obj.and_then(|obj| obj.get(name));

        match validate_parameter(name, def, value) {
            Ok(validated_value) => {
                result.insert(name.clone(), validated_value);
            }
            Err(e) => {
                errors.push(e.to_string());
            }
        }
    }

    if !errors.is_empty() {
        return Err(Error::Validation(format!(
            "Parameter validation failed:\n  - {}",
            errors.join("\n  - ")
        )));
    }

    // Merge any extra input fields that aren't defined as parameters
    if let Some(obj) = input_obj {
        for (key, value) in obj {
            if !result.contains_key(key) {
                result.insert(key.clone(), value.clone());
            }
        }
    }

    Ok(Value::Object(result))
}

/// Validate a single parameter value.
fn validate_parameter(
    name: &str,
    def: &InputDefinition,
    value: Option<&Value>,
) -> Result<Value> {
    // Check if required
    match value {
        None | Some(Value::Null) => {
            if def.required {
                if let Some(default) = &def.default {
                    return Ok(default.clone());
                }
                return Err(Error::Validation(format!(
                    "Missing required parameter: {}",
                    name
                )));
            }
            // Not required, use default or null
            return Ok(def.default.clone().unwrap_or(Value::Null));
        }
        Some(v) => {
            // Validate type
            validate_type(name, &def.input_type, v)?;
            Ok(v.clone())
        }
    }
}

/// Validate value against expected type.
fn validate_type(name: &str, expected_type: &str, value: &Value) -> Result<()> {
    let valid = match expected_type {
        "string" => value.is_string(),
        "number" | "integer" | "float" => value.is_number(),
        "boolean" | "bool" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "any" => true,
        _ => true, // Unknown type, allow anything
    };

    if !valid {
        return Err(Error::Validation(format!(
            "Parameter '{}' expected type '{}', got '{}'",
            name,
            expected_type,
            json_type_name(value)
        )));
    }

    Ok(())
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Get parameter definitions with their requirements.
pub fn get_parameter_info(workflow: &Workflow) -> Vec<ParameterInfo> {
    workflow
        .inputs
        .iter()
        .map(|(name, def)| ParameterInfo {
            name: name.clone(),
            input_type: def.input_type.clone(),
            required: def.required,
            default: def.default.clone(),
            description: def.description.clone(),
        })
        .collect()
}

/// Information about a workflow parameter.
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    pub input_type: String,
    pub required: bool,
    pub default: Option<Value>,
    pub description: String,
}

/// Parse CLI parameter strings into a JSON object.
///
/// Accepts parameters in format: key=value
/// Values are parsed as JSON if valid, otherwise treated as strings.
pub fn parse_cli_params(params: &[(String, String)]) -> Result<Value> {
    let mut result = serde_json::Map::new();

    for (key, value) in params {
        // Try to parse as JSON first
        let json_value = serde_json::from_str(value).unwrap_or_else(|_| {
            // If not valid JSON, treat as string
            Value::String(value.clone())
        });
        result.insert(key.clone(), json_value);
    }

    Ok(Value::Object(result))
}

/// Merge CLI parameters with existing input.
pub fn merge_params(base: &Value, params: &Value) -> Value {
    match (base, params) {
        (Value::Object(base_obj), Value::Object(params_obj)) => {
            let mut merged = base_obj.clone();
            for (key, value) in params_obj {
                merged.insert(key.clone(), value.clone());
            }
            Value::Object(merged)
        }
        (_, params) if params.is_object() && !params.as_object().unwrap().is_empty() => {
            params.clone()
        }
        (base, _) => base.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn make_workflow_with_params() -> Workflow {
        let mut inputs = HashMap::new();
        inputs.insert(
            "name".to_string(),
            InputDefinition {
                input_type: "string".to_string(),
                required: true,
                default: None,
                description: "User name".to_string(),
            },
        );
        inputs.insert(
            "count".to_string(),
            InputDefinition {
                input_type: "number".to_string(),
                required: false,
                default: Some(json!(10)),
                description: "Item count".to_string(),
            },
        );
        inputs.insert(
            "enabled".to_string(),
            InputDefinition {
                input_type: "boolean".to_string(),
                required: false,
                default: Some(json!(true)),
                description: "Enable feature".to_string(),
            },
        );

        Workflow {
            name: "test".to_string(),
            description: String::new(),
            version: 1,
            triggers: vec![],
            inputs,
            input_schema: None,
            variables: HashMap::new(),
            depends_on_workflows: vec![],
            nodes: vec![],
            settings: Default::default(),
        }
    }

    #[test]
    fn test_validate_required_param() {
        let workflow = make_workflow_with_params();

        // Missing required param
        let input = json!({});
        let result = validate_parameters(&workflow, &input);
        assert!(result.is_err());

        // With required param
        let input = json!({"name": "Alice"});
        let result = validate_parameters(&workflow, &input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_values() {
        let workflow = make_workflow_with_params();

        let input = json!({"name": "Alice"});
        let result = validate_parameters(&workflow, &input).unwrap();

        assert_eq!(result["name"], "Alice");
        assert_eq!(result["count"], 10);
        assert_eq!(result["enabled"], true);
    }

    #[test]
    fn test_type_validation() {
        let workflow = make_workflow_with_params();

        // Wrong type for count
        let input = json!({"name": "Alice", "count": "not a number"});
        let result = validate_parameters(&workflow, &input);
        assert!(result.is_err());

        // Correct type
        let input = json!({"name": "Alice", "count": 5});
        let result = validate_parameters(&workflow, &input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_cli_params() {
        let params = vec![
            ("name".to_string(), "Alice".to_string()),
            ("count".to_string(), "42".to_string()),
            ("enabled".to_string(), "true".to_string()),
            ("data".to_string(), r#"{"key": "value"}"#.to_string()),
        ];

        let result = parse_cli_params(&params).unwrap();

        assert_eq!(result["name"], "Alice");
        assert_eq!(result["count"], 42);
        assert_eq!(result["enabled"], true);
        assert_eq!(result["data"]["key"], "value");
    }

    #[test]
    fn test_merge_params() {
        let base = json!({"a": 1, "b": 2});
        let params = json!({"b": 3, "c": 4});

        let merged = merge_params(&base, &params);

        assert_eq!(merged["a"], 1);
        assert_eq!(merged["b"], 3); // Overwritten
        assert_eq!(merged["c"], 4);
    }
}
