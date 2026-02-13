//! Workflow scheduler for cron-based triggers.
//!
//! Manages cron jobs that automatically execute workflows on schedule.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{FixedOffset, TimeZone, Utc};
use chrono_tz::Tz;
use tokio::sync::{Mutex, RwLock};
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info, warn};

use crate::api::Monitor;
use crate::engine::Executor;
use crate::nodes::NodeRegistry;
use crate::storage::SqliteStorage;
use crate::workflow::{parse_workflow, Trigger, Workflow};
use crate::Result;

/// Manages scheduled workflow executions.
pub struct Scheduler {
    /// The underlying job scheduler (wrapped for interior mutability)
    job_scheduler: Arc<Mutex<JobScheduler>>,
    /// Map of workflow name to job UUIDs for tracking
    jobs: Arc<RwLock<HashMap<String, Vec<uuid::Uuid>>>>,
    /// Storage for loading workflows
    storage: SqliteStorage,
    /// Node registry for execution
    registry: Arc<NodeRegistry>,
    /// Optional monitor for broadcasting execution lifecycle events
    monitor: Option<Arc<Monitor>>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub async fn new(storage: SqliteStorage, registry: Arc<NodeRegistry>) -> Result<Self> {
        let job_scheduler = JobScheduler::new()
            .await
            .map_err(|e| crate::Error::Internal(format!("Failed to create scheduler: {}", e)))?;

        Ok(Self {
            job_scheduler: Arc::new(Mutex::new(job_scheduler)),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            storage,
            registry,
            monitor: None,
        })
    }

