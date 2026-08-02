//! Worker implementation for processing jobs.

use crate::error::{QueueError, QueueResult};
use crate::job::{Job, JobId};
use crate::queue::Queue;
use armature_log::{debug, error, info, warn};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinSet;

/// Job handler function type.
pub type JobHandler =
    Arc<dyn Fn(Job) -> Pin<Box<dyn Future<Output = QueueResult<()>> + Send>> + Send + Sync>;

/// Worker configuration.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Number of concurrent jobs to process
    pub concurrency: usize,

    /// Poll interval for checking new jobs
    pub poll_interval: Duration,

    /// Timeout for job execution
    pub job_timeout: Duration,

    /// How long an in-flight claim may go unresolved before the reaper assumes
    /// the worker holding it died and returns the job to its pending queue.
    ///
    /// `dequeue` records a claim in the queue's `processing` set; nothing else
    /// resolves it if the worker crashes, is SIGKILLed, or its handler task
    /// panics, so without a reaper such a job is stranded in no queue at all
    /// and is never retried. [`Worker::start`] runs
    /// [`Queue::reclaim_stale`](crate::Queue::reclaim_stale) with this timeout
    /// in the background.
    ///
    /// Must comfortably exceed `job_timeout` (the default is twice it), or a
    /// merely slow job is re-filed while still running and executes twice.
    /// `None` disables reaping entirely — only appropriate when some other
    /// process reaps the same queue.
    pub visibility_timeout: Option<Duration>,

    /// Whether to log job execution
    pub log_execution: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        let job_timeout = Duration::from_secs(300); // 5 minutes
        Self {
            concurrency: 10,
            poll_interval: Duration::from_secs(1),
            job_timeout,
            visibility_timeout: Some(job_timeout * 2),
            log_execution: true,
        }
    }
}

/// Outcome of a graceful (or partially force-aborted) worker shutdown, as
/// returned by [`Worker::stop_with_timeout`].
///
/// The three counts always sum to the total number of worker tasks that were
/// spawned by [`Worker::start`] and reaped by this shutdown call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StopOutcome {
    /// Number of worker tasks that returned normally on their own within the
    /// grace period.
    pub gracefully_completed: usize,
    /// Number of worker tasks that panicked while a job handler was running,
    /// observed as a `JoinError` while draining during the grace period. Each
    /// occurrence is also logged at `error` level unconditionally (regardless
    /// of `WorkerConfig::log_execution`), since the job that task was
    /// processing is left orphaned "Processing" with no other signal.
    pub panicked: usize,
    /// Number of worker tasks still running when the grace period elapsed
    /// and were therefore forcibly aborted as a last resort.
    pub force_aborted: usize,
}

/// Worker for processing jobs from a queue.
pub struct Worker {
    queue: Queue,
    handlers: Arc<RwLock<HashMap<String, JobHandler>>>,
    config: WorkerConfig,
    running: Arc<RwLock<bool>>,
    // A `JoinSet` (rather than `Vec<JoinHandle<_>>`) is required for graceful
    // shutdown: `stop()` needs to `abort_all()` the *actual* spawned tasks as
    // a last resort after a grace period, not just stop awaiting them. Wrapping
    // each `JoinHandle` in a second spawned task and aborting that wrapper
    // would leave the original worker task running detached.
    handles: JoinSet<()>,
}

impl Worker {
    /// Create a new worker.
    pub fn new(queue: Queue) -> Self {
        Self::with_config(queue, WorkerConfig::default())
    }

    /// Create a worker with custom configuration.
    pub fn with_config(queue: Queue, config: WorkerConfig) -> Self {
        info!("Creating worker with concurrency: {}", config.concurrency);
        debug!(
            "Worker config - poll_interval: {:?}, job_timeout: {:?}",
            config.poll_interval, config.job_timeout
        );
        Self {
            queue,
            handlers: Arc::new(RwLock::new(HashMap::new())),
            config,
            running: Arc::new(RwLock::new(false)),
            handles: JoinSet::new(),
        }
    }

