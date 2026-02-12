//! Workflow executor.

use chrono::Utc;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::nodes::{Node, NodeContext, NodeRegistry, NodeResult};
use crate::storage::{Execution, ExecutionStatus, NodeExecution, SqliteStorage};
use crate::workflow::{parse_workflow, BackoffType, Node as WorkflowNode, RetryConfig, Workflow};

/// Workflow executor.
pub struct Executor {
    registry: NodeRegistry,
    storage: SqliteStorage,
}

impl Executor {
    /// Create a new executor.
    pub fn new(registry: NodeRegistry, storage: SqliteStorage) -> Self {
        Self { registry, storage }
    }

    /// Execute a workflow.
    pub async fn execute(
        &self,
        workflow: &Workflow,
        workflow_id: &str,
        trigger_type: &str,
        input: Value,
    ) -> Result<Execution> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let timeout_seconds = workflow.settings.timeout_seconds.max(1);
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);

        info!(
            "Starting execution {} of workflow '{}'",
            execution_id, workflow.name
        );

        // Create execution record
        let mut execution = Execution {
            id: execution_id.clone(),
            workflow_id: workflow_id.to_string(),
            workflow_name: workflow.name.clone(),
            status: ExecutionStatus::Running,
            trigger_type: trigger_type.to_string(),
            input: input.clone(),
            output: None,
            started_at,
            finished_at: None,
            error: None,
        };

        self.storage.save_execution(&execution).await?;

        // Create context
        let mut ctx = NodeContext::new(&execution_id, &workflow.name).with_input(input);

        // Get topological order
        let order = workflow.topological_sort();
        debug!("Execution order: {:?}", order);

        // Execute nodes in order
        let mut last_output = Value::Null;
        let mut failed = false;
        let mut error_msg = None;

        for node_id in order {
            if remaining_until(deadline).is_none() {
                failed = true;
                error_msg = Some(timeout_error_message(timeout_seconds));
                break;
            }

            let node = match workflow.get_node(node_id) {
                Some(n) => n,
                None => {
                    warn!("Node '{}' not found in workflow", node_id);
                    continue;
                }
            };

            let node_impl = match self.registry.get(&node.node_type) {
                Some(n) => n,
                None => {
                    failed = true;
                    error_msg = Some(format!("Unknown node type: {}", node.node_type));
                    error!("{}", error_msg.as_deref().unwrap_or("Unknown node type"));
                    break;
                }
            };

            // Gather input from dependencies
            let node_input = if node.depends_on.is_empty() {
                ctx.input.clone()
            } else if node.depends_on.len() == 1 {
                ctx.get_output(&node.depends_on[0])
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                // Multiple dependencies: merge outputs
                let mut merged = serde_json::Map::new();
                for dep in &node.depends_on {
                    if let Some(output) = ctx.get_output(dep) {
                        merged.insert(dep.clone(), output.clone());
                    }
                }
                Value::Object(merged)
            };

            // Create node execution record
            let mut node_exec = NodeExecution {
                id: uuid::Uuid::new_v4().to_string(),
                execution_id: execution_id.clone(),
                node_id: node_id.to_string(),
                status: ExecutionStatus::Running,
                input: node_input.clone(),
                output: None,
                started_at: Utc::now(),
                finished_at: None,
                error: None,
            };
            self.storage.save_node_execution(&node_exec).await?;

            // Evaluate optional condition before execution.
            if let Some(condition) = &node.condition {
                match evaluate_condition(condition, &node_input, &ctx) {
                    Ok(false) => {
                        info!("Skipping node '{}' due to false condition", node_id);
                        let skipped_output = Value::Null;
                        ctx.add_output(node_id, skipped_output.clone());
                        last_output = skipped_output.clone();

                        node_exec.status = ExecutionStatus::Completed;
                        node_exec.output = Some(skipped_output);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        if continue_on_error(node) {
                            warn!(
                                "Node '{}' condition evaluation failed (continuing): {}",
                                node_id, e
                            );
                            ctx.add_output(node_id, Value::Null);
                            last_output = Value::Null;

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            continue;
                        }

                        failed = true;
                        error_msg = Some(e.to_string());
                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.error = error_msg.clone();
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        break;
                    }
                }
            }

            info!("Executing node '{}' [{}]", node_id, node.node_type);

            if node.for_each {
                let Some(items) = node_input.as_array() else {
                    let message = format!(
                        "Node '{}' has for_each=true but input is not an array",
                        node_id
                    );

                    if continue_on_error(node) {
                        warn!("{}", message);
                        ctx.add_output(node_id, Value::Null);
                        last_output = Value::Null;

                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.error = Some(message);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        continue;
                    }

                    error!("{}", message);
                    failed = true;
                    error_msg = Some(message.clone());
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = Some(message);
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    break;
                };

                let mut results = vec![Value::Null; items.len()];
                let mut join_set: JoinSet<(usize, Result<Value>)> = JoinSet::new();
                let max_concurrency = workflow.settings.max_concurrency.max(1);
                let mut next_index = 0usize;
                let task_template = ForEachTaskTemplate {
                    base_ctx: ctx.clone(),
                    node_impl: node_impl.clone(),
                    node_type: node.node_type.clone(),
                    node_config: node.config.clone(),
                    retry: node.retry.clone(),
                    deadline,
                };

                while next_index < items.len() && join_set.len() < max_concurrency {
                    if remaining_until(deadline).is_none() {
                        failed = true;
                        error_msg = Some(timeout_error_message(timeout_seconds));
                        break;
                    }

                    spawn_for_each_item(
                        &mut join_set,
                        next_index,
                        items[next_index].clone(),
                        &task_template,
                    );
                    next_index += 1;
                }

                while !failed {
                    let joined = join_set.join_next().await;
                    let Some(joined) = joined else {
                        break;
                    };

                    match joined {
                        Ok((idx, Ok(data))) => {
                            results[idx] = data;
                        }
                        Ok((idx, Err(e))) => {
                            if continue_on_error(node) {
                                warn!("Node '{}' item {} failed (continuing): {}", node_id, idx, e);
                                results[idx] = Value::Null;
                            } else {
                                error!("Node '{}' item {} failed: {}", node_id, idx, e);
                                failed = true;
                                error_msg = Some(format!("{} (at item {})", e, idx));
                                join_set.abort_all();
                                while join_set.join_next().await.is_some() {}
                                break;
                            }
                        }
                        Err(e) => {
                            failed = true;
                            error_msg =
                                Some(format!("Node '{}' worker task join failed: {}", node_id, e));
                            join_set.abort_all();
                            while join_set.join_next().await.is_some() {}
                            break;
                        }
                    }

                    while !failed && next_index < items.len() && join_set.len() < max_concurrency {
                        if remaining_until(deadline).is_none() {
                            failed = true;
                            error_msg = Some(timeout_error_message(timeout_seconds));
                            join_set.abort_all();
                            while join_set.join_next().await.is_some() {}
                            break;
                        }

                        spawn_for_each_item(
                            &mut join_set,
                            next_index,
                            items[next_index].clone(),
                            &task_template,
                        );
                        next_index += 1;
                    }
                }

                if failed {
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = error_msg.clone();
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    break;
                }

                let output = Value::Array(results);
                ctx.add_output(node_id, output.clone());
                last_output = output.clone();

                node_exec.status = ExecutionStatus::Completed;
                node_exec.output = Some(output);
                node_exec.finished_at = Some(Utc::now());
                self.storage.save_node_execution(&node_exec).await?;
            } else {
                // Regular execution
                let node_ctx = NodeContext {
                    input: node_input,
                    node_outputs: ctx.node_outputs.clone(),
                    variables: ctx.variables.clone(),
                    execution_id: ctx.execution_id.clone(),
                    workflow_name: ctx.workflow_name.clone(),
                    item_index: None,
                };

                let result = execute_node_with_retry(
                    node_impl,
                    &node.node_type,
                    &node.config,
                    &node_ctx,
                    node.retry.as_ref(),
                    deadline,
                )
                .await;

                match result {
                    Ok(result) => {
                        info!("Node '{}' completed successfully", node_id);
                        ctx.add_output(node_id, result.data.clone());
                        last_output = result.data.clone();

                        node_exec.status = ExecutionStatus::Completed;
                        node_exec.output = Some(result.data);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                    }
                    Err(e) => {
                        if continue_on_error(node) {
                            warn!("Node '{}' failed (continuing): {}", node_id, e);
                            ctx.add_output(node_id, Value::Null);
                            last_output = Value::Null;

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                        } else {
                            error!("Node '{}' failed: {}", node_id, e);
                            failed = true;
                            error_msg = Some(e.to_string());

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            break;
                        }
                    }
                }
            }
        }

        // Update execution record
        execution.finished_at = Some(Utc::now());
        if failed {
            execution.status = ExecutionStatus::Failed;
            execution.error = error_msg;
        } else {
            execution.status = ExecutionStatus::Completed;
            execution.output = Some(last_output);
        }

        self.storage.save_execution(&execution).await?;

        let duration = execution
            .finished_at
            .map(|f| f - execution.started_at)
            .map(|d| d.num_milliseconds())
            .unwrap_or(0);

        info!(
            "Execution {} completed with status {} ({}ms)",
            execution_id, execution.status, duration
        );

        Ok(execution)
    }

    /// Replay a previous execution with optional new input.
    pub async fn replay(
        &self,
        execution_id: &str,
        modified_input: Option<Value>,
    ) -> Result<Execution> {
        let original = self
            .storage
            .get_execution(execution_id)
            .await?
            .ok_or_else(|| Error::Execution(format!("Execution not found: {}", execution_id)))?;

        let stored_workflow = self
            .storage
            .get_workflow_by_id(&original.workflow_id)
            .await?
            .ok_or_else(|| {
                Error::Execution(format!("Workflow not found for execution {}", execution_id))
            })?;

        let workflow = parse_workflow(&stored_workflow.definition)?;
        let input = modified_input.unwrap_or(original.input);

        self.execute(&workflow, &stored_workflow.id, "replay", input)
            .await
    }
}

