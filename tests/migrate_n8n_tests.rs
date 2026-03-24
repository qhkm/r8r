/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */

//! Integration tests for n8n migration.

use r8r::migrate::n8n::convert_n8n_workflow;
use r8r::migrate::WarningCategory;

#[test]
fn test_migrate_simple_http() {
    let input = include_bytes!("fixtures/n8n/simple-http.json");
    let result = convert_n8n_workflow(input).unwrap();

    assert_eq!(result.workflow.name, "simple-http-workflow");
    assert!(!result.workflow.triggers.is_empty());
    assert!(result.workflow.nodes.len() >= 2);

    // Check HTTP node was mapped
    let http_node = result
        .workflow
        .nodes
        .iter()
        .find(|n| n.node_type == "http")
        .expect("Should have an http node");
    assert!(http_node.config.get("url").is_some());
}

#[test]
fn test_migrate_if_branch() {
    let input = include_bytes!("fixtures/n8n/if-branch.json");
    let result = convert_n8n_workflow(input).unwrap();

    // Should have webhook trigger
    assert!(!result.workflow.triggers.is_empty());

    // Should have nodes with dependencies
    assert!(!result.workflow.nodes.is_empty());
}

#[test]
fn test_migrate_expressions() {
    let input = include_bytes!("fixtures/n8n/expressions.json");
    let result = convert_n8n_workflow(input).unwrap();

    // Should have credential warnings
    assert!(result
        .warnings
        .iter()
        .any(|w| w.category == WarningCategory::CredentialReference));

    // Should have nodes
    assert!(!result.workflow.nodes.is_empty());
}

#[test]
fn test_migrate_invalid_json() {
    let result = convert_n8n_workflow(b"not json");
    assert!(result.is_err());
}

#[test]
fn test_warnings_are_structured() {
    let input = include_bytes!("fixtures/n8n/expressions.json");
    let result = convert_n8n_workflow(input).unwrap();

    for warning in &result.warnings {
        assert!(!warning.message.is_empty());
        let display = format!("{}", warning);
        assert!(!display.is_empty());
    }
}
