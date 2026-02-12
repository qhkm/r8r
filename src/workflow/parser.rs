//! Workflow YAML parser.

use std::path::Path;

use super::types::Workflow;
use crate::error::{Error, Result};

/// Parse a workflow from a YAML string.
pub fn parse_workflow(yaml: &str) -> Result<Workflow> {
    if yaml.trim().is_empty() {
        return Err(Error::Parse("Empty workflow definition".to_string()));
    }

    let workflow: Workflow = serde_yaml::from_str(yaml).map_err(|e| {
        let msg = e.to_string();
        if let Some(field) = extract_missing_field(&msg) {
            Error::Parse(format!("Missing required field: {}", field))
        } else {
            Error::Parse(format!("Invalid YAML: {}", msg))
        }
    })?;
    Ok(workflow)
}

/// Parse a workflow from a file path.
pub fn parse_workflow_file(path: &Path) -> Result<Workflow> {
    let content = std::fs::read_to_string(path)?;
    parse_workflow(&content)
}

fn extract_missing_field(error_message: &str) -> Option<&str> {
    let marker = "missing field `";
    let start = error_message.find(marker)? + marker.len();
    let rest = &error_message[start..];
    let end = rest.find('`')?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_workflow() {
        let yaml = r#"
name: test-workflow
description: A test workflow

triggers:
  - type: manual

nodes:
  - id: step1
    type: http
    config:
      url: https://example.com
      method: GET

  - id: step2
    type: transform
    config:
      expression: "data.items"
    depends_on: [step1]
"#;

        let workflow = parse_workflow(yaml).unwrap();
        assert_eq!(workflow.name, "test-workflow");
        assert_eq!(workflow.nodes.len(), 2);
        assert_eq!(workflow.nodes[1].depends_on, vec!["step1"]);
    }

    #[test]
    fn test_parse_workflow_with_agent_node() {
        let yaml = r#"
name: smart-router
description: Route with AI

nodes:
  - id: classify
    type: agent
    config:
      prompt: "Classify this: {{ input }}"
      response_format: json

  - id: route
    type: switch
    depends_on: [classify]
"#;

        let workflow = parse_workflow(yaml).unwrap();
        assert!(workflow.has_agent_nodes());
        assert_eq!(workflow.node_types(), vec!["agent", "switch"]);
    }

    #[test]
    fn test_topological_sort() {
        let yaml = r#"
name: test
nodes:
  - id: c
    type: http
    depends_on: [a, b]
  - id: a
    type: http
  - id: b
    type: http
    depends_on: [a]
"#;

        let workflow = parse_workflow(yaml).unwrap();
        let order = workflow.topological_sort();

        // a must come before b and c
        // b must come before c
        let a_pos = order.iter().position(|&x| x == "a").unwrap();
        let b_pos = order.iter().position(|&x| x == "b").unwrap();
        let c_pos = order.iter().position(|&x| x == "c").unwrap();

        assert!(a_pos < b_pos);
        assert!(a_pos < c_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn test_parse_cron_trigger() {
        let yaml = r#"
name: scheduled
triggers:
  - type: cron
    schedule: "*/5 * * * *"
    timezone: Asia/Kuala_Lumpur
nodes:
  - id: run
    type: http
    config:
      url: https://example.com
"#;

        let workflow = parse_workflow(yaml).unwrap();
        assert_eq!(workflow.triggers.len(), 1);

        match &workflow.triggers[0] {
            super::super::Trigger::Cron { schedule, timezone } => {
                assert_eq!(schedule, "*/5 * * * *");
                assert_eq!(timezone.as_deref(), Some("Asia/Kuala_Lumpur"));
            }
            _ => panic!("Expected cron trigger"),
        }
    }

    #[test]
    fn test_parse_inputs() {
        let yaml = r#"
name: with-inputs
inputs:
  threshold:
    type: number
    default: 10
    description: Stock threshold
    required: false
nodes:
  - id: check
    type: transform
    config:
      expression: "input >= threshold"
"#;

        let workflow = parse_workflow(yaml).unwrap();
        assert!(workflow.inputs.contains_key("threshold"));

        let threshold = &workflow.inputs["threshold"];
        assert_eq!(threshold.input_type, "number");
        assert_eq!(threshold.default, Some(serde_json::json!(10)));
    }

    #[test]
    fn test_parse_empty_workflow() {
        let result = parse_workflow("");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("empty workflow"));
    }

    #[test]
    fn test_parse_invalid_yaml() {
        let result = parse_workflow("name: [broken");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .to_lowercase()
            .contains("invalid yaml"));
    }

    #[test]
    fn test_parse_missing_required_field_name() {
        let yaml = r#"
nodes:
  - id: step1
    type: transform
    config:
      expression: "1"
"#;
        let result = parse_workflow(yaml);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing required field: name"));
    }
}
