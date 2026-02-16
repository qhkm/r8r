//! Shared template rendering utilities.
//!
//! This module provides common template functionality used by multiple nodes
//! (http, agent, etc.) to reduce code duplication.

use std::sync::OnceLock;

use regex_lite::Regex;
use serde_json::Value;

use super::types::NodeContext;

/// Get the regex for matching environment variable templates: {{ env.VAR }}
pub fn env_template_regex() -> &'static Regex {
    static ENV_TEMPLATE_REGEX: OnceLock<Regex> = OnceLock::new();
    ENV_TEMPLATE_REGEX.get_or_init(|| Regex::new(r"\{\{\s*env\.(\w+)\s*\}\}").expect("valid regex"))
}

/// Check if an environment variable is safe to expose in templates.
///
/// By default, only R8R_* prefixed variables are allowed.
/// Additional variables can be whitelisted via R8R_ALLOWED_ENV_VARS
/// (comma-separated list).
pub fn is_safe_env_var(var_name: &str) -> bool {
    // Always allow R8R_ prefixed variables
    if var_name.starts_with("R8R_") {
        return true;
    }

    // Check additional allowlist
    if let Ok(allowed) = std::env::var("R8R_ALLOWED_ENV_VARS") {
        let allowed_vars: Vec<&str> = allowed.split(',').map(|s| s.trim()).collect();
        if allowed_vars.contains(&var_name) {
            return true;
        }
    }

    false
}

/// Render environment variable templates in a string.
///
/// Replaces {{ env.VAR }} with the value of environment variable VAR,
/// but only for safe variables (R8R_* or in allowlist).
pub fn render_env_vars(input: &str) -> String {
    let env_regex = env_template_regex();
    env_regex
        .replace_all(input, |caps: &regex_lite::Captures| {
            let var_name = &caps[1];
            if is_safe_env_var(var_name) {
                std::env::var(var_name).unwrap_or_default()
            } else {
                tracing::warn!(
                    "Blocked access to environment variable '{}' in template (not in allowlist)",
                    var_name
                );
                String::new()
            }
        })
        .to_string()
}

/// Rendering mode for JSON values in templates.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Compact JSON output (default, used by HTTP node)
    #[default]
    Compact,
    /// Pretty-printed JSON output (used by agent node for better LLM readability)
    Pretty,
}

/// Convert a JSON value to a string for template substitution.
pub fn value_to_string(value: &Value, mode: RenderMode) -> String {
    match value {
        Value::String(s) => s.clone(),
        _ => match mode {
            RenderMode::Compact => value.to_string(),
            RenderMode::Pretty => serde_json::to_string_pretty(value).unwrap_or_default(),
        },
    }
}

/// Render basic template substitutions (input, nodes, env vars).
///
/// This handles common patterns:
/// - {{ input }} - Full input value
/// - {{ input.field }} - Input field access
/// - {{ nodes.node_id.output }} - Node output access
/// - {{ nodes.node_id.output.field }} - Node output field access
/// - {{ env.VAR }} - Environment variable (safe vars only)
pub fn render_template(template: &str, ctx: &NodeContext, mode: RenderMode) -> String {
    let mut result = template.to_string();

    // Replace {{ input }} with full input
    let input_str = value_to_string(&ctx.input, mode);
    result = result.replace("{{ input }}", &input_str);
    result = result.replace("{{input}}", &input_str);

    // Replace {{ input.field }}
    if let Some(obj) = ctx.input.as_object() {
        for (key, value) in obj {
            let pattern1 = format!("{{{{ input.{} }}}}", key);
            let pattern2 = format!("{{{{input.{}}}}}", key);
            let replacement = value_to_string(value, mode);
            result = result.replace(&pattern1, &replacement);
            result = result.replace(&pattern2, &replacement);
        }
    }

    // Replace {{ nodes.node_id.output... }}
    for (node_id, output) in ctx.node_outputs.iter() {
        let output_str = value_to_string(output, mode);
        let pattern = format!("{{{{ nodes.{}.output }}}}", node_id);
        result = result.replace(&pattern, &output_str);

        if let Some(obj) = output.as_object() {
            for (key, value) in obj {
                let pattern = format!("{{{{ nodes.{}.output.{} }}}}", node_id, key);
                let replacement = value_to_string(value, mode);
                result = result.replace(&pattern, &replacement);
            }
        }
    }

    // Replace {{ env.VAR }} with safe environment variables
    result = render_env_vars(&result);

    result
}

