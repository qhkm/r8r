//! Workflow executor.

use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};
use tracing::{debug, error, info, instrument, warn, Span};

/// Registry that tracks per-execution pause signals.
///
/// This allows the pause API endpoint to signal a running execution to stop
/// between node executions. Without this, `pause_execution` would only update
/// the DB status while the executor loop continues running.
#[derive(Clone, Default)]
pub struct PauseRegistry {
    signals: Arc<tokio::sync::Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl PauseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an execution and return its pause signal.
    pub async fn register(&self, execution_id: &str) -> Arc<AtomicBool> {
        let signal = Arc::new(AtomicBool::new(false));
        self.signals
            .lock()
            .await
            .insert(execution_id.to_string(), signal.clone());
        signal
    }

    /// Request pause for a specific execution. Returns false if execution not found.
    pub async fn request_pause(&self, execution_id: &str) -> bool {
        if let Some(signal) = self.signals.lock().await.get(execution_id) {
            signal.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Unregister an execution (called when execution completes).
    pub async fn unregister(&self, execution_id: &str) {
        self.signals.lock().await.remove(execution_id);
    }
}

use crate::metrics;

use crate::api::Monitor;
use crate::credentials::CredentialStore;
use crate::engine::rate_limiter::RateLimiterRegistry;
use crate::error::{Error, Result};
use crate::nodes::{Node, NodeContext, NodeRegistry, NodeResult};
use crate::shutdown::ShutdownCoordinator;
use crate::storage::{Checkpoint, Execution, ExecutionStatus, NodeExecution, SqliteStorage};
use crate::workflow::{
    parse_workflow, BackoffType, Node as WorkflowNode, OnErrorAction, RetryConfig, Workflow,
};

/// Workflow executor.
pub struct Executor {
    registry: NodeRegistry,
    storage: SqliteStorage,
    monitor: Option<Arc<Monitor>>,
    inherited_credentials: Option<HashMap<String, String>>,
    timeout_override_seconds: Option<u64>,
    rate_limiter: Option<Arc<RateLimiterRegistry>>,
    shutdown: Option<Arc<ShutdownCoordinator>>,
    /// Default checkpoint setting (can be overridden per-workflow).
    enable_checkpoints: bool,
    /// Shared registry for per-execution pause signals.
    pause_registry: Option<PauseRegistry>,
}

impl Executor {
    /// Create a new executor.
    pub fn new(registry: NodeRegistry, storage: SqliteStorage) -> Self {
        Self {
            registry,
            storage,
            monitor: None,
            inherited_credentials: None,
            timeout_override_seconds: None,
            rate_limiter: None,
            shutdown: None,
            enable_checkpoints: true, // Default to enabled for safety
            pause_registry: None,
        }
    }

    /// Disable checkpointing for this execution.
    ///
    /// Use this for short-lived workflows where performance is critical
    /// and durability is not required.
    pub fn without_checkpoints(mut self) -> Self {
        self.enable_checkpoints = false;
        self
    }

    /// Attach a pause registry for per-execution pause signalling.
    pub fn with_pause_registry(mut self, registry: PauseRegistry) -> Self {
        self.pause_registry = Some(registry);
        self
    }

    /// Attach a live monitor for execution/node lifecycle events.
    pub fn with_monitor(mut self, monitor: Arc<Monitor>) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Attach a rate limiter for per-workflow rate limiting.
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<RateLimiterRegistry>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Merge parent credentials into the execution context.
    pub fn with_inherited_credentials(mut self, credentials: HashMap<String, String>) -> Self {
        self.inherited_credentials = Some(credentials);
        self
    }

    /// Override workflow timeout for this execution.
    pub fn with_timeout_override(mut self, timeout_seconds: u64) -> Self {
        self.timeout_override_seconds = Some(timeout_seconds);
        self
    }

    /// Attach a shutdown coordinator to enable graceful shutdown handling.
    pub fn with_shutdown(mut self, shutdown: Arc<ShutdownCoordinator>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Check if shutdown has been requested.
    ///
    /// Returns true if shutdown has been requested and the executor should
    /// stop processing and save its current state.
    fn is_shutdown_requested(&self) -> bool {
        self.shutdown
            .as_ref()
            .map(|s| s.is_shutdown_requested())
            .unwrap_or(false)
    }

    /// Execute a workflow.
    ///
    /// Optionally provide a correlation_id for distributed tracing.
    /// If not provided, the execution_id will be used as the correlation_id.
    #[instrument(
        name = "workflow.execute",
        skip(self, workflow, input),
        fields(
            workflow_id = %workflow_id,
            workflow_name = %workflow.name,
            trigger_type = %trigger_type,
            execution_id = tracing::field::Empty,
            correlation_id = tracing::field::Empty,
        )
    )]
    pub async fn execute(
        &self,
        workflow: &Workflow,
        workflow_id: &str,
        trigger_type: &str,
        input: Value,
    ) -> Result<Execution> {
        self.execute_with_correlation(workflow, workflow_id, trigger_type, input, None)
            .await
    }

    /// Execute a workflow with an explicit correlation ID.
    #[instrument(
        name = "workflow.execute",
        skip(self, workflow, input, correlation_id),
        fields(
            workflow_id = %workflow_id,
            workflow_name = %workflow.name,
            trigger_type = %trigger_type,
            execution_id = tracing::field::Empty,
            correlation_id = tracing::field::Empty,
        )
    )]
    pub async fn execute_with_correlation(
        &self,
        workflow: &Workflow,
        workflow_id: &str,
        trigger_type: &str,
        input: Value,
        correlation_id: Option<String>,
    ) -> Result<Execution> {
        // Determine checkpointing for this execution (per-workflow setting overrides global default)
        let checkpoints_enabled = workflow.settings.checkpoints_enabled() && self.enable_checkpoints;

        // Check rate limit if configured
        if let Some(ref rate_limiter) = self.rate_limiter {
            if !rate_limiter.try_acquire(&workflow.name) {
                warn!(
                    workflow_name = %workflow.name,
                    "Rate limit exceeded for workflow"
                );
                return Err(Error::Execution(format!(
                    "Rate limit exceeded for workflow '{}'",
                    workflow.name
                )));
            }
        }

        // Validate and process parameters
        let input = crate::workflow::validate_parameters(workflow, &input)?;

        // Validate input against JSON schema if defined
        if let Some(ref schema) = workflow.input_schema {
            crate::validation::validate_input(Some(schema), &input)?;
        }

        let workflow_version = self
            .storage
            .get_latest_workflow_version_number(workflow_id)
            .await?;
        let execution_id = uuid::Uuid::new_v4().to_string();
        let correlation_id = correlation_id.unwrap_or_else(|| execution_id.clone());
        let started_at = Utc::now();
        let timeout_seconds = self
            .timeout_override_seconds
            .unwrap_or(workflow.settings.timeout_seconds)
            .max(1);
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);

        // Record execution and correlation IDs in the current span
        Span::current().record("execution_id", &execution_id);
        Span::current().record("correlation_id", &correlation_id);

        info!(
            "Starting execution {} of workflow '{}' (correlation: {})",
            execution_id, workflow.name, correlation_id
        );

        // Record metrics
        metrics::inc_active_executions();
        let start_time = Instant::now();

        // Create execution record
        let mut execution = Execution {
            id: execution_id.clone(),
            workflow_id: workflow_id.to_string(),
            workflow_name: workflow.name.clone(),
            workflow_version,
            status: ExecutionStatus::Running,
            trigger_type: trigger_type.to_string(),
            input: input.clone(),
            output: None,
            started_at,
            finished_at: None,
            error: None,
        };

        self.storage.save_execution(&execution).await?;
        emit_execution_started(self.monitor.as_ref(), &execution);

        // Register pause signal so the pause API can stop this execution
        let pause_signal = if let Some(ref registry) = self.pause_registry {
            Some(registry.register(&execution_id).await)
        } else {
            None
        };

        // Load credentials for this execution
        let credentials = merge_inherited_credentials(
            load_workflow_credentials(workflow).await?,
            self.inherited_credentials.as_ref(),
        );

        // Load workflow variables
        let variables = serde_json::to_value(&workflow.variables).unwrap_or_default();

        // Create context with credentials, variables, and infrastructure for sub-workflows
        let mut ctx = NodeContext::new(&execution_id, &workflow.name)
            .with_input(input)
            .with_correlation_id(&correlation_id)
            .with_credentials(credentials)
            .with_variables(variables)
            .with_storage(self.storage.clone())
            .with_registry(Arc::new(self.registry.clone()));

        // Get topological order
        let order = workflow.topological_sort();
        debug!("Execution order: {:?}", order);

        // Execute nodes in order
        let mut last_output = Value::Null;
        let mut failed = false;
        let mut error_msg = None;

        for node_id in order {
            // Check for shutdown signal or per-execution pause signal
            let pause_requested = pause_signal
                .as_ref()
                .map(|s| s.load(Ordering::SeqCst))
                .unwrap_or(false);

            if self.is_shutdown_requested() || pause_requested {
                let reason = if pause_requested { "API pause request" } else { "server shutdown" };
                info!("Pausing execution {} due to {}", execution_id, reason);

                // Save final checkpoint before pausing
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;

                // Update execution status to paused
                execution.status = ExecutionStatus::Paused;
                execution.finished_at = Some(Utc::now());
                self.storage.save_execution(&execution).await?;
                emit_execution_finished(self.monitor.as_ref(), &execution);

                // Record metrics
                metrics::dec_active_executions();
                metrics::record_workflow_execution("paused", trigger_type);

                // Unregister from pause registry
                if let Some(ref registry) = self.pause_registry {
                    registry.unregister(&execution_id).await;
                }

                return Err(Error::Execution(format!(
                    "Execution {} paused due to {}",
                    execution_id, reason
                )));
            }

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
            emit_node_started(
                self.monitor.as_ref(),
                &execution_id,
                node_id,
                &node.node_type,
            );

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
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after skipped node
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        if let Some(recovery_output) = on_error_recovery_output(node) {
                            warn!(
                                "Node '{}' condition evaluation failed (recovering): {}",
                                node_id, e
                            );
                            ctx.add_output(node_id, recovery_output.clone());
                            last_output = recovery_output.clone();

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.output = Some(recovery_output);
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                            continue;
                        }

                        failed = true;
                        error_msg = Some(e.to_string());
                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.error = error_msg.clone();
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        break;
                    }
                }
            }

            info!("Executing node '{}' [{}]", node_id, node.node_type);

            // Check for pinned data (mock data for testing)
            if let Some(pinned) = &node.pinned_data {
                info!("Using pinned data for node '{}'", node_id);
                let output = pinned.clone();
                ctx.add_output(node_id, output.clone());
                last_output = output.clone();

                node_exec.status = ExecutionStatus::Completed;
                node_exec.output = Some(output);
                node_exec.finished_at = Some(Utc::now());
                self.storage.save_node_execution(&node_exec).await?;
                emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                // Save checkpoint after pinned data node
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                continue;
            }

            if node.for_each {
                let Some(items) = node_input.as_array() else {
                    let message = format!(
                        "Node '{}' has for_each=true but input is not an array",
                        node_id
                    );

                    if let Some(recovery_output) = on_error_recovery_output(node) {
                        warn!("{}", message);
                        ctx.add_output(node_id, recovery_output.clone());
                        last_output = recovery_output.clone();

                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.output = Some(recovery_output);
                        node_exec.error = Some(message);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after recovered node
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                        continue;
                    }

                    error!("{}", message);
                    failed = true;
                    error_msg = Some(message.clone());
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = Some(message);
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                };

                // Check item count limit to prevent memory exhaustion
                let max_items = workflow.settings.max_for_each_items;
                if items.len() > max_items {
                    let message = format!(
                        "Node '{}' for_each has {} items, exceeding limit of {} (adjust settings.max_for_each_items)",
                        node_id, items.len(), max_items
                    );

                    if let Some(recovery_output) = on_error_recovery_output(node) {
                        warn!("{}", message);
                        ctx.add_output(node_id, recovery_output.clone());
                        last_output = recovery_output.clone();

                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.output = Some(recovery_output);
                        node_exec.error = Some(message);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after recovered node
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                        continue;
                    }

                    error!("{}", message);
                    failed = true;
                    error_msg = Some(message.clone());
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = Some(message);
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                }

                let max_concurrency = workflow.settings.max_concurrency.max(1);
                let chunk_size = workflow.settings.chunk_size;
                let task_template = ForEachTaskTemplate {
                    base_ctx: ctx.clone(),
                    node_impl: node_impl.clone(),
                    node_type: node.node_type.clone(),
                    node_config: node.config.clone(),
                    retry: node.retry.clone(),
                    deadline,
                };

                // Use chunked processing for large datasets
                let (results, chunk_failed, chunk_error) = process_for_each_items(
                    items,
                    &task_template,
                    max_concurrency,
                    chunk_size,
                    timeout_seconds,
                    node_id,
                    on_error_recovery_output(node),
                )
                .await;

                if chunk_failed {
                    failed = true;
                    error_msg = chunk_error;
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = error_msg.clone();
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                }

                let output = Value::Array(results);
                ctx.add_output(node_id, output.clone());
                last_output = output.clone();

                node_exec.status = ExecutionStatus::Completed;
                node_exec.output = Some(output);
                node_exec.finished_at = Some(Utc::now());
                self.storage.save_node_execution(&node_exec).await?;
                emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                // Save checkpoint after for_each node
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
            } else {
                // Regular execution
                let node_ctx = NodeContext {
                    input: node_input,
                    node_outputs: ctx.node_outputs.clone(),
                    variables: ctx.variables.clone(),
                    execution_id: ctx.execution_id.clone(),
                    workflow_name: ctx.workflow_name.clone(),
                    correlation_id: ctx.correlation_id.clone(),
                    item_index: None,
                    credentials: ctx.credentials.clone(),
                    storage: ctx.storage.clone(),
                    registry: ctx.registry.clone(),
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
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after successful node execution
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                    }
                    Err(e) => {
                        if let Some(recovery_output) = on_error_recovery_output(node) {
                            warn!("Node '{}' failed (recovering): {}", node_id, e);
                            ctx.add_output(node_id, recovery_output.clone());
                            last_output = recovery_output.clone();

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.output = Some(recovery_output);
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        } else {
                            error!("Node '{}' failed: {}", node_id, e);
                            failed = true;
                            error_msg = Some(e.to_string());

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
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
            execution.error = error_msg.clone();
        } else {
            execution.status = ExecutionStatus::Completed;
            execution.output = Some(last_output);
        }

        // Unregister from pause registry
        if let Some(ref registry) = self.pause_registry {
            registry.unregister(&execution_id).await;
        }

        // Record metrics
        metrics::dec_active_executions();
        metrics::record_workflow_duration(start_time.elapsed(), &workflow.name);
        metrics::record_workflow_execution(execution.status.to_string().as_str(), trigger_type);

        self.storage.save_execution(&execution).await?;
        emit_execution_finished(self.monitor.as_ref(), &execution);

        let duration = execution
            .finished_at
            .map(|f| f - execution.started_at)
            .map(|d| d.num_milliseconds())
            .unwrap_or(0);

        info!(
            "Execution {} completed with status {} ({}ms)",
            execution_id, execution.status, duration
        );

        // Trigger error workflow if configured and execution failed
        if failed {
            if let Some(error_workflow_name) = &workflow.settings.error_workflow {
                trigger_error_workflow_task(
                    self.storage.clone(),
                    self.registry.clone(),
                    error_workflow_name,
                    &execution,
                    error_msg.as_deref(),
                )
                .await;
            }
        }

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

        let (workflow, resolved_version) = self.resolve_workflow_for_execution(&original).await?;
        let input = modified_input.unwrap_or(original.input);

        let mut replayed = self
            .execute(&workflow, &original.workflow_id, "replay", input)
            .await?;
        if replayed.workflow_version != resolved_version {
            replayed.workflow_version = resolved_version;
            self.storage.save_execution(&replayed).await?;
        }

        Ok(replayed)
    }

    /// Resume a failed execution from the failed node checkpoint.
    ///
    /// This function continues execution from where it failed, restoring
    /// the context (node outputs) from successfully completed nodes.
    pub async fn resume(&self, execution_id: &str) -> Result<Execution> {
        let original = self
            .storage
            .get_execution(execution_id)
            .await?
            .ok_or_else(|| Error::Execution(format!("Execution not found: {}", execution_id)))?;

        if original.status != ExecutionStatus::Failed {
            return Err(Error::Execution(format!(
                "Cannot resume execution '{}': status is '{}', expected 'failed'",
                execution_id, original.status
            )));
        }

        let (workflow, resolved_version) = self.resolve_workflow_for_execution(&original).await?;

        // Get all node executions from the failed run
        let node_executions = self.storage.get_node_executions(execution_id).await?;

        // Build checkpoint outputs from nodes that produced stable outputs
        // in the previous execution. This includes:
        // - completed nodes, and
        // - recovered failed nodes (on_error continue/skip/fallback) that have output.
        let mut completed_outputs: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        for node_exec in &node_executions {
            if matches!(
                node_exec.status,
                ExecutionStatus::Completed | ExecutionStatus::Failed
            ) {
                if let Some(output) = &node_exec.output {
                    completed_outputs.insert(node_exec.node_id.clone(), output.clone());
                }
            }
        }

        info!(
            "Resuming execution {} with {} completed nodes",
            execution_id,
            completed_outputs.len()
        );

        // Execute with checkpoint context
        self.execute_with_checkpoint(
            &workflow,
            &original.workflow_id,
            "resume",
            original.input,
            completed_outputs,
            resolved_version,
        )
        .await
    }

    /// Execute a workflow with pre-existing checkpoint context.
    ///
    /// Skips nodes that already have outputs in the checkpoint and continues
    /// from where the previous execution left off.
    async fn execute_with_checkpoint(
        &self,
        workflow: &Workflow,
        workflow_id: &str,
        trigger_type: &str,
        input: Value,
        checkpoint: std::collections::HashMap<String, Value>,
        workflow_version: Option<u32>,
    ) -> Result<Execution> {
        // Determine checkpointing for this execution (per-workflow setting overrides global default)
        let checkpoints_enabled = workflow.settings.checkpoints_enabled() && self.enable_checkpoints;

        let execution_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();
        let timeout_seconds = self
            .timeout_override_seconds
            .unwrap_or(workflow.settings.timeout_seconds)
            .max(1);
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);

        info!(
            "Starting resumed execution {} of workflow '{}' (skipping {} completed nodes)",
            execution_id,
            workflow.name,
            checkpoint.len()
        );

        // Create execution record
        let mut execution = Execution {
            id: execution_id.clone(),
            workflow_id: workflow_id.to_string(),
            workflow_name: workflow.name.clone(),
            workflow_version,
            status: ExecutionStatus::Running,
            trigger_type: trigger_type.to_string(),
            input: input.clone(),
            output: None,
            started_at,
            finished_at: None,
            error: None,
        };

        self.storage.save_execution(&execution).await?;
        emit_execution_started(self.monitor.as_ref(), &execution);

        // Register pause signal so the pause API can stop this execution
        let pause_signal = if let Some(ref registry) = self.pause_registry {
            Some(registry.register(&execution_id).await)
        } else {
            None
        };

        // Load credentials for this execution
        let credentials = merge_inherited_credentials(
            load_workflow_credentials(workflow).await?,
            self.inherited_credentials.as_ref(),
        );

        // Load workflow variables
        let variables = serde_json::to_value(&workflow.variables).unwrap_or_default();

        // Create context with checkpoint outputs, credentials, variables, and infrastructure pre-loaded
        let mut ctx = NodeContext::new(&execution_id, &workflow.name)
            .with_input(input)
            .with_credentials(credentials)
            .with_variables(variables)
            .with_storage(self.storage.clone())
            .with_registry(Arc::new(self.registry.clone()));
        for (node_id, output) in &checkpoint {
            ctx.add_output(node_id, output.clone());
        }

        // Get topological order
        let order = workflow.topological_sort();
        debug!("Execution order: {:?}", order);

        // Execute nodes in order, skipping checkpointed ones
        let mut last_output = checkpoint.values().last().cloned().unwrap_or(Value::Null);
        let mut failed = false;
        let mut error_msg = None;

        for node_id in order {
            // Skip nodes that were already completed in the checkpoint
            if checkpoint.contains_key(node_id) {
                info!("Skipping checkpointed node '{}'", node_id);
                // Update last_output to the checkpointed value
                if let Some(output) = checkpoint.get(node_id) {
                    last_output = output.clone();
                }
                continue;
            }

            // Check for shutdown signal or per-execution pause signal
            let pause_requested = pause_signal
                .as_ref()
                .map(|s| s.load(Ordering::SeqCst))
                .unwrap_or(false);

            if self.is_shutdown_requested() || pause_requested {
                let reason = if pause_requested { "API pause request" } else { "server shutdown" };
                info!("Pausing resumed execution {} due to {}", execution_id, reason);

                // Save final checkpoint before pausing
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;

                // Update execution status to paused
                execution.status = ExecutionStatus::Paused;
                execution.finished_at = Some(Utc::now());
                self.storage.save_execution(&execution).await?;
                emit_execution_finished(self.monitor.as_ref(), &execution);

                // Unregister from pause registry
                if let Some(ref registry) = self.pause_registry {
                    registry.unregister(&execution_id).await;
                }

                return Err(Error::Execution(format!(
                    "Execution {} paused due to {}",
                    execution_id, reason
                )));
            }

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
            emit_node_started(
                self.monitor.as_ref(),
                &execution_id,
                node_id,
                &node.node_type,
            );

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
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after skipped node
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        if let Some(recovery_output) = on_error_recovery_output(node) {
                            warn!(
                                "Node '{}' condition evaluation failed (recovering): {}",
                                node_id, e
                            );
                            ctx.add_output(node_id, recovery_output.clone());
                            last_output = recovery_output.clone();

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.output = Some(recovery_output);
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                            // Save checkpoint after recovered node
                            self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                            continue;
                        }

                        failed = true;
                        error_msg = Some(e.to_string());
                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.error = error_msg.clone();
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        break;
                    }
                }
            }

            info!("Executing node '{}' [{}]", node_id, node.node_type);

            // Check for pinned data (mock data for testing)
            if let Some(pinned) = &node.pinned_data {
                info!("Using pinned data for node '{}'", node_id);
                let output = pinned.clone();
                ctx.add_output(node_id, output.clone());
                last_output = output.clone();

                node_exec.status = ExecutionStatus::Completed;
                node_exec.output = Some(output);
                node_exec.finished_at = Some(Utc::now());
                self.storage.save_node_execution(&node_exec).await?;
                emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                // Save checkpoint after pinned data node
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                continue;
            }

            if node.for_each {
                let Some(items) = node_input.as_array() else {
                    let message = format!(
                        "Node '{}' has for_each=true but input is not an array",
                        node_id
                    );

                    if let Some(recovery_output) = on_error_recovery_output(node) {
                        warn!("{}", message);
                        ctx.add_output(node_id, recovery_output.clone());
                        last_output = recovery_output.clone();

                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.output = Some(recovery_output);
                        node_exec.error = Some(message);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        continue;
                    }

                    error!("{}", message);
                    failed = true;
                    error_msg = Some(message.clone());
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = Some(message);
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                };

                // Check item count limit to prevent memory exhaustion
                let max_items = workflow.settings.max_for_each_items;
                if items.len() > max_items {
                    let message = format!(
                        "Node '{}' for_each has {} items, exceeding limit of {} (adjust settings.max_for_each_items)",
                        node_id, items.len(), max_items
                    );

                    if let Some(recovery_output) = on_error_recovery_output(node) {
                        warn!("{}", message);
                        ctx.add_output(node_id, recovery_output.clone());
                        last_output = recovery_output.clone();

                        node_exec.status = ExecutionStatus::Failed;
                        node_exec.output = Some(recovery_output);
                        node_exec.error = Some(message);
                        node_exec.finished_at = Some(Utc::now());
                        self.storage.save_node_execution(&node_exec).await?;
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        continue;
                    }

                    error!("{}", message);
                    failed = true;
                    error_msg = Some(message.clone());
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = Some(message);
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                }

                let max_concurrency = workflow.settings.max_concurrency.max(1);
                let chunk_size = workflow.settings.chunk_size;
                let task_template = ForEachTaskTemplate {
                    base_ctx: ctx.clone(),
                    node_impl: node_impl.clone(),
                    node_type: node.node_type.clone(),
                    node_config: node.config.clone(),
                    retry: node.retry.clone(),
                    deadline,
                };

                // Use chunked processing for large datasets
                let (results, chunk_failed, chunk_error) = process_for_each_items(
                    items,
                    &task_template,
                    max_concurrency,
                    chunk_size,
                    timeout_seconds,
                    node_id,
                    on_error_recovery_output(node),
                )
                .await;

                if chunk_failed {
                    failed = true;
                    error_msg = chunk_error;
                    node_exec.status = ExecutionStatus::Failed;
                    node_exec.error = error_msg.clone();
                    node_exec.finished_at = Some(Utc::now());
                    self.storage.save_node_execution(&node_exec).await?;
                    emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                    break;
                }

                let output = Value::Array(results);
                ctx.add_output(node_id, output.clone());
                last_output = output.clone();

                node_exec.status = ExecutionStatus::Completed;
                node_exec.output = Some(output);
                node_exec.finished_at = Some(Utc::now());
                self.storage.save_node_execution(&node_exec).await?;
                emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                // Save checkpoint after for_each node
                self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
            } else {
                // Regular execution
                let node_ctx = NodeContext {
                    input: node_input,
                    node_outputs: ctx.node_outputs.clone(),
                    variables: ctx.variables.clone(),
                    execution_id: ctx.execution_id.clone(),
                    workflow_name: ctx.workflow_name.clone(),
                    correlation_id: ctx.correlation_id.clone(),
                    item_index: None,
                    credentials: ctx.credentials.clone(),
                    storage: ctx.storage.clone(),
                    registry: ctx.registry.clone(),
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
                        emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        // Save checkpoint after successful node execution
                        self.save_checkpoint(&execution_id, node_id, &ctx.node_outputs, checkpoints_enabled).await?;
                    }
                    Err(e) => {
                        if let Some(recovery_output) = on_error_recovery_output(node) {
                            warn!("Node '{}' failed (recovering): {}", node_id, e);
                            ctx.add_output(node_id, recovery_output.clone());
                            last_output = recovery_output.clone();

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.output = Some(recovery_output);
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                        } else {
                            error!("Node '{}' failed: {}", node_id, e);
                            failed = true;
                            error_msg = Some(e.to_string());

                            node_exec.status = ExecutionStatus::Failed;
                            node_exec.error = Some(e.to_string());
                            node_exec.finished_at = Some(Utc::now());
                            self.storage.save_node_execution(&node_exec).await?;
                            emit_node_terminal_event(self.monitor.as_ref(), &node_exec);
                            break;
                        }
                    }
                }
            }
        }

        // Unregister from pause registry
        if let Some(ref registry) = self.pause_registry {
            registry.unregister(&execution_id).await;
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
            "Resumed execution {} completed with status {} ({}ms)",
            execution_id, execution.status, duration
        );

        Ok(execution)
    }

    async fn resolve_workflow_for_execution(
        &self,
        execution: &Execution,
    ) -> Result<(Workflow, Option<u32>)> {
        if let Some(version) = execution.workflow_version {
            let snapshot = self
                .storage
                .get_workflow_version_by_id(&execution.workflow_id, version)
                .await?
                .ok_or_else(|| {
                    Error::Execution(format!(
                        "Workflow snapshot not found for execution {}: {} v{}",
                        execution.id, execution.workflow_id, version
                    ))
                })?;
            let workflow = parse_workflow(&snapshot.definition)?;
            return Ok((workflow, Some(snapshot.version)));
        }

        if let Some(snapshot) = self
            .storage
            .get_workflow_version_at_or_before(&execution.workflow_id, execution.started_at)
            .await?
        {
            let workflow = parse_workflow(&snapshot.definition)?;
            return Ok((workflow, Some(snapshot.version)));
        }

        let stored_workflow = self
            .storage
            .get_workflow_by_id(&execution.workflow_id)
            .await?
            .ok_or_else(|| {
                Error::Execution(format!("Workflow not found for execution {}", execution.id))
            })?;
        let workflow = parse_workflow(&stored_workflow.definition)?;
        let version = self
            .storage
            .get_latest_workflow_version_number(&execution.workflow_id)
            .await?;
        Ok((workflow, version))
    }

    /// Save a checkpoint for the current execution state.
    ///
    /// If checkpointing is disabled, this is a no-op.
    async fn save_checkpoint(
        &self,
        execution_id: &str,
        node_id: &str,
        node_outputs: &std::collections::HashMap<String, Value>,
        checkpoints_enabled: bool,
    ) -> Result<()> {
        if !checkpoints_enabled {
            debug!("Checkpointing disabled for execution {}, skipping checkpoint", execution_id);
            return Ok(());
        }

        let checkpoint = Checkpoint {
            id: uuid::Uuid::new_v4().to_string(),
            execution_id: execution_id.to_string(),
            node_id: node_id.to_string(),
            node_outputs: serde_json::to_value(node_outputs).unwrap_or_default(),
            created_at: Utc::now(),
        };

        self.storage.save_checkpoint(&checkpoint).await?;
        debug!("Checkpoint saved for execution {} at node {}", execution_id, node_id);
        Ok(())
    }

    /// Pause an execution by signalling the running executor loop to stop.
    ///
    /// Signals the execution via the pause registry. The executor loop checks
    /// this signal between nodes and will save a checkpoint and exit gracefully.
    /// If the execution is not tracked in the registry (e.g., already finished
    /// between the status check and this call), falls back to a direct DB update.
    pub async fn pause_execution(&self, execution_id: &str) -> Result<Execution> {
        let execution = self
            .storage
            .get_execution(execution_id)
            .await?
            .ok_or_else(|| Error::Execution(format!("Execution not found: {}", execution_id)))?;

        if execution.status != ExecutionStatus::Running {
            return Err(Error::Execution(format!(
                "Cannot pause execution '{}': status is '{}', expected 'running'",
                execution_id, execution.status
            )));
        }

        info!("Requesting pause for execution {}", execution_id);

        // Signal the running executor loop to pause at the next node boundary
        let signalled = if let Some(ref registry) = self.pause_registry {
            registry.request_pause(execution_id).await
        } else {
            false
        };

        if signalled {
            // The executor loop will handle checkpoint saving, status update,
            // and cleanup. Wait briefly for it to take effect, then return
            // the updated execution status.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let updated = self
                .storage
                .get_execution(execution_id)
                .await?
                .unwrap_or(execution);
            info!("Execution {} pause signalled (status: {})", execution_id, updated.status);
            Ok(updated)
        } else {
            // Execution not in registry (may have just finished). Fall back to
            // direct DB update for executions that are still marked Running.
            warn!(
                "Execution {} not found in pause registry, falling back to direct DB update",
                execution_id
            );
            let mut execution = execution;
            execution.status = ExecutionStatus::Paused;
            execution.finished_at = Some(Utc::now());
            self.storage.save_execution(&execution).await?;
            info!("Execution {} paused (direct)", execution_id);
            Ok(execution)
        }
    }

    /// Resume execution from the latest checkpoint.
    ///
    /// Loads the latest checkpoint for the execution and continues
    /// execution from where it left off.
    pub async fn resume_from_checkpoint(&self, execution_id: &str) -> Result<Execution> {
        let original = self
            .storage
            .get_execution(execution_id)
            .await?
            .ok_or_else(|| Error::Execution(format!("Execution not found: {}", execution_id)))?;

        if original.status != ExecutionStatus::Paused {
            return Err(Error::Execution(format!(
                "Cannot resume execution '{}': status is '{}', expected 'paused'",
                execution_id, original.status
            )));
        }

        // Get the latest checkpoint
        let checkpoint = self
            .storage
            .get_latest_checkpoint(execution_id)
            .await?
            .ok_or_else(|| {
                Error::Execution(format!(
                    "No checkpoint found for execution '{}', cannot resume",
                    execution_id
                ))
            })?;

        info!(
            "Resuming execution {} from checkpoint at node {}",
            execution_id, checkpoint.node_id
        );

        // Deserialize checkpoint outputs
        let checkpoint_outputs: std::collections::HashMap<String, Value> =
            serde_json::from_value(checkpoint.node_outputs.clone()).unwrap_or_default();

        // Resolve workflow
        let (workflow, resolved_version) = self.resolve_workflow_for_execution(&original).await?;

        // Resume execution using the checkpoint
        self.execute_with_checkpoint(
            &workflow,
            &original.workflow_id,
            "resume",
            original.input,
            checkpoint_outputs,
            resolved_version,
        )
        .await
    }
}