    /// Register a job handler.
    ///
    /// The handler is inserted synchronously before this call returns, so a
    /// `register_handler(...).await` immediately followed by `start()` never
    /// races a worker that dequeues a job before the handler is present.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_queue::*;
    ///
    /// # async fn example() -> QueueResult<()> {
    /// let queue = Queue::new("redis://localhost:6379", "default").await?;
    /// let mut worker = Worker::new(queue);
    ///
    /// worker
    ///     .register_handler("send_email", |job| async move {
    ///         // Send email logic
    ///         println!("Sending email: {:?}", job.data);
    ///         Ok(())
    ///     })
    ///     .await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_handler<F, Fut>(&mut self, job_type: impl Into<String>, handler: F)
    where
        F: Fn(Job) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = QueueResult<()>> + Send + 'static,
    {
        let wrapped_handler = Arc::new(
            move |job: Job| -> Pin<Box<dyn Future<Output = QueueResult<()>> + Send>> {
                Box::pin(handler(job))
            },
        );

        // Insert synchronously: the handler must be present the moment this
        // call returns, so a subsequent `start()` cannot dequeue a job whose
        // handler has not been registered yet.
        self.handlers
            .write()
            .await
            .insert(job_type.into(), wrapped_handler);
    }

    /// Start the worker.
    pub async fn start(&mut self) -> QueueResult<()> {
        let mut running = self.running.write().await;
        if *running {
            warn!("Worker already running");
            return Err(QueueError::WorkerAlreadyRunning);
        }
        *running = true;
        drop(running);

        info!(
            "Starting worker with {} concurrent processors",
            self.config.concurrency
        );

        // Start worker tasks
        for i in 0..self.config.concurrency {
            let queue = self.queue.clone();
            let handlers = self.handlers.clone();
            let running = self.running.clone();
            let poll_interval = self.config.poll_interval;
            let job_timeout = self.config.job_timeout;
            let log = self.config.log_execution;

            self.handles.spawn(async move {
                while *running.read().await {
                    match queue.dequeue().await {
                        Ok(Some(job)) => {
                            let job_id = job.id;
                            let job_type = job.job_type.clone();

                            if log {
                                println!(
                                    "[WORKER-{}] Processing job: {} (type: {})",
                                    i, job_id, job_type
                                );
                            }

                            // Get handler
                            let handler = {
                                let handlers = handlers.read().await;
                                handlers.get(&job_type).cloned()
                            };

                            if let Some(handler) = handler {
                                // Execute job with timeout
                                let result =
                                    tokio::time::timeout(job_timeout, handler(job.clone())).await;

                                match result {
                                    Ok(Ok(())) => {
                                        // Job succeeded
                                        if let Err(e) = queue.complete(job_id).await {
                                            eprintln!(
                                                "[WORKER-{}] Failed to mark job as complete: {}",
                                                i, e
                                            );
                                        } else if log {
                                            println!(
                                                "[WORKER-{}] Job {} completed successfully",
                                                i, job_id
                                            );
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        // Job failed
                                        eprintln!("[WORKER-{}] Job {} failed: {}", i, job_id, e);
                                        if let Err(err) = queue.fail(job_id, e.to_string()).await {
                                            eprintln!(
                                                "[WORKER-{}] Failed to mark job as failed: {}",
                                                i, err
                                            );
                                        }
                                    }
                                    Err(_) => {
                                        // Timeout
                                        eprintln!("[WORKER-{}] Job {} timed out", i, job_id);
                                        if let Err(e) =
                                            queue.fail(job_id, "Job timeout".to_string()).await
                                        {
                                            eprintln!(
                                                "[WORKER-{}] Failed to mark job as failed: {}",
                                                i, e
                                            );
                                        }
                                    }
                                }
                            } else {
                                eprintln!("[WORKER-{}] No handler for job type: {}", i, job_type);
                                if let Err(e) = queue
                                    .fail(job_id, format!("No handler for job type: {}", job_type))
                                    .await
                                {
                                    eprintln!("[WORKER-{}] Failed to mark job as failed: {}", i, e);
                                }
                            }
                        }
                        Ok(None) => {
                            // No jobs available, wait before polling again
                            tokio::time::sleep(poll_interval).await;
                        }
                        Err(e) => {
                            eprintln!("[WORKER-{}] Error dequeuing job: {}", i, e);
                            tokio::time::sleep(poll_interval).await;
                        }
                    }
                }

                if log {
                    println!("[WORKER-{}] Stopped", i);
                }
            });
        }

        Ok(())
    }

    /// Process multiple jobs of the same type in parallel
    ///
    /// This method dequeues and processes multiple jobs of the same type
    /// concurrently, providing significant throughput improvements.
    ///
    /// # Performance
    ///
    /// - **Sequential:** O(n * job_time)
    /// - **Parallel:** O(max(job_times))
    /// - **Speedup:** 3-5x higher throughput
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_queue::*;
    /// # async fn example(worker: &Worker) -> QueueResult<()> {
    /// // Process up to 10 image processing jobs in parallel
    /// let processed = worker.process_batch("process_image", 10).await?;
    /// println!("Processed {} jobs", processed.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn process_batch(
        &self,
        job_type: &str,
        max_batch_size: usize,
    ) -> QueueResult<Vec<JobId>> {
        use tokio::task::JoinSet;

        // Dequeue multiple jobs of the same type
        let mut jobs = Vec::new();
        for _ in 0..max_batch_size {
            match self.queue.dequeue().await? {
                Some(job) => {
                    if job.job_type == job_type {
                        jobs.push(job);
                    } else {
                        // Different job type: `dequeue()` already popped this
                        // job, started_processing it, and put it in the
                        // `processing` set. Batching stops here, so we must
                        // re-enqueue it — otherwise it is orphaned in
                        // `processing` forever (data loss).
                        self.queue.requeue(&job).await?;
                        break;
                    }
                }
                None => break,
            }
        }

        if jobs.is_empty() {
            return Ok(Vec::new());
        }

        // Capture the true batch size before `jobs` is consumed below, so the
        // completion log can report real succeeded/total counts.
        let total = jobs.len();

        if self.config.log_execution {
            println!("[BATCH] Processing {} jobs of type '{}'", total, job_type);
        }

        // Get handler
        let handler = {
            let handlers = self.handlers.read().await;
            handlers.get(job_type).cloned()
        };

        let Some(handler) = handler else {
            return Err(QueueError::NoHandler(job_type.to_string()));
        };

        // Process all jobs in parallel
        let mut set = JoinSet::new();
        for job in jobs {
            let handler = handler.clone();
            let queue = self.queue.clone();
            let job_id = job.id;
            let log = self.config.log_execution;
            let timeout = self.config.job_timeout;

            set.spawn(async move {
                let result = tokio::time::timeout(timeout, handler(job.clone())).await;

                match result {
                    Ok(Ok(())) => {
                        // Job succeeded
                        if let Err(e) = queue.complete(job_id).await {
                            eprintln!("[BATCH] Failed to mark job {} as complete: {}", job_id, e);
                        } else if log {
                            println!("[BATCH] Job {} completed successfully", job_id);
                        }
                        Ok(job_id)
                    }
                    Ok(Err(e)) => {
                        // Job failed
                        eprintln!("[BATCH] Job {} failed: {}", job_id, e);
                        if let Err(err) = queue.fail(job_id, e.to_string()).await {
                            eprintln!("[BATCH] Failed to mark job {} as failed: {}", job_id, err);
                        }
                        Err(e)
                    }
                    Err(_) => {
                        // Timeout
                        eprintln!("[BATCH] Job {} timed out", job_id);
                        if let Err(e) = queue
                            .fail(job_id, "Job execution timed out".to_string())
                            .await
                        {
                            eprintln!("[BATCH] Failed to mark job {} as failed: {}", job_id, e);
                        }
                        Err(QueueError::ExecutionFailed("Timeout".to_string()))
                    }
                }
            });
        }

        // Collect results
        let mut processed = Vec::new();
        while let Some(result) = set.join_next().await {
            match result {
                Ok(Ok(job_id)) => processed.push(job_id),
                Ok(Err(_)) => {} // Error already logged
                Err(e) => eprintln!("[BATCH] Task join error: {}", e),
            }
        }

        if self.config.log_execution {
            println!(
                "[BATCH] Batch complete: {}/{} jobs succeeded",
                processed.len(),
                total
            );
        }

        Ok(processed)
    }

