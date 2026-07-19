//! Cron job scheduler.

use crate::error::{CronError, CronResult};
use crate::expression::CronExpression;
use crate::job::{Job, JobContext, JobFn, JobStatus};
use armature_log::{debug, error, info, warn};
use chrono::Utc;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinHandle;

/// Scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Tick interval for checking scheduled jobs
    pub tick_interval: Duration,

    /// Whether to run missed jobs on startup.
    ///
    /// When `true`, jobs whose next scheduled fire time has already elapsed when
    /// the scheduler starts are run immediately (catch-up). When `false`, such
    /// past-due fire times are skipped and the job is advanced to its next
    /// future occurrence.
    pub run_missed_jobs: bool,

    /// Maximum concurrent jobs
    pub max_concurrent_jobs: usize,

    /// Whether to log job execution
    pub log_execution: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(1),
            run_missed_jobs: false,
            max_concurrent_jobs: 10,
            log_execution: true,
        }
    }
}

/// Cron job scheduler.
pub struct CronScheduler {
    jobs: Arc<RwLock<HashMap<String, Job>>>,
    config: SchedulerConfig,
    running: Arc<RwLock<bool>>,
    handle: Option<JoinHandle<()>>,
}

impl CronScheduler {
    /// Create a new scheduler with default configuration.
    pub fn new() -> Self {
        Self::with_config(SchedulerConfig::default())
    }

