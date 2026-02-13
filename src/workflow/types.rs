//! Workflow type definitions.
//!
//! These types are designed to be LLM-friendly - consistent patterns
//! that AI agents can easily generate.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete workflow definition.
///
/// # Example YAML
///
/// ```yaml
/// name: order-notification
/// description: Send notifications for new orders
/// version: 1
///
/// triggers:
///   - type: cron
///     schedule: "*/5 * * * *"
///
/// nodes:
///   - id: fetch-orders
///     type: http
///     config:
///       url: https://api.example.com/orders
///       method: GET
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique workflow name (used as identifier)
    pub name: String,

    /// Human-readable description
    #[serde(default)]
    pub description: String,

    /// Version number (for tracking changes)
    #[serde(default = "default_version")]
    pub version: u32,

    /// Triggers that start this workflow
    #[serde(default)]
    pub triggers: Vec<Trigger>,

    /// Input schema for the workflow
    #[serde(default)]
    pub inputs: HashMap<String, InputDefinition>,

    /// JSON Schema for validating workflow input
    /// Use this for comprehensive validation with patterns, constraints, etc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,

    /// Workflow variables (constants and computed values)
    #[serde(default)]
    pub variables: HashMap<String, serde_json::Value>,

    /// Dependencies on other workflows (for workflow pipelines)
    #[serde(default)]
    pub depends_on_workflows: Vec<super::dag::WorkflowDependency>,

    /// Nodes (steps) in the workflow
    pub nodes: Vec<Node>,

    /// Global workflow settings
    #[serde(default)]
    pub settings: WorkflowSettings,
}

fn default_version() -> u32 {
    1
}

/// Input parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputDefinition {
    /// Parameter type: string, number, boolean, object, array
    #[serde(rename = "type")]
    pub input_type: String,

    /// Default value if not provided
    #[serde(default)]
    pub default: Option<serde_json::Value>,

    /// Description of the parameter
    #[serde(default)]
    pub description: String,

    /// Whether this input is required
    #[serde(default)]
    pub required: bool,
}

/// Workflow trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Cron-based schedule
    Cron {
        /// Cron expression (e.g., "*/5 * * * *")
        schedule: String,
        /// Timezone (default: UTC)
        #[serde(default)]
        timezone: Option<String>,
    },
    /// Manual trigger via CLI/API
    Manual,
    /// Webhook trigger (HTTP endpoint)
    Webhook {
        /// Custom path (default: /webhooks/{workflow_name})
        #[serde(default)]
        path: Option<String>,
        /// HTTP method (default: POST)
        #[serde(default)]
        method: Option<String>,
    },
    /// Event-based trigger (from another workflow or external system)
    Event {
        /// Event name to listen for
        event: String,
        /// Optional filter expression
        #[serde(default)]
        filter: Option<String>,
    },
}

/// A node (step) in the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique node ID within this workflow
    pub id: String,

    /// Node type (http, transform, agent, etc.)
    #[serde(rename = "type")]
    pub node_type: String,

    /// Node-specific configuration
    #[serde(default)]
    pub config: serde_json::Value,

    /// Dependencies - nodes that must complete before this one
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Run this node once per item in input array
    #[serde(default)]
    pub for_each: bool,

    /// Condition expression - only run if true
    #[serde(default)]
    pub condition: Option<String>,

    /// Retry configuration
    #[serde(default)]
    pub retry: Option<RetryConfig>,

    /// Error handling configuration
    #[serde(default)]
    pub on_error: Option<ErrorConfig>,

    /// Pinned data for testing - when set, this data is used instead of executing the node.
    /// Useful for testing workflows without hitting external services.
    #[serde(default)]
    pub pinned_data: Option<serde_json::Value>,

    /// Timeout for this specific node in seconds (overrides workflow default)
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// Retry configuration for a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum retry attempts
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Delay between retries in seconds
    #[serde(default = "default_delay_seconds")]
    pub delay_seconds: u64,

    /// Backoff strategy
    #[serde(default)]
    pub backoff: BackoffType,
}

fn default_max_attempts() -> u32 {
    3
}

fn default_delay_seconds() -> u64 {
    60
}

