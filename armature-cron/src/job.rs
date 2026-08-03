//! Job definition and execution.

use crate::error::CronResult;
use crate::expression::CronExpression;
use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Job execution function type.
pub type JobFn =
    Arc<dyn Fn(JobContext) -> Pin<Box<dyn Future<Output = CronResult<()>> + Send>> + Send + Sync>;

/// Job execution context.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// Job name
    pub name: String,

    /// Scheduled execution time
    pub scheduled_time: DateTime<Utc>,

    /// Actual execution time
    pub execution_time: DateTime<Utc>,

    /// Execution count (0-based)
    pub execution_count: u64,
}

impl JobContext {
    /// Create a new job context.
    pub fn new(name: String, scheduled_time: DateTime<Utc>, execution_count: u64) -> Self {
        Self {
            name,
            scheduled_time,
            execution_time: Utc::now(),
            execution_count,
        }
    }

    /// Get the delay between scheduled and actual execution time.
    pub fn delay(&self) -> chrono::Duration {
        self.execution_time - self.scheduled_time
    }
}

/// Job status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    /// Job is scheduled and waiting
    Scheduled,

    /// Job is currently running
    Running,

    /// Job completed successfully
    Completed,

    /// Job failed
    Failed(String),
}

/// Scheduled job.
pub struct Job {
    /// Job name
    pub name: String,

    /// Cron expression
    pub expression: CronExpression,

    /// Job function
    pub function: JobFn,

    /// Job status
    pub status: JobStatus,

    /// Next execution time
    pub next_run: Option<DateTime<Utc>>,

    /// Last execution time
    pub last_run: Option<DateTime<Utc>>,

    /// Total execution count
    pub execution_count: u64,

    /// Whether the job is enabled
    pub enabled: bool,

    /// Whether to prevent overlapping executions
    pub prevent_overlap: bool,

    /// Job metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl Job {
    /// Create a new job.
    pub fn new<F, Fut>(name: impl Into<String>, expression: CronExpression, function: F) -> Self
    where
        F: Fn(JobContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = CronResult<()>> + Send + 'static,
    {
        let name = name.into();
        let next_run = expression.next();

        let wrapped_fn = Arc::new(
            move |ctx: JobContext| -> Pin<Box<dyn Future<Output = CronResult<()>> + Send>> {
                Box::pin(function(ctx))
            },
        );

        Self {
            name,
            expression,
            function: wrapped_fn,
            status: JobStatus::Scheduled,
            next_run,
            last_run: None,
            execution_count: 0,
            enabled: true,
            prevent_overlap: true,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Check if the job should run now.
    pub fn should_run(&self) -> bool {
        if !self.enabled {
            return false;
        }

        if self.prevent_overlap && self.status == JobStatus::Running {
            return false;
        }

        if let Some(next_run) = self.next_run {
            Utc::now() >= next_run
        } else {
            false
        }
    }

    /// Execute the job.
    pub async fn execute(&mut self) -> CronResult<()> {
        if !self.enabled {
            return Ok(());
        }

        if self.prevent_overlap && self.status == JobStatus::Running {
            return Ok(());
        }

        self.status = JobStatus::Running;

        let scheduled = self.next_run.unwrap_or_else(Utc::now);

        // Advance the schedule at dispatch, not on completion. Advancing on
        // completion leaves a now-past `next_run` in place for the whole
        // duration of the run — so a long job stays due and is re-dispatched on
        // every tick — and computes the following fire from the completion
        // instant, which drifts the schedule by the execution time.
        self.next_run = self.expression.next_after(scheduled);

        let context = JobContext::new(self.name.clone(), scheduled, self.execution_count);

        let result = (self.function)(context).await;

        self.last_run = Some(Utc::now());
        self.execution_count += 1;

        match result {
            Ok(()) => {
                self.status = JobStatus::Completed;
                Ok(())
            }
            Err(e) => {
                self.status = JobStatus::Failed(e.to_string());
                Err(e)
            }
        }
    }

    /// Enable the job.
    pub fn enable(&mut self) {
        self.enabled = true;
        if self.next_run.is_none() {
            self.next_run = self.expression.next();
        }
    }

    /// Disable the job.
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// Set metadata.
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Get metadata.
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CronExpression;
    use crate::error::CronError;

    #[tokio::test]
    async fn test_job_creation() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let job = Job::new("test", expr, |_ctx| async { Ok(()) });

        assert_eq!(job.name, "test");
        assert_eq!(job.execution_count, 0);
        assert!(job.enabled);
    }

    #[tokio::test]
    async fn test_job_execution() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("test", expr, |_ctx| async { Ok(()) });

        job.next_run = Some(Utc::now()); // Force it to run now

        let result = job.execute().await;
        assert!(result.is_ok());
        assert_eq!(job.execution_count, 1);
        assert_eq!(job.status, JobStatus::Completed);
    }

    #[tokio::test]
    async fn test_job_failure() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("test", expr, |_ctx| async {
            Err(crate::CronError::ExecutionFailed("test error".to_string()))
        });

        job.next_run = Some(Utc::now());

        let result = job.execute().await;
        assert!(result.is_err());
        assert!(matches!(job.status, JobStatus::Failed(_)));
    }

