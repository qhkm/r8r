//! Node implementations.
//!
//! Nodes are the building blocks of workflows. Each node type performs
//! a specific action (HTTP request, data transformation, AI decision, etc.).

mod agent;
mod aggregate;
pub mod circuit_breaker;
mod crypto;
mod database;
mod datetime;
mod debug;
mod dedupe;
mod email;
mod filter;
mod http;
mod if_node;
mod limit;
mod merge;
mod registry;
mod s3;
mod set;
mod slack;
mod sort;
mod split;
mod subworkflow;
mod summarize;
mod switch;
pub mod template;
mod transform;
mod types;
mod variables;
mod wait;

pub use agent::AgentNode;
pub use aggregate::AggregateNode;
pub use crypto::CryptoNode;
pub use database::DatabaseNode;
pub use datetime::DateTimeNode;
pub use debug::DebugNode;
pub use dedupe::DedupeNode;
pub use email::EmailNode;
pub use filter::FilterNode;
pub use http::HttpNode;
pub use if_node::IfNode;
pub use limit::LimitNode;
pub use merge::MergeNode;
pub use registry::NodeRegistry;
pub use s3::S3Node;
pub use set::SetNode;
pub use slack::SlackNode;
pub use sort::SortNode;
pub use split::SplitNode;
pub use subworkflow::SubWorkflowNode;
pub use summarize::SummarizeNode;
pub use switch::SwitchNode;
pub use transform::TransformNode;
pub use types::{Node, NodeContext, NodeResult};
pub use variables::VariablesNode;
pub use wait::WaitNode;
