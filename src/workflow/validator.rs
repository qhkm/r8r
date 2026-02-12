//! Workflow validation.

use std::collections::{HashMap, HashSet};

use super::types::{OnErrorAction, Workflow};
use crate::error::{Error, Result};

/// Validate a workflow definition.
///
/// Checks for:
/// - Required fields (name, nodes)
/// - Unique node IDs
/// - Valid dependencies (referenced nodes exist)
/// - No circular dependencies
/// - Valid node types (if registry provided)
pub fn validate_workflow(workflow: &Workflow) -> Result<()> {
    // Check workflow has a name
    if workflow.name.is_empty() {
        return Err(Error::Validation("Workflow name is required".into()));
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

    // Check all node IDs are unique
    let mut ids = HashSet::new();
    for node in &workflow.nodes {
        if node.id.is_empty() {
            return Err(Error::Validation("Node ID cannot be empty".into()));
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
}
