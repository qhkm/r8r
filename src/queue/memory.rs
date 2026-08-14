/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! In-process FIFO job queue.
//!
//! Used by default (no Redis required) and in unit/integration tests. Jobs
//! live in a `VecDeque` guarded by a Tokio mutex, with a notify channel so
//! `dequeue` can block without busy-polling.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Notify};

use super::{JobQueue, RunJob};
use crate::error::Result;

/// In-process FIFO job queue.
#[derive(Debug, Clone)]
pub struct MemoryQueue {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    pending: Mutex<VecDeque<RunJob>>,
    notify: Notify,
}

impl Default for MemoryQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryQueue {
    /// Create an empty in-memory queue.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                pending: Mutex::new(VecDeque::new()),
                notify: Notify::new(),
            }),
        }
    }
}

#[async_trait::async_trait]
impl JobQueue for MemoryQueue {
    async fn enqueue(&self, job: RunJob) -> Result<()> {
        self.inner.pending.lock().await.push_back(job);
        self.inner.notify.notify_one();
        Ok(())
    }

    async fn dequeue(&self, timeout: Duration) -> Result<Option<RunJob>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(job) = self.inner.pending.lock().await.pop_front() {
                return Ok(Some(job));
            }
            let notified = self.inner.notify.notified();
            // Re-check before sleeping to avoid missing a wakeup race.
            if !self.inner.pending.lock().await.is_empty() {
                continue;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                // Timeout: one final check (a job may have arrived concurrently).
                if let Some(job) = self.inner.pending.lock().await.pop_front() {
                    return Ok(Some(job));
                }
                return Ok(None);
            }
        }
    }

    async fn ack(&self, _job: &RunJob) -> Result<()> {
        // In-memory queue: nothing is in-flight, nothing to remove.
        Ok(())
    }

    async fn requeue(&self, job: &RunJob) -> Result<()> {
        let mut pending = self.inner.pending.lock().await;
        let mut retried = job.clone();
        retried.attempts += 1;
        pending.push_front(retried); // LIFO retry: failed job gets next shot.
        self.inner.notify.notify_one();
        Ok(())
    }

    async fn pending_len(&self) -> Result<usize> {
        Ok(self.inner.pending.lock().await.len())
    }

    async fn requeue_stale(&self, _max_age: Duration) -> Result<usize> {
        // Nothing is in-flight in the in-memory queue.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_job(id: &str) -> RunJob {
        RunJob::new(
            id.to_string(),
            "wf-1".to_string(),
            None,
            serde_json::json!({"x": 1}),
        )
    }

    #[tokio::test]
    async fn enqueue_dequeue_fifo_order() {
        let q = MemoryQueue::new();
        q.enqueue(sample_job("a")).await.unwrap();
        q.enqueue(sample_job("b")).await.unwrap();

        let a = q
            .dequeue(Duration::from_millis(100))
            .await
            .unwrap()
            .unwrap();
        let b = q
            .dequeue(Duration::from_millis(100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.job_id, "a");
        assert_eq!(b.job_id, "b");
        assert_eq!(q.pending_len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dequeue_timeout_returns_none() {
        let q = MemoryQueue::new();
        let job = q.dequeue(Duration::from_millis(50)).await.unwrap();
        assert!(job.is_none());
    }

    #[tokio::test]
    async fn dequeue_blocks_until_enqueue() {
        let q = MemoryQueue::new();
        let q2 = q.clone();
        let handle = tokio::spawn(async move { q2.dequeue(Duration::from_secs(2)).await.unwrap() });
        // Give the dequeue a moment to start blocking, then enqueue.
        tokio::time::sleep(Duration::from_millis(50)).await;
        q.enqueue(sample_job("wake")).await.unwrap();
        let job = handle.await.unwrap().unwrap();
        assert_eq!(job.job_id, "wake");
    }

    #[tokio::test]
    async fn requeue_increments_attempts_and_prioritizes() {
        let q = MemoryQueue::new();
        q.enqueue(sample_job("first")).await.unwrap();
        let job = q.dequeue(Duration::from_millis(50)).await.unwrap().unwrap();
        q.requeue(&job).await.unwrap();

        // Requeued job sits at the front and has attempts = 1.
        let retried = q.dequeue(Duration::from_millis(50)).await.unwrap().unwrap();
        assert_eq!(retried.job_id, "first");
        assert_eq!(retried.attempts, 1);
    }

    #[tokio::test]
    async fn ack_is_noop() {
        let q = MemoryQueue::new();
        q.enqueue(sample_job("x")).await.unwrap();
        let job = q.dequeue(Duration::from_millis(50)).await.unwrap().unwrap();
        q.ack(&job).await.unwrap();
        assert_eq!(q.pending_len().await.unwrap(), 0);
    }
}
