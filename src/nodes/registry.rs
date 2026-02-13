//! Node registry - manages available node types.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::types::{Node, NodeContext, NodeResult};
use super::{
    AgentNode, AggregateNode, CryptoNode, DatabaseNode, DateTimeNode, DebugNode, DedupeNode,
    EmailNode, FilterNode, HttpNode, IfNode, LimitNode, MergeNode, S3Node, SetNode, SlackNode,
    SortNode, SplitNode, SubWorkflowNode, SummarizeNode, SwitchNode, TransformNode, VariablesNode,
    WaitNode,
};
use crate::error::{Error, Result};

/// Registry of available node types.
#[derive(Clone)]
pub struct NodeRegistry {
    nodes: HashMap<String, Arc<dyn Node>>,
}

impl NodeRegistry {
    /// Create a new registry with default nodes.
    pub fn new() -> Self {
        let mut registry = Self {
            nodes: HashMap::new(),
        };

        // Register built-in nodes
        registry.register(Arc::new(HttpNode::new()));
        registry.register(Arc::new(TransformNode::new()));
        registry.register(Arc::new(AgentNode::new()));
        registry.register(Arc::new(IfNode::new()));
        registry.register(Arc::new(SwitchNode::new()));
        registry.register(Arc::new(MergeNode::new()));
        registry.register(Arc::new(FilterNode::new()));
        registry.register(Arc::new(SortNode::new()));
        registry.register(Arc::new(LimitNode::new()));
        registry.register(Arc::new(SetNode::new()));
        registry.register(Arc::new(AggregateNode::new()));
        registry.register(Arc::new(SplitNode::new()));
        registry.register(Arc::new(WaitNode::new()));
        registry.register(Arc::new(CryptoNode::new()));
        registry.register(Arc::new(DateTimeNode::new()));
        registry.register(Arc::new(DedupeNode::new()));
        registry.register(Arc::new(SubWorkflowNode::new()));
        registry.register(Arc::new(VariablesNode::new()));
        registry.register(Arc::new(DebugNode::new()));
        registry.register(Arc::new(SummarizeNode::new()));
        registry.register(Arc::new(EmailNode::new()));
        registry.register(Arc::new(SlackNode::new()));
        registry.register(Arc::new(DatabaseNode::new()));
        registry.register(Arc::new(S3Node::new()));

        registry
    }

    /// Create an empty registry (for testing).
    pub fn empty() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Register a node type.
    pub fn register(&mut self, node: Arc<dyn Node>) {
        self.nodes.insert(node.node_type().to_string(), node);
    }

    /// Get a node by type name.
    pub fn get(&self, node_type: &str) -> Option<Arc<dyn Node>> {
        self.nodes.get(node_type).cloned()
    }

    /// Check if a node type is registered.
    pub fn has(&self, node_type: &str) -> bool {
        self.nodes.contains_key(node_type)
    }

    /// Execute a node by type.
    pub async fn execute(
        &self,
        node_type: &str,
        config: &Value,
        ctx: &NodeContext,
    ) -> Result<NodeResult> {
        let node = self
            .get(node_type)
            .ok_or_else(|| Error::Node(format!("Unknown node type: {}", node_type)))?;

        node.execute(config, ctx).await
    }

    /// List all registered node types.
    pub fn list(&self) -> Vec<&str> {
        self.nodes.keys().map(|s| s.as_str()).collect()
    }

    /// Get descriptions of all registered nodes.
    pub fn descriptions(&self) -> Vec<(&str, &str)> {
        self.nodes
            .iter()
            .map(|(name, node)| (name.as_str(), node.description()))
            .collect()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_default_nodes() {
        let registry = NodeRegistry::new();

        assert!(registry.has("http"));
        assert!(registry.has("transform"));
        assert!(registry.has("agent"));
        assert!(registry.has("if"));
        assert!(registry.has("switch"));
        assert!(registry.has("merge"));
        assert!(registry.has("filter"));
        assert!(registry.has("sort"));
        assert!(registry.has("limit"));
        assert!(registry.has("set"));
        assert!(registry.has("aggregate"));
        assert!(registry.has("split"));
        assert!(registry.has("wait"));
        assert!(registry.has("crypto"));
        assert!(registry.has("datetime"));
        assert!(registry.has("dedupe"));
        assert!(registry.has("subworkflow"));
        assert!(registry.has("variables"));
        assert!(registry.has("debug"));
        assert!(registry.has("summarize"));
        assert!(registry.has("email"));
        assert!(registry.has("slack"));
        assert!(registry.has("database"));
        assert!(registry.has("s3"));
        assert!(!registry.has("nonexistent"));
    }

    #[test]
    fn test_registry_list() {
        let registry = NodeRegistry::new();
        let types = registry.list();

        assert!(types.contains(&"http"));
        assert!(types.contains(&"transform"));
        assert!(types.contains(&"agent"));
        assert!(types.contains(&"if"));
        assert!(types.contains(&"switch"));
        assert!(types.contains(&"merge"));
        assert!(types.contains(&"filter"));
        assert!(types.contains(&"sort"));
        assert!(types.contains(&"limit"));
        assert!(types.contains(&"set"));
        assert!(types.contains(&"aggregate"));
        assert!(types.contains(&"split"));
        assert!(types.contains(&"wait"));
        assert!(types.contains(&"crypto"));
        assert!(types.contains(&"datetime"));
        assert!(types.contains(&"dedupe"));
        assert!(types.contains(&"subworkflow"));
        assert!(types.contains(&"variables"));
        assert!(types.contains(&"debug"));
        assert!(types.contains(&"summarize"));
        assert!(types.contains(&"email"));
        assert!(types.contains(&"slack"));
        assert!(types.contains(&"database"));
        assert!(types.contains(&"s3"));
    }
}