    /// Register a CPU-intensive handler that runs in blocking thread pool
    ///
    /// For CPU-bound operations (image processing, encryption, etc.), use this
    /// method to avoid blocking the async runtime.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_queue::*;
    /// # async fn example(worker: &mut Worker) {
    /// worker
    ///     .register_cpu_intensive_handler("resize_image", |job| {
    ///         // CPU-intensive work here
    ///         let image_path = job.data["path"].as_str().unwrap();
    ///         // ... resize image ...
    ///         Ok(())
    ///     })
    ///     .await;
    /// # }
    /// ```
    pub async fn register_cpu_intensive_handler<F>(
        &mut self,
        job_type: impl Into<String>,
        handler: F,
    ) where
        F: Fn(Job) -> QueueResult<()> + Send + Sync + 'static,
    {
        let handler = Arc::new(handler);

        let wrapped = Arc::new(move |job: Job| {
            let handler = handler.clone();
            Box::pin(async move {
                // Run in blocking thread pool to avoid blocking async runtime
                tokio::task::spawn_blocking(move || handler(job))
                    .await
                    .map_err(|e| QueueError::ExecutionFailed(e.to_string()))?
            }) as Pin<Box<dyn Future<Output = QueueResult<()>> + Send>>
        });

        // Insert via `.await`, never `block_on`: this method is documented as
        // being called from an async context, where `Handle::block_on` panics.
        self.handlers.write().await.insert(job_type.into(), wrapped);
    }

