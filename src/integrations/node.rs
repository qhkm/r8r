/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! IntegrationNode — delegates to HttpNode after resolving integration
//! definitions, validating parameters, and building the HTTP config.

use std::collections::HashMap;

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tracing::debug;

use crate::error::{Error, Result};
use crate::integrations::definition::{IntegrationDefinition, OperationDef};
use crate::integrations::loader::IntegrationLoader;
use crate::integrations::validator::validate_and_resolve_params;
use crate::nodes::{HttpNode, Node, NodeContext, NodeResult};

/// Private configuration struct deserialized from the workflow YAML.
#[derive(Debug, Deserialize)]
struct IntegrationConfig {
    service: String,
    operation: String,
    #[serde(default)]
    credential: Option<String>,
    #[serde(default)]
    params: Value,
}

/// Node that translates a declarative integration + operation into an HTTP call.
pub struct IntegrationNode {
    http: HttpNode,
    loader: IntegrationLoader,
}

impl IntegrationNode {
    pub fn new() -> Self {
        Self {
            http: HttpNode::new(),
            loader: IntegrationLoader::new(),
        }
    }
}

impl Default for IntegrationNode {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Public helpers (exported for testing)
// ---------------------------------------------------------------------------

/// Replace `{{ params.X }}` and `{{params.X}}` placeholders with the string
/// representation of the resolved value.
pub fn substitute_params_in_string(template: &str, resolved: &HashMap<String, Value>) -> String {
    let mut result = template.to_string();
    for (key, value) in resolved {
        let str_value = match value {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        // With spaces: {{ params.key }}
        let pattern_spaced = format!("{{{{ params.{} }}}}", key);
        result = result.replace(&pattern_spaced, &str_value);
        // Without spaces: {{params.key}}
        let pattern_compact = format!("{{{{params.{}}}}}", key);
        result = result.replace(&pattern_compact, &str_value);
    }
    result
}

/// If `template` is exactly a single param reference like `"{{ params.X }}"`
/// (with or without spaces), return `Some(param_name)`. Otherwise `None`.
pub fn extract_param_ref(template: &str) -> Option<String> {
    let trimmed = template.trim();
    // Try spaced variant: {{ params.X }}
    if let Some(inner) = trimmed.strip_prefix("{{ params.") {
        if let Some(name) = inner.strip_suffix(" }}") {
            let name = name.trim();
            if !name.is_empty() && !name.contains(' ') {
                return Some(name.to_string());
            }
        }
    }
    // Try compact variant: {{params.X}}
    if let Some(inner) = trimmed.strip_prefix("{{params.") {
        if let Some(name) = inner.strip_suffix("}}") {
            let name = name.trim();
            if !name.is_empty() && !name.contains(' ') {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Build a JSON body from the body template and resolved params.
///
/// For each key in the body template object:
/// - If the value is a direct param reference (`"{{ params.X }}"`), insert the
///   typed value from `resolved` (arrays stay arrays, numbers stay numbers).
///   If the param is absent from `resolved` (optional, no default), **omit**
///   the field entirely.
/// - For mixed/compound templates, do string substitution. If the result still
///   contains unresolved template markers, omit the field.
/// - For non-string values, pass through as-is.
pub fn build_typed_body(body_template: &Value, resolved: &HashMap<String, Value>) -> Value {
    match body_template {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                match val {
                    Value::String(s) => {
                        if let Some(param_name) = extract_param_ref(s) {
                            // Direct param reference — insert typed value or omit
                            if let Some(typed_val) = resolved.get(&param_name) {
                                out.insert(key.clone(), typed_val.clone());
                            }
                            // else: absent optional param → omit
                        } else {
                            // Mixed template — do string substitution
                            let substituted = substitute_params_in_string(s, resolved);
                            // If the result still has unresolved {{ params.* }}, omit
                            if !substituted.contains("{{ params.")
                                && !substituted.contains("{{params.")
                            {
                                out.insert(key.clone(), Value::String(substituted));
                            }
                        }
                    }
                    Value::Object(_) => {
                        let nested = build_typed_body(val, resolved);
                        if let Value::Object(nested_map) = &nested {
                            if !nested_map.is_empty() {
                                out.insert(key.clone(), nested);
                            }
                        } else {
                            out.insert(key.clone(), nested);
                        }
                    }
                    other => {
                        out.insert(key.clone(), other.clone());
                    }
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Build a complete HttpNode-compatible JSON config from an integration
/// definition, operation, resolved params, and optional credential.
pub fn build_http_config(
    def: &IntegrationDefinition,
    op: &OperationDef,
    resolved: &HashMap<String, Value>,
    credential: Option<&str>,
) -> Value {
    // --- URL: base_url + path (with params substituted) ---
    let path = substitute_params_in_string(&op.path, resolved);
    let mut url = format!("{}{}", def.base_url, path);

    // --- Query params ---
    if let Some(query_template) = &op.query {
        let mut query_parts: Vec<String> = Vec::new();
        for (qk, qv) in query_template {
            // Resolve the value via param ref or substitution
            let value = if let Some(param_name) = extract_param_ref(qv) {
                resolved.get(&param_name).cloned()
            } else {
                let substituted = substitute_params_in_string(qv, resolved);
                if substituted.contains("{{ params.") || substituted.contains("{{params.") {
                    None
                } else {
                    Some(Value::String(substituted))
                }
            };

            if let Some(val) = value {
                let str_val = match &val {
                    Value::String(s) => s.clone(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                let encoded = utf8_percent_encode(&str_val, NON_ALPHANUMERIC).to_string();
                query_parts.push(format!("{}={}", qk, encoded));
            }
        }
        if !query_parts.is_empty() {
            url = format!("{}?{}", url, query_parts.join("&"));
        }
    }

    // --- Headers: merge service-level with operation-level ---
    let mut headers_map: Map<String, Value> = Map::new();
    if let Some(service_headers) = &def.headers {
        for (k, v) in service_headers {
            headers_map.insert(k.clone(), Value::String(v.clone()));
        }
    }
    if let Some(op_headers) = &op.headers {
        for (k, v) in op_headers {
            headers_map.insert(k.clone(), Value::String(v.clone()));
        }
    }

    // --- Body ---
    let body = op
        .body
        .as_ref()
        .map(|tmpl| build_typed_body(tmpl, resolved));

    // --- Assemble ---
    let mut config = json!({
        "url": url,
        "method": op.method,
    });

    if !headers_map.is_empty() {
        config["headers"] = Value::Object(headers_map);
    }

    if let Some(body_val) = body {
        config["body"] = body_val;
    }

    // Set body_type if specified (e.g. "form" for application/x-www-form-urlencoded)
    if let Some(body_type) = &op.body_type {
        config["body_type"] = json!(body_type);
    }

    if let Some(cred) = credential {
        config["credential"] = Value::String(cred.to_string());
        // Derive auth_type from the integration definition
        if let Some(auth) = &def.auth {
            for method in &auth.methods {
                match method {
                    crate::integrations::definition::AuthMethod::Bearer { .. } => {
                        config["auth_type"] = Value::String("bearer".to_string());
                        break;
                    }
                    crate::integrations::definition::AuthMethod::ApiKey { header_name, .. } => {
                        if let Some(hdr) = header_name {
                            config["auth_type"] = Value::String(format!("header:{}", hdr));
                        } else {
                            config["auth_type"] = Value::String("api_key".to_string());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    config
}

// ---------------------------------------------------------------------------
// Node trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Node for IntegrationNode {
    fn node_type(&self) -> &str {
        "integration"
    }

    fn description(&self) -> &str {
        "Declarative integration — translates service + operation into an HTTP call"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let ic: IntegrationConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid integration config: {}", e)))?;

        // Load the integration definition
        let def = self.loader.get(&ic.service).ok_or_else(|| {
            Error::Node(format!(
                "Unknown integration service '{}'. Available: {:?}",
                ic.service,
                self.loader.list_all()
            ))
        })?;

        // Look up the operation
        let op = def.operations.get(&ic.operation).ok_or_else(|| {
            Error::Node(format!(
                "Unknown operation '{}' for service '{}'. Available: {:?}",
                ic.operation,
                ic.service,
                def.operations.keys().collect::<Vec<_>>()
            ))
        })?;

        // Validate and resolve params
        let resolved = validate_and_resolve_params(&op.params, &ic.params)
            .map_err(|e| Error::Node(format!("Parameter validation failed: {}", e)))?;

        debug!(
            service = %ic.service,
            operation = %ic.operation,
            "building HTTP config from integration definition"
        );

        // Build the HttpNode config
        let http_config = build_http_config(&def, op, &resolved, ic.credential.as_deref());

        // Delegate to HttpNode
        self.http.execute(&http_config, ctx).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::definition::*;

    /// Helper: minimal IntegrationDefinition for testing.
    fn test_def() -> IntegrationDefinition {
        IntegrationDefinition {
            version: 1,
            name: "github".into(),
            display_name: Some("GitHub".into()),
            description: None,
            base_url: "https://api.github.com".into(),
            docs_url: None,
            auth: Some(AuthConfig {
                methods: vec![AuthMethod::Bearer {
                    description: Some("PAT".into()),
                }],
            }),
            headers: Some({
                let mut h = HashMap::new();
                h.insert("Accept".into(), "application/vnd.github+json".into());
                h
            }),
            operations: HashMap::new(),
        }
    }

    fn list_issues_op() -> OperationDef {
        OperationDef {
            description: "List issues".into(),
            method: "GET".into(),
            path: "/repos/{{ params.owner }}/{{ params.repo }}/issues".into(),
            params: {
                let mut p = HashMap::new();
                p.insert(
                    "owner".into(),
                    ParamDef {
                        required: true,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "repo".into(),
                    ParamDef {
                        required: true,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "state".into(),
                    ParamDef {
                        required: false,
                        param_type: Some("string".into()),
                        description: None,
                        default: Some(json!("open")),
                        enum_values: Some(vec!["open".into(), "closed".into(), "all".into()]),
                    },
                );
                p.insert(
                    "per_page".into(),
                    ParamDef {
                        required: false,
                        param_type: Some("integer".into()),
                        description: None,
                        default: Some(json!(30)),
                        enum_values: None,
                    },
                );
                p
            },
            headers: None,
            body: None,
            query: Some({
                let mut q = HashMap::new();
                q.insert("state".into(), "{{ params.state }}".into());
                q.insert("per_page".into(), "{{ params.per_page }}".into());
                q
            }),
            body_type: None,
        }
    }

    fn create_issue_op() -> OperationDef {
        OperationDef {
            description: "Create issue".into(),
            method: "POST".into(),
            path: "/repos/{{ params.owner }}/{{ params.repo }}/issues".into(),
            params: {
                let mut p = HashMap::new();
                p.insert(
                    "owner".into(),
                    ParamDef {
                        required: true,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "repo".into(),
                    ParamDef {
                        required: true,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "title".into(),
                    ParamDef {
                        required: true,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "body".into(),
                    ParamDef {
                        required: false,
                        param_type: Some("string".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "labels".into(),
                    ParamDef {
                        required: false,
                        param_type: Some("array".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p.insert(
                    "assignees".into(),
                    ParamDef {
                        required: false,
                        param_type: Some("array".into()),
                        description: None,
                        default: None,
                        enum_values: None,
                    },
                );
                p
            },
            headers: Some({
                let mut h = HashMap::new();
                h.insert("Content-Type".into(), "application/json".into());
                h
            }),
            body: Some(json!({
                "title": "{{ params.title }}",
                "body": "{{ params.body }}",
                "labels": "{{ params.labels }}",
                "assignees": "{{ params.assignees }}"
            })),
            query: None,
            body_type: None,
        }
    }

    // ------------------------------------------------------------------
    // 1. GET with query params encoded in URL
    // ------------------------------------------------------------------
    #[test]
    fn test_build_http_config_get_with_query() {
        let def = test_def();
        let op = list_issues_op();

        let mut resolved = HashMap::new();
        resolved.insert("owner".into(), json!("acme"));
        resolved.insert("repo".into(), json!("widgets"));
        resolved.insert("state".into(), json!("open"));
        resolved.insert("per_page".into(), json!(30));

        let config = build_http_config(&def, &op, &resolved, None);

        let url = config["url"].as_str().unwrap();
        assert!(
            url.starts_with("https://api.github.com/repos/acme/widgets/issues?"),
            "URL should start with base + path, got: {}",
            url
        );
        assert!(
            url.contains("state=open"),
            "URL should contain state=open, got: {}",
            url
        );
        assert!(
            url.contains("per_page=30"),
            "URL should contain per_page=30, got: {}",
            url
        );
        assert_eq!(config["method"], "GET");
        // No credential
        assert!(config.get("credential").is_none());
    }

    // ------------------------------------------------------------------
    // 2. POST with array params staying as arrays
    // ------------------------------------------------------------------
    #[test]
    fn test_build_http_config_post_with_typed_body() {
        let def = test_def();
        let op = create_issue_op();

        let mut resolved = HashMap::new();
        resolved.insert("owner".into(), json!("acme"));
        resolved.insert("repo".into(), json!("widgets"));
        resolved.insert("title".into(), json!("Bug report"));
        resolved.insert("labels".into(), json!(["bug", "urgent"]));

        let config = build_http_config(&def, &op, &resolved, None);

        assert_eq!(config["method"], "POST");

        let body = &config["body"];
        assert_eq!(body["title"], "Bug report");
        // labels should be a real JSON array, not a string
        assert!(body["labels"].is_array(), "labels should be an array");
        assert_eq!(body["labels"], json!(["bug", "urgent"]));
        // absent optional body and assignees should be omitted
        assert!(
            body.get("body").is_none(),
            "absent optional 'body' should be omitted"
        );
        assert!(
            body.get("assignees").is_none(),
            "absent optional 'assignees' should be omitted"
        );
    }

    // ------------------------------------------------------------------
    // 3. credential + auth_type + headers set
    // ------------------------------------------------------------------
    #[test]
    fn test_build_http_config_with_credential() {
        let def = test_def();
        let op = list_issues_op();

        let mut resolved = HashMap::new();
        resolved.insert("owner".into(), json!("acme"));
        resolved.insert("repo".into(), json!("widgets"));
        resolved.insert("state".into(), json!("open"));
        resolved.insert("per_page".into(), json!(30));

        let config = build_http_config(&def, &op, &resolved, Some("github"));

        assert_eq!(config["credential"], "github");
        assert_eq!(config["auth_type"], "bearer");
        // Service-level headers present
        assert_eq!(config["headers"]["Accept"], "application/vnd.github+json");
    }

    // ------------------------------------------------------------------
    // 4. {{ params.X }} replaced correctly in path
    // ------------------------------------------------------------------
    #[test]
    fn test_params_substitution_in_path() {
        let mut resolved = HashMap::new();
        resolved.insert("owner".into(), json!("octocat"));
        resolved.insert("repo".into(), json!("hello-world"));

        let result = substitute_params_in_string(
            "/repos/{{ params.owner }}/{{ params.repo }}/issues",
            &resolved,
        );
        assert_eq!(result, "/repos/octocat/hello-world/issues");

        // Also test compact form
        let result2 = substitute_params_in_string(
            "/repos/{{params.owner}}/{{params.repo}}/issues",
            &resolved,
        );
        assert_eq!(result2, "/repos/octocat/hello-world/issues");
    }

    // ------------------------------------------------------------------
    // 5. defaults applied in query (null optional params omitted)
    // ------------------------------------------------------------------
    #[test]
    fn test_null_optional_params_omitted_from_query() {
        let def = test_def();
        let op = list_issues_op();

        // Only provide required params; optional params get defaults via validator
        let input = json!({"owner": "acme", "repo": "widgets"});
        let resolved = validate_and_resolve_params(&op.params, &input).unwrap();

        let config = build_http_config(&def, &op, &resolved, None);
        let url = config["url"].as_str().unwrap();

        // Defaults should be applied: state=open, per_page=30
        assert!(
            url.contains("state=open"),
            "default state should appear: {}",
            url
        );
        assert!(
            url.contains("per_page=30"),
            "default per_page should appear: {}",
            url
        );
    }

    // ------------------------------------------------------------------
    // 6. absent optional body params NOT in body (no template literals)
    // ------------------------------------------------------------------
    #[test]
    fn test_absent_optional_body_params_omitted() {
        let def = test_def();
        let op = create_issue_op();

        // Only provide required params — body, labels, assignees are optional
        let mut resolved = HashMap::new();
        resolved.insert("owner".into(), json!("acme"));
        resolved.insert("repo".into(), json!("widgets"));
        resolved.insert("title".into(), json!("A title"));

        let config = build_http_config(&def, &op, &resolved, None);
        let body = &config["body"];

        // title should be present
        assert_eq!(body["title"], "A title");
        // Optional params that were NOT supplied must be completely absent
        assert!(
            body.get("body").is_none(),
            "absent optional 'body' param should be omitted from body, got: {:?}",
            body
        );
        assert!(
            body.get("labels").is_none(),
            "absent optional 'labels' param should be omitted from body, got: {:?}",
            body
        );
        assert!(
            body.get("assignees").is_none(),
            "absent optional 'assignees' param should be omitted from body, got: {:?}",
            body
        );
        // Must NOT contain template literal strings like "{{ params.body }}"
        let body_str = body.to_string();
        assert!(
            !body_str.contains("{{ params."),
            "body should not contain unresolved template literals: {}",
            body_str
        );
        assert!(
            !body_str.contains("{{params."),
            "body should not contain unresolved template literals: {}",
            body_str
        );
    }
}
