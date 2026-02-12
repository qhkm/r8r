//! Node trait and context types.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::Result;

/// Result of node execution.
#[derive(Debug, Clone)]
pub struct NodeResult {
    /// Output data from the node
    pub data: Value,
    /// Metadata (timing, debug info, etc.)
    pub metadata: Value,
}

impl NodeResult {
    /// Create a new result with just data.
    pub fn new(data: Value) -> Self {
        Self {
            data,
            metadata: serde_json::json!({}),
        }
    }

    /// Create a result with data and metadata.
    pub fn with_metadata(data: Value, metadata: Value) -> Self {
        Self { data, metadata }
    }

    /// Create an empty result.
    pub fn empty() -> Self {
        Self::new(Value::Null)
    }

    /// Check if the data is an array (for for_each processing).
    pub fn is_array(&self) -> bool {
        self.data.is_array()
    }

    /// Get data as array if it is one.
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        self.data.as_array()
    }
}

/// Context passed to a node during execution.
#[derive(Debug, Clone)]
pub struct NodeContext {
    /// Input data (from previous nodes or workflow input)
    pub input: Value,

    /// All node outputs so far (keyed by node ID)
    pub node_outputs: std::collections::HashMap<String, Value>,

    /// Workflow variables
    pub variables: Value,

    /// Execution ID
    pub execution_id: String,

    /// Workflow name
    pub workflow_name: String,

    /// Current item index (for for_each processing)
    pub item_index: Option<usize>,
}

impl NodeContext {
    /// Create a new context.
    pub fn new(execution_id: &str, workflow_name: &str) -> Self {
        Self {
            input: Value::Null,
            node_outputs: std::collections::HashMap::new(),
            variables: serde_json::json!({}),
            execution_id: execution_id.to_string(),
            workflow_name: workflow_name.to_string(),
            item_index: None,
        }
    }

    /// Set the input data.
    pub fn with_input(mut self, input: Value) -> Self {
        self.input = input;
        self
    }

    /// Set variables.
    pub fn with_variables(mut self, variables: Value) -> Self {
        self.variables = variables;
        self
    }

    /// Add a node output.
    pub fn add_output(&mut self, node_id: &str, output: Value) {
        self.node_outputs.insert(node_id.to_string(), output);
    }

    /// Get a previous node's output.
    pub fn get_output(&self, node_id: &str) -> Option<&Value> {
        self.node_outputs.get(node_id)
    }

    /// Clone context for a specific item (for for_each).
    pub fn for_item(&self, item: Value, index: usize) -> Self {
        Self {
            input: item,
            node_outputs: self.node_outputs.clone(),
            variables: self.variables.clone(),
            execution_id: self.execution_id.clone(),
            workflow_name: self.workflow_name.clone(),
            item_index: Some(index),
        }
    }
}

/// Trait that all node types must implement.
#[async_trait]
pub trait Node: Send + Sync {
    /// Get the node type name (e.g., "http", "transform", "agent").
    fn node_type(&self) -> &str;

    /// Execute the node with the given configuration and context.
    ///
    /// # Arguments
    /// * `config` - Node-specific configuration from the workflow YAML
    /// * `ctx` - Execution context with input data and previous outputs
    ///
    /// # Returns
    /// The node's output data wrapped in NodeResult
    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult>;

    /// Get a description of this node type.
    fn description(&self) -> &str {
        "A workflow node"
    }
}