/// Render a template string with a simple JSON context.
///
/// This is a simpler version that doesn't require a full NodeContext.
/// Used by webhook debouncing and other non-node contexts.
pub fn render_template_simple(template: &str, context: &Value) -> Result<String, String> {
    let mut result = template.to_string();
    
    // Handle {{ input.path }} patterns
    if let Some(obj) = context.as_object() {
        // Handle 'input' key specially
        if let Some(input) = obj.get("input") {
            // Replace {{ input }} with full input
            let input_str = serde_json::to_string(input).unwrap_or_default();
            result = result.replace("{{ input }}", &input_str);
            result = result.replace("{{input}}", &input_str);
            
            // Replace {{ input.field }}
            if let Some(input_obj) = input.as_object() {
                for (key, value) in input_obj {
                    let pattern1 = format!("{{{{ input.{} }}}}", key);
                    let pattern2 = format!("{{{{input.{}}}}}", key);
                    let replacement = match value {
                        Value::String(s) => s.clone(),
                        other => other.to_string().trim_matches('"').to_string(),
                    };
                    result = result.replace(&pattern1, &replacement);
                    result = result.replace(&pattern2, &replacement);
                    
                    // Handle nested paths like {{ input.repository.id }}
                    if let Some(nested_obj) = value.as_object() {
                        for (nested_key, nested_value) in nested_obj {
                            let nested_pattern1 = format!("{{{{ input.{}.{} }}}}", key, nested_key);
                            let nested_pattern2 = format!("{{{{input.{}.{}}}}}", key, nested_key);
                            let nested_replacement = match nested_value {
                                Value::String(s) => s.clone(),
                                other => other.to_string().trim_matches('"').to_string(),
                            };
                            result = result.replace(&nested_pattern1, &nested_replacement);
                            result = result.replace(&nested_pattern2, &nested_replacement);
                        }
                    }
                }
            }
        }
        
        // Handle 'headers' key
        if let Some(headers) = obj.get("headers") {
            if let Some(headers_obj) = headers.as_object() {
                for (key, value) in headers_obj {
                    let pattern1 = format!("{{{{ headers.{} }}}}", key);
                    let pattern2 = format!("{{{{headers.{}}}}}", key);
                    let replacement = match value {
                        Value::String(s) => s.clone(),
                        other => other.to_string().trim_matches('"').to_string(),
                    };
                    result = result.replace(&pattern1, &replacement);
                    result = result.replace(&pattern2, &replacement);
                }
            }
        }
    }
    
    // Check if there are any unresolved template patterns
    // Look for patterns like {{ ... }} that weren't substituted
    if result.contains("{{") {
        return Err(format!("Unresolved template placeholders in: {}", result));
    }
    
    Ok(result)
}

/// Render template variables in a JSON value recursively.
pub fn render_body(body: &Value, ctx: &NodeContext, mode: RenderMode) -> Value {
    match body {
        Value::String(s) => Value::String(render_template(s, ctx, mode)),
        Value::Object(obj) => {
            let mut new_obj = serde_json::Map::new();
            for (k, v) in obj {
                new_obj.insert(k.clone(), render_body(v, ctx, mode));
            }
            Value::Object(new_obj)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| render_body(v, ctx, mode)).collect()),
        _ => body.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_ctx(input: Value, node_outputs: HashMap<String, Value>) -> NodeContext {
        NodeContext {
            input,
            node_outputs: Arc::new(node_outputs),
            variables: Arc::new(Value::Null),
            execution_id: Arc::from("test"),
            workflow_name: Arc::from("test"),
            correlation_id: None,
            item_index: None,
            credentials: Arc::new(HashMap::new()),
            storage: None,
            registry: None,
        }
    }

    #[test]
    fn test_render_input() {
        let ctx = make_ctx(
            serde_json::json!({"name": "Alice", "age": 30}),
            HashMap::new(),
        );

        let result = render_template("Hello {{ input.name }}", &ctx, RenderMode::Compact);
        assert_eq!(result, "Hello Alice");
    }

    #[test]
    fn test_render_node_output() {
        let mut outputs = HashMap::new();
        outputs.insert("fetch".to_string(), serde_json::json!({"data": "result"}));

        let ctx = make_ctx(Value::Null, outputs);

        let result = render_template(
            "Got {{ nodes.fetch.output.data }}",
            &ctx,
            RenderMode::Compact,
        );
        assert_eq!(result, "Got result");
    }

    #[test]
    fn test_render_env_var_blocked() {
        let ctx = make_ctx(Value::Null, HashMap::new());

        // Non-R8R vars should be blocked
        let result = render_template("Secret: {{ env.SECRET_KEY }}", &ctx, RenderMode::Compact);
        assert_eq!(result, "Secret: ");
    }

    #[test]
    fn test_render_env_var_allowed() {
        std::env::set_var("R8R_TEST_VAR", "test_value");
        let ctx = make_ctx(Value::Null, HashMap::new());

        let result = render_template("Value: {{ env.R8R_TEST_VAR }}", &ctx, RenderMode::Compact);
        assert_eq!(result, "Value: test_value");

        std::env::remove_var("R8R_TEST_VAR");
    }

    #[test]
    fn test_render_body_recursive() {
        let ctx = make_ctx(serde_json::json!({"msg": "hello"}), HashMap::new());

        let body = serde_json::json!({
            "message": "{{ input.msg }}",
            "nested": {
                "value": "{{ input.msg }}"
            },
            "array": ["{{ input.msg }}", "static"]
        });

        let result = render_body(&body, &ctx, RenderMode::Compact);

        assert_eq!(result["message"], "hello");
        assert_eq!(result["nested"]["value"], "hello");
        assert_eq!(result["array"][0], "hello");
        assert_eq!(result["array"][1], "static");
    }

    #[test]
    fn test_render_mode_pretty() {
        let ctx = make_ctx(serde_json::json!({"a": 1, "b": 2}), HashMap::new());

        let compact = render_template("Data: {{ input }}", &ctx, RenderMode::Compact);
        let pretty = render_template("Data: {{ input }}", &ctx, RenderMode::Pretty);

        // Compact should be on one line
        assert!(!compact.contains('\n'));
        // Pretty should have newlines
        assert!(pretty.contains('\n'));
    }
}
