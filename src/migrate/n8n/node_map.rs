/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Maps n8n node types to their r8r equivalents.
//!
//! Each n8n node type (e.g. `httpRequest`, `code`, `slack`) is mapped to a
//! corresponding r8r step type with appropriate configuration translation.
//! Unknown node types produce a placeholder `http` step with a TODO comment
//! and an `UnsupportedNode` warning.

use serde_json::{json, Value};

use crate::migrate::{MigrateWarning, WarningCategory};

/// Result of mapping a single n8n node type to r8r.
pub struct NodeMapResult {
    pub r8r_type: String,
    pub config: Value,
    pub warnings: Vec<MigrateWarning>,
}

/// Returns `true` if the given n8n node type is a trigger (entry-point) node.
pub fn is_trigger_node(n8n_type: &str) -> bool {
    matches!(
        n8n_type,
        "manualTrigger"
            | "scheduleTrigger"
            | "cron"
            | "webhook"
            | "emailTrigger"
            | "intervalTrigger"
    )
}

/// Map an n8n node type and its parameters to an r8r step type + config.
pub fn map_node(n8n_type: &str, params: &Value) -> NodeMapResult {
    match n8n_type {
        // ── HTTP ───────────────────────────────────────────────────────
        "httpRequest" => NodeMapResult {
            r8r_type: "http".into(),
            config: json!({
                "url": params.get("url").cloned().unwrap_or(Value::Null),
                "method": params.get("method").cloned().unwrap_or(json!("GET")),
                "headers": params.get("headers").cloned().unwrap_or(Value::Null),
                "body": params.get("body").cloned().unwrap_or(Value::Null),
            }),
            warnings: vec![],
        },

        // ── Data manipulation ──────────────────────────────────────────
        "set" => NodeMapResult {
            r8r_type: "set".into(),
            config: params.clone(),
            warnings: vec![],
        },

        // ── Code / Sandbox ─────────────────────────────────────────────
        "code" => {
            let language = params
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("javascript");

            let (runtime, code) = if language == "python" {
                (
                    "python3",
                    params
                        .get("pythonCode")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
            } else {
                (
                    "node",
                    params
                        .get("jsCode")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
            };

            NodeMapResult {
                r8r_type: "sandbox".into(),
                config: json!({
                    "runtime": runtime,
                    "code": code,
                }),
                warnings: vec![MigrateWarning {
                    node_name: None,
                    category: WarningCategory::FeatureGate,
                    message: "Code node requires --features sandbox to execute".into(),
                }],
            }
        }

        // ── Wait / delay ───────────────────────────────────────────────
        "wait" => {
            let amount = params
                .get("amount")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let unit = params
                .get("unit")
                .and_then(|v| v.as_str())
                .unwrap_or("seconds");

            let seconds = match unit {
                "minutes" => amount * 60.0,
                "hours" => amount * 3600.0,
                "days" => amount * 86400.0,
                _ => amount, // seconds (default)
            };

            NodeMapResult {
                r8r_type: "wait".into(),
                config: json!({ "seconds": seconds }),
                warnings: vec![],
            }
        }

        // ── Pass-through nodes (params forwarded as-is) ────────────────
        "merge" => passthrough("merge", params),
        "filter" => passthrough("filter", params),
        "sort" => passthrough("sort", params),
        "limit" => passthrough("limit", params),
        "aggregate" => passthrough("aggregate", params),
        "crypto" => passthrough("crypto", params),
        "dateTime" => passthrough("datetime", params),

        // ── No-op → debug ──────────────────────────────────────────────
        "noOp" => NodeMapResult {
            r8r_type: "debug".into(),
            config: json!({ "log_input": true }),
            warnings: vec![],
        },

        // ── Email ──────────────────────────────────────────────────────
        "emailSend" | "emailSendV2" => NodeMapResult {
            r8r_type: "email".into(),
            config: params.clone(),
            warnings: vec![],
        },

        // ── Integration: Slack ─────────────────────────────────────────
        "slack" => NodeMapResult {
            r8r_type: "integration".into(),
            config: json!({
                "service": "slack",
                "operation": params.get("operation").cloned().unwrap_or(Value::Null),
                "params": params,
            }),
            warnings: vec![],
        },

        // ── Integration: OpenAI ────────────────────────────────────────
        "openAi" => NodeMapResult {
            r8r_type: "integration".into(),
            config: json!({
                "service": "openai",
                "operation": params.get("operation").cloned().unwrap_or(Value::Null),
                "params": params,
            }),
            warnings: vec![],
        },

        // ── Control-flow (complex translation deferred to converter.rs)
        "if" => passthrough("if", params),
        "switch" => passthrough("switch", params),

        // ── Unknown / unsupported ──────────────────────────────────────
        other => {
            let param_str = serde_json::to_string(params).unwrap_or_default();
            let truncated = if param_str.len() > 200 {
                format!("{}...", &param_str[..200])
            } else {
                param_str
            };

            NodeMapResult {
                r8r_type: "http".into(),
                config: json!({
                    "url": format!(
                        "# TODO: Convert n8n node '{}' (n8n-nodes-base.{})",
                        other, other
                    ),
                }),
                warnings: vec![MigrateWarning {
                    node_name: None,
                    category: WarningCategory::UnsupportedNode,
                    message: format!(
                        "Unsupported n8n node type '{}'; original params: {}",
                        other, truncated
                    ),
                }],
            }
        }
    }
}

/// Helper: pass params through unchanged under the given r8r type.
fn passthrough(r8r_type: &str, params: &Value) -> NodeMapResult {
    NodeMapResult {
        r8r_type: r8r_type.into(),
        config: params.clone(),
        warnings: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_http_request_mapping() {
        let params = json!({
            "url": "https://api.example.com/data",
            "method": "POST",
            "headers": { "Authorization": "Bearer {{token}}" },
            "body": { "key": "value" }
        });

        let result = map_node("httpRequest", &params);

        assert_eq!(result.r8r_type, "http");
        assert_eq!(result.config["url"], "https://api.example.com/data");
        assert_eq!(result.config["method"], "POST");
        assert_eq!(
            result.config["headers"]["Authorization"],
            "Bearer {{token}}"
        );
        assert_eq!(result.config["body"]["key"], "value");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_code_node_javascript() {
        let params = json!({
            "language": "javascript",
            "jsCode": "return items.map(i => i.json);"
        });

        let result = map_node("code", &params);

        assert_eq!(result.r8r_type, "sandbox");
        assert_eq!(result.config["runtime"], "node");
        assert_eq!(result.config["code"], "return items.map(i => i.json);");
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].category, WarningCategory::FeatureGate);
        assert!(result.warnings[0].message.contains("--features sandbox"));
    }

    #[test]
    fn test_code_node_python() {
        let params = json!({
            "language": "python",
            "pythonCode": "return [{'json': x} for x in items]"
        });

        let result = map_node("code", &params);

        assert_eq!(result.r8r_type, "sandbox");
        assert_eq!(result.config["runtime"], "python3");
        assert_eq!(
            result.config["code"],
            "return [{'json': x} for x in items]"
        );
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].category, WarningCategory::FeatureGate);
    }

    #[test]
    fn test_trigger_nodes() {
        assert!(is_trigger_node("manualTrigger"));
        assert!(is_trigger_node("scheduleTrigger"));
        assert!(is_trigger_node("webhook"));
        assert!(is_trigger_node("cron"));
        assert!(is_trigger_node("emailTrigger"));
        assert!(is_trigger_node("intervalTrigger"));

        // Non-triggers
        assert!(!is_trigger_node("httpRequest"));
        assert!(!is_trigger_node("code"));
        assert!(!is_trigger_node("set"));
    }

    #[test]
    fn test_unsupported_node() {
        let params = json!({ "foo": "bar", "baz": 42 });
        let result = map_node("myCustomNode", &params);

        assert_eq!(result.r8r_type, "http");
        assert!(result.config["url"]
            .as_str()
            .unwrap()
            .contains("TODO: Convert n8n node 'myCustomNode'"));
        assert!(result.config["url"]
            .as_str()
            .unwrap()
            .contains("n8n-nodes-base.myCustomNode"));
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            result.warnings[0].category,
            WarningCategory::UnsupportedNode
        );
        assert!(result.warnings[0].message.contains("myCustomNode"));
    }

    #[test]
    fn test_set_node() {
        let params = json!({
            "values": {
                "string": [{ "name": "key", "value": "hello" }]
            }
        });

        let result = map_node("set", &params);

        assert_eq!(result.r8r_type, "set");
        assert_eq!(result.config, params);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_wait_node() {
        // Minutes → seconds
        let params = json!({ "amount": 5, "unit": "minutes" });
        let result = map_node("wait", &params);

        assert_eq!(result.r8r_type, "wait");
        assert_eq!(result.config["seconds"], 300.0);
        assert!(result.warnings.is_empty());

        // Hours → seconds
        let params_h = json!({ "amount": 2, "unit": "hours" });
        let result_h = map_node("wait", &params_h);
        assert_eq!(result_h.config["seconds"], 7200.0);

        // Days → seconds
        let params_d = json!({ "amount": 1, "unit": "days" });
        let result_d = map_node("wait", &params_d);
        assert_eq!(result_d.config["seconds"], 86400.0);

        // Default (seconds)
        let params_s = json!({ "amount": 30, "unit": "seconds" });
        let result_s = map_node("wait", &params_s);
        assert_eq!(result_s.config["seconds"], 30.0);
    }
}