fn continue_on_error(node: &WorkflowNode) -> bool {
    node.on_error
        .as_ref()
        .map(|c| c.continue_on_error)
        .unwrap_or(false)
}

fn timeout_error_message(timeout_seconds: u64) -> String {
    format!("Workflow timed out after {} seconds", timeout_seconds)
}

fn remaining_until(deadline: Instant) -> Option<Duration> {
    let now = Instant::now();
    if now >= deadline {
        None
    } else {
        Some(deadline.saturating_duration_since(now))
    }
}

fn retry_delay(config: &RetryConfig, attempt: u32) -> Duration {
    let base = config.delay_seconds;
    let secs = match config.backoff {
        BackoffType::Fixed => base,
        BackoffType::Linear => base.saturating_mul(attempt as u64),
        BackoffType::Exponential => {
            let shift = attempt.saturating_sub(1).min(20);
            base.saturating_mul(1u64 << shift)
        }
    };
    Duration::from_secs(secs)
}

fn evaluate_condition(condition: &str, node_input: &Value, ctx: &NodeContext) -> Result<bool> {
    let engine = rhai::Engine::new();
    let mut scope = rhai::Scope::new();

    let input_json = serde_json::to_string(node_input).unwrap_or_default();
    scope.push("input", input_json.clone());
    scope.push("data", input_json);

    for (node_id, output) in &ctx.node_outputs {
        let output_json = serde_json::to_string(output).unwrap_or_default();
        scope.push(node_id.replace('-', "_"), output_json);
    }

    engine
        .eval_with_scope::<bool>(&mut scope, condition)
        .map_err(|e| Error::Execution(format!("Condition evaluation failed: {}", e)))
}