/// Trigger an error workflow when the main workflow fails.
///
/// This is a standalone function to avoid recursive async issues.
/// It runs in a spawned task and logs any errors.
async fn trigger_error_workflow_task(
    storage: SqliteStorage,
    registry: NodeRegistry,
    error_workflow_name: &str,
    failed_execution: &Execution,
    error_message: Option<&str>,
) {
    info!(
        "Triggering error workflow '{}' for failed execution '{}'",
        error_workflow_name, failed_execution.id
    );

    // Load the error workflow
    let stored = match storage.get_workflow(error_workflow_name).await {
        Ok(Some(w)) if w.enabled => w,
        Ok(Some(_)) => {
            warn!(
                "Error workflow '{}' is disabled, skipping",
                error_workflow_name
            );
            return;
        }
        Ok(None) => {
            warn!(
                "Error workflow '{}' not found, skipping",
                error_workflow_name
            );
            return;
        }
        Err(e) => {
            error!(
                "Failed to load error workflow '{}': {}",
                error_workflow_name, e
            );
            return;
        }
    };

    let workflow = match parse_workflow(&stored.definition) {
        Ok(w) => w,
        Err(e) => {
            error!(
                "Failed to parse error workflow '{}': {}",
                error_workflow_name, e
            );
            return;
        }
    };

    // Build error context input
    let error_input = serde_json::json!({
        "error": {
            "message": error_message,
            "execution_id": failed_execution.id,
            "workflow_id": failed_execution.workflow_id,
            "workflow_name": failed_execution.workflow_name,
            "trigger_type": failed_execution.trigger_type,
            "started_at": failed_execution.started_at.to_rfc3339(),
            "finished_at": failed_execution.finished_at.map(|t| t.to_rfc3339()),
        },
        "original_input": failed_execution.input,
    });

    // Create executor and execute the error workflow
    // Use Box::pin to break the recursive async future cycle
    let executor = Executor::new(registry, storage);
    let future = Box::pin(executor.execute(&workflow, &stored.id, "error_handler", error_input));
    match future.await {
        Ok(execution) => {
            info!(
                "Error workflow '{}' completed with status: {}",
                error_workflow_name, execution.status
            );
        }
        Err(e) => {
            error!("Error workflow '{}' failed: {}", error_workflow_name, e);
        }
    }
}

