/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

use std::collections::HashMap;

use serde_json::{json, Value};

use super::definition::ParamDef;

/// Validates user-supplied parameters against their definitions and resolves
/// defaults and type coercions.
///
/// For each defined parameter:
/// - If provided and not null: validate enum constraints (if any), coerce the
///   value to the expected type, and insert into the result map.
/// - If missing/null and required: return an error.
/// - If missing/null and not required: apply the default value if one is
///   defined; otherwise omit from the result.
pub fn validate_and_resolve_params(
    defs: &HashMap<String, ParamDef>,
    input: &Value,
) -> Result<HashMap<String, Value>, String> {
    let empty = serde_json::Map::new();
    let input_obj = input.as_object().unwrap_or(&empty);
    let mut resolved = HashMap::new();

    for (name, def) in defs {
        let value = input_obj.get(name).cloned();
        match value {
            Some(v) if !v.is_null() => {
                // Validate enum constraint
                if let Some(enum_values) = &def.enum_values {
                    if let Some(s) = v.as_str() {
                        if !enum_values.contains(&s.to_string()) {
                            return Err(format!(
                                "Parameter '{}' value '{}' not in allowed values: {:?}",
                                name, s, enum_values
                            ));
                        }
                    }
                }
                // Coerce to expected type
                let coerced = coerce_type(&v, def.param_type.as_deref());
                resolved.insert(name.clone(), coerced);
            }
            _ => {
                if def.required {
                    return Err(format!("Required parameter '{}' is missing", name));
                }
                if let Some(default) = &def.default {
                    resolved.insert(name.clone(), default.clone());
                }
            }
        }
    }

    Ok(resolved)
}

/// Attempts to coerce a JSON value to the expected type.
///
/// Currently handles:
/// - `"integer"`: string representations of integers (e.g. `"42"` -> `42`)
/// - `"boolean"`: string `"true"`/`"false"` -> `true`/`false`
///
/// All other types (including `None`) pass through unchanged.
fn coerce_type(value: &Value, expected_type: Option<&str>) -> Value {
    match expected_type {
        Some("integer") => {
            if let Some(s) = value.as_str() {
                if let Ok(n) = s.parse::<i64>() {
                    return json!(n);
                }
            }
            value.clone()
        }
        Some("boolean") => {
            if let Some(s) = value.as_str() {
                match s {
                    "true" => return json!(true),
                    "false" => return json!(false),
                    _ => {}
                }
            }
            value.clone()
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a ParamDef quickly for tests.
    fn param(
        required: bool,
        param_type: Option<&str>,
        default: Option<Value>,
        enum_values: Option<Vec<String>>,
    ) -> ParamDef {
        ParamDef {
            required,
            param_type: param_type.map(String::from),
            description: None,
            default,
            enum_values,
        }
    }

    #[test]
    fn test_valid_params() {
        let mut defs = HashMap::new();
        defs.insert("owner".into(), param(true, Some("string"), None, None));
        defs.insert(
            "page".into(),
            param(false, Some("integer"), Some(json!(1)), None),
        );

        let input = json!({ "owner": "acme" });
        let result = validate_and_resolve_params(&defs, &input).unwrap();

        assert_eq!(result.get("owner").unwrap(), &json!("acme"));
        // Default applied for missing optional param
        assert_eq!(result.get("page").unwrap(), &json!(1));
    }

    #[test]
    fn test_missing_required_param() {
        let mut defs = HashMap::new();
        defs.insert("owner".into(), param(true, Some("string"), None, None));

        let input = json!({});
        let result = validate_and_resolve_params(&defs, &input);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Required parameter 'owner'"));
    }

    #[test]
    fn test_invalid_enum_value() {
        let mut defs = HashMap::new();
        defs.insert(
            "status".into(),
            param(
                true,
                Some("string"),
                None,
                Some(vec![
                    "active".into(),
                    "inactive".into(),
                    "pending".into(),
                ]),
            ),
        );

        let input = json!({ "status": "deleted" });
        let result = validate_and_resolve_params(&defs, &input);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("not in allowed values"));
        assert!(err.contains("deleted"));
    }

    #[test]
    fn test_type_coercion_integer() {
        let mut defs = HashMap::new();
        defs.insert("page".into(), param(false, Some("integer"), None, None));

        let input = json!({ "page": "42" });
        let result = validate_and_resolve_params(&defs, &input).unwrap();

        assert_eq!(result.get("page").unwrap(), &json!(42));
    }

    #[test]
    fn test_array_param() {
        let mut defs = HashMap::new();
        defs.insert("tags".into(), param(false, Some("array"), None, None));

        let input = json!({ "tags": ["rust", "cli"] });
        let result = validate_and_resolve_params(&defs, &input).unwrap();

        assert_eq!(result.get("tags").unwrap(), &json!(["rust", "cli"]));
    }

    #[test]
    fn test_defaults_not_overridden() {
        let mut defs = HashMap::new();
        defs.insert(
            "page".into(),
            param(false, Some("integer"), Some(json!(1)), None),
        );

        // Explicit value should win over the default
        let input = json!({ "page": 5 });
        let result = validate_and_resolve_params(&defs, &input).unwrap();

        assert_eq!(result.get("page").unwrap(), &json!(5));
    }
}