async fn execute_node_with_retry(
    node: Arc<dyn Node>,
    node_type: &str,
    config: &Value,
    ctx: &NodeContext,
    retry: Option<&RetryConfig>,
    deadline: Instant,
) -> Result<NodeResult> {
    let max_attempts = retry.map(|r| r.max_attempts.max(1)).unwrap_or(1);
    let mut attempt = 1u32;

    loop {
        let remaining = remaining_until(deadline)
            .ok_or_else(|| Error::Execution("Workflow execution timed out".to_string()))?;

        match timeout(remaining, node.execute(config, ctx)).await {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => {
                if attempt >= max_attempts {
                    return Err(e);
                }

                if let Some(retry_cfg) = retry {
                    let delay = retry_delay(retry_cfg, attempt);
                    let remaining = remaining_until(deadline).ok_or_else(|| {
                        Error::Execution("Workflow execution timed out".to_string())
                    })?;

                    if delay > remaining {
                        return Err(Error::Execution(format!(
                            "Node '{}' failed and retry delay exceeds remaining workflow timeout",
                            node_type
                        )));
                    }

                    warn!(
                        "Node '{}' attempt {}/{} failed: {}. Retrying in {}s",
                        node_type,
                        attempt,
                        max_attempts,
                        e,
                        delay.as_secs()
                    );

                    sleep(delay).await;
                }

                attempt = attempt.saturating_add(1);
            }
            Err(_) => {
                return Err(Error::Execution(format!(
                    "Node '{}' timed out before completion",
                    node_type
                )));
            }
        }
    }
}