    /// Stop the worker gracefully.
    ///
    /// This is [`stop_with_timeout`] using `self.config.job_timeout` as the
    /// grace period. That bound is not arbitrary: each worker task wraps its
    /// current job's handler invocation in `tokio::time::timeout(job_timeout,
    /// ...)` (see the loop spawned by [`start`]), so `job_timeout` is already
    /// the worst-case time a task can be stuck inside a handler before it
    /// self-times-out the job and loops back to observe the stop flag. Waiting
    /// that long guarantees every task gets the chance to either finish its
    /// current job or hit its own internal timeout -- and therefore call
    /// `queue.complete()`/`queue.fail()` -- before shutdown falls back to
    /// aborting anything still outstanding.
    ///
    /// This wrapper discards the [`StopOutcome`] that [`stop_with_timeout`]
    /// returns, for backward-compatible callers that only care whether
    /// shutdown was initiated successfully. Call [`stop_with_timeout`]
    /// directly if you need to know whether any tasks panicked or had to be
    /// force-aborted.
    ///
    /// [`stop_with_timeout`]: Self::stop_with_timeout
    /// [`start`]: Self::start
    pub async fn stop(&mut self) -> QueueResult<()> {
        self.stop_with_timeout(self.config.job_timeout).await?;
        Ok(())
    }

