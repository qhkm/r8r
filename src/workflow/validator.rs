//! Workflow validation.

use std::collections::{HashMap, HashSet};

use super::types::{OnErrorAction, Workflow};
use crate::error::{Error, Result};

/// Maximum allowed length for workflow and node names.
const MAX_NAME_LENGTH: usize = 128;

/// Maximum allowed number of nodes in a workflow.
const MAX_NODES: usize = 1000;

/// Maximum allowed timeout in seconds (24 hours).
const MAX_TIMEOUT_SECONDS: u64 = 86400;

/// Validate a workflow definition.
///
/// Checks for:
/// - Required fields (name, nodes)
/// - Name and ID format/length constraints
/// - Unique node IDs
/// - Valid dependencies (referenced nodes exist)
/// - No circular dependencies
/// - Valid node types (if registry provided)
/// - Settings bounds validation
pub fn validate_workflow(workflow: &Workflow) -> Result<()> {
    // Check workflow has a name
    if workflow.name.is_empty() {
        return Err(Error::Validation("Workflow name is required".into()));
    }

    // Check workflow name length
    if workflow.name.len() > MAX_NAME_LENGTH {
        return Err(Error::Validation(format!(
            "Workflow name exceeds maximum length of {} characters",
            MAX_NAME_LENGTH
        )));
    }

    // Check workflow name is valid (alphanumeric, hyphens, underscores)
    if !workflow
        .name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Validation(
            "Workflow name must contain only alphanumeric characters, hyphens, and underscores"
                .into(),
        ));
    }

    // Check workflow has at least one node
    if workflow.nodes.is_empty() {
        return Err(Error::Validation(
            "Workflow must have at least one node".into(),
        ));
    }

    // Check workflow doesn't have too many nodes (DoS protection)
    if workflow.nodes.len() > MAX_NODES {
        return Err(Error::Validation(format!(
            "Workflow has {} nodes, exceeding maximum of {}",
            workflow.nodes.len(),
            MAX_NODES
        )));
    }

    // Validate settings bounds
    if workflow.settings.timeout_seconds > MAX_TIMEOUT_SECONDS {
        return Err(Error::Validation(format!(
            "Timeout of {} seconds exceeds maximum of {} (24 hours)",
            workflow.settings.timeout_seconds, MAX_TIMEOUT_SECONDS
        )));
    }

    if workflow.settings.max_concurrency == 0 {
        return Err(Error::Validation(
            "max_concurrency must be at least 1".into(),
        ));
    }

    if workflow.settings.max_concurrency > 1000 {
        return Err(Error::Validation(
            "max_concurrency cannot exceed 1000".into(),
        ));
    }

    // Check all node IDs are unique and valid
    let mut ids = HashSet::new();
    for node in &workflow.nodes {
        if node.id.is_empty() {
            return Err(Error::Validation("Node ID cannot be empty".into()));
        }

        // Validate node ID length
        if node.id.len() > MAX_NAME_LENGTH {
            return Err(Error::Validation(format!(
                "Node ID '{}...' exceeds maximum length of {} characters",
                &node.id[..20.min(node.id.len())],
                MAX_NAME_LENGTH
            )));
        }

        // Validate node ID format (same as workflow name)
        if !node
            .id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Error::Validation(format!(
                "Node ID '{}' must contain only alphanumeric characters, hyphens, and underscores",
                node.id
            )));
        }

        if !ids.insert(&node.id) {
            return Err(Error::Validation(format!("Duplicate node ID: {}", node.id)));
        }
    }

    // Check all dependencies exist
    for node in &workflow.nodes {
        for dep in &node.depends_on {
            if !ids.contains(dep) {
                return Err(Error::Validation(format!(
                    "Node '{}' depends on non-existent node '{}'",
                    node.id, dep
                )));
            }
        }

        // Check fallback nodes exist
        if let Some(error_config) = &node.on_error {
            if let Some(action) = &error_config.action {
                if matches!(action, OnErrorAction::Fallback)
                    && error_config.fallback_value.is_none()
                {
                    return Err(Error::Validation(format!(
                        "Node '{}' has on_error.action=fallback but missing fallback_value",
                        node.id
                    )));
                }
            }

            if let Some(fallback) = &error_config.fallback_node {
                if !ids.contains(fallback) {
                    return Err(Error::Validation(format!(
                        "Node '{}' has fallback to non-existent node '{}'",
                        node.id, fallback
                    )));
                }
            }
        }
    }

    // Check for cycles
    if has_cycle(workflow) {
        return Err(Error::Validation(
            "Workflow has circular dependencies".into(),
        ));
    }

    // Check node types are not empty
    for node in &workflow.nodes {
        if node.node_type.is_empty() {
            return Err(Error::Validation(format!(
                "Node '{}' has empty type",
                node.id
            )));
        }
    }

    Ok(())
}

