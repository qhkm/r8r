/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! n8n → r8r workflow converter.
//!
//! Assembles the parser, node mapper, and expression transpiler into a
//! single [`convert_n8n_workflow`] entry-point that turns an n8n JSON
//! export into an r8r [`Workflow`] plus migration warnings.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::error::Result;
use crate::migrate::{MigrateResult, MigrateWarning, WarningCategory};
use crate::workflow::{Node, Trigger, Workflow, WorkflowSettings};

use super::expressions::{transpile_all_expressions, TranspileResult};
use super::node_map::{is_trigger_node, map_node};
use super::parser::{parse_n8n_workflow, N8nNode, N8nNodeConnections};

// ── Public API ──────────────────────────────────────────────────────────

/// Convert raw n8n JSON bytes into an r8r [`Workflow`] with warnings.
///
/// Steps:
/// 1. Parse n8n JSON
/// 2. Build name mapping (sanitise + deduplicate)
/// 3. Separate trigger nodes from regular nodes
/// 4. Invert connections (source→targets ➜ target.depends_on)
/// 5. Map each non-trigger node via [`map_node`]
/// 6. Transpile `{{ }}` expressions in all config values
/// 7. Extract credential references as warnings
/// 8. Assemble the r8r Workflow
pub fn convert_n8n_workflow(input: &[u8]) -> Result<MigrateResult> {
    let n8n = parse_n8n_workflow(input)?;
    let mut warnings: Vec<MigrateWarning> = Vec::new();

    // ── 1. Build name mapping ───────────────────────────────────────
    let mut used_names: HashSet<String> = HashSet::new();
    let mut name_map: HashMap<String, String> = HashMap::new();

    for node in &n8n.nodes {
        let sanitized = sanitize_node_name(&node.name);
        let unique = deduplicate_name(&sanitized, &mut used_names);
        name_map.insert(node.name.clone(), unique);
    }

    // ── 2. Identify trigger nodes ───────────────────────────────────
    let trigger_original_names: HashSet<String> = n8n
        .nodes
        .iter()
        .filter(|n| is_trigger_node(n.node_type()))
        .map(|n| n.name.clone())
        .collect();

    let triggers = extract_triggers(&n8n.nodes, &name_map);

    // ── 3. Invert connections ───────────────────────────────────────
    let deps = build_depends_on(&n8n.connections, &name_map, &trigger_original_names);

    // ── 4. Convert non-trigger nodes ────────────────────────────────
    let mut nodes: Vec<Node> = Vec::new();

    for n8n_node in &n8n.nodes {
        if is_trigger_node(n8n_node.node_type()) {
            continue;
        }

        let r8r_id = name_map
            .get(&n8n_node.name)
            .cloned()
            .unwrap_or_else(|| sanitize_node_name(&n8n_node.name));

        // Map node type + config
        let mut map_result = map_node(n8n_node.node_type(), &n8n_node.parameters);

        // Tag warnings with node name
        for w in &mut map_result.warnings {
            if w.node_name.is_none() {
                w.node_name = Some(r8r_id.clone());
            }
        }
        warnings.extend(map_result.warnings);

        // ── 5. Transpile expressions in config ──────────────────────
        let (transpiled_config, expr_warnings) = transpile_value_expressions(&map_result.config);

        warnings.extend(expr_warnings.into_iter().map(|w| MigrateWarning {
            node_name: Some(r8r_id.clone()),
            ..w
        }));

        // ── 6. Extract credential references ────────────────────────
        if let Some(creds) = &n8n_node.credentials {
            for (cred_type, cred_ref) in creds {
                warnings.push(MigrateWarning {
                    node_name: Some(r8r_id.clone()),
                    category: WarningCategory::CredentialReference,
                    message: format!(
                        "Credential '{}' ({}) referenced — configure as r8r secret",
                        cred_ref.name, cred_type
                    ),
                });
            }
        }

        // ── Build r8r Node ──────────────────────────────────────────
        let depends_on = deps.get(&r8r_id).cloned().unwrap_or_default();

        nodes.push(Node {
            id: r8r_id,
            node_type: map_result.r8r_type,
            config: transpiled_config,
            depends_on,
            for_each: false,
            condition: None,
            retry: None,
            on_error: None,
            pinned_data: None,
            timeout_seconds: None,
        });
    }

    // ── 7. Assemble workflow ────────────────────────────────────────
    let workflow_name = sanitize_node_name(&n8n.name);

    let workflow = Workflow {
        name: workflow_name,
        description: format!("Migrated from n8n: {}", n8n.name),
        version: 1,
        triggers,
        inputs: HashMap::new(),
        input_schema: None,
        variables: HashMap::new(),
        depends_on_workflows: Vec::new(),
        nodes,
        settings: WorkflowSettings::default(),
    };

    Ok(MigrateResult { workflow, warnings })
}

