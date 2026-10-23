/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Distributed worker — pulls [`RunJob`]s from a [`JobQueue`] and executes them.
//!
//! A worker is intentionally thin:
//!
//! 1. Dequeue a job (blocking with timeout).
//! 2. Load the workflow definition from shared storage (SQLite/PostgreSQL).
//! 3. Execute via the engine with the job ID as the execution ID.
//! 4. `ack` on success; `requeue` (bounded) on failure.
//!
//! Because storage is shared and executions are keyed by `job_id`, any number
//! of workers can run on any number of machines against the same queue. This
//! is at-least-once delivery with idempotent executions — r8r's documented
//! semantics (see `docs/ARCHITECTURE.md`).
//!
//! Graceful shutdown: [`Worker::shutdown`] signals all consumer tasks to stop
//! after their current job, then joins them.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use super::{JobQueue, RunJob, DEFAULT_STALE_JOB_SECS};
use crate::engine::executor::{ExecutionMetadata, Executor};
use crate::error::Result;
use crate::nodes::NodeRegistry;
use crate::storage::Storage;
use crate::workflow::parse_workflow;

/// Default number of concurrent consumers per worker process.
pub const DEFAULT_CONCURRENCY: usize = 4;

/// Maximum delivery attempts before a job is abandoned (never requeued again).
pub const MAX_ATTEMPTS: u32 = 5;

/// A worker process consuming jobs from a shared queue.
pub struct Worker {
    queue: Arc<dyn JobQueue>,
    storage: Arc<dyn Storage>,
    registry: NodeRegistry,
    concurrency: usize,
    /// Watch channel: `true` means "finish current job and exit".
    shutdown_tx: watch::Sender<bool>,
    consumers: Vec<JoinHandle<()>>,
}

