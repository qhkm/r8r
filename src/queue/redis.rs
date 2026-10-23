/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Redis-backed reliable job queue (optional, `--features redis`).
//!
//! Implements the classic **BRPOPLPUSH** pattern:
//!
//! ```text
//! enqueue           dequeue                    ack
//!   ─────────┐     ┌──────────────┐          ┌──────────┐
//!  r8r:queue:runs ──► r8r:queue:runs:processing ──► (removed)
//!            │     └──────────────┘             requeue
//!            │           │ stale / crash            │
//!            └───────────┴──────────────────────────┘
//! ```
//!
//! `BRPOPLPUSH` is atomic, so a worker that crashes *after* dequeuing leaves
//! the job in the in-flight list rather than losing it. A periodic/startup
//! [`JobQueue::requeue_stale`] moves jobs older than the max age back to
//! pending. Executions keyed by `job_id` keep replay idempotent.

use std::time::Duration;

use redis::AsyncCommands;
use tracing::{debug, warn};

use super::{JobQueue, RunJob, DEFAULT_STALE_JOB_SECS};
use crate::error::Result;

/// Prefix for all r8r queue keys.
const KEY_PREFIX: &str = "r8r:queue";

/// Pending (ready-to-run) list key.
fn pending_key() -> String {
    format!("{KEY_PREFIX}:runs")
}

/// In-flight (leased) list key.
fn processing_key() -> String {
    format!("{KEY_PREFIX}:runs:processing")
}

/// Redis-backed job queue.
#[derive(Debug, Clone)]
pub struct RedisQueue {
    client: redis::Client,
}

impl RedisQueue {
    /// Create a queue from a `redis://` URL (e.g. `redis://localhost:6379`).
    pub fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| crate::error::Error::Internal(format!("invalid Redis URL: {e}")))?;
        Ok(Self { client })
    }

    async fn conn(&self) -> Result<redis::aio::MultiplexedConnection> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis connection failed: {e}")))
    }
}

#[async_trait::async_trait]
impl JobQueue for RedisQueue {
    async fn enqueue(&self, job: RunJob) -> Result<()> {
        let payload = serde_json::to_string(&job)
            .map_err(|e| crate::error::Error::Internal(format!("job serialization failed: {e}")))?;
        let mut conn = self.conn().await?;
        conn.lpush::<_, _, ()>(pending_key(), &payload)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LPUSH failed: {e}")))?;
        debug!(
            "enqueued job {} for workflow {}",
            job.job_id, job.workflow_id
        );
        Ok(())
    }

    async fn dequeue(&self, timeout: Duration) -> Result<Option<RunJob>> {
        let secs = timeout.as_secs().max(1);
        let mut conn = self.conn().await?;
        let raw: Option<String> = conn
            .brpoplpush::<_, _, Option<String>>(pending_key(), processing_key(), secs as f64)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis BRPOPLPUSH failed: {e}")))?;

        let Some(raw) = raw else {
            return Ok(None);
        };
        match serde_json::from_str::<RunJob>(&raw) {
            Ok(job) => Ok(Some(job)),
            Err(e) => {
                // Poisoned payload: drop it (it is already in processing) but
                // surface a warning so operators can investigate.
                warn!("dropped unparsable queue job: {e}: {raw:.200}");
                Ok(None)
            }
        }
    }

    async fn ack(&self, job: &RunJob) -> Result<()> {
        let payload = serde_json::to_string(job)
            .map_err(|e| crate::error::Error::Internal(format!("job serialization failed: {e}")))?;
        let mut conn = self.conn().await?;
        conn.lrem::<_, _, i64>(processing_key(), 1, &payload)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LREM failed: {e}")))?;
        debug!("acked job {}", job.job_id);
        Ok(())
    }

    async fn requeue(&self, job: &RunJob) -> Result<()> {
        let mut retried = job.clone();
        retried.attempts += 1;
        let payload = serde_json::to_string(&retried)
            .map_err(|e| crate::error::Error::Internal(format!("job serialization failed: {e}")))?;

        let mut conn = self.conn().await?;
        // Remove the old (lower-attempt) copy from in-flight…
        let old_payload = serde_json::to_string(job)
            .map_err(|e| crate::error::Error::Internal(format!("job serialization failed: {e}")))?;
        conn.lrem::<_, _, i64>(processing_key(), 1, &old_payload)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LREM failed: {e}")))?;
        // …and push the bumped copy back to pending.
        conn.lpush::<_, _, ()>(pending_key(), &payload)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LPUSH failed: {e}")))?;
        debug!("requeued job {} (attempt {})", job.job_id, retried.attempts);
        Ok(())
    }

    async fn pending_len(&self) -> Result<usize> {
        let mut conn = self.conn().await?;
        let len: i64 = conn
            .llen(pending_key())
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LLEN failed: {e}")))?;
        Ok(len.max(0) as usize)
    }

    async fn requeue_stale(&self, max_age: Duration) -> Result<usize> {
        let mut conn = self.conn().await?;

        // Snapshot the whole in-flight list. Fine for typical queue depths;
        // avoids a Lua script dependency.
        let inflight: Vec<String> = conn
            .lrange(processing_key(), 0, -1)
            .await
            .map_err(|e| crate::error::Error::Internal(format!("Redis LRANGE failed: {e}")))?;

        let cutoff = chrono::Utc::now()
            - chrono::Duration::from_std(max_age)
                .unwrap_or(chrono::Duration::seconds(DEFAULT_STALE_JOB_SECS as i64));
        let mut moved = 0usize;

        for raw in inflight {
            let Ok(job) = serde_json::from_str::<RunJob>(&raw) else {
                // Unparsable in-flight entry: drop it to keep the list clean.
                conn.lrem::<_, _, i64>(processing_key(), 0, &raw).await.ok();
                continue;
            };
            if job.enqueued_at < cutoff {
                let mut retried = job.clone();
                retried.attempts += 1;
                let bumped = serde_json::to_string(&retried).unwrap_or_else(|_| raw.clone());
                conn.lrem::<_, _, i64>(processing_key(), 1, &raw).await.ok();
                conn.lpush::<_, _, ()>(pending_key(), &bumped).await.ok();
                moved += 1;
            }
        }
        if moved > 0 {
            warn!("requeued {moved} stale in-flight job(s) from {max_age:?}");
        }
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_job(id: &str) -> RunJob {
        RunJob::new(
            id.to_string(),
            "wf-1".to_string(),
            None,
            serde_json::json!({"x": 1}),
        )
    }

    #[test]
    fn pending_and_processing_keys_are_distinct() {
        assert_ne!(pending_key(), processing_key());
        assert!(pending_key().starts_with("r8r:queue"));
        assert!(processing_key().contains("processing"));
    }

    #[test]
    fn job_serialization_roundtrip_preserves_fields() {
        let job = sample_job("abc")
            .with_trigger("api")
            .with_correlation("corr-1")
            .with_idempotency_key("idem-1")
            .with_origin("cli");
        let raw = serde_json::to_string(&job).unwrap();
        let back: RunJob = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, job);
        assert_eq!(back.attempts, 0);
        assert_eq!(back.trigger_type, "api");
        assert_eq!(back.correlation_id.as_deref(), Some("corr-1"));
    }

    #[test]
    fn invalid_redis_url_errors() {
        assert!(RedisQueue::new("not-a-url").is_err());
    }
}