#[derive(Clone)]
struct ForEachTaskTemplate {
    base_ctx: NodeContext,
    node_impl: Arc<dyn Node>,
    node_type: String,
    node_config: Value,
    retry: Option<RetryConfig>,
    deadline: Instant,
}

fn spawn_for_each_item(
    join_set: &mut JoinSet<(usize, Result<Value>)>,
    index: usize,
    item: Value,
    template: &ForEachTaskTemplate,
) {
    let item_ctx = template.base_ctx.for_item(item, index);
    let node_type = template.node_type.clone();
    let config = template.node_config.clone();
    let node_impl = template.node_impl.clone();
    let retry = template.retry.clone();
    let deadline = template.deadline;

    join_set.spawn(async move {
        let result = execute_node_with_retry(
            node_impl,
            &node_type,
            &config,
            &item_ctx,
            retry.as_ref(),
            deadline,
        )
        .await
        .map(|r| r.data);

        (index, result)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredWorkflow;
    use crate::workflow::parse_workflow;

    #[tokio::test]
    async fn test_execute_simple_workflow() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: test-workflow
nodes:
  - id: transform
    type: transform
    config:
      expression: '"hello world"'
"#;

        let workflow = parse_workflow(yaml).unwrap();
        let now = Utc::now();
        storage
            .save_workflow(&StoredWorkflow {
                id: "wf-1".to_string(),
                name: workflow.name.clone(),
                definition: yaml.to_string(),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let execution = executor
            .execute(&workflow, "wf-1", "manual", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn test_replay_execution() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: replay-workflow
nodes:
  - id: transform
    type: transform
    config:
      expression: "\"ok\""
"#;

        let workflow = parse_workflow(yaml).unwrap();
        let now = Utc::now();
        storage
            .save_workflow(&StoredWorkflow {
                id: "wf-replay".to_string(),
                name: workflow.name.clone(),
                definition: yaml.to_string(),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let first = executor
            .execute(
                &workflow,
                "wf-replay",
                "manual",
                serde_json::json!({"run": 1}),
            )
            .await
            .unwrap();

        let replayed = executor
            .replay(&first.id, Some(serde_json::json!({"run": 2})))
            .await
            .unwrap();

        assert_eq!(replayed.trigger_type, "replay");
        assert_eq!(replayed.status, ExecutionStatus::Completed);
    }
}
