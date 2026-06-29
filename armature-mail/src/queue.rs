//! Async email queue for non-blocking email sending.
//!
//! The email queue allows you to enqueue emails for background processing,
//! with automatic retries, persistence, and dead letter handling.
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_mail::{EmailQueue, EmailQueueConfig, Email, Mailer};
//! use armature_redis::RedisService;
//!
//! // Create queue with Redis backend
//! let queue = EmailQueue::new(redis_service, EmailQueueConfig::default());
//!
//! // Enqueue an email (returns immediately)
//! let email = Email::new()
//!     .to("user@example.com")
//!     .subject("Hello!")
//!     .text("This email will be sent asynchronously.");
//!
//! queue.enqueue(email).await?;
//!
//! // Start the queue worker (in a separate task)
//! let worker = queue.worker(mailer);
//! tokio::spawn(worker.run());
//! ```

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{Email, MailError, Mailer, Result};

/// Email job stored in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailJob {
    /// Unique job ID.
    pub id: String,
    /// The email to send.
    pub email: Email,
    /// Number of attempts made.
    pub attempts: u32,
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Created timestamp (Unix ms).
    pub created_at: i64,
    /// Next retry timestamp (Unix ms).
    pub next_retry_at: Option<i64>,
    /// Last error message.
    pub last_error: Option<String>,
    /// Priority (lower = higher priority).
    pub priority: u8,
    /// Optional metadata.
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

impl EmailJob {
    /// Create a new email job.
    pub fn new(email: Email) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            email,
            attempts: 0,
            max_retries: 3,
            created_at: chrono_now_ms(),
            next_retry_at: None,
            last_error: None,
            priority: 5,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set the maximum retries.
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Set the priority (0 = highest, 255 = lowest).
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Add metadata.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Check if the job should be retried.
    pub fn should_retry(&self) -> bool {
        self.attempts < self.max_retries
    }

    /// Increment attempts and calculate next retry time.
    pub fn prepare_retry(&mut self, delay: Duration) {
        self.attempts += 1;
        self.next_retry_at = Some(chrono_now_ms() + delay.as_millis() as i64);
    }
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Email queue configuration.
#[derive(Debug, Clone)]
pub struct EmailQueueConfig {
    /// Queue name/key prefix.
    pub queue_name: String,
    /// Worker concurrency.
    pub concurrency: usize,
    /// Batch size for fetching jobs.
    pub batch_size: usize,
    /// Poll interval when queue is empty.
    pub poll_interval: Duration,
    /// Initial retry delay (exponential backoff).
    pub retry_delay: Duration,
    /// Maximum retry delay.
    pub max_retry_delay: Duration,
    /// Dead letter queue enabled.
    pub dead_letter_queue: bool,
    /// Job timeout.
    pub job_timeout: Duration,
}

impl Default for EmailQueueConfig {
    fn default() -> Self {
        Self {
            queue_name: "armature:email:queue".to_string(),
            concurrency: 4,
            batch_size: 10,
            poll_interval: Duration::from_secs(1),
            retry_delay: Duration::from_secs(5),
            max_retry_delay: Duration::from_secs(300),
            dead_letter_queue: true,
            job_timeout: Duration::from_secs(60),
        }
    }
}

impl EmailQueueConfig {
    /// Set the queue name.
    pub fn queue_name(mut self, name: impl Into<String>) -> Self {
        self.queue_name = name.into();
        self
    }

    /// Set the concurrency.
    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Set the batch size.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the poll interval.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Set the initial retry delay.
    pub fn retry_delay(mut self, delay: Duration) -> Self {
        self.retry_delay = delay;
        self
    }

    /// Enable/disable dead letter queue.
    pub fn dead_letter_queue(mut self, enabled: bool) -> Self {
        self.dead_letter_queue = enabled;
        self
    }
}

/// Email queue backend trait.
#[async_trait::async_trait]
pub trait EmailQueueBackend: Send + Sync {
    /// Push a job to the queue.
    async fn push(&self, job: EmailJob) -> Result<()>;

