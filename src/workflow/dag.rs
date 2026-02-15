//! Workflow DAG (Directed Acyclic Graph) support.
//!
//! Allows workflows to declare dependencies on other workflows,
//! enabling multi-workflow pipelines.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::error::{Error, Result};

/// Workflow dependency declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDependency {
    /// Name of the workflow this depends on
    pub workflow: String,

    /// Input mapping from parent workflow output to this workflow's input
    /// Keys are input parameter names, values are expressions like "output.field"
    #[serde(default)]
    pub input_mapping: HashMap<String, String>,

    /// Condition for when this dependency is required
    /// If not specified, always required
    #[serde(default)]
    pub condition: Option<String>,
}

/// A DAG of workflows.
#[derive(Debug, Clone)]
pub struct WorkflowDag {
    /// Map of workflow name to its dependencies
    dependencies: HashMap<String, Vec<WorkflowDependency>>,
}

impl WorkflowDag {
    /// Create a new empty DAG.
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
        }
    }

    /// Add a workflow with its dependencies.
    pub fn add_workflow(&mut self, name: &str, deps: Vec<WorkflowDependency>) {
        self.dependencies.insert(name.to_string(), deps);
    }

    /// Check if a workflow has dependencies.
    pub fn has_dependencies(&self, name: &str) -> bool {
        self.dependencies
            .get(name)
            .map(|d| !d.is_empty())
            .unwrap_or(false)
    }

    /// Get dependencies for a workflow.
    pub fn get_dependencies(&self, name: &str) -> Option<&Vec<WorkflowDependency>> {
        self.dependencies.get(name)
    }

    /// Validate the DAG for cycles.
    pub fn validate(&self) -> Result<()> {
        // Build adjacency list
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut all_nodes: HashSet<&str> = HashSet::new();

        for (workflow, deps) in &self.dependencies {
            all_nodes.insert(workflow.as_str());
            let edges: Vec<&str> = deps.iter().map(|d| d.workflow.as_str()).collect();
            for dep in &edges {
                all_nodes.insert(dep);
            }
            graph.insert(workflow.as_str(), edges);
        }

        // Check for cycles using DFS
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for node in &all_nodes {
            if !visited.contains(node)
                && self.has_cycle(node, &graph, &mut visited, &mut rec_stack)?
            {
                return Err(Error::Validation(format!(
                    "Circular dependency detected involving workflow '{}'",
                    node
                )));
            }
        }

        Ok(())
    }

    fn has_cycle<'a>(
        &self,
        node: &'a str,
        graph: &HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        rec_stack: &mut HashSet<&'a str>,
    ) -> Result<bool> {
        visited.insert(node);
        rec_stack.insert(node);

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if self.has_cycle(neighbor, graph, visited, rec_stack)? {
                        return Ok(true);
                    }
                } else if rec_stack.contains(neighbor) {
                    return Ok(true);
                }
            }
        }

        rec_stack.remove(node);
        Ok(false)
    }

    /// Get execution order (topological sort).
    /// Returns workflows in order where dependencies come first.
    pub fn execution_order(&self, target: &str) -> Result<Vec<String>> {
        self.validate()?;

        let mut order = Vec::new();
        let mut visited = HashSet::new();

        self.visit_for_order(target, &mut visited, &mut order)?;

        Ok(order)
    }

    fn visit_for_order(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(node) {
            return Ok(());
        }

        visited.insert(node.to_string());

        // Visit dependencies first
        if let Some(deps) = self.dependencies.get(node) {
            for dep in deps {
                self.visit_for_order(&dep.workflow, visited, order)?;
            }
        }

        order.push(node.to_string());
        Ok(())
    }

    /// Get all workflows that depend on the given workflow.
    pub fn get_dependents(&self, name: &str) -> Vec<&str> {
        self.dependencies
            .iter()
            .filter_map(|(workflow, deps)| {
                if deps.iter().any(|d| d.workflow == name) {
                    Some(workflow.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Generate a simple text representation of the DAG.
    pub fn to_text(&self, root: &str) -> String {
        let mut lines = Vec::new();
        let mut visited = HashSet::new();

        self.format_node(root, 0, &mut visited, &mut lines);

        lines.join("\n")
    }

    fn format_node(
        &self,
        node: &str,
        depth: usize,
        visited: &mut HashSet<String>,
        lines: &mut Vec<String>,
    ) {
        let indent = "  ".repeat(depth);
        let marker = if depth == 0 { "" } else { "└─ " };

        if visited.contains(node) {
            lines.push(format!("{}{}{}  (already shown)", indent, marker, node));
            return;
        }

        visited.insert(node.to_string());
        lines.push(format!("{}{}{}", indent, marker, node));

        if let Some(deps) = self.dependencies.get(node) {
            for dep in deps {
                self.format_node(&dep.workflow, depth + 1, visited, lines);
            }
        }
    }
}

impl Default for WorkflowDag {
    fn default() -> Self {
        Self::new()
    }
}

/// Map outputs from a parent workflow to inputs for a child workflow.
pub fn map_workflow_outputs(
    parent_output: &serde_json::Value,
    mapping: &HashMap<String, String>,
) -> serde_json::Value {
    let mut result = serde_json::Map::new();

    for (input_key, output_expr) in mapping {
        let value = extract_value(parent_output, output_expr);
        result.insert(input_key.clone(), value);
    }

    serde_json::Value::Object(result)
}

/// Extract a value from output using a simple path expression.
fn extract_value(output: &serde_json::Value, expr: &str) -> serde_json::Value {
    let parts: Vec<&str> = expr.split('.').collect();
    let mut current = output;

    for part in parts {
        match current {
            serde_json::Value::Object(obj) => {
                if let Some(v) = obj.get(part) {
                    current = v;
                } else {
                    return serde_json::Value::Null;
                }
            }
            serde_json::Value::Array(arr) => {
                if let Ok(idx) = part.parse::<usize>() {
                    if let Some(v) = arr.get(idx) {
                        current = v;
                    } else {
                        return serde_json::Value::Null;
                    }
                } else {
                    return serde_json::Value::Null;
                }
            }
            _ => return serde_json::Value::Null,
        }
    }

    current.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_no_cycles() {
        let mut dag = WorkflowDag::new();

        dag.add_workflow(
            "workflow-c",
            vec![
                WorkflowDependency {
                    workflow: "workflow-a".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
                WorkflowDependency {
                    workflow: "workflow-b".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
            ],
        );

        dag.add_workflow(
            "workflow-b",
            vec![WorkflowDependency {
                workflow: "workflow-a".to_string(),
                input_mapping: HashMap::new(),
                condition: None,
            }],
        );

        dag.add_workflow("workflow-a", vec![]);

        assert!(dag.validate().is_ok());
    }

    #[test]
    fn test_dag_with_cycle() {
        let mut dag = WorkflowDag::new();

        dag.add_workflow(
            "workflow-a",
            vec![WorkflowDependency {
                workflow: "workflow-b".to_string(),
                input_mapping: HashMap::new(),
                condition: None,
            }],
        );

        dag.add_workflow(
            "workflow-b",
            vec![WorkflowDependency {
                workflow: "workflow-a".to_string(),
                input_mapping: HashMap::new(),
                condition: None,
            }],
        );

        assert!(dag.validate().is_err());
    }

    #[test]
    fn test_execution_order() {
        let mut dag = WorkflowDag::new();

        dag.add_workflow(
            "pipeline",
            vec![
                WorkflowDependency {
                    workflow: "fetch".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
                WorkflowDependency {
                    workflow: "transform".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
            ],
        );

        dag.add_workflow(
            "transform",
            vec![WorkflowDependency {
                workflow: "fetch".to_string(),
                input_mapping: HashMap::new(),
                condition: None,
            }],
        );

        dag.add_workflow("fetch", vec![]);

        let order = dag.execution_order("pipeline").unwrap();

        // fetch must come before transform, both must come before pipeline
        let fetch_idx = order.iter().position(|s| s == "fetch").unwrap();
        let transform_idx = order.iter().position(|s| s == "transform").unwrap();
        let pipeline_idx = order.iter().position(|s| s == "pipeline").unwrap();

        assert!(fetch_idx < transform_idx);
        assert!(transform_idx < pipeline_idx);
    }

    #[test]
    fn test_map_outputs() {
        let output = serde_json::json!({
            "data": {
                "items": [1, 2, 3],
                "count": 3
            },
            "status": "success"
        });

        let mut mapping = HashMap::new();
        mapping.insert("item_count".to_string(), "data.count".to_string());
        mapping.insert("result_status".to_string(), "status".to_string());

        let result = map_workflow_outputs(&output, &mapping);

        assert_eq!(result["item_count"], 3);
        assert_eq!(result["result_status"], "success");
    }

    #[test]
    fn test_dag_to_text() {
        let mut dag = WorkflowDag::new();

        dag.add_workflow(
            "main",
            vec![
                WorkflowDependency {
                    workflow: "step1".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
                WorkflowDependency {
                    workflow: "step2".to_string(),
                    input_mapping: HashMap::new(),
                    condition: None,
                },
            ],
        );

        dag.add_workflow("step1", vec![]);
        dag.add_workflow("step2", vec![]);

        let text = dag.to_text("main");
        assert!(text.contains("main"));
        assert!(text.contains("step1"));
        assert!(text.contains("step2"));
    }
}