fn on_error_recovery_output(node: &WorkflowNode) -> Option<Value> {
    let config = node.on_error.as_ref()?;

    if let Some(action) = &config.action {
        return match action {
            OnErrorAction::Fail => None,
            OnErrorAction::Continue | OnErrorAction::Skip => Some(Value::Null),
            OnErrorAction::Fallback => Some(config.fallback_value.clone().unwrap_or(Value::Null)),
        };
    }

    if config.continue_on_error {
        Some(Value::Null)
    } else {
        None
    }
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

    for (node_id, output) in ctx.node_outputs.iter() {
        let output_json = serde_json::to_string(output).unwrap_or_default();
        scope.push(node_id.replace('-', "_"), output_json);
    }

    engine
        .eval_with_scope::<bool>(&mut scope, condition)
        .map_err(|e| Error::Execution(format!("Condition evaluation failed: {}", e)))
}

#[instrument(
    name = "node.execute",
    skip(node, config, ctx, retry, deadline),
    fields(
        node_type = %node_type,
        execution_id = %ctx.execution_id,
        correlation_id = ?ctx.correlation_id,
        item_index = ?ctx.item_index,
    )
)]
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
    let node_start = Instant::now();

    loop {
        let remaining = remaining_until(deadline)
            .ok_or_else(|| Error::Execution("Workflow execution timed out".to_string()))?;

        match timeout(remaining, node.execute(config, ctx)).await {
            Ok(Ok(result)) => {
                metrics::record_node_execution(node_type, "success");
                metrics::record_node_duration(node_start.elapsed(), node_type);
                return Ok(result);
            }
            Ok(Err(e)) => {
                if attempt >= max_attempts {
                    metrics::record_node_execution(node_type, "failed");
                    metrics::record_node_duration(node_start.elapsed(), node_type);
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
                metrics::record_node_execution(node_type, "timeout");
                metrics::record_node_duration(node_start.elapsed(), node_type);
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

/// Process for_each items with optional chunking for large datasets.
///
/// When `chunk_size` is 0, processes all items at once (default behavior).
/// When `chunk_size` > 0, processes items in batches, reducing peak memory usage.
///
/// Returns (results, failed, error_msg).
async fn process_for_each_items(
    items: &[Value],
    template: &ForEachTaskTemplate,
    max_concurrency: usize,
    chunk_size: usize,
    timeout_seconds: u64,
    node_id: &str,
    on_error_recovery: Option<Value>,
) -> (Vec<Value>, bool, Option<String>) {
    let total = items.len();
    if total == 0 {
        return (Vec::new(), false, None);
    }

    let mut results = vec![Value::Null; total];
    let mut failed = false;
    let mut error_msg: Option<String> = None;

    // Determine effective chunk size (0 = no chunking)
    let effective_chunk_size = if chunk_size > 0 { chunk_size } else { total };

    // Process in chunks
    for chunk_start in (0..total).step_by(effective_chunk_size) {
        if failed {
            break;
        }

        let chunk_end = (chunk_start + effective_chunk_size).min(total);
        let chunk_items = &items[chunk_start..chunk_end];

        if chunk_size > 0 && total > chunk_size {
            debug!(
                "Processing chunk {}-{} of {} items for node '{}'",
                chunk_start, chunk_end, total, node_id
            );
        }

        let mut join_set: JoinSet<(usize, Result<Value>)> = JoinSet::new();
        let mut next_local_index = 0usize;

        // Spawn initial batch up to max_concurrency
        while next_local_index < chunk_items.len() && join_set.len() < max_concurrency {
            if remaining_until(template.deadline).is_none() {
                failed = true;
                error_msg = Some(timeout_error_message(timeout_seconds));
                break;
            }

            let global_index = chunk_start + next_local_index;
            spawn_for_each_item(
                &mut join_set,
                global_index,
                chunk_items[next_local_index].clone(),
                template,
            );
            next_local_index += 1;
        }

        // Process results and spawn more as slots free up
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
                    if let Some(ref recovery_output) = on_error_recovery {
                        warn!("Node '{}' item {} failed (recovering): {}", node_id, idx, e);
                        results[idx] = recovery_output.clone();
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
                    error_msg = Some(format!("Node '{}' worker task join failed: {}", node_id, e));
                    join_set.abort_all();
                    while join_set.join_next().await.is_some() {}
                    break;
                }
            }

            // Spawn more items from this chunk
            while !failed
                && next_local_index < chunk_items.len()
                && join_set.len() < max_concurrency
            {
                if remaining_until(template.deadline).is_none() {
                    failed = true;
                    error_msg = Some(timeout_error_message(timeout_seconds));
                    join_set.abort_all();
                    while join_set.join_next().await.is_some() {}
                    break;
                }

                let global_index = chunk_start + next_local_index;
                spawn_for_each_item(
                    &mut join_set,
                    global_index,
                    chunk_items[next_local_index].clone(),
                    template,
                );
                next_local_index += 1;
            }
        }
    }

    (results, failed, error_msg)
}

fn merge_inherited_credentials(
    mut credentials: HashMap<String, String>,
    inherited: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    if let Some(inherited) = inherited {
        for (key, value) in inherited {
            credentials
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    credentials
}

fn emit_execution_started(monitor: Option<&Arc<Monitor>>, execution: &Execution) {
    if let Some(monitor) = monitor {
        monitor.execution_started(
            &execution.id,
            &execution.workflow_name,
            &execution.trigger_type,
        );
    }
}

fn emit_execution_finished(monitor: Option<&Arc<Monitor>>, execution: &Execution) {
    let Some(monitor) = monitor else {
        return;
    };

    let duration_ms = execution
        .finished_at
        .map(|f| (f - execution.started_at).num_milliseconds())
        .unwrap_or(0);

    match execution.status {
        ExecutionStatus::Completed => monitor.execution_completed(
            &execution.id,
            &execution.workflow_name,
            &execution.status.to_string(),
            duration_ms,
        ),
        ExecutionStatus::Failed => monitor.execution_failed(
            &execution.id,
            &execution.workflow_name,
            execution.error.as_deref().unwrap_or("Execution failed"),
        ),
        _ => {}
    }
}

fn emit_node_started(
    monitor: Option<&Arc<Monitor>>,
    execution_id: &str,
    node_id: &str,
    node_type: &str,
) {
    if let Some(monitor) = monitor {
        monitor.node_started(execution_id, node_id, node_type);
    }
}

fn emit_node_terminal_event(monitor: Option<&Arc<Monitor>>, node_exec: &NodeExecution) {
    let Some(monitor) = monitor else {
        return;
    };

    let duration_ms = node_exec
        .finished_at
        .map(|f| (f - node_exec.started_at).num_milliseconds())
        .unwrap_or(0);

    match node_exec.status {
        ExecutionStatus::Completed => monitor.node_completed(
            &node_exec.execution_id,
            &node_exec.node_id,
            "completed",
            duration_ms,
        ),
        ExecutionStatus::Failed => monitor.node_failed(
            &node_exec.execution_id,
            &node_exec.node_id,
            node_exec.error.as_deref().unwrap_or("Node failed"),
        ),
        _ => {}
    }
}

/// Load credentials required by the workflow nodes.
///
/// Scans all nodes for `credential` config fields and loads
/// the corresponding credentials from the credential store.
async fn load_workflow_credentials(workflow: &Workflow) -> Result<HashMap<String, String>> {
    let mut credentials = HashMap::new();

    // Try to load the credential store
    let store = match CredentialStore::load().await {
        Ok(s) => s,
        Err(e) => {
            warn!(
                "Failed to load credential store: {}. Continuing without credentials.",
                e
            );
            return Ok(credentials);
        }
    };

    // Scan nodes for credential requirements
    for node in &workflow.nodes {
        if let Some(credential_name) = node.config.get("credential").and_then(|v| v.as_str()) {
            // Skip if already loaded
            if credentials.contains_key(credential_name) {
                continue;
            }

            // Load credential value
            match store.get(credential_name) {
                Ok(Some(value)) => {
                    debug!(
                        "Loaded credential '{}' for workflow '{}'",
                        credential_name, workflow.name
                    );
                    credentials.insert(credential_name.to_string(), value);
                }
                Ok(None) => {
                    // Credential not found - node will fail when it tries to use it
                    debug!(
                        "Credential '{}' not found in store (workflow '{}')",
                        credential_name, workflow.name
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to load credential '{}' for workflow '{}': {}",
                        credential_name, workflow.name, e
                    );
                }
            }
        }
    }

    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::MonitorEvent;
    use crate::storage::StoredWorkflow;
    use crate::workflow::parse_workflow;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ============================================================================
    // Test Node: Configurable Failing Node
    // ============================================================================

    /// A test node that fails a configurable number of times before succeeding.
    struct FailingNode {
        /// How many times to fail before succeeding (per execution)
        fail_count: Arc<AtomicUsize>,
    }

    impl FailingNode {
        fn new() -> Self {
            Self {
                fail_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn reset(&self) {
            self.fail_count.store(0, Ordering::SeqCst);
        }
    }

    #[async_trait::async_trait]
    impl Node for FailingNode {
        fn node_type(&self) -> &str {
            "failing"
        }

        async fn execute(&self, config: &Value, _ctx: &NodeContext) -> Result<NodeResult> {
            let fail_times = config
                .get("fail_times")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let current = self.fail_count.fetch_add(1, Ordering::SeqCst);

            if current < fail_times {
                Err(Error::Execution(format!(
                    "Simulated failure {}/{}",
                    current + 1,
                    fail_times
                )))
            } else {
                Ok(NodeResult::new(serde_json::json!({
                    "success": true,
                    "attempts": current + 1
                })))
            }
        }

        fn description(&self) -> &str {
            "A test node that fails N times before succeeding"
        }
    }

    /// A test node that always fails.
    struct AlwaysFailNode;

    #[async_trait::async_trait]
    impl Node for AlwaysFailNode {
        fn node_type(&self) -> &str {
            "always_fail"
        }

        async fn execute(&self, _config: &Value, _ctx: &NodeContext) -> Result<NodeResult> {
            Err(Error::Execution("Always fails".to_string()))
        }

        fn description(&self) -> &str {
            "A test node that always fails"
        }
    }

    /// A test node that sleeps for a configurable duration.
    struct SlowNode;

    #[async_trait::async_trait]
    impl Node for SlowNode {
        fn node_type(&self) -> &str {
            "slow"
        }

        async fn execute(&self, config: &Value, _ctx: &NodeContext) -> Result<NodeResult> {
            let sleep_ms = config
                .get("sleep_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(100);

            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            Ok(NodeResult::new(serde_json::json!({"slept_ms": sleep_ms})))
        }

        fn description(&self) -> &str {
            "A test node that sleeps"
        }
    }

    // ============================================================================
    // Helper Functions
    // ============================================================================

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

    // ============================================================================
    // Basic Execution Tests
    // ============================================================================

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
    async fn test_monitor_emits_execution_lifecycle_events() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let monitor = Arc::new(Monitor::new());
        let mut rx = monitor.subscribe();
        let executor = Executor::new(registry, storage.clone()).with_monitor(monitor);

        let yaml = r#"
name: monitor-success
nodes:
  - id: transform
    type: transform
    config:
      expression: '"ok"'
"#;

        let workflow = parse_workflow(yaml).unwrap();
        save_test_workflow(&storage, "wf-monitor-success", yaml).await;

        let execution = executor
            .execute(&workflow, "wf-monitor-success", "manual", Value::Null)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Completed);

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(matches!(
            events.as_slice(),
            [
                MonitorEvent::ExecutionStarted { .. },
                MonitorEvent::NodeStarted { .. },
                MonitorEvent::NodeCompleted { .. },
                MonitorEvent::ExecutionCompleted { .. }
            ]
        ));
    }

    #[tokio::test]
    async fn test_monitor_emits_failure_events() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let monitor = Arc::new(Monitor::new());
        let mut rx = monitor.subscribe();
        let executor = Executor::new(registry, storage.clone()).with_monitor(monitor);

        let yaml = r#"
name: monitor-failure
nodes:
  - id: failing
    type: always_fail
"#;

        let workflow = parse_workflow(yaml).unwrap();
        save_test_workflow(&storage, "wf-monitor-failure", yaml).await;

        let execution = executor
            .execute(&workflow, "wf-monitor-failure", "manual", Value::Null)
            .await
            .unwrap();
        assert_eq!(execution.status, ExecutionStatus::Failed);

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert!(events
            .iter()
            .any(|e| matches!(e, MonitorEvent::NodeFailed { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, MonitorEvent::ExecutionFailed { .. })));
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

    #[tokio::test]
    async fn test_replay_uses_original_workflow_version() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let v1 = r#"
name: replay-versioned-workflow
nodes:
  - id: step
    type: transform
    config:
      expression: '"v1"'
"#;
        save_test_workflow(&storage, "wf-replay-versioned", v1).await;
        let workflow_v1 = parse_workflow(v1).unwrap();

        let first = executor
            .execute(&workflow_v1, "wf-replay-versioned", "manual", Value::Null)
            .await
            .unwrap();
        assert_eq!(first.status, ExecutionStatus::Completed);
        assert_eq!(first.output, Some(serde_json::json!("v1")));
        assert_eq!(first.workflow_version, Some(1));

        let v2 = r#"
name: replay-versioned-workflow
nodes:
  - id: step
    type: transform
    config:
      expression: '"v2"'
"#;
        save_test_workflow(&storage, "wf-replay-versioned", v2).await;

        let replayed = executor.replay(&first.id, None).await.unwrap();
        assert_eq!(replayed.status, ExecutionStatus::Completed);
        assert_eq!(replayed.output, Some(serde_json::json!("v1")));
        assert_eq!(replayed.workflow_version, Some(1));
    }

    // ============================================================================
    // Retry Behavior Tests
    // ============================================================================

    #[tokio::test]
    async fn test_retry_with_fixed_backoff_succeeds() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let failing_node = Arc::new(FailingNode::new());
        let mut registry = NodeRegistry::new();
        registry.register(failing_node.clone());
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: retry-fixed-workflow
settings:
  timeout_seconds: 30
nodes:
  - id: retry_node
    type: failing
    config:
      fail_times: 2
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: fixed
"#;

        save_test_workflow(&storage, "wf-retry-fixed", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();
        failing_node.reset();

        let execution = executor
            .execute(&workflow, "wf-retry-fixed", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        let output = execution.output.unwrap();
        assert_eq!(output.get("success"), Some(&serde_json::json!(true)));
        assert_eq!(output.get("attempts"), Some(&serde_json::json!(3)));
    }

    #[tokio::test]
    async fn test_retry_with_linear_backoff_succeeds() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let failing_node = Arc::new(FailingNode::new());
        let mut registry = NodeRegistry::new();
        registry.register(failing_node.clone());
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: retry-linear-workflow
settings:
  timeout_seconds: 30
nodes:
  - id: retry_node
    type: failing
    config:
      fail_times: 1
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: linear
"#;

        save_test_workflow(&storage, "wf-retry-linear", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();
        failing_node.reset();

        let execution = executor
            .execute(&workflow, "wf-retry-linear", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        let output = execution.output.unwrap();
        assert_eq!(output.get("attempts"), Some(&serde_json::json!(2)));
    }

    #[tokio::test]
    async fn test_retry_with_exponential_backoff_succeeds() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let failing_node = Arc::new(FailingNode::new());
        let mut registry = NodeRegistry::new();
        registry.register(failing_node.clone());
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: retry-exponential-workflow
settings:
  timeout_seconds: 60
nodes:
  - id: retry_node
    type: failing
    config:
      fail_times: 1
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: exponential
"#;

        save_test_workflow(&storage, "wf-retry-exp", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();
        failing_node.reset();

        let execution = executor
            .execute(&workflow, "wf-retry-exp", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn test_retry_exhausted_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: retry-exhausted-workflow
settings:
  timeout_seconds: 30
nodes:
  - id: always_fail_node
    type: always_fail
    config: {}
    retry:
      max_attempts: 3
      delay_seconds: 1
      backoff: fixed
"#;

        save_test_workflow(&storage, "wf-retry-exhausted", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-retry-exhausted", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.is_some());
        assert!(execution.error.unwrap().contains("Always fails"));
    }

    #[tokio::test]
    async fn test_no_retry_fails_immediately() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: no-retry-workflow
nodes:
  - id: fail_node
    type: always_fail
    config: {}
"#;

        save_test_workflow(&storage, "wf-no-retry", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-no-retry", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
    }

    // ============================================================================
    // Timeout Tests
    // ============================================================================

    #[tokio::test]
    async fn test_workflow_timeout() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(SlowNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: timeout-workflow
settings:
  timeout_seconds: 1
nodes:
  - id: slow_node
    type: slow
    config:
      sleep_ms: 5000
"#;

        save_test_workflow(&storage, "wf-timeout", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-timeout", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.is_some());
        let error = execution.error.unwrap();
        assert!(
            error.contains("timed out") || error.contains("timeout"),
            "Expected timeout error, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_retry_delay_exceeds_timeout() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: retry-timeout-workflow
settings:
  timeout_seconds: 2
nodes:
  - id: fail_node
    type: always_fail
    config: {}
    retry:
      max_attempts: 5
      delay_seconds: 10
      backoff: fixed
"#;

        save_test_workflow(&storage, "wf-retry-timeout", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-retry-timeout", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        let error = execution.error.unwrap();
        assert!(
            error.contains("timeout") || error.contains("Always fails"),
            "Expected timeout or failure error, got: {}",
            error
        );
    }

    // ============================================================================
    // On-Error / Fallback Tests
    // ============================================================================

    #[tokio::test]
    async fn test_on_error_fallback_value() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: fallback-workflow
nodes:
  - id: risky
    type: transform
    config:
      expression: "1 +"
    on_error:
      action: fallback
      fallback_value:
        source: "cached"
        items: []
"#;

        let workflow = parse_workflow(yaml).unwrap();
        let now = Utc::now();
        storage
            .save_workflow(&StoredWorkflow {
                id: "wf-fallback".to_string(),
                name: workflow.name.clone(),
                definition: yaml.to_string(),
                enabled: true,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let execution = executor
            .execute(&workflow, "wf-fallback", "manual", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(
            execution.output,
            Some(serde_json::json!({"source": "cached", "items": []}))
        );

        let node_execs = storage.get_node_executions(&execution.id).await.unwrap();
        assert_eq!(node_execs.len(), 1);
        assert_eq!(node_execs[0].status, ExecutionStatus::Failed);
        assert_eq!(
            node_execs[0].output,
            Some(serde_json::json!({"source": "cached", "items": []}))
        );
    }

    #[tokio::test]
    async fn test_on_error_continue() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: continue-workflow
nodes:
  - id: fail_node
    type: always_fail
    config: {}
    on_error:
      action: continue
  - id: next_node
    type: transform
    depends_on: [fail_node]
    config:
      expression: '"continued"'
"#;

        save_test_workflow(&storage, "wf-continue", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-continue", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!("continued")));
    }

    #[tokio::test]
    async fn test_on_error_skip() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: skip-workflow
nodes:
  - id: fail_node
    type: always_fail
    config: {}
    on_error:
      action: skip
  - id: next_node
    type: transform
    depends_on: [fail_node]
    config:
      expression: '"skipped failure"'
"#;

        save_test_workflow(&storage, "wf-skip", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-skip", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
    }

    // ============================================================================
    // Condition Evaluation Tests
    // ============================================================================

    #[tokio::test]
    async fn test_condition_skips_node_when_false() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: condition-workflow
nodes:
  - id: conditional_node
    type: transform
    condition: "false"
    config:
      expression: '"should not run"'
  - id: final_node
    type: transform
    depends_on: [conditional_node]
    config:
      expression: '"final"'
"#;

        save_test_workflow(&storage, "wf-condition", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-condition", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!("final")));
    }

    #[tokio::test]
    async fn test_condition_runs_node_when_true() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: condition-true-workflow
nodes:
  - id: conditional_node
    type: transform
    condition: "true"
    config:
      expression: '"ran"'
"#;

        save_test_workflow(&storage, "wf-condition-true", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-condition-true", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!("ran")));
    }

    #[tokio::test]
    async fn test_condition_evaluation_error_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: condition-error-workflow
nodes:
  - id: bad_condition
    type: transform
    condition: "invalid_syntax("
    config:
      expression: '"should fail"'
"#;

        save_test_workflow(&storage, "wf-bad-condition", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-bad-condition", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.is_some());
    }

    // ============================================================================
    // For-Each Tests
    // ============================================================================

    #[tokio::test]
    async fn test_for_each_non_array_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: foreach-error-workflow
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "input"
"#;

        save_test_workflow(&storage, "wf-foreach-error", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(
                &workflow,
                "wf-foreach-error",
                "test",
                serde_json::json!({"not": "an array"}),
            )
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.is_some());
        assert!(execution
            .error
            .unwrap()
            .contains("for_each=true but input is not an array"));
    }

    #[tokio::test]
    async fn test_for_each_with_fallback_recovers() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: foreach-fallback-workflow
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "input"
    on_error:
      action: fallback
      fallback_value: []
"#;

        save_test_workflow(&storage, "wf-foreach-fallback", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(
                &workflow,
                "wf-foreach-fallback",
                "test",
                serde_json::json!({"not": "an array"}),
            )
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!([])));
    }

    #[tokio::test]
    async fn test_for_each_processes_array() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // Note: input is passed as JSON string, so we need to parse it
        let yaml = r#"
name: foreach-success-workflow
settings:
  max_concurrency: 2
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) * 2"
"#;

        save_test_workflow(&storage, "wf-foreach-success", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(
                &workflow,
                "wf-foreach-success",
                "test",
                serde_json::json!([1, 2, 3]),
            )
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!([2, 4, 6])));
    }

    #[tokio::test]
    async fn test_for_each_empty_array_succeeds() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: foreach-empty-workflow
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) * 2"
"#;

        save_test_workflow(&storage, "wf-foreach-empty", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-foreach-empty", "test", serde_json::json!([]))
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!([])));
    }

    #[tokio::test]
    async fn test_for_each_max_items_limit() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // Set a low max_for_each_items limit for testing
        let yaml = r#"
name: foreach-limit-workflow
settings:
  max_for_each_items: 3
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) * 2"
"#;

        save_test_workflow(&storage, "wf-foreach-limit", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        // Try to process 5 items (exceeds limit of 3)
        let execution = executor
            .execute(
                &workflow,
                "wf-foreach-limit",
                "test",
                serde_json::json!([1, 2, 3, 4, 5]),
            )
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.unwrap().contains("exceeding limit of 3"));
    }

    #[tokio::test]
    async fn test_for_each_within_limit_succeeds() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // Set max_for_each_items to 5
        let yaml = r#"
name: foreach-within-limit-workflow
settings:
  max_for_each_items: 5
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) * 2"
"#;

        save_test_workflow(&storage, "wf-foreach-within", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        // Process exactly 5 items (within limit)
        let execution = executor
            .execute(
                &workflow,
                "wf-foreach-within",
                "test",
                serde_json::json!([1, 2, 3, 4, 5]),
            )
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!([2, 4, 6, 8, 10])));
    }

    // ============================================================================
    // Backoff Calculation Tests
    // ============================================================================

    #[test]
    fn test_retry_delay_fixed() {
        let config = RetryConfig {
            max_attempts: 3,
            delay_seconds: 5,
            backoff: BackoffType::Fixed,
        };

        assert_eq!(retry_delay(&config, 1), Duration::from_secs(5));
        assert_eq!(retry_delay(&config, 2), Duration::from_secs(5));
        assert_eq!(retry_delay(&config, 3), Duration::from_secs(5));
    }

    #[test]
    fn test_retry_delay_linear() {
        let config = RetryConfig {
            max_attempts: 3,
            delay_seconds: 2,
            backoff: BackoffType::Linear,
        };

        assert_eq!(retry_delay(&config, 1), Duration::from_secs(2));
        assert_eq!(retry_delay(&config, 2), Duration::from_secs(4));
        assert_eq!(retry_delay(&config, 3), Duration::from_secs(6));
    }

    #[test]
    fn test_retry_delay_exponential() {
        let config = RetryConfig {
            max_attempts: 4,
            delay_seconds: 1,
            backoff: BackoffType::Exponential,
        };

        assert_eq!(retry_delay(&config, 1), Duration::from_secs(1));
        assert_eq!(retry_delay(&config, 2), Duration::from_secs(2));
        assert_eq!(retry_delay(&config, 3), Duration::from_secs(4));
        assert_eq!(retry_delay(&config, 4), Duration::from_secs(8));
    }

    // ============================================================================
    // Unknown Node Type Test
    // ============================================================================

    #[tokio::test]
    async fn test_unknown_node_type_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: unknown-node-workflow
nodes:
  - id: unknown
    type: nonexistent_node_type
    config: {}
"#;

        save_test_workflow(&storage, "wf-unknown", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-unknown", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);
        assert!(execution.error.is_some());
        assert!(execution
            .error
            .unwrap()
            .contains("Unknown node type: nonexistent_node_type"));
    }

    // ============================================================================
    // Multi-Node Dependency Tests
    // ============================================================================

    #[tokio::test]
    async fn test_node_chain_propagates_data() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // Note: input is passed as JSON string, so we parse with from_json()
        let yaml = r#"
name: chain-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "10"
  - id: step2
    type: transform
    depends_on: [step1]
    config:
      expression: "from_json(input) * 2"
  - id: step3
    type: transform
    depends_on: [step2]
    config:
      expression: "from_json(input) + 5"
"#;

        save_test_workflow(&storage, "wf-chain", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-chain", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!(25))); // (10 * 2) + 5
    }

    #[tokio::test]
    async fn test_failure_in_chain_stops_execution() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: chain-fail-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "10"
  - id: step2
    type: always_fail
    depends_on: [step1]
    config: {}
  - id: step3
    type: transform
    depends_on: [step2]
    config:
      expression: '"should not reach"'
"#;

        save_test_workflow(&storage, "wf-chain-fail", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-chain-fail", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Failed);

        // Verify step3 was never executed
        let node_execs = storage.get_node_executions(&execution.id).await.unwrap();
        let step3_executed = node_execs.iter().any(|n| n.node_id == "step3");
        assert!(
            !step3_executed,
            "step3 should not have been executed after step2 failed"
        );
    }

    // ============================================================================
    // Resume Tests
    // ============================================================================

    #[tokio::test]
    async fn test_resume_failed_execution() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let failing_node = Arc::new(FailingNode::new());
        let mut registry = NodeRegistry::new();
        registry.register(failing_node.clone());
        let executor = Executor::new(registry, storage.clone());

        // Create workflow where step2 will fail first time, then succeed
        let yaml = r#"
name: resume-workflow
settings:
  timeout_seconds: 30
nodes:
  - id: step1
    type: transform
    config:
      expression: "100"
  - id: step2
    type: failing
    depends_on: [step1]
    config:
      fail_times: 1
  - id: step3
    type: transform
    depends_on: [step2]
    config:
      expression: '"final"'
"#;

        save_test_workflow(&storage, "wf-resume", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        // First execution should fail at step2
        failing_node.reset();
        let first_execution = executor
            .execute(&workflow, "wf-resume", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(first_execution.status, ExecutionStatus::Failed);

        // Verify step1 completed, step2 failed
        let node_execs = storage
            .get_node_executions(&first_execution.id)
            .await
            .unwrap();
        let step1_status = node_execs
            .iter()
            .find(|n| n.node_id == "step1")
            .map(|n| &n.status);
        let step2_status = node_execs
            .iter()
            .find(|n| n.node_id == "step2")
            .map(|n| &n.status);
        assert_eq!(step1_status, Some(&ExecutionStatus::Completed));
        assert_eq!(step2_status, Some(&ExecutionStatus::Failed));

        // Now resume - step2 should succeed on retry (fail_times=1 already exhausted)
        // But wait - the FailingNode counter is global, so it already failed once.
        // The resume will skip step1 and re-execute step2 which should now succeed.
        let resumed = executor.resume(&first_execution.id).await.unwrap();

        assert_eq!(resumed.status, ExecutionStatus::Completed);
        assert_eq!(resumed.trigger_type, "resume");
        assert_eq!(resumed.output, Some(serde_json::json!("final")));
    }

    #[tokio::test]
    async fn test_resume_non_failed_execution_errors() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: success-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "42"
"#;

        save_test_workflow(&storage, "wf-success", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let execution = executor
            .execute(&workflow, "wf-success", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);

        // Attempting to resume a completed execution should fail
        let result = executor.resume(&execution.id).await;
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("Cannot resume"),
            "Expected 'Cannot resume' error, got: {}",
            error
        );
    }

    #[tokio::test]
    async fn test_resume_uses_original_workflow_version() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let v1 = r#"
name: resume-versioned-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "1"
  - id: step2
    type: always_fail
    depends_on: [step1]
    config: {}
"#;
        save_test_workflow(&storage, "wf-resume-versioned", v1).await;
        let workflow_v1 = parse_workflow(v1).unwrap();

        let first = executor
            .execute(&workflow_v1, "wf-resume-versioned", "manual", Value::Null)
            .await
            .unwrap();
        assert_eq!(first.status, ExecutionStatus::Failed);
        assert_eq!(first.workflow_version, Some(1));

        // Change the workflow so the failing node would succeed if latest definition were used.
        let v2 = r#"
name: resume-versioned-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "1"
  - id: step2
    type: transform
    depends_on: [step1]
    config:
      expression: '"ok"'
"#;
        save_test_workflow(&storage, "wf-resume-versioned", v2).await;

        let resumed = executor.resume(&first.id).await.unwrap();
        assert_eq!(resumed.status, ExecutionStatus::Failed);
        assert_eq!(resumed.workflow_version, Some(1));
    }

    // ============================================================================
    // Chunked/Streaming Processing Tests
    // ============================================================================

    #[tokio::test]
    async fn test_chunked_for_each_processing() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // Test with chunk_size=3 processing 10 items
        let yaml = r#"