    /// Pop jobs from the queue.
    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>>;

    /// Mark a job as complete.
    async fn complete(&self, job_id: &str) -> Result<()>;

    /// Mark a job as failed and schedule retry.
    async fn fail(&self, job: EmailJob, error: &str) -> Result<()>;

    /// Move a job to the dead letter queue.
    async fn dead_letter(&self, job: EmailJob) -> Result<()>;

    /// Get queue statistics.
    async fn stats(&self) -> Result<QueueStats>;
}

/// Queue statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    /// Pending jobs.
    pub pending: u64,
    /// Processing jobs.
    pub processing: u64,
    /// Failed jobs (in retry).
    pub retrying: u64,
    /// Dead letter jobs.
    pub dead_letter: u64,
    /// Total processed.
    pub processed: u64,
}

/// In-memory email queue backend (for testing/development).
pub struct InMemoryBackend {
    queue: tokio::sync::Mutex<std::collections::VecDeque<EmailJob>>,
    dead_letter: tokio::sync::Mutex<Vec<EmailJob>>,
    processed: std::sync::atomic::AtomicU64,
}

impl InMemoryBackend {
    /// Create a new in-memory backend.
    pub fn new() -> Self {
        Self {
            queue: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
            dead_letter: tokio::sync::Mutex::new(Vec::new()),
            processed: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl EmailQueueBackend for InMemoryBackend {
    async fn push(&self, job: EmailJob) -> Result<()> {
        let mut queue = self.queue.lock().await;
        queue.push_back(job);
        Ok(())
    }

    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>> {
        let mut queue = self.queue.lock().await;
        let now = chrono_now_ms();
        let mut jobs = Vec::with_capacity(count);

        let mut i = 0;
        while i < queue.len() && jobs.len() < count {
            if let Some(next_retry) = queue[i].next_retry_at
                && next_retry > now
            {
                i += 1;
                continue;
            }
            if let Some(job) = queue.remove(i) {
                jobs.push(job);
            }
        }

        Ok(jobs)
    }

    async fn complete(&self, _job_id: &str) -> Result<()> {
        self.processed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn fail(&self, mut job: EmailJob, error: &str) -> Result<()> {
        job.last_error = Some(error.to_string());
        let mut queue = self.queue.lock().await;
        queue.push_back(job);
        Ok(())
    }

    async fn dead_letter(&self, job: EmailJob) -> Result<()> {
        let mut dl = self.dead_letter.lock().await;
        dl.push(job);
        Ok(())
    }

    async fn stats(&self) -> Result<QueueStats> {
        let queue = self.queue.lock().await;
        let dl = self.dead_letter.lock().await;
        let now = chrono_now_ms();

        let (pending, retrying) = queue.iter().fold((0, 0), |(p, r), job| {
            if let Some(next_retry) = job.next_retry_at
                && next_retry > now
            {
                return (p, r + 1);
            }
            (p + 1, r)
        });

        Ok(QueueStats {
            pending,
            processing: 0,
            retrying,
            dead_letter: dl.len() as u64,
            processed: self.processed.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}

/// Redis-backed email queue.
#[cfg(feature = "redis")]
pub struct RedisBackend {
    redis: Arc<armature_redis::RedisService>,
    config: EmailQueueConfig,
}

#[cfg(feature = "redis")]
impl RedisBackend {
    /// Create a new Redis backend.
    pub fn new(redis: Arc<armature_redis::RedisService>, config: EmailQueueConfig) -> Self {
        Self { redis, config }
    }

    fn pending_key(&self) -> String {
        format!("{}:pending", self.config.queue_name)
    }

    fn retry_key(&self) -> String {
        format!("{}:retry", self.config.queue_name)
    }

    fn dead_letter_key(&self) -> String {
        format!("{}:dead", self.config.queue_name)
    }

    fn job_key(&self, id: &str) -> String {
        format!("{}:job:{}", self.config.queue_name, id)
    }

    fn stats_key(&self) -> String {
        format!("{}:stats", self.config.queue_name)
    }
}

#[cfg(feature = "redis")]
#[async_trait::async_trait]
impl EmailQueueBackend for RedisBackend {
    async fn push(&self, job: EmailJob) -> Result<()> {
        let job_json = serde_json::to_string(&job)?;
        let score = job.priority as f64 * 1_000_000_000.0 + job.created_at as f64;

        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Store job data
        redis::cmd("SET")
            .arg(self.job_key(&job.id))
            .arg(&job_json)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Add to pending sorted set
        redis::cmd("ZADD")
            .arg(self.pending_key())
            .arg(score)
            .arg(&job.id)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job.id, "Email job enqueued");
        Ok(())
    }

    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;
        let now = chrono_now_ms() as f64;

        // Get job IDs from pending queue
        let job_ids: Vec<String> = redis::cmd("ZPOPMIN")
            .arg(self.pending_key())
            .arg(count)
            .query_async(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Also check retry queue for jobs ready to retry
        let retry_ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(self.retry_key())
            .arg(0.0)
            .arg(now)
            .arg("LIMIT")
            .arg(0)
            .arg(count)
            .query_async(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Remove from retry queue
        if !retry_ids.is_empty() {
            redis::cmd("ZREM")
                .arg(self.retry_key())
                .arg(&retry_ids)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;
        }

        let mut jobs = Vec::new();

        for id in job_ids.into_iter().chain(retry_ids.into_iter()) {
            let job_json: Option<String> = redis::cmd("GET")
                .arg(self.job_key(&id))
                .query_async(&mut *conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;

            if let Some(json) = job_json {
                match serde_json::from_str(&json) {
                    Ok(job) => jobs.push(job),
                    Err(e) => error!(job_id = %id, error = %e, "Failed to deserialize job"),
                }
            }
        }

        Ok(jobs)
    }

    async fn complete(&self, job_id: &str) -> Result<()> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Remove job data
        redis::cmd("DEL")
            .arg(self.job_key(job_id))
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Increment processed count
        redis::cmd("HINCRBY")
            .arg(self.stats_key())
            .arg("processed")
            .arg(1)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job_id, "Email job completed");
        Ok(())
    }

    async fn fail(&self, mut job: EmailJob, error: &str) -> Result<()> {
        job.last_error = Some(error.to_string());

        let job_json = serde_json::to_string(&job)?;
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Update job data
        redis::cmd("SET")
            .arg(self.job_key(&job.id))
            .arg(&job_json)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Add to retry queue with next retry timestamp
        let score = job.next_retry_at.unwrap_or_else(chrono_now_ms) as f64;
        redis::cmd("ZADD")
            .arg(self.retry_key())
            .arg(score)
            .arg(&job.id)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job.id, attempts = job.attempts, "Email job scheduled for retry");
        Ok(())
    }

    async fn dead_letter(&self, job: EmailJob) -> Result<()> {
        let job_json = serde_json::to_string(&job)?;
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Add to dead letter list
        redis::cmd("LPUSH")
            .arg(self.dead_letter_key())
            .arg(&job_json)
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Remove job data
        redis::cmd("DEL")
            .arg(self.job_key(&job.id))
            .query_async::<()>(&mut *conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        warn!(job_id = %job.id, "Email job moved to dead letter queue");
        Ok(())
    }

    async fn stats(&self) -> Result<QueueStats> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let pending: u64 = redis::cmd("ZCARD")
            .arg(self.pending_key())
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        let retrying: u64 = redis::cmd("ZCARD")
            .arg(self.retry_key())
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        let dead_letter: u64 = redis::cmd("LLEN")
            .arg(self.dead_letter_key())
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        let processed: u64 = redis::cmd("HGET")
            .arg(self.stats_key())
            .arg("processed")
            .query_async(&mut *conn)
            .await
            .unwrap_or(0);

        Ok(QueueStats {
            pending,
            processing: 0,
            retrying,
            dead_letter,
            processed,
        })
    }
}

/// Email queue for async email sending.
pub struct EmailQueue {
    backend: Arc<dyn EmailQueueBackend>,
    config: EmailQueueConfig,
}

impl EmailQueue {
    /// Create a new email queue with an in-memory backend.
    pub fn in_memory(config: EmailQueueConfig) -> Self {
        Self {
            backend: Arc::new(InMemoryBackend::new()),
            config,
        }
    }

    /// Create a new email queue with a Redis backend.
    #[cfg(feature = "redis")]
    pub fn redis(redis: Arc<armature_redis::RedisService>, config: EmailQueueConfig) -> Self {
        Self {
            backend: Arc::new(RedisBackend::new(redis, config.clone())),
            config,
        }
    }

    /// Create with a custom backend.
    pub fn with_backend(
        backend: impl EmailQueueBackend + 'static,
        config: EmailQueueConfig,
    ) -> Self {
        Self {
            backend: Arc::new(backend),
            config,
        }
    }

    /// Enqueue an email for async sending.
    pub async fn enqueue(&self, email: Email) -> Result<String> {
        let job = EmailJob::new(email);
        let job_id = job.id.clone();
        self.backend.push(job).await?;
        Ok(job_id)
    }

    /// Enqueue with custom job options.
    pub async fn enqueue_job(&self, job: EmailJob) -> Result<String> {
        let job_id = job.id.clone();
        self.backend.push(job).await?;
        Ok(job_id)
    }

    /// Enqueue multiple emails.
    pub async fn enqueue_batch(&self, emails: Vec<Email>) -> Result<Vec<String>> {
        let mut job_ids = Vec::with_capacity(emails.len());
        for email in emails {
            let id = self.enqueue(email).await?;
            job_ids.push(id);
        }
        Ok(job_ids)
    }

    /// Get queue statistics.
    pub async fn stats(&self) -> Result<QueueStats> {
        self.backend.stats().await
    }

    /// Create a worker for processing the queue.
    pub fn worker(&self, mailer: Arc<Mailer>) -> EmailQueueWorker {
        EmailQueueWorker {
            queue: self.backend.clone(),
            mailer,
            config: self.config.clone(),
            shutdown: None,
        }
    }
}

/// Worker for processing the email queue.
pub struct EmailQueueWorker {
    queue: Arc<dyn EmailQueueBackend>,
    mailer: Arc<Mailer>,
    config: EmailQueueConfig,
    shutdown: Option<tokio::sync::broadcast::Receiver<()>>,
}

impl EmailQueueWorker {
    /// Set a shutdown signal.
    pub fn with_shutdown(mut self, shutdown: tokio::sync::broadcast::Receiver<()>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Run the worker.
    pub async fn run(mut self) {
        info!(
            concurrency = self.config.concurrency,
            queue = %self.config.queue_name,
            "Email queue worker started"
        );

        let (job_tx, job_rx) = async_channel::bounded::<EmailJob>(self.config.batch_size * 2);
        let job_rx = Arc::new(job_rx);

        // Spawn worker tasks
        let mut handles = Vec::new();
        for i in 0..self.config.concurrency {
            let rx = job_rx.clone();
            let queue = self.queue.clone();
            let mailer = self.mailer.clone();
            let config = self.config.clone();

            handles.push(tokio::spawn(async move {
                Self::process_jobs(i, rx, queue, mailer, config).await;
            }));
        }

        // Main loop: fetch jobs and distribute to workers
        loop {
            if let Some(ref mut shutdown) = self.shutdown
                && shutdown.try_recv().is_ok()
            {
                info!("Email queue worker shutting down");
                break;
            }

            match self.queue.pop(self.config.batch_size).await {
                Ok(jobs) => {
                    if jobs.is_empty() {
                        tokio::time::sleep(self.config.poll_interval).await;
                    } else {
                        for job in jobs {
                            if job_tx.send(job).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch jobs from queue");
                    tokio::time::sleep(self.config.poll_interval).await;
                }
            }
        }

        // Wait for workers to finish
        drop(job_tx);
        for handle in handles {
            let _ = handle.await;
        }

        info!("Email queue worker stopped");
    }

    async fn process_jobs(
        worker_id: usize,
        rx: Arc<async_channel::Receiver<EmailJob>>,
        queue: Arc<dyn EmailQueueBackend>,
        mailer: Arc<Mailer>,
        config: EmailQueueConfig,
    ) {
        while let Ok(mut job) = rx.recv().await {
            debug!(worker = worker_id, job_id = %job.id, "Processing email job");

            match mailer.send(job.email.clone()).await {
                Ok(()) => {
                    if let Err(e) = queue.complete(&job.id).await {
                        error!(job_id = %job.id, error = %e, "Failed to mark job complete");
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();

                    if job.should_retry() && e.is_retryable() {
                        // Calculate backoff delay
                        let delay = Self::calculate_backoff(&config, job.attempts);
                        job.prepare_retry(delay);

                        if let Err(err) = queue.fail(job, &error_msg).await {
                            error!(error = %err, "Failed to schedule job retry");
                        }
                    } else if config.dead_letter_queue {
                        job.last_error = Some(error_msg);
                        if let Err(err) = queue.dead_letter(job).await {
                            error!(error = %err, "Failed to move job to dead letter queue");
                        }
                    }
                }
            }
        }
    }

    fn calculate_backoff(config: &EmailQueueConfig, attempts: u32) -> Duration {
        let base_delay = config.retry_delay.as_secs_f64();
        let delay = base_delay * 2_f64.powi(attempts as i32);
        let delay = delay.min(config.max_retry_delay.as_secs_f64());
        Duration::from_secs_f64(delay)
    }
}

/// Extension trait for Mailer to add queue support.
pub trait MailerQueueExt {
    /// Create an email queue using this mailer's transport.
    fn queue(&self, config: EmailQueueConfig) -> EmailQueue;

    /// Create a Redis-backed email queue.
    #[cfg(feature = "redis")]
    fn queue_redis(
        &self,
        redis: Arc<armature_redis::RedisService>,
        config: EmailQueueConfig,
    ) -> EmailQueue;
}

impl MailerQueueExt for Mailer {
    fn queue(&self, config: EmailQueueConfig) -> EmailQueue {
        EmailQueue::in_memory(config)
    }

    #[cfg(feature = "redis")]
    fn queue_redis(
        &self,
        redis: Arc<armature_redis::RedisService>,
        config: EmailQueueConfig,
    ) -> EmailQueue {
        EmailQueue::redis(redis, config)
    }
}

// Need async-channel for worker communication
#[allow(dead_code)]
mod async_channel {
    use std::sync::Arc;
    use tokio::sync::{Mutex, mpsc};

    pub struct Sender<T> {
        tx: mpsc::Sender<T>,
    }

    pub struct Receiver<T> {
        rx: Arc<Mutex<mpsc::Receiver<T>>>,
    }

    pub fn bounded<T>(size: usize) -> (Sender<T>, Receiver<T>) {
        let (tx, rx) = mpsc::channel(size);
        (
            Sender { tx },
            Receiver {
                rx: Arc::new(Mutex::new(rx)),
            },
        )
    }

    impl<T> Sender<T> {
        pub async fn send(&self, value: T) -> Result<(), ()> {
            self.tx.send(value).await.map_err(|_| ())
        }
    }

    impl<T> Clone for Receiver<T> {
        fn clone(&self) -> Self {
            Self {
                rx: self.rx.clone(),
            }
        }
    }

    impl<T> Receiver<T> {
        pub async fn recv(&self) -> Result<T, ()> {
            self.rx.lock().await.recv().await.ok_or(())
        }
    }
}