impl Worker {
    /// Create a new worker.
    pub fn new(
        queue: Arc<dyn JobQueue>,
        storage: Arc<dyn Storage>,
        registry: NodeRegistry,
        concurrency: usize,
    ) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            queue,
            storage,
            registry,
            concurrency: concurrency.max(1),
            shutdown_tx,
            consumers: Vec::new(),
        }
    }

    /// Start the worker loop, spawning `concurrency` consumer tasks.
    ///
    /// Returns once consumers are running — this is a long-lived background
    /// process. Call [`Worker::shutdown`] to stop it gracefully.
    pub async fn run(&mut self) -> Result<()> {
        // Requeue stale in-flight jobs (from crashed workers) once at startup.
        match self
            .queue
            .requeue_stale(Duration::from_secs(DEFAULT_STALE_JOB_SECS))
            .await
        {
            Ok(0) => {}
            Ok(n) => info!("worker startup: requeued {n} stale in-flight job(s)"),
            Err(e) => warn!("worker startup: stale requeue failed: {e}"),
        }

        for i in 0..self.concurrency {
            let queue = self.queue.clone();
            let storage = self.storage.clone();
            let registry = self.registry.clone();
            let mut shutdown_rx = self.shutdown_tx.subscribe();
            let handle = tokio::spawn(async move {
                let executor = Executor::new(registry, storage.clone());
                loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            debug!("consumer {i}: shutdown signal received");
                            break;
                        }
                        result = queue.dequeue(Duration::from_secs(1)) => {
                            match result {
                                Ok(Some(job)) => {
                                    if let Err(e) =
                                        Self::process(&executor, &queue, &storage, job).await
                                    {
                                        warn!("consumer {i}: job processing error: {e}");
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    // Transient queue error (e.g. Redis down). Back off.
                                    warn!("consumer {i}: dequeue error: {e}; retrying in 2s");
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            }
                        }
                    }
                }
            });
            self.consumers.push(handle);
        }
        info!(
            "worker started: {} consumers, queue backend active",
            self.concurrency
        );
        Ok(())
    }

    /// Signal all consumers to stop after their current job, then join them.
    pub async fn shutdown(&mut self) {
        let _ = self.shutdown_tx.send(true);
        for handle in self.consumers.drain(..) {
            handle.await.ok();
        }
        info!("worker stopped");
    }

    /// Process at most one job, then return.
    ///
    /// Used by `r8r worker --once` (cron/systemd-driven workers) and tests.
    /// Returns `true` when a job was processed, `false` when the queue had no
    /// job within the (short) wait window.
    pub async fn process_one(&self, wait: Duration) -> Result<bool> {
        let executor = Executor::new(self.registry.clone(), self.storage.clone());
        // First recover stale jobs (same semantics as `run`).
        match self
            .queue
            .requeue_stale(Duration::from_secs(DEFAULT_STALE_JOB_SECS))
            .await
        {
            Ok(0) => {}
            Ok(n) => info!("worker startup: requeued {n} stale in-flight job(s)"),
            Err(e) => warn!("worker startup: stale requeue failed: {e}"),
        }
        match self.queue.dequeue(wait).await {
            Ok(Some(job)) => {
                Self::process(&executor, &self.queue, &self.storage, job).await?;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Process one job: load workflow → execute → ack/requeue.
    ///
    /// Any failure (workflow missing, parse error, execution error) is handled
    /// internally: the job is requeued (bounded by [`MAX_ATTEMPTS`]) or dropped.
    /// Only queue-level failures (ack/requeue themselves failing) propagate.
    async fn process(
        executor: &Executor,
        queue: &Arc<dyn JobQueue>,
        storage: &Arc<dyn Storage>,
        job: RunJob,
    ) -> Result<()> {
        // Load the workflow definition (pinned version if requested).
        let stored = match storage.get_workflow_by_id(&job.workflow_id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                return Self::handle_failure(
                    queue,
                    &job,
                    crate::error::Error::Workflow(format!(
                        "queue job {} references missing workflow {}",
                        job.job_id, job.workflow_id
                    )),
                )
                .await;
            }
            Err(e) => return Self::handle_failure(queue, &job, e).await,
        };

        let definition = match job.version {
            Some(ver) => match storage
                .get_workflow_version_by_id(&job.workflow_id, ver)
                .await
            {
                Ok(Some(v)) => v.definition,
                Ok(None) => {
                    return Self::handle_failure(
                        queue,
                        &job,
                        crate::error::Error::Workflow(format!(
                            "queue job {} references missing version {ver} of {}",
                            job.job_id, job.workflow_id
                        )),
                    )
                    .await;
                }
                Err(e) => return Self::handle_failure(queue, &job, e).await,
            },
            None => stored.definition.clone(),
        };
        let workflow = match parse_workflow(&definition) {
            Ok(w) => w,
            Err(e) => return Self::handle_failure(queue, &job, e).await,
        };

        debug!(
            job_id = %job.job_id,
            workflow_id = %job.workflow_id,
            version = ?job.version,
            "worker executing queued job"
        );

        let mut metadata = ExecutionMetadata {
            correlation_id: job.correlation_id.clone(),
            idempotency_key: job.idempotency_key.clone(),
            origin: job.origin.clone(),
        };
        if metadata.origin.is_none() {
            metadata.origin = Some("queue".to_string());
        }

        match executor
            .execute_with_id_and_metadata(
                &workflow,
                &job.workflow_id,
                &job.trigger_type,
                job.input.clone(),
                job.job_id.clone(),
                metadata,
            )
            .await
        {
            Ok(_execution) => {
                queue.ack(&job).await?;
                debug!("job {} completed and acked", job.job_id);
            }
            Err(e) => Self::handle_failure(queue, &job, e).await?,
        }
        Ok(())
    }

    /// Requeue a failed job (bounded by [`MAX_ATTEMPTS`]) or drop it.
    async fn handle_failure(
        queue: &Arc<dyn JobQueue>,
        job: &RunJob,
        err: crate::error::Error,
    ) -> Result<()> {
        if job.attempts + 1 >= MAX_ATTEMPTS {
            error!(
                job_id = %job.job_id,
                "job failed after {} attempts ({err}); dropping",
                job.attempts + 1
            );
            // Acknowledge (drop) so it doesn't loop forever.
            queue.ack(job).await?;
        } else {
            warn!(
                job_id = %job.job_id,
                attempts = job.attempts,
                "job failed ({err}); requeueing"
            );
            queue.requeue(job).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::memory::MemoryQueue;
    use crate::storage::SqliteStorage;

    const TEST_WF: &str = r#"
name: test-echo
version: 1
nodes:
  - id: start
    type: set
    config:
      fields:
        - name: result
          value: hello
"#;

    async fn save_test_workflow(
        storage: &Arc<dyn Storage>,
    ) -> Result<crate::storage::StoredWorkflow> {
        let workflow = parse_workflow(TEST_WF)?;
        let stored = crate::storage::StoredWorkflow {
            id: uuid::Uuid::new_v4().to_string(),
            name: workflow.name.clone(),
            definition: TEST_WF.to_string(),
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        storage.save_workflow(&stored).await?;
        Ok(stored)
    }

    #[tokio::test]
    async fn worker_executes_queued_job_and_acks() -> Result<()> {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open_in_memory()?);
        let stored = save_test_workflow(&storage).await?;

        let queue: Arc<dyn JobQueue> = Arc::new(MemoryQueue::new());
        let job = RunJob::new(
            "job-1".to_string(),
            stored.id.clone(),
            None,
            serde_json::json!({}),
        );
        queue.enqueue(job).await?;

        let registry = NodeRegistry::default();
        let executor = Executor::new(registry, storage.clone());
        let job = queue.dequeue(Duration::from_millis(200)).await?.unwrap();
        Worker::process(&executor, &queue, &storage, job).await?;

        assert_eq!(queue.pending_len().await?, 0);
        let executions = storage.list_executions(&stored.name, 10).await?;
        assert_eq!(executions.len(), 1);
        eprintln!("execution error: {:?}", executions[0].error);
        eprintln!("execution output: {:?}", executions[0].output);
        assert_eq!(executions[0].status.to_string(), "completed");
        Ok(())
    }

    #[tokio::test]
    async fn worker_requeues_failed_job() -> Result<()> {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open_in_memory()?);
        let queue: Arc<dyn JobQueue> = Arc::new(MemoryQueue::new());

        // Job referencing a missing workflow → processing fails → requeued.
        let bad_job = RunJob::new(
            "job-missing-wf".to_string(),
            "no-such-workflow".to_string(),
            None,
            serde_json::json!({}),
        );
        queue.enqueue(bad_job).await?;

        let registry = NodeRegistry::default();
        let executor = Executor::new(registry, storage.clone());
        let job = queue.dequeue(Duration::from_millis(200)).await?.unwrap();
        // Failure is swallowed inside process (job requeued, no error surfaced).
        let _ = Worker::process(&executor, &queue, &storage, job).await;

        let retried = queue.dequeue(Duration::from_millis(200)).await?.unwrap();
        assert_eq!(retried.job_id, "job-missing-wf");
        assert_eq!(retried.attempts, 1);
        Ok(())
    }

    #[tokio::test]
    async fn worker_drops_job_after_max_attempts() -> Result<()> {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open_in_memory()?);
        let queue: Arc<dyn JobQueue> = Arc::new(MemoryQueue::new());
        let mut job = RunJob::new(
            "job-exhausted".to_string(),
            "no-such-workflow".to_string(),
            None,
            serde_json::json!({}),
        );
        job.attempts = MAX_ATTEMPTS - 1; // next failure hits the cap
        queue.enqueue(job).await?;

        let registry = NodeRegistry::default();
        let executor = Executor::new(registry, storage.clone());
        let job = queue.dequeue(Duration::from_millis(200)).await?.unwrap();
        Worker::process(&executor, &queue, &storage, job).await?;

        // Dropped (acked) — nothing left pending.
        assert_eq!(queue.pending_len().await?, 0);
        Ok(())
    }
}