fn has_cycle(workflow: &Workflow) -> bool {
    let mut visited = HashSet::new();
    let mut rec_stack = HashSet::new();

    fn dfs(
        node_id: &str,
        deps: &HashMap<&str, Vec<&str>>,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(node_id.to_string());
        rec_stack.insert(node_id.to_string());

        if let Some(neighbors) = deps.get(node_id) {
            for neighbor in neighbors {
                if !visited.contains(*neighbor) {
                    if dfs(neighbor, deps, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(*neighbor) {
                    return true;
                }
            }
        }

        rec_stack.remove(node_id);
        false
    }

    let deps: HashMap<&str, Vec<&str>> = workflow
        .nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                n.depends_on.iter().map(|s| s.as_str()).collect(),
            )
        })
        .collect();

    for node in &workflow.nodes {
        if !visited.contains(&node.id) && dfs(&node.id, &deps, &mut visited, &mut rec_stack) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::parse_workflow;

    #[test]
    fn test_validate_empty_name() {
        let yaml = r#"
name: ""
nodes:
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_invalid_name() {
        let yaml = r#"
name: "my workflow!"
nodes:
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_duplicate_ids() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_missing_dependency() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
    depends_on: [nonexistent]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_validate_cycle() {
        let yaml = r#"
name: test
nodes:
  - id: a
    type: http
    depends_on: [b]
  - id: b
    type: http
    depends_on: [a]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_err());
    }

    #[test]
    fn test_validate_valid_workflow() {
        let yaml = r#"
name: valid-workflow
nodes:
  - id: step1
    type: http
    config:
      url: https://example.com
  - id: step2
    type: transform
    depends_on: [step1]
"#;
        let workflow = parse_workflow(yaml).unwrap();
        assert!(validate_workflow(&workflow).is_ok());
    }

    #[test]
    fn test_validate_fallback_action_requires_value() {
        let yaml = r#"
name: fallback-workflow
nodes:
  - id: risky
    type: transform
    config:
      expression: "1 +"
    on_error:
      action: fallback
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("fallback_value"));
    }

    #[test]
    fn test_validate_workflow_name_too_long() {
        let long_name = "a".repeat(200);
        let yaml = format!(
            r#"
name: {}
nodes:
  - id: a
    type: http
"#,
            long_name
        );
        let workflow = parse_workflow(&yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("maximum length"));
    }

    #[test]
    fn test_validate_node_id_invalid_chars() {
        let yaml = r#"
name: test
nodes:
  - id: "node with spaces!"
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("alphanumeric"));
    }

    #[test]
    fn test_validate_timeout_too_large() {
        let yaml = r#"
name: test
settings:
  timeout_seconds: 999999999
nodes:
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("exceeds maximum"));
    }

    #[test]
    fn test_validate_max_concurrency_zero() {
        let yaml = r#"
name: test
settings:
  max_concurrency: 0
nodes:
  - id: a
    type: http
"#;
        let workflow = parse_workflow(yaml).unwrap();
        let err = validate_workflow(&workflow).unwrap_err();
        assert!(err.to_string().contains("at least 1"));
    }
}