name: chunked-workflow
settings:
  max_concurrency: 2
  chunk_size: 3
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) + 1"
"#;

        save_test_workflow(&storage, "wf-chunked", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        // Create array of 10 items
        let input = serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

        let execution = executor
            .execute(&workflow, "wf-chunked", "test", input)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        // Each item should be incremented by 1
        assert_eq!(
            execution.output,
            Some(serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]))
        );
    }

    #[tokio::test]
    async fn test_chunked_processing_with_failure() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        // Chunk size 2, but the node always fails
        let yaml = r#"
name: chunked-fail-workflow
settings:
  chunk_size: 2
nodes:
  - id: fail_node
    type: always_fail
    for_each: true
    config: {}
"#;

        save_test_workflow(&storage, "wf-chunked-fail", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let input = serde_json::json!([1, 2, 3, 4]);

        let execution = executor
            .execute(&workflow, "wf-chunked-fail", "test", input)
            .await
            .unwrap();

        // Should fail on first item of first chunk
        assert_eq!(execution.status, ExecutionStatus::Failed);
    }

    #[tokio::test]
    async fn test_chunk_size_zero_processes_all_at_once() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let registry = NodeRegistry::new();
        let executor = Executor::new(registry, storage.clone());

        // chunk_size=0 means no chunking (default behavior)
        let yaml = r#"
