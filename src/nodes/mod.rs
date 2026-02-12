//! Node implementations.
//!
//! Nodes are the building blocks of workflows. Each node type performs
//! a specific action (HTTP request, data transformation, AI decision, etc.).

mod agent;
mod aggregate;
mod filter;
mod http;
mod if_node;
mod limit;
mod merge;
mod registry;
mod set;
mod sort;
mod split;
mod switch;
mod transform;
mod types;

pub use agent::AgentNode;
pub use aggregate::AggregateNode;
pub use filter::FilterNode;
pub use http::HttpNode;
pub use if_node::IfNode;
pub use limit::LimitNode;
pub use merge::MergeNode;
pub use registry::NodeRegistry;
pub use set::SetNode;
pub use sort::SortNode;
pub use split::SplitNode;
pub use switch::SwitchNode;
pub use transform::TransformNode;
pub use types::{Node, NodeContext, NodeResult};