    /// Create a new scheduler with custom configuration.
    pub fn with_config(config: SchedulerConfig) -> Self {
        info!("Initializing cron scheduler");
        debug!(
            "Scheduler config - tick_interval: {:?}, max_concurrent: {}",
            config.tick_interval, config.max_concurrent_jobs
        );
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            config,
            running: Arc::new(RwLock::new(false)),
            handle: None,
        }
    }

    /// Add a job to the scheduler.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cron::*;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), CronError> {
    /// let mut scheduler = CronScheduler::new();
    ///
    /// scheduler.add_job(
    ///     "cleanup",
    ///     "0 0 0 * * *", // Every day at midnight
    ///     |ctx| Box::pin(async move {
    ///         println!("Running cleanup job");
    ///         Ok(())
    ///     })
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Returns [`CronError::JobAlreadyExists`] if a job with the same name is
    /// already registered. The job is registered synchronously before this
    /// returns.
    pub async fn add_job<F, Fut>(
        &mut self,
        name: impl Into<String>,
        expression: &str,
        function: F,
    ) -> CronResult<()>
    where
        F: Fn(JobContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = CronResult<()>> + Send + 'static,
    {
        let name = name.into();
        let expr = CronExpression::parse(expression)?;
        info!("Adding cron job '{}' with schedule '{}'", name, expression);

        let mut jobs = self.jobs.write().await;
        if jobs.contains_key(&name) {
            warn!("Job '{}' already exists", name);
            return Err(CronError::JobAlreadyExists(name));
        }

        let job = Job::new(name.clone(), expr, function);
        jobs.insert(name.clone(), job);
        debug!("Job '{}' registered successfully", name);

        Ok(())
    }

    /// Remove a job from the scheduler.
    pub async fn remove_job(&mut self, name: &str) -> CronResult<()> {
        let mut jobs = self.jobs.write().await;
        jobs.remove(name)
            .ok_or_else(|| CronError::JobNotFound(name.to_string()))?;
        Ok(())
    }

    /// Get a list of all job names.
    pub async fn list_jobs(&self) -> Vec<String> {
        let jobs = self.jobs.read().await;
        jobs.keys().cloned().collect()
    }

    /// Enable a job.
    pub async fn enable_job(&self, name: &str) -> CronResult<()> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(name)
            .ok_or_else(|| CronError::JobNotFound(name.to_string()))?;
        job.enable();
        Ok(())
    }

    /// Disable a job.
    pub async fn disable_job(&self, name: &str) -> CronResult<()> {
        let mut jobs = self.jobs.write().await;
        let job = jobs
            .get_mut(name)
            .ok_or_else(|| CronError::JobNotFound(name.to_string()))?;
        job.disable();
        Ok(())
    }

    /// Start the scheduler.
    pub async fn start(&mut self) -> CronResult<()> {
        let mut running = self.running.write().await;
        if *running {
            warn!("Cron scheduler already running");
            return Err(CronError::SchedulerAlreadyRunning);
        }
        *running = true;
        drop(running);

        info!("Cron scheduler started");

        let jobs = self.jobs.clone();
        let running = self.running.clone();
        let tick_interval = self.config.tick_interval;
        let log_execution = self.config.log_execution;
        let run_missed_jobs = self.config.run_missed_jobs;
        // Cap simultaneous job executions. Guard against a zero permit count,
        // which would otherwise deadlock every job.
        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_jobs.max(1)));

        let handle = tokio::spawn(async move {
            // Startup catch-up policy: when we are *not* running missed jobs,
            // advance any already-past fire time to the next future occurrence so
            // it is skipped instead of firing immediately.
            if !run_missed_jobs {
                let now = Utc::now();
                let mut jobs = jobs.write().await;
                for job in jobs.values_mut() {
                    if let Some(next_run) = job.next_run
                        && next_run < now
                    {
                        job.next_run = job.expression.next_after(now);
                    }
                }
            }

            while *running.read().await {
                // Take the jobs map write lock ONCE per tick and decide which
                // jobs are due while holding it, instead of spawning a task
                // (and acquiring a concurrency permit) per registered job on
                // every tick. Jobs that are due are flipped to `Running`
                // under this single lock, which preserves the overlap-skip
                // guarantee, and only those due jobs are handed off for
                // execution.
                let due: Vec<(String, JobFn, JobContext)> = {
                    let mut jobs = jobs.write().await;
                    let mut due = Vec::new();
                    for job in jobs.values_mut() {
                        if !job.should_run() {
                            continue;
                        }
                        job.status = JobStatus::Running;
                        let context = JobContext::new(
                            job.name.clone(),
                            job.next_run.unwrap_or_else(Utc::now),
                            job.execution_count,
                        );
                        due.push((job.name.clone(), job.function.clone(), context));
                    }
                    due
                };

                for (name, job_fn, context) in due {
                    let jobs_clone = jobs.clone();
                    let log = log_execution;
                    let semaphore = semaphore.clone();

                    tokio::spawn(async move {
                        // Bound concurrency. Held for the whole execution. The
                        // permit is acquired here, inside the spawned task for
                        // an already-due job, rather than before `should_run()`
                        // was checked.
                        let _permit = match semaphore.acquire_owned().await {
                            Ok(permit) => permit,
                            Err(_) => return, // semaphore closed
                        };

                        if log {
                            println!("[CRON] Executing job: {}", name);
                        }

                        // Run the job body on its own task so that if it
                        // panics, the panic unwinds *that* task and is
                        // reported as a `JoinError` here rather than
                        // unwinding this task and skipping the
                        // outcome-recording block below. Without this, a
                        // panicking job would be left stuck in `Running`
                        // forever, and (with `prevent_overlap`, the default)
                        // would never be scheduled again.
                        let outcome = tokio::spawn(job_fn(context)).await;

                        let result: CronResult<()> = match outcome {
                            Ok(inner) => inner,
                            Err(join_err) if join_err.is_panic() => {
                                error!("Job '{}' panicked during execution", name);
                                Err(CronError::ExecutionFailed(format!(
                                    "job '{}' panicked during execution",
                                    name
                                )))
                            }
                            Err(join_err) => {
                                error!("Job '{}' task failed: {}", name, join_err);
                                Err(CronError::ExecutionFailed(format!(
                                    "job '{}' task failed: {}",
                                    name, join_err
                                )))
                            }
                        };

                        // Re-acquire briefly to record the outcome. This runs
                        // even when the job body panicked, since the panic
                        // was contained to the nested task above.
                        {
                            let mut jobs = jobs_clone.write().await;
                            if let Some(job) = jobs.get_mut(&name) {
                                job.last_run = Some(Utc::now());
                                job.execution_count += 1;
                                job.next_run = job.expression.next();
                                job.status = match &result {
                                    Ok(()) => JobStatus::Completed,
                                    Err(e) => JobStatus::Failed(e.to_string()),
                                };
                            }
                        }

                        match result {
                            Ok(()) => {
                                if log {
                                    println!("[CRON] Job {} completed successfully", name);
                                }
                            }
                            Err(e) => eprintln!("[CRON] Job {} failed: {}", name, e),
                        }
                    });
                }

                tokio::time::sleep(tick_interval).await;
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    /// Stop the scheduler.
    pub async fn stop(&mut self) -> CronResult<()> {
        let mut running = self.running.write().await;
        if !*running {
            return Err(CronError::SchedulerNotRunning);
        }
        *running = false;
        drop(running);

        if let Some(handle) = self.handle.take() {
            handle.abort();
        }

        Ok(())
    }

    /// Check if the scheduler is running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Get job statistics.
    pub async fn get_stats(&self, name: &str) -> CronResult<JobStats> {
        let jobs = self.jobs.read().await;
        let job = jobs
            .get(name)
            .ok_or_else(|| CronError::JobNotFound(name.to_string()))?;

        Ok(JobStats {
            name: job.name.clone(),
            enabled: job.enabled,
            execution_count: job.execution_count,
            last_run: job.last_run,
            next_run: job.next_run,
            status: job.status.clone(),
        })
    }
}

impl Default for CronScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Job statistics.
#[derive(Debug, Clone)]
pub struct JobStats {
    pub name: String,
    pub enabled: bool,
    pub execution_count: u64,
    pub last_run: Option<chrono::DateTime<chrono::Utc>>,
    pub next_run: Option<chrono::DateTime<chrono::Utc>>,
    pub status: crate::job::JobStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_scheduler_creation() {
        let scheduler = CronScheduler::new();
        assert!(!scheduler.is_running().await);
    }

    #[tokio::test]
    async fn test_add_job() {
        let mut scheduler = CronScheduler::new();
        let result = scheduler
            .add_job("test", "0 * * * * *", |_| async { Ok(()) })
            .await;
        assert!(result.is_ok());

        // Registration is synchronous: no sleep required.
        let jobs = scheduler.list_jobs().await;
        assert!(jobs.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_add_duplicate_job_returns_error() {
        let mut scheduler = CronScheduler::new();
        scheduler
            .add_job("dup", "0 * * * * *", |_| async { Ok(()) })
            .await
            .unwrap();

        let result = scheduler
            .add_job("dup", "0 0 * * * *", |_| async { Ok(()) })
            .await;

        assert!(
            matches!(&result, Err(CronError::JobAlreadyExists(name)) if name == "dup"),
            "duplicate name must return JobAlreadyExists, got {result:?}"
        );

        // The original job is still present exactly once.
        let jobs = scheduler.list_jobs().await;
        assert_eq!(jobs.iter().filter(|n| *n == "dup").count(), 1);
    }

    #[tokio::test]
    async fn test_remove_job() {
        let mut scheduler = CronScheduler::new();
        scheduler
            .add_job("test", "0 * * * * *", |_| async { Ok(()) })
            .await
            .unwrap();

        let result = scheduler.remove_job("test").await;
        assert!(result.is_ok());

        let jobs = scheduler.list_jobs().await;
        assert!(!jobs.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_start_stop() {
        let mut scheduler = CronScheduler::new();

        assert!(!scheduler.is_running().await);

        scheduler.start().await.unwrap();
        assert!(scheduler.is_running().await);

        scheduler.stop().await.unwrap();
        assert!(!scheduler.is_running().await);
    }

    // Two distinct jobs firing on the same second must run concurrently rather
    // than being serialized by a global lock held across `execute()`.
    #[tokio::test]
    async fn test_jobs_run_concurrently_not_serialized() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let inflight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let make = |inflight: Arc<AtomicUsize>, max_seen: Arc<AtomicUsize>| {
            move |_ctx| {
                let inflight = inflight.clone();
                let max_seen = max_seen.clone();
                async move {
                    let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(cur, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        };

        let mut scheduler = CronScheduler::with_config(SchedulerConfig {
            tick_interval: Duration::from_millis(50),
            log_execution: false,
            ..Default::default()
        });
        scheduler
            .add_job("a", "* * * * * *", make(inflight.clone(), max_seen.clone()))
            .await
            .unwrap();
        scheduler
            .add_job("b", "* * * * * *", make(inflight.clone(), max_seen.clone()))
            .await
            .unwrap();

        scheduler.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        scheduler.stop().await.unwrap();

        assert!(
            max_seen.load(Ordering::SeqCst) >= 2,
            "jobs should run concurrently; observed max concurrency was {}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    // `max_concurrent_jobs` must actually bound simultaneous executions.
    #[tokio::test]
    async fn test_max_concurrent_jobs_enforced() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let inflight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let make = |inflight: Arc<AtomicUsize>, max_seen: Arc<AtomicUsize>| {
            move |_ctx| {
                let inflight = inflight.clone();
                let max_seen = max_seen.clone();
                async move {
                    let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(cur, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        };

        let mut scheduler = CronScheduler::with_config(SchedulerConfig {
            tick_interval: Duration::from_millis(50),
            max_concurrent_jobs: 1,
            log_execution: false,
            ..Default::default()
        });
        scheduler
            .add_job("a", "* * * * * *", make(inflight.clone(), max_seen.clone()))
            .await
            .unwrap();
        scheduler
            .add_job("b", "* * * * * *", make(inflight.clone(), max_seen.clone()))
            .await
            .unwrap();

        scheduler.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        scheduler.stop().await.unwrap();

        assert_eq!(
            max_seen.load(Ordering::SeqCst),
            1,
            "max_concurrent_jobs = 1 must serialize executions"
        );
    }

    // With run_missed_jobs = true, a fire time that elapsed while stopped is
    // caught up on the first tick, even with a long tick interval.
    #[tokio::test]
    async fn test_run_missed_jobs_catches_up() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();

        let mut scheduler = CronScheduler::with_config(SchedulerConfig {
            tick_interval: Duration::from_secs(30),
            run_missed_jobs: true,
            log_execution: false,
            ..Default::default()
        });
        scheduler
            .add_job("m", "* * * * * *", move |_ctx| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await
            .unwrap();

        // Let next_run elapse while the scheduler is stopped.
        tokio::time::sleep(Duration::from_millis(1500)).await;

        scheduler.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(400)).await;
        let observed = count.load(Ordering::SeqCst);
        scheduler.stop().await.unwrap();

        assert!(
            observed >= 1,
            "missed fire should be caught up on startup, observed {observed}"
        );
    }

    // With run_missed_jobs = false, an elapsed fire time is skipped: the job is
    // advanced to its next future occurrence and does not fire immediately.
    #[tokio::test]
    async fn test_run_missed_jobs_disabled_skips_past_due() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();

        let mut scheduler = CronScheduler::with_config(SchedulerConfig {
            tick_interval: Duration::from_secs(30),
            run_missed_jobs: false,
            log_execution: false,
            ..Default::default()
        });
        scheduler
            .add_job("m", "* * * * * *", move |_ctx| {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .await
            .unwrap();

        // Let next_run elapse while stopped.
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Align close to a whole-second boundary so the freshly-advanced
        // next_run (computed inside `start()`) lands comfortably in the
        // future, leaving a wide safety margin to confirm nothing fired
        // before it does. A looser alignment window combined with ordinary
        // scheduling/CI jitter could previously shrink this margin enough to
        // flake the `== 0` assertion below; aligning earlier in the second
        // and checking sooner after `start()` keeps a wide buffer in both
        // directions without weakening what the assertion proves.
        loop {
            if Utc::now().timestamp_subsec_millis() < 20 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        scheduler.start().await.unwrap();
        // Checked well before the ~1s-out next_run, so ordinary scheduling
        // latency cannot cause a false failure; a genuine regression that
        // fires the job immediately on start would still be caught.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let observed = count.load(Ordering::SeqCst);
        scheduler.stop().await.unwrap();

        assert_eq!(
            observed, 0,
            "past-due fire must be skipped when run_missed_jobs is false"
        );
    }

    // A panicking job must not wedge the schedule: the outcome-recording
    // step must still run even though the job body unwound, leaving the job
    // in a terminal status (not stuck `Running`, which combined with the
    // default `prevent_overlap` would otherwise mean it can never run
    // again) and advanced to a future `next_run`.
    #[tokio::test]
    async fn test_panicking_job_does_not_wedge_schedule() {
        let mut scheduler = CronScheduler::with_config(SchedulerConfig {
            tick_interval: Duration::from_millis(50),
            log_execution: false,
            ..Default::default()
        });
        scheduler
            .add_job("boom", "* * * * * *", |_ctx| async {
                panic!("job intentionally panics");
            })
            .await
            .unwrap();

        scheduler.start().await.unwrap();
        // Allow at least one full second (the job's schedule granularity)
        // plus margin for the panic to be caught and the outcome recorded.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        scheduler.stop().await.unwrap();

        let stats = scheduler.get_stats("boom").await.unwrap();
        assert_ne!(
            stats.status,
            JobStatus::Running,
            "a panicking job must not be left stuck in Running; got {:?}",
            stats.status
        );
        assert!(
            stats.next_run.is_some(),
            "a panicking job must still be advanced to its next scheduled run"
        );
        assert!(
            stats.execution_count >= 1,
            "the panicking execution must still be recorded"
        );
    }
}