// ── Helper functions ────────────────────────────────────────────────────

/// Sanitise an n8n node name into an r8r-compatible ID.
///
/// Lowercase, spaces→hyphens, strip non-alphanumeric/non-hyphen chars,
/// trim leading/trailing hyphens.
pub fn sanitize_node_name(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' { '-' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect();
    s.trim_matches('-').to_string()
}

/// Pick a unique name: if `name` is already in `used`, try name-2, name-3, etc.
fn deduplicate_name(name: &str, used: &mut HashSet<String>) -> String {
    if !used.contains(name) {
        used.insert(name.to_string());
        return name.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{}-{}", name, suffix);
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
        suffix += 1;
    }
}

/// Convert trigger-type n8n nodes to r8r [`Trigger`] structs.
fn extract_triggers(nodes: &[N8nNode], name_map: &HashMap<String, String>) -> Vec<Trigger> {
    let mut triggers = Vec::new();

    for node in nodes {
        let n8n_type = node.node_type();
        if !is_trigger_node(n8n_type) {
            continue;
        }

        let _r8r_name = name_map
            .get(&node.name)
            .cloned()
            .unwrap_or_else(|| sanitize_node_name(&node.name));

        let trigger = match n8n_type {
            "manualTrigger" => Trigger::Manual,
            "webhook" => {
                let path = node
                    .parameters
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let method = node
                    .parameters
                    .get("httpMethod")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Trigger::Webhook {
                    path,
                    method,
                    debounce: None,
                    header_filter: None,
                    response_mode: None,
                }
            }
            "cron" | "scheduleTrigger" | "intervalTrigger" => {
                let schedule = node
                    .parameters
                    .get("rule")
                    .and_then(|v| v.get("interval"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.get("field1"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("* * * * *")
                    .to_string();
                Trigger::Cron {
                    schedule,
                    timezone: None,
                }
            }
            // emailTrigger and any other trigger type → Manual fallback
            _ => Trigger::Manual,
        };

        triggers.push(trigger);
    }

    triggers
}

/// Invert n8n's source→targets connection map into target.depends_on lists.
///
/// Trigger nodes are excluded as sources — their downstream nodes become
/// root nodes (empty depends_on).
fn build_depends_on(
    connections: &HashMap<String, N8nNodeConnections>,
    name_map: &HashMap<String, String>,
    trigger_names: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();

    for (source_name, conn) in connections {
        // Skip trigger nodes as dependency sources
        if trigger_names.contains(source_name) {
            continue;
        }

        let source_id = match name_map.get(source_name) {
            Some(id) => id.clone(),
            None => continue,
        };

        for output_group in &conn.main {
            for target in output_group {
                let target_id = match name_map.get(&target.node) {
                    Some(id) => id.clone(),
                    None => continue,
                };

                deps.entry(target_id).or_default().push(source_id.clone());
            }
        }
    }

    // Deduplicate within each list
    for list in deps.values_mut() {
        list.sort();
        list.dedup();
    }

    deps
}

/// Recursively walk a [`Value`], transpiling all `{{ }}` expressions in strings.
fn transpile_value_expressions(value: &Value) -> (Value, Vec<MigrateWarning>) {
    let mut warnings = Vec::new();

    let result = match value {
        Value::String(s) => {
            if s.contains("{{") {
                let (transpiled, results) = transpile_all_expressions(s);
                for (result, original) in &results {
                    match result {
                        TranspileResult::Approximate(_, reason) => {
                            warnings.push(MigrateWarning {
                                node_name: None,
                                category: WarningCategory::ApproximateExpression,
                                message: format!(
                                    "Expression '{}' approximately converted: {}",
                                    original, reason
                                ),
                            });
                        }
                        TranspileResult::Failed(_, reason) => {
                            warnings.push(MigrateWarning {
                                node_name: None,
                                category: WarningCategory::UnconvertedExpression,
                                message: format!(
                                    "Expression '{}' could not be converted: {}",
                                    original, reason
                                ),
                            });
                        }
                        TranspileResult::Exact(_) => {}
                    }
                }
                Value::String(transpiled)
            } else {
                value.clone()
            }
        }
        Value::Array(arr) => {
            let mut new_arr = Vec::with_capacity(arr.len());
            for item in arr {
                let (new_item, item_warnings) = transpile_value_expressions(item);
                new_arr.push(new_item);
                warnings.extend(item_warnings);
            }
            Value::Array(new_arr)
        }
        Value::Object(obj) => {
            let mut new_obj = serde_json::Map::with_capacity(obj.len());
            for (k, v) in obj {
                let (new_v, v_warnings) = transpile_value_expressions(v);
                new_obj.insert(k.clone(), new_v);
                warnings.extend(v_warnings);
            }
            Value::Object(new_obj)
        }
        // Numbers, bools, null — pass through
        _ => value.clone(),
    };

    (result, warnings)
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_node_name("HTTP Request"), "http-request");
        assert_eq!(sanitize_node_name("My Node #1"), "my-node-1");
        assert_eq!(sanitize_node_name("  Leading Spaces  "), "leading-spaces");
        assert_eq!(sanitize_node_name("--dashes--"), "dashes");
        assert_eq!(sanitize_node_name("UPPER_case"), "uppercase");
    }

    #[test]
    fn test_deduplicate_names() {
        let mut used = HashSet::new();

        let first = deduplicate_name("http-request", &mut used);
        assert_eq!(first, "http-request");

        let second = deduplicate_name("http-request", &mut used);
        assert_eq!(second, "http-request-2");

        let third = deduplicate_name("http-request", &mut used);
        assert_eq!(third, "http-request-3");
    }

    #[test]
    fn test_invert_connections() {
        // Build a 3-node chain: ManualTrigger → "Node A" → "Node B"
        let json = serde_json::json!({
            "name": "Test Workflow",
            "nodes": [
                {
                    "id": "t1",
                    "name": "Manual Trigger",
                    "type": "n8n-nodes-base.manualTrigger",
                    "typeVersion": 1,
                    "parameters": {}
                },
                {
                    "id": "a1",
                    "name": "Node A",
                    "type": "n8n-nodes-base.httpRequest",
                    "typeVersion": 3,
                    "parameters": { "url": "https://a.example.com" }
                },
                {
                    "id": "b1",
                    "name": "Node B",
                    "type": "n8n-nodes-base.httpRequest",
                    "typeVersion": 3,
                    "parameters": { "url": "https://b.example.com" }
                }
            ],
            "connections": {
                "Manual Trigger": {
                    "main": [[{ "node": "Node A", "type": "main", "index": 0 }]]
                },
                "Node A": {
                    "main": [[{ "node": "Node B", "type": "main", "index": 0 }]]
                }
            }
        });

        let input = serde_json::to_vec(&json).unwrap();
        let result = convert_n8n_workflow(&input).unwrap();

        // Trigger should be extracted
        assert_eq!(result.workflow.triggers.len(), 1);
        assert!(matches!(result.workflow.triggers[0], Trigger::Manual));

        // Two non-trigger nodes
        assert_eq!(result.workflow.nodes.len(), 2);

        let node_a = result
            .workflow
            .nodes
            .iter()
            .find(|n| n.id == "node-a")
            .unwrap();
        let node_b = result
            .workflow
            .nodes
            .iter()
            .find(|n| n.id == "node-b")
            .unwrap();

        // Node A: root node (trigger's target → no depends_on)
        assert!(node_a.depends_on.is_empty());

        // Node B: depends on Node A
        assert_eq!(node_b.depends_on, vec!["node-a"]);
    }

    #[test]
    fn test_credential_extraction() {
        let json = serde_json::json!({
            "name": "Cred Workflow",
            "nodes": [
                {
                    "id": "s1",
                    "name": "Slack Post",
                    "type": "n8n-nodes-base.slack",
                    "typeVersion": 2,
                    "parameters": { "channel": "#general" },
                    "credentials": {
                        "slackApi": {
                            "id": "cred-42",
                            "name": "My Slack Token"
                        }
                    }
                }
            ],
            "connections": {}
        });

        let input = serde_json::to_vec(&json).unwrap();
        let result = convert_n8n_workflow(&input).unwrap();

        let cred_warnings: Vec<_> = result
            .warnings
            .iter()
            .filter(|w| w.category == WarningCategory::CredentialReference)
            .collect();

        assert!(!cred_warnings.is_empty(), "Should have credential warnings");
        assert!(
            cred_warnings[0].message.contains("My Slack Token"),
            "Warning should mention the credential name"
        );
        assert!(
            cred_warnings[0].message.contains("slackApi"),
            "Warning should mention the credential type"
        );
        assert_eq!(
            cred_warnings[0].node_name.as_deref(),
            Some("slack-post"),
            "Warning should be tagged with the node name"
        );
    }
}
