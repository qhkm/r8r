//! SubWorkflow node - execute another workflow as a node.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, info};

use super::types::{Node, NodeContext, NodeResult};
use crate::engine::Executor;
use crate::error::{Error, Result};
use crate::workflow::parse_workflow;

/// SubWorkflow node for executing other workflows.
pub struct SubWorkflowNode;

impl SubWorkflowNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SubWorkflowNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct SubWorkflowConfig {
    /// Name of the workflow to execute
    workflow: String,

    /// Input to pass to the sub-workflow (optional, defaults to current input)
    #[serde(default)]
    input: Option<Value>,

    /// Field path in current input to use as sub-workflow input
    #[serde(default)]
    input_field: Option<String>,

    /// Whether to pass the parent's credentials to the sub-workflow
    #[serde(default = "default_true")]
    inherit_credentials: bool,

    /// Maximum execution time in seconds (0 = use workflow's default)
    #[serde(default)]
    timeout_seconds: u64,
}

fn default_true() -> bool {
    true
}

#[async_trait]
impl Node for SubWorkflowNode {
    fn node_type(&self) -> &str {
        "subworkflow"
    }

    fn description(&self) -> &str {
        "Execute another workflow as a sub-workflow"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: SubWorkflowConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid subworkflow config: {}", e)))?;

        // Get storage and registry from context
        let storage = ctx.storage.as_ref().ok_or_else(|| {
            Error::Node("SubWorkflow node requires storage in context".to_string())
        })?;

        let registry = ctx.registry.as_ref().ok_or_else(|| {
            Error::Node("SubWorkflow node requires registry in context".to_string())
        })?;

        // Load the workflow
        let stored = storage
            .get_workflow(&config.workflow)
            .await?
            .ok_or_else(|| Error::Node(format!("Workflow '{}' not found", config.workflow)))?;

        if !stored.enabled {
            return Err(Error::Node(format!(
                "Workflow '{}' is disabled",
                config.workflow
            )));
        }

        let workflow = parse_workflow(&stored.definition)?;

        info!(
            "SubWorkflow: executing '{}' from parent '{}'",
            config.workflow, ctx.workflow_name
        );

        // Determine input for sub-workflow
        let sub_input = if let Some(input) = config.input {
            input
        } else if let Some(field) = &config.input_field {
            get_field_value(&ctx.input, field)?
        } else {
            ctx.input.clone()
        };

        debug!("SubWorkflow input: {:?}", sub_input);

        // Create executor and apply optional inheritance/timeout overrides.
        let mut executor = Executor::new((**registry).clone(), storage.clone());
        if config.inherit_credentials {
            executor = executor.with_inherited_credentials((*ctx.credentials).clone());
        }
        if config.timeout_seconds > 0 {
            executor = executor.with_timeout_override(config.timeout_seconds);
        }

        let execution = executor
            .execute(&workflow, &stored.id, "subworkflow", sub_input)
            .await?;

        // Check execution result
        let output = match execution.status.to_string().as_str() {
            "completed" => execution.output.unwrap_or(Value::Null),
            "failed" => {
                return Err(Error::Node(format!(
                    "Sub-workflow '{}' failed: {}",
                    config.workflow,
                    execution
                        .error
                        .unwrap_or_else(|| "Unknown error".to_string())
                )));
            }
            status => {
                return Err(Error::Node(format!(
                    "Sub-workflow '{}' ended with unexpected status: {}",
                    config.workflow, status
                )));
            }
        };

        let duration_ms = execution
            .finished_at
            .map(|f| (f - execution.started_at).num_milliseconds());

        Ok(NodeResult::with_metadata(
            output,
            json!({
                "sub_workflow": config.workflow,
                "sub_execution_id": execution.id,
                "duration_ms": duration_ms,
            }),
        ))
    }
}