/// Backoff strategy for retries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffType {
    /// Fixed delay between retries
    #[default]
    Fixed,
    /// Linearly increasing delay
    Linear,
    /// Exponentially increasing delay
    Exponential,
}

/// Error handling configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorConfig {
    /// Continue workflow even if this node fails
    #[serde(default, rename = "continue")]
    pub continue_on_error: bool,

    /// Error action for this node: fail | continue | skip | fallback
    #[serde(default)]
    pub action: Option<OnErrorAction>,

    /// Inline fallback output value when action is `fallback`
    #[serde(default)]
    pub fallback_value: Option<serde_json::Value>,

    /// Fallback node to run on error
    #[serde(default)]
    pub fallback_node: Option<String>,
}

/// Inline error actions for node failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnErrorAction {
    /// Fail execution immediately
    Fail,
    /// Continue execution with null output
    Continue,
    /// Skip node output (null) and continue
    Skip,
    /// Use `fallback_value` as node output and continue
    Fallback,
}

/// Global workflow settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSettings {
    /// Maximum execution time in seconds
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,

    /// Maximum concurrent node executions
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,

    /// Whether to store execution traces
    #[serde(default = "default_true")]
    pub store_traces: bool,

    /// Chunk size for streaming for_each processing.
    /// When > 0, for_each nodes process arrays in batches of this size,
    /// reducing memory usage for very large datasets.
    /// Default: 0 (no chunking, process all items at once)
    #[serde(default)]
    pub chunk_size: usize,

    /// Maximum number of items allowed in a for_each loop.
    /// Prevents memory exhaustion from processing extremely large arrays.
    /// Default: 10,000
    #[serde(default = "default_max_for_each_items")]
    pub max_for_each_items: usize,

    /// Name of workflow to execute when this workflow fails.
    /// The error workflow receives the error context as input.
    #[serde(default)]
    pub error_workflow: Option<String>,

    /// Enable debug mode for detailed execution tracing.
    /// When enabled, captures full input/output data for each node.
    #[serde(default)]
    pub debug: bool,
}

impl Default for WorkflowSettings {
    fn default() -> Self {
        Self {
            timeout_seconds: default_timeout(),
            max_concurrency: default_concurrency(),
            store_traces: true,
            chunk_size: 0,
            max_for_each_items: default_max_for_each_items(),
            error_workflow: None,
            debug: false,
        }
    }
}

fn default_timeout() -> u64 {
    3600 // 1 hour
}

fn default_concurrency() -> usize {
    10
}

fn default_max_for_each_items() -> usize {
    10_000 // 10,000 items max per for_each loop
}

fn default_true() -> bool {
    true
}

impl Workflow {
    /// Get a node by ID.
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Get nodes in topological order (respecting dependencies).
    ///
    /// Returns node IDs in an order where all dependencies come before
    /// the nodes that depend on them.
    pub fn topological_sort(&self) -> Vec<&str> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut temp_visited = std::collections::HashSet::new();

        for node in &self.nodes {
            if !visited.contains(node.id.as_str()) {
                self.visit_node(&node.id, &mut visited, &mut temp_visited, &mut result);
            }
        }

        result
    }

    fn visit_node<'a>(
        &'a self,
        node_id: &'a str,
        visited: &mut std::collections::HashSet<&'a str>,
        temp: &mut std::collections::HashSet<&'a str>,
        result: &mut Vec<&'a str>,
    ) {
        if temp.contains(node_id) {
            return; // Cycle detected, skip
        }
        if visited.contains(node_id) {
            return;
        }

        temp.insert(node_id);

        if let Some(node) = self.get_node(node_id) {
            for dep in &node.depends_on {
                self.visit_node(dep, visited, temp, result);
            }
        }

        temp.remove(node_id);
        visited.insert(node_id);
        result.push(node_id);
    }

    /// Check if this workflow has any agent nodes.
    pub fn has_agent_nodes(&self) -> bool {
        self.nodes.iter().any(|n| n.node_type == "agent")
    }

    /// Get all node types used in this workflow.
    pub fn node_types(&self) -> Vec<&str> {
        let mut types: Vec<&str> = self.nodes.iter().map(|n| n.node_type.as_str()).collect();
        types.sort();
        types.dedup();
        types
    }
}