    /// Attach a live execution monitor for scheduler-triggered runs.
    pub fn with_monitor(mut self, monitor: Arc<Monitor>) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Start the scheduler and load all cron triggers.
    pub async fn start(&self) -> Result<()> {
        info!("Starting workflow scheduler...");

        // Load all enabled workflows
        let workflows = self.storage.list_workflows().await?;
        let mut scheduled_count = 0;

        for stored in workflows {
            if !stored.enabled {
                continue;
            }

            match parse_workflow(&stored.definition) {
                Ok(workflow) => {
                    if let Err(e) = self.register_workflow(&stored.id, &workflow).await {
                        warn!(
                            "Failed to register triggers for workflow '{}': {}",
                            workflow.name, e
                        );
                    } else {
                        let cron_count = workflow
                            .triggers
                            .iter()
                            .filter(|t| matches!(t, Trigger::Cron { .. }))
                            .count();
                        if cron_count > 0 {
                            scheduled_count += cron_count;
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse workflow '{}': {}", stored.name, e);
                }
            }
        }

        // Start the scheduler
        {
            let sched = self.job_scheduler.lock().await;
            sched
                .start()
                .await
                .map_err(|e| crate::Error::Internal(format!("Failed to start scheduler: {}", e)))?;
        }

        info!("Scheduler started with {} cron job(s)", scheduled_count);
        Ok(())
    }

    /// Stop the scheduler gracefully.
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping workflow scheduler...");

        {
            let mut sched = self.job_scheduler.lock().await;
            sched
                .shutdown()
                .await
                .map_err(|e| crate::Error::Internal(format!("Failed to stop scheduler: {}", e)))?;
        }

        info!("Scheduler stopped");
        Ok(())
    }

    /// Register all triggers for a workflow.
    pub async fn register_workflow(&self, workflow_id: &str, workflow: &Workflow) -> Result<()> {
        let mut job_ids = Vec::new();

        for trigger in &workflow.triggers {
            if let Trigger::Cron { schedule, timezone } = trigger {
                let job_id = self
                    .add_cron_job(workflow_id, &workflow.name, schedule, timezone.as_deref())
                    .await?;
                job_ids.push(job_id);
            }
        }

        if !job_ids.is_empty() {
            let mut jobs = self.jobs.write().await;
            jobs.insert(workflow.name.clone(), job_ids);
        }

        Ok(())
    }

    /// Unregister all triggers for a workflow.
    pub async fn unregister_workflow(&self, workflow_name: &str) -> Result<()> {
        let mut jobs = self.jobs.write().await;

        if let Some(job_ids) = jobs.remove(workflow_name) {
            let sched = self.job_scheduler.lock().await;
            for job_id in job_ids {
                if let Err(e) = sched.remove(&job_id).await {
                    warn!("Failed to remove job {}: {}", job_id, e);
                }
            }
            info!("Unregistered triggers for workflow '{}'", workflow_name);
        }

        Ok(())
    }

    /// Add a cron job for a workflow.
    async fn add_cron_job(
        &self,
        workflow_id: &str,
        workflow_name: &str,
        schedule: &str,
        timezone: Option<&str>,
    ) -> Result<uuid::Uuid> {
        let storage = self.storage.clone();
        let registry = self.registry.clone();
        let monitor = self.monitor.clone();
        let wf_id = workflow_id.to_string();
        let wf_name = workflow_name.to_string();

        let job = match timezone {
            Some(raw_timezone) => match parse_timezone(raw_timezone)? {
                CronTimezone::Utc => {
                    create_cron_job(schedule, Utc, storage, registry, wf_id, wf_name, monitor)?
                }
                CronTimezone::FixedOffset(offset) => {
                    create_cron_job(schedule, offset, storage, registry, wf_id, wf_name, monitor)?
                }
                CronTimezone::Named(timezone) => create_cron_job(
                    schedule, timezone, storage, registry, wf_id, wf_name, monitor,
                )?,
            },
            None => create_cron_job(schedule, Utc, storage, registry, wf_id, wf_name, monitor)?,
        };

        let job_id = job.guid();

        {
            let sched = self.job_scheduler.lock().await;
            sched
                .add(job)
                .await
                .map_err(|e| crate::Error::Internal(format!("Failed to add cron job: {}", e)))?;
        }

        info!(
            "Registered cron trigger for '{}': {}{}",
            workflow_name,
            schedule,
            timezone
                .map(|tz| format!(" (timezone: {})", tz))
                .unwrap_or_default()
        );

        Ok(job_id)
    }

    /// Reload a workflow's triggers (unregister old, register new).
    pub async fn reload_workflow(&self, workflow_id: &str, workflow: &Workflow) -> Result<()> {
        self.unregister_workflow(&workflow.name).await?;
        self.register_workflow(workflow_id, workflow).await?;
        Ok(())
    }

    /// Get the number of active jobs.
    pub async fn job_count(&self) -> usize {
        let jobs = self.jobs.read().await;
        jobs.values().map(|v| v.len()).sum()
    }
}

enum CronTimezone {
    Utc,
    FixedOffset(FixedOffset),
    Named(Tz),
}

fn parse_timezone(raw: &str) -> Result<CronTimezone> {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("utc") {
        return Ok(CronTimezone::Utc);
    }

    if let Ok(tz) = trimmed.parse::<Tz>() {
        return Ok(CronTimezone::Named(tz));
    }

    if let Some(offset) = parse_fixed_offset(trimmed) {
        return Ok(CronTimezone::FixedOffset(offset));
    }

    Err(crate::Error::Internal(format!(
        "Invalid timezone '{}'. Use IANA name (e.g. 'Asia/Kuala_Lumpur') or UTC offset (e.g. '+08:00')",
        raw
    )))
}

fn parse_fixed_offset(raw: &str) -> Option<FixedOffset> {
    let sign = if raw.starts_with('+') {
        1
    } else if raw.starts_with('-') {
        -1
    } else {
        return None;
    };

    let tz = &raw[1..];
    let (hours, minutes) = if let Some((h, m)) = tz.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if tz.len() == 4 {
        (tz[0..2].parse::<i32>().ok()?, tz[2..4].parse::<i32>().ok()?)
    } else {
        return None;
    };

    if hours > 23 || minutes > 59 {
        return None;
    }

    let total_seconds = sign * (hours * 3600 + minutes * 60);
    FixedOffset::east_opt(total_seconds)
}

fn create_cron_job<TZ: TimeZone + Send + Sync + 'static>(
    schedule: &str,
    timezone: TZ,
    storage: SqliteStorage,
    registry: Arc<NodeRegistry>,
    workflow_id: String,
    workflow_name: String,
    monitor: Option<Arc<Monitor>>,
) -> Result<Job> {
    Job::new_async_tz(schedule, timezone, move |_uuid, _lock| {
        let storage = storage.clone();
        let registry = registry.clone();
        let workflow_id = workflow_id.clone();
        let workflow_name = workflow_name.clone();
        let monitor = monitor.clone();

        Box::pin(async move {
            info!("Cron trigger firing for workflow '{}'", workflow_name);

            let stored = match storage.get_workflow(&workflow_name).await {
                Ok(Some(w)) => w,
                Ok(None) => {
                    error!(
                        "Workflow '{}' not found for scheduled execution",
                        workflow_name
                    );
                    return;
                }
                Err(e) => {
                    error!("Failed to load workflow '{}': {}", workflow_name, e);
                    return;
                }
            };

            let workflow = match parse_workflow(&stored.definition) {
                Ok(w) => w,
                Err(e) => {
                    error!("Failed to parse workflow '{}': {}", workflow_name, e);
                    return;
                }
            };

            let mut executor = Executor::new((*registry).clone(), storage.clone());
            if let Some(monitor) = monitor {
                executor = executor.with_monitor(monitor);
            }

            match executor
                .execute(&workflow, &workflow_id, "cron", serde_json::json!({}))
                .await
            {
                Ok(execution) => {
                    info!(
                        "Cron execution of '{}' completed: {} ({})",
                        workflow_name, execution.status, execution.id
                    );
                }
                Err(e) => {
                    error!("Cron execution of '{}' failed: {}", workflow_name, e);
                }
            }
        })
    })
    .map_err(|e| crate::Error::Internal(format!("Invalid cron expression '{}': {}", schedule, e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timezone_named() {
        let parsed = parse_timezone("Asia/Kuala_Lumpur").unwrap();
        assert!(matches!(parsed, CronTimezone::Named(_)));
    }

    #[test]
    fn test_parse_timezone_fixed_offset() {
        let parsed = parse_timezone("+08:00").unwrap();
        assert!(matches!(parsed, CronTimezone::FixedOffset(_)));

        let parsed_compact = parse_timezone("-0530").unwrap();
        assert!(matches!(parsed_compact, CronTimezone::FixedOffset(_)));
    }

    #[test]
    fn test_parse_timezone_utc() {
        let parsed = parse_timezone("utc").unwrap();
        assert!(matches!(parsed, CronTimezone::Utc));
    }

    #[test]
    fn test_parse_timezone_invalid() {
        assert!(parse_timezone("not-a-real-tz").is_err());
    }

    #[test]
    fn test_parse_fixed_offset_rejects_out_of_range() {
        assert!(parse_fixed_offset("+25:00").is_none());
        assert!(parse_fixed_offset("+10:99").is_none());
    }
}