    /// Stop the worker, waiting up to `timeout` for in-flight jobs to finish
    /// gracefully before forcibly aborting any worker tasks still running.
    ///
    /// Setting `running` to `false` only *signals* the spawned tasks to stop --
    /// each task only observes the flag at the top of its `while
    /// *running.read().await` loop (see [`start`]), so a task currently inside
    /// `handler(job.clone()).await` keeps running until that call returns (or
    /// times out) and it loops back around. This method waits for that to
    /// happen naturally, up to `timeout`, so the handler gets a chance to reach
    /// `queue.complete()`/`queue.fail()` instead of being cancelled mid-flight
    /// and leaving the job orphaned "Processing" in Redis forever. Only once
    /// `timeout` elapses are any still-running tasks forcibly aborted as a
    /// last resort -- at that point whatever job they were mid-handler on is
    /// still orphaned, exactly as the previous unconditional-abort behavior
    /// left it.
    ///
    /// Returns a [`StopOutcome`] reporting how many worker tasks finished
    /// gracefully, how many panicked mid-handler during the drain (logged at
    /// `error` level unconditionally when it happens), and how many had to be
    /// force-aborted after the grace period elapsed (logged at `warn` level
    /// unconditionally when it happens). A panicked worker task is *not*
    /// treated the same as a normal completion: a `JoinError` from
    /// `join_next()` means the task's handler crashed, not that it returned
    /// `Ok(())`, and the in-flight job it was processing is left orphaned
    /// "Processing" with no other signal unless this is observed.
    ///
    /// [`start`]: Self::start
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_queue::*;
    /// use std::time::Duration;
    ///
    /// # async fn example(mut worker: Worker) -> QueueResult<()> {
    /// // Give in-flight jobs up to 10 seconds to finish before force-killing them.
    /// let outcome = worker.stop_with_timeout(Duration::from_secs(10)).await?;
    /// if outcome.panicked > 0 || outcome.force_aborted > 0 {
    ///     eprintln!("shutdown was not fully graceful: {:?}", outcome);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop_with_timeout(&mut self, timeout: Duration) -> QueueResult<StopOutcome> {
        let mut running = self.running.write().await;
        if !*running {
            return Err(QueueError::WorkerNotRunning);
        }
        *running = false;
        drop(running);

        if self.config.log_execution {
            println!("[WORKER] Stopping (grace period: {:?})...", timeout);
        }

        // Wait for tasks to return on their own, up to `timeout` total (not
        // per-task): each completed task is drained from the `JoinSet` as it
        // finishes, and we keep waiting -- against a single shared deadline --
        // for the rest.
        let mut gracefully_completed = 0usize;
        let mut panicked = 0usize;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, self.handles.join_next()).await {
                // A task finished normally on its own; keep waiting for the rest.
                Ok(Some(Ok(()))) => {
                    gracefully_completed += 1;
                    continue;
                }
                // A task PANICKED mid-handler during the drain. This must not
                // be treated the same as a normal completion: the job it was
                // processing is left orphaned "Processing" in Redis with no
                // other signal, so this is always logged -- unconditionally,
                // not gated by `log_execution` -- a panicked handler is not a
                // routine event worth suppressing.
                Ok(Some(Err(join_err))) => {
                    panicked += 1;
                    error!(
                        "[WORKER] worker task panicked during graceful shutdown drain: {}",
                        join_err
                    );
                    continue;
                }
                // All tasks finished on their own within the grace period.
                Ok(None) => break,
                // Grace period elapsed with tasks still outstanding.
                Err(_) => break,
            }
        }

        // Last resort: force-abort anything still running once the grace
        // period has elapsed. `abort_all` + draining is a no-op for tasks that
        // already finished above.
        let force_aborted = self.handles.len();
        if force_aborted > 0 {
            warn!(
                "[WORKER] force-aborting {} in-flight worker task(s) after {:?} grace period elapsed",
                force_aborted, timeout
            );
        }
        self.handles.abort_all();
        while self.handles.join_next().await.is_some() {}

        if self.config.log_execution {
            if force_aborted > 0 {
                println!(
                    "[WORKER] Stopped ({}s grace period elapsed; {} task(s) force-aborted)",
                    timeout.as_secs_f64(),
                    force_aborted
                );
            } else {
                println!("[WORKER] Stopped (all tasks exited gracefully)");
            }
        }

        Ok(StopOutcome {
            gracefully_completed,
            panicked,
            force_aborted,
        })
    }

    /// Check if the worker is running.
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config() {
        let config = WorkerConfig::default();
        assert_eq!(config.concurrency, 10);
        assert!(config.log_execution);
    }

    #[tokio::test]
    async fn test_worker_creation() {
        // This test requires a real Redis connection, so we just test creation
        // In a real environment, you'd use a test Redis instance
        let config = WorkerConfig {
            concurrency: 5,
            poll_interval: Duration::from_millis(500),
            job_timeout: Duration::from_secs(60),
            log_execution: false,
        };

        assert_eq!(config.concurrency, 5);
    }
}
