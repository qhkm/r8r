//! Workflow definition, parsing, and validation.
//!
//! Workflows are defined in YAML format and consist of:
//! - Triggers: What starts the workflow (cron, webhook, manual)
//! - Nodes: The steps to execute
//! - Settings: Global configuration

mod cache;
mod parser;
mod schema;
mod types;
mod validator;

pub use cache::{CacheStats, WorkflowCache};
pub use parser::{parse_workflow, parse_workflow_file};
pub use schema::{
    export_workflow_yaml, validate_workflow_yaml, workflow_schema, ValidationError,
    WorkflowSchemaValidator,
};
pub use types::*;
pub use validator::validate_workflow;