name: no-chunk-workflow
settings:
  chunk_size: 0
  max_concurrency: 10
nodes:
  - id: foreach_node
    type: transform
    for_each: true
    config:
      expression: "from_json(input) * 2"
"#;

        save_test_workflow(&storage, "wf-no-chunk", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let input = serde_json::json!([1, 2, 3, 4, 5]);

        let execution = executor
            .execute(&workflow, "wf-no-chunk", "test", input)
            .await
            .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Completed);
        assert_eq!(execution.output, Some(serde_json::json!([2, 4, 6, 8, 10])));
    }

    // ============================================================================
    // Resume Tests
    // ============================================================================

    #[tokio::test]
    async fn test_resume_skips_completed_nodes() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: skip-completed-workflow
nodes:
  - id: step1
    type: transform
    config:
      expression: "1"
  - id: step2
    type: transform
    depends_on: [step1]
    config:
      expression: "2"
  - id: step3
    type: always_fail
    depends_on: [step2]
    config: {}
"#;

        save_test_workflow(&storage, "wf-skip-completed", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        // Execute - should fail at step3
        let first = executor
            .execute(&workflow, "wf-skip-completed", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(first.status, ExecutionStatus::Failed);

        // Verify step1 and step2 completed
        let node_execs = storage.get_node_executions(&first.id).await.unwrap();
        let completed: Vec<_> = node_execs
            .iter()
            .filter(|n| n.status == ExecutionStatus::Completed)
            .map(|n| n.node_id.as_str())
            .collect();
        assert!(completed.contains(&"step1"));
        assert!(completed.contains(&"step2"));

        // Resume - step1 and step2 should be skipped (from checkpoint)
        // step3 will still fail but the point is to verify checkpoint works
        let resumed = executor.resume(&first.id).await.unwrap();

        // Still fails because step3 always fails
        assert_eq!(resumed.status, ExecutionStatus::Failed);

        // Verify in the new execution, step1 and step2 were NOT re-executed
        // (they don't appear in node_executions for the resumed run)
        let resumed_node_execs = storage.get_node_executions(&resumed.id).await.unwrap();
        let resumed_node_ids: Vec<_> = resumed_node_execs
            .iter()
            .map(|n| n.node_id.as_str())
            .collect();

        // Only step3 should be executed in the resumed run
        assert!(
            !resumed_node_ids.contains(&"step1"),
            "step1 should have been skipped"
        );
        assert!(
            !resumed_node_ids.contains(&"step2"),
            "step2 should have been skipped"
        );
        assert!(
            resumed_node_ids.contains(&"step3"),
            "step3 should have been executed"
        );
    }

    #[tokio::test]
    async fn test_resume_skips_recovered_failed_nodes() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let mut registry = NodeRegistry::new();
        registry.register(Arc::new(AlwaysFailNode));
        let executor = Executor::new(registry, storage.clone());

        let yaml = r#"
name: resume-recovered-workflow
nodes:
  - id: step1
    type: always_fail
    config: {}
    on_error:
      action: fallback
      fallback_value: 10
  - id: step2
    type: transform
    depends_on: [step1]
    config:
      expression: "from_json(input) + 1"
  - id: step3
    type: always_fail
    depends_on: [step2]
    config: {}
"#;

        save_test_workflow(&storage, "wf-resume-recovered", yaml).await;
        let workflow = parse_workflow(yaml).unwrap();

        let first = executor
            .execute(&workflow, "wf-resume-recovered", "test", Value::Null)
            .await
            .unwrap();

        assert_eq!(first.status, ExecutionStatus::Failed);

        let first_node_execs = storage.get_node_executions(&first.id).await.unwrap();
        let step1_status = first_node_execs
            .iter()
            .find(|n| n.node_id == "step1")
            .map(|n| &n.status);
        let step1_output = first_node_execs
            .iter()
            .find(|n| n.node_id == "step1")
            .and_then(|n| n.output.as_ref())
            .cloned();
        assert_eq!(step1_status, Some(&ExecutionStatus::Failed));
        assert_eq!(step1_output, Some(serde_json::json!(10)));

        let resumed = executor.resume(&first.id).await.unwrap();
        assert_eq!(resumed.status, ExecutionStatus::Failed);

        // step1 and step2 should be reused from checkpoint, only failing step3 should run again.
        let resumed_node_execs = storage.get_node_executions(&resumed.id).await.unwrap();
        let resumed_node_ids: Vec<_> = resumed_node_execs
            .iter()
            .map(|n| n.node_id.as_str())
            .collect();

        assert!(!resumed_node_ids.contains(&"step1"));
        assert!(!resumed_node_ids.contains(&"step2"));
        assert!(resumed_node_ids.contains(&"step3"));
    }
}