    #[test]
    fn test_job_enable_disable() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("test", expr, |_ctx| async { Ok(()) });

        assert!(job.enabled);

        job.disable();
        assert!(!job.enabled);

        job.enable();
        assert!(job.enabled);
    }

    #[tokio::test]
    async fn test_job_initial_execution_count() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let job = Job::new("test", expr, |_ctx| async { Ok(()) });

        assert_eq!(job.execution_count, 0);
    }

    #[tokio::test]
    async fn test_job_execution_increments_count() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("counter", expr, |_ctx| async { Ok(()) });

        let initial_count = job.execution_count;
        let _ = job.execute().await;
        let new_count = job.execution_count;

        assert_eq!(new_count, initial_count + 1);
    }

    #[tokio::test]
    async fn test_job_success_status() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("success", expr, |_ctx| async { Ok(()) });

        let _ = job.execute().await;
        assert!(matches!(job.status, JobStatus::Completed));
    }

    #[tokio::test]
    async fn test_job_failure_status() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("failure", expr, |_ctx| async {
            Err(CronError::ExecutionFailed("test error".to_string()))
        });

        let _ = job.execute().await;
        assert!(matches!(job.status, JobStatus::Failed(_)));
    }

    #[tokio::test]
    async fn test_job_multiple_executions() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("multi", expr, |_ctx| async { Ok(()) });

        for _ in 0..3 {
            let _ = job.execute().await;
        }

        assert_eq!(job.execution_count, 3);
    }

    #[tokio::test]
    async fn test_job_status_before_execution() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let job = Job::new("test", expr, |_ctx| async { Ok(()) });

        assert!(matches!(job.status, JobStatus::Scheduled));
    }

    #[test]
    fn test_job_creation_with_different_schedules() {
        let schedules = vec![
            "0 * * * * *", // every minute
            "0 0 * * * *", // every hour
            "0 0 0 * * *", // every day
        ];

        for schedule in schedules {
            let expr = CronExpression::parse(schedule).unwrap();
            let job = Job::new("test", expr, |_ctx| async { Ok(()) });
            assert_eq!(job.name, "test");
        }
    }

    #[tokio::test]
    async fn test_job_context_data() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("ctx_test", expr, |ctx| async move {
            assert_eq!(ctx.name, "ctx_test");
            // execution_count is always >= 0 (u64 type)
            Ok(())
        });

        let result = job.execute().await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_job_name_consistency() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let job = Job::new("consistent", expr, |_ctx| async { Ok(()) });

        assert_eq!(job.name, "consistent");
        assert_eq!(job.name, "consistent"); // Multiple reads should be consistent
    }

    #[tokio::test]
    async fn test_job_disabled_flag() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let mut job = Job::new("disabled", expr, |_ctx| async { Ok(()) });

        job.disable();

        assert!(!job.enabled);
    }

    #[tokio::test]
    async fn test_job_mixed_results() {
        let expr = CronExpression::parse("0 * * * * *").unwrap();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter_clone = counter.clone();

        let mut job = Job::new("mixed", expr, move |_ctx| {
            let c = counter_clone.clone();
            async move {
                let count = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count.is_multiple_of(2) {
                    Ok(())
                } else {
                    Err(CronError::ExecutionFailed("odd execution".to_string()))
                }
            }
        });

        for _ in 0..4 {
            let _ = job.execute().await;
        }

        assert_eq!(job.execution_count, 4);
    }

    // The schedule must advance from the scheduled instant at dispatch, not
    // from the completion instant: otherwise every run pushes the following
    // fire out by however long the job took.
    #[tokio::test]
    async fn test_execute_advances_from_scheduled_instant_not_completion() {
        let expr = CronExpression::parse("* * * * * *").unwrap();
        let mut job = Job::new("slow", expr, |_ctx| async {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            Ok(())
        });

        let scheduled = Utc::now();
        job.next_run = Some(scheduled);

        job.execute().await.unwrap();

        let next_run = job.next_run.expect("next_run must be set");
        assert!(
            next_run <= scheduled + chrono::Duration::seconds(1),
            "next_run {next_run} was computed from the completion instant, not the scheduled instant {scheduled}"
        );
    }
}