/// Get nested field value from JSON.
fn get_field_value(value: &Value, path: &str) -> Result<Value> {
    let mut current = value.clone();
    for part in path.split('.') {
        current = match current {
            Value::Object(obj) => obj
                .get(part)
                .cloned()
                .ok_or_else(|| Error::Node(format!("Field '{}' not found", path)))?,
            Value::Array(arr) => {
                let idx: usize = part
                    .parse()
                    .map_err(|_| Error::Node(format!("Invalid array index: {}", part)))?;
                arr.get(idx)
                    .cloned()
                    .ok_or_else(|| Error::Node(format!("Index {} out of bounds", idx)))?
            }
            _ => return Err(Error::Node(format!("Cannot index into {:?}", current))),
        };
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::NodeRegistry;
    use crate::storage::{SqliteStorage, StoredWorkflow};
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    struct RequireCredentialNode;

    #[async_trait]
    impl Node for RequireCredentialNode {
        fn node_type(&self) -> &str {
            "require_credential"
        }

        fn description(&self) -> &str {
            "Fails unless credential 'child-token' exists"
        }

        async fn execute(&self, _config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
            let token = ctx
                .get_credential("child-token")
                .cloned()
                .ok_or_else(|| Error::Node("missing credential".to_string()))?;
            Ok(NodeResult::new(json!({ "token": token })))
        }
    }

    struct SlowChildNode;

    #[async_trait]
    impl Node for SlowChildNode {
        fn node_type(&self) -> &str {
            "slow_child"
        }

        fn description(&self) -> &str {
            "Sleeps for configurable milliseconds"
        }

        async fn execute(&self, config: &Value, _ctx: &NodeContext) -> Result<NodeResult> {
            let sleep_ms = config
                .get("sleep_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(100);
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            Ok(NodeResult::new(json!({ "slept_ms": sleep_ms })))
        }
    }

    async fn save_test_workflow(storage: &SqliteStorage, id: &str, yaml: &str) {
        let workflow = parse_workflow(yaml).unwrap();
        let now = Utc::now();
        storage
            .save_workflow(&StoredWorkflow {
                id: id.to_string(),
                name: workflow.name.clone(),
                definition: yaml.to_string(),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }

    #[test]
    fn test_get_field_value_simple() {
        let input = json!({"name": "test", "value": 123});
        let result = get_field_value(&input, "name").unwrap();
        assert_eq!(result, json!("test"));
    }

    #[test]
    fn test_get_field_value_nested() {
        let input = json!({"data": {"user": {"id": 42}}});
        let result = get_field_value(&input, "data.user.id").unwrap();
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_get_field_value_array() {
        let input = json!({"items": ["a", "b", "c"]});
        let result = get_field_value(&input, "items.1").unwrap();
        assert_eq!(result, json!("b"));
    }

    #[test]
    fn test_get_field_value_missing() {
        let input = json!({"name": "test"});
        let result = get_field_value(&input, "missing");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_subworkflow_requires_storage() {
        let node = SubWorkflowNode::new();
        let config = json!({
            "workflow": "child-workflow"
        });
        // Context without storage should fail
        let ctx = NodeContext::new("exec-1", "parent-workflow");

        let result = node.execute(&config, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("requires storage"));
    }

    #[tokio::test]
    async fn test_subworkflow_inherits_credentials_when_enabled() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::default();
        registry.register(Arc::new(RequireCredentialNode));
        let registry = Arc::new(registry);

        let child_yaml = r#"
name: child-cred
nodes:
  - id: check
    type: require_credential
"#;
        save_test_workflow(&storage, "wf-child-cred", child_yaml).await;

        let mut creds = HashMap::new();
        creds.insert("child-token".to_string(), "abc123".to_string());

        let ctx = NodeContext::new("exec-1", "parent")
            .with_storage(storage.clone())
            .with_registry(registry)
            .with_credentials(creds);

        let node = SubWorkflowNode::new();
        let config = json!({
            "workflow": "child-cred",
            "inherit_credentials": true
        });

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["token"], "abc123");
    }

    #[tokio::test]
    async fn test_subworkflow_does_not_inherit_credentials_when_disabled() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::default();
        registry.register(Arc::new(RequireCredentialNode));
        let registry = Arc::new(registry);

        let child_yaml = r#"
name: child-no-inherit
nodes:
  - id: check
    type: require_credential
"#;
        save_test_workflow(&storage, "wf-child-no-inherit", child_yaml).await;

        let mut creds = HashMap::new();
        creds.insert("child-token".to_string(), "abc123".to_string());

        let ctx = NodeContext::new("exec-1", "parent")
            .with_storage(storage.clone())
            .with_registry(registry)
            .with_credentials(creds);

        let node = SubWorkflowNode::new();
        let config = json!({
            "workflow": "child-no-inherit",
            "inherit_credentials": false
        });

        let err = node.execute(&config, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("missing credential"));
    }

    #[tokio::test]
    async fn test_subworkflow_timeout_override_applies() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::default();
        registry.register(Arc::new(SlowChildNode));
        let registry = Arc::new(registry);

        let child_yaml = r#"
name: child-slow
settings:
  timeout_seconds: 5
nodes:
  - id: wait
    type: slow_child
    config:
      sleep_ms: 1500
"#;
        save_test_workflow(&storage, "wf-child-slow", child_yaml).await;

        let ctx = NodeContext::new("exec-1", "parent")
            .with_storage(storage.clone())
            .with_registry(registry);

        let node = SubWorkflowNode::new();
        let config = json!({
            "workflow": "child-slow",
            "timeout_seconds": 1
        });

        let err = node.execute(&config, &ctx).await.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }
}
