//! Async email queue for non-blocking email sending.
//!
//! The email queue allows you to enqueue emails for background processing,
//! with automatic retries, persistence, and dead letter handling.
//!
//! # Example
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use armature_mail::{Email, EmailQueue, EmailQueueConfig, Mailer, SmtpConfig};
//! use std::sync::Arc;
//!
//! // In-memory backend; use `EmailQueue::redis(redis_service, config)` for a
//! // persistent one (requires the `redis` feature).
//! let queue = EmailQueue::in_memory(EmailQueueConfig::default());
//!
//! // Enqueue an email (returns immediately with the job ID)
//! let email = Email::new()
//!     .to("user@example.com")
//!     .subject("Hello!")
//!     .text("This email will be sent asynchronously.");
//!
//! let _job_id = queue.enqueue(email).await?;
//!
//! // Start the queue worker (in a separate task). The worker takes a shared
//! // mailer so retries do not deep-copy the email.
//! let mailer = Arc::new(Mailer::smtp(SmtpConfig::new("smtp.example.com")).await?);
//! let worker = queue.worker(mailer);
//! tokio::spawn(worker.run());
//! # Ok(())
//! # }
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
    ///
    /// Shared rather than owned so a retry does not deep-copy every attachment.
    /// The worker previously cloned the whole `Email` per attempt, which at
    /// 10 MB attachments and `concurrency: 4` is tens of megabytes of copies per
    /// batch, repeated on each retry.
    pub email: Arc<Email>,
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
    ///
    /// A stable `Message-ID` is stamped here when the email does not already
    /// carry one. Without it, `to_lettre` lets lettre mint a fresh `Message-ID`
    /// on every attempt, so a send that a timeout abandoned mid-flight — but
    /// which the peer actually accepted — is redelivered under a *different*
    /// identity and nothing downstream can deduplicate it. Stamping once at
    /// enqueue time means every retry of this job carries the same id.
    pub fn new(email: Email) -> Self {
        let mut email = email;
        if email.message_id.is_none() {
            email.message_id = Some(format!("{}@armature", Uuid::new_v4()));
        }

        Self {
            id: Uuid::new_v4().to_string(),
            email: Arc::new(email),
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

    /// Set the maximum retry delay.
    pub fn max_retry_delay(mut self, delay: Duration) -> Self {
        self.max_retry_delay = delay;
        self
    }

    /// Set the per-job send timeout.
    ///
    /// A send that exceeds this is abandoned and treated as a retryable failure,
    /// so a hung transport cannot occupy a worker slot indefinitely.
    pub fn job_timeout(mut self, timeout: Duration) -> Self {
        self.job_timeout = timeout;
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

    /// Push several jobs at once.
    ///
    /// The default implementation pushes them one at a time; backends that can
    /// batch (e.g. Redis pipelining) should override this to avoid paying the
    /// per-job round-trip cost N times.
    async fn push_batch(&self, jobs: Vec<EmailJob>) -> Result<()> {
        for job in jobs {
            self.push(job).await?;
        }
        Ok(())
    }

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
    /// Jobs claimed by `pop` and not yet completed, failed, or dead-lettered.
    ///
    /// A non-zero value that never drains indicates workers that died between
    /// `pop` and `complete`.
    pub processing: u64,
    /// Failed jobs (in retry).
    pub retrying: u64,
    /// Dead letter jobs.
    pub dead_letter: u64,
    /// Total processed.
    pub processed: u64,
}

/// In-memory email queue backend (for testing/development).
///
/// # Not for large backlogs
///
/// `pop` scans the queue from the front and removes matching entries with
/// `VecDeque::remove`, which is O(n) per removal, and not-yet-due retry jobs are
/// re-scanned on every poll. That is fine for tests and development; a
/// production deployment with a real backlog should use [`RedisBackend`].
pub struct InMemoryBackend {
    queue: tokio::sync::Mutex<std::collections::VecDeque<EmailJob>>,
    /// Jobs handed out by `pop` and not yet completed, failed, or dead-lettered.
    processing: tokio::sync::Mutex<std::collections::HashSet<String>>,
    dead_letter: tokio::sync::Mutex<Vec<EmailJob>>,
    processed: std::sync::atomic::AtomicU64,
}

impl InMemoryBackend {
    /// Create a new in-memory backend.
    ///
    /// See the type-level docs: this backend is intended for testing and
    /// development, and `pop` is O(n) in the queue length.
    pub fn new() -> Self {
        Self {
            queue: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
            processing: tokio::sync::Mutex::new(std::collections::HashSet::new()),
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

        // Claim the popped jobs so `QueueStats::processing` is observable and a
        // job in flight is distinguishable from one that was never popped.
        let mut processing = self.processing.lock().await;
        processing.extend(jobs.iter().map(|j| j.id.clone()));

        Ok(jobs)
    }

    async fn complete(&self, job_id: &str) -> Result<()> {
        self.processing.lock().await.remove(job_id);
        self.processed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn fail(&self, mut job: EmailJob, error: &str) -> Result<()> {
        self.processing.lock().await.remove(&job.id);
        job.last_error = Some(error.to_string());
        let mut queue = self.queue.lock().await;
        queue.push_back(job);
        Ok(())
    }

    async fn dead_letter(&self, job: EmailJob) -> Result<()> {
        self.processing.lock().await.remove(&job.id);
        let mut dl = self.dead_letter.lock().await;
        dl.push(job);
        Ok(())
    }

    async fn stats(&self) -> Result<QueueStats> {
        let queue = self.queue.lock().await;
        let dl = self.dead_letter.lock().await;
        let processing = self.processing.lock().await.len() as u64;
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
            processing,
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

    /// Sorted set of jobs claimed by `pop` but not yet completed or failed.
    ///
    /// Scored by claim time, so a sweeper can recover jobs whose worker crashed
    /// between `pop` and `complete` — without it, such a job is simply lost and
    /// `QueueStats::processing` can only ever report 0.
    fn processing_key(&self) -> String {
        format!("{}:processing", self.config.queue_name)
    }

    /// Release a claim made by `pop`.
    async fn release_processing(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        job_id: &str,
    ) -> Result<()> {
        redis::cmd("ZREM")
            .arg(self.processing_key())
            .arg(job_id)
            .query_async::<()>(conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))
    }

    /// Dead-letter ids whose body was missing or unreadable.
    ///
    /// These have already been removed from the pending/retry sets, so they
    /// would otherwise vanish. There is no job body to serialize, so a stub
    /// recording the id goes to the dead-letter list instead.
    async fn discard_lost(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        ids: &[String],
    ) -> Result<()> {
        for id in ids {
            self.release_processing(conn, id).await?;

            if self.config.dead_letter_queue {
                let stub = serde_json::json!({
                    "id": id,
                    "error": "job body missing or corrupt in Redis",
                    "lost_at": chrono_now_ms(),
                })
                .to_string();

                redis::cmd("LPUSH")
                    .arg(self.dead_letter_key())
                    .arg(stub)
                    .query_async::<()>(conn)
                    .await
                    .map_err(|e| MailError::Queue(e.to_string()))?;

                warn!(job_id = %id, "Unreadable email job moved to dead letter queue");
            }

            redis::cmd("DEL")
                .arg(self.job_key(id))
                .query_async::<()>(conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;
        }
        Ok(())
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
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Add to pending sorted set
        redis::cmd("ZADD")
            .arg(self.pending_key())
            .arg(score)
            .arg(&job.id)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job.id, "Email job enqueued");
        Ok(())
    }

    /// Enqueue every job in one pipelined round-trip (one connection acquire,
    /// one SET+ZADD batch) instead of N sequential `push` calls.
    async fn push_batch(&self, jobs: Vec<EmailJob>) -> Result<()> {
        if jobs.is_empty() {
            return Ok(());
        }

        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let mut pipe = redis::pipe();
        for job in &jobs {
            let job_json = serde_json::to_string(job)?;
            let score = job.priority as f64 * 1_000_000_000.0 + job.created_at as f64;
            pipe.cmd("SET").arg(self.job_key(&job.id)).arg(job_json);
            pipe.cmd("ZADD")
                .arg(self.pending_key())
                .arg(score)
                .arg(&job.id);
        }

        pipe.query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(count = jobs.len(), "Email jobs enqueued (pipelined)");
        Ok(())
    }

    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;
        let now = chrono_now_ms() as f64;

        if count == 0 {
            return Ok(Vec::new());
        }

        // Retries first, then only the remaining budget from pending. Applying
        // `count` to both sets independently and concatenating returned up to
        // `2 * count` jobs, contradicting `InMemoryBackend::pop`, which honors
        // `count` exactly.
        let retry_ids: Vec<String> = redis::cmd("ZRANGEBYSCORE")
            .arg(self.retry_key())
            .arg(0.0)
            .arg(now)
            .arg("LIMIT")
            .arg(0)
            .arg(count)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Remove from retry queue
        if !retry_ids.is_empty() {
            redis::cmd("ZREM")
                .arg(self.retry_key())
                .arg(&retry_ids)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;
        }

        let remaining = count.saturating_sub(retry_ids.len());

        // `ZPOPMIN key <count>` replies with a flat `member,score,member,score…`
        // array in RESP2 and a nested array in RESP3. Deserializing into
        // `Vec<String>` therefore produced `2 * count` entries — every other one a
        // score, which became a bogus `…:job:<score>` key in the MGET below — and
        // failed outright under RESP3. `Vec<(String, f64)>` is correct for both.
        let pending: Vec<(String, f64)> = if remaining > 0 {
            redis::cmd("ZPOPMIN")
                .arg(self.pending_key())
                .arg(remaining)
                .query_async(&mut conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?
        } else {
            Vec::new()
        };

        let ids: Vec<String> = retry_ids
            .into_iter()
            .chain(pending.into_iter().map(|(id, _score)| id))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Claim the ids: they are now off the pending/retry sets, so without this
        // a crash between here and `complete` loses them with no trace. The
        // score is the claim time, which is what a visibility-timeout sweeper
        // would key off.
        redis::cmd("ZADD")
            .arg(self.processing_key())
            .arg(
                ids.iter()
                    .flat_map(|id| [now.to_string(), id.clone()])
                    .collect::<Vec<_>>(),
            )
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Fetch every job body in a single MGET rather than one GET per id: this
        // runs on the worker's hot path, and N round-trips per poll dominates the
        // poll cost once batch_size grows.
        let keys: Vec<String> = ids.iter().map(|id| self.job_key(id)).collect();
        let payloads: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let mut jobs = Vec::with_capacity(ids.len());
        let mut lost: Vec<String> = Vec::new();
        for (id, payload) in ids.iter().zip(payloads) {
            // The id is already off the pending/retry sets here, so anything we
            // drop is gone for good — and `enqueue` told the caller it succeeded.
            // Never fail silently: log, and dead-letter the id so it is at least
            // visible and recoverable.
            let Some(json) = payload else {
                warn!(
                    job_id = %id,
                    "Email job body missing from Redis (expired or evicted); job cannot be sent"
                );
                lost.push(id.clone());
                continue;
            };
            match serde_json::from_str(&json) {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    error!(job_id = %id, error = %e, "Failed to deserialize job");
                    lost.push(id.clone());
                }
            }
        }

        if !lost.is_empty() {
            self.discard_lost(&mut conn, &lost).await?;
        }

        Ok(jobs)
    }

    async fn complete(&self, job_id: &str) -> Result<()> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Release the claim taken by `pop`.
        self.release_processing(&mut conn, job_id).await?;

        // Remove job data
        redis::cmd("DEL")
            .arg(self.job_key(job_id))
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Increment processed count
        redis::cmd("HINCRBY")
            .arg(self.stats_key())
            .arg("processed")
            .arg(1)
            .query_async::<()>(&mut conn)
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
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Release the claim taken by `pop` — the job moves to the retry set.
        self.release_processing(&mut conn, &job.id).await?;

        // Add to retry queue with next retry timestamp
        let score = job.next_retry_at.unwrap_or_else(chrono_now_ms) as f64;
        redis::cmd("ZADD")
            .arg(self.retry_key())
            .arg(score)
            .arg(&job.id)
            .query_async::<()>(&mut conn)
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

        // Release the claim taken by `pop`.
        self.release_processing(&mut conn, &job.id).await?;

        // Add to dead letter list
        redis::cmd("LPUSH")
            .arg(self.dead_letter_key())
            .arg(&job_json)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Remove job data
        redis::cmd("DEL")
            .arg(self.job_key(&job.id))
            .query_async::<()>(&mut conn)
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
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let retrying: u64 = redis::cmd("ZCARD")
            .arg(self.retry_key())
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let dead_letter: u64 = redis::cmd("LLEN")
            .arg(self.dead_letter_key())
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Jobs claimed by `pop` and not yet completed, failed, or dead-lettered.
        let processing: u64 = redis::cmd("ZCARD")
            .arg(self.processing_key())
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // HGET returns nil (deserialized as None by the redis crate) when the
        // "processed" field hasn't been set yet (e.g. no jobs processed since
        // startup) — that's a legitimate 0, not an error, so default it after
        // the Redis-call error is propagated rather than masking real outages.
        let processed: u64 = redis::cmd("HGET")
            .arg(self.stats_key())
            .arg("processed")
            .query_async::<Option<u64>>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?
            .unwrap_or(0);

        Ok(QueueStats {
            pending,
            processing,
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
    ///
    /// Intended for testing and development. Jobs live only in this process (a
    /// restart loses the whole queue) and [`InMemoryBackend`]'s `pop` is O(n) in
    /// the queue length, so it is unsuitable for large backlogs. Use
    /// [`EmailQueue::redis`] in production.
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
    ///
    /// Dispatches through [`EmailQueueBackend::push_batch`], so backends that can
    /// pipeline (Redis) pay one round-trip rather than one per email.
    pub async fn enqueue_batch(&self, emails: Vec<Email>) -> Result<Vec<String>> {
        let jobs: Vec<EmailJob> = emails.into_iter().map(EmailJob::new).collect();
        let job_ids: Vec<String> = jobs.iter().map(|j| j.id.clone()).collect();
        self.backend.push_batch(jobs).await?;
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

            // Bound every send by `job_timeout`: without this a transport that
            // hangs (dead SMTP socket, provider that never responds) holds this
            // worker slot forever. An elapsed timeout is a retryable failure.
            //
            // Dropping the future does NOT un-send anything: a slow `250 OK` that
            // arrives after the deadline means the message WAS delivered, and the
            // job is still requeued. That redelivery is why `EmailJob::new` stamps
            // a stable `Message-ID` — every attempt carries the same one, so a
            // receiving MTA can deduplicate. Configure the transport-level timeout
            // (`SmtpConfig::timeout`) below `job_timeout` so the peer connection is
            // torn down deterministically instead of the future merely being
            // dropped.
            //
            // `Arc::clone` here is a refcount bump, not a copy of the attachments.
            let outcome = match tokio::time::timeout(
                config.job_timeout,
                mailer.send_shared(job.email.clone()),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    warn!(
                        worker = worker_id,
                        job_id = %job.id,
                        timeout = ?config.job_timeout,
                        message_id = ?job.email.message_id,
                        "Email job timed out; the send may still have been delivered \
                         (retries reuse the same Message-ID so it can be deduplicated)"
                    );
                    Err(MailError::Timeout)
                }
            };

            match outcome {
                Ok(()) => {
                    if let Err(e) = queue.complete(&job.id).await {
                        error!(job_id = %job.id, error = %e, "Failed to mark job complete");
                    }
                }
                Err(e) => {
                    let error_msg = e.to_string();

                    if job.should_retry() && e.is_retryable() {
                        // Calculate backoff delay, honoring the provider's
                        // `Retry-After` when it gave us one.
                        let delay = Self::calculate_backoff(&config, job.attempts, &e);
                        job.prepare_retry(delay);

                        if let Err(err) = queue.fail(job, &error_msg).await {
                            error!(error = %err, "Failed to schedule job retry");
                        }
                    } else if config.dead_letter_queue {
                        job.last_error = Some(error_msg);
                        if let Err(err) = queue.dead_letter(job).await {
                            error!(error = %err, "Failed to move job to dead letter queue");
                        }
                    } else {
                        // With the DLQ disabled there is nowhere to put the job,
                        // but it must not disappear without a trace: `enqueue`
                        // returned an id to a caller that believes the email is
                        // still in flight.
                        error!(
                            worker = worker_id,
                            job_id = %job.id,
                            attempts = job.attempts,
                            error = %error_msg,
                            "Email job failed permanently and was dropped \
                             (dead_letter_queue is disabled)"
                        );
                        if let Err(err) = queue.complete(&job.id).await {
                            error!(job_id = %job.id, error = %err, "Failed to discard job");
                        }
                    }
                }
            }
        }
    }

    /// Delay before the next attempt at `job`.
    ///
    /// When the provider told us how long to wait — SendGrid parses `Retry-After`
    /// into [`MailError::RateLimited`] — that value wins over the local
    /// exponential backoff. Retrying a `Retry-After: 300` after the configured
    /// 5s just gets re-throttled and burns quota, so the provider's figure is
    /// deliberately *not* capped by `max_retry_delay`: capping it would
    /// reintroduce exactly the re-throttling this avoids.
    fn calculate_backoff(config: &EmailQueueConfig, attempts: u32, error: &MailError) -> Duration {
        if let Some(retry_after) = error.retry_after() {
            return retry_after;
        }

        let base_delay = config.retry_delay.as_secs_f64();
        let delay = base_delay * 2_f64.powi(attempts as i32);
        let delay = delay.min(config.max_retry_delay.as_secs_f64());
        Duration::from_secs_f64(delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> EmailQueueConfig {
        EmailQueueConfig::default()
            .retry_delay(Duration::from_secs(5))
            .max_retry_delay(Duration::from_secs(300))
    }

    /// WF6 audit finding 20: `MailError::retry_after` was parsed from the
    /// provider's `Retry-After` header and then ignored by both retry paths, so
    /// a `Retry-After: 300` was retried after the local delay and re-throttled.
    #[test]
    fn backoff_honors_the_providers_retry_after() {
        let delay = EmailQueueWorker::calculate_backoff(&config(), 0, &MailError::RateLimited(300));
        assert_eq!(delay, Duration::from_secs(300));

        // Even beyond `max_retry_delay`: capping would re-throttle us.
        let delay =
            EmailQueueWorker::calculate_backoff(&config(), 0, &MailError::RateLimited(3600));
        assert_eq!(delay, Duration::from_secs(3600));
    }

    #[test]
    fn backoff_falls_back_to_exponential_without_a_retry_after() {
        let cfg = config();
        let err = MailError::Network("boom".into());

        assert_eq!(
            EmailQueueWorker::calculate_backoff(&cfg, 0, &err),
            Duration::from_secs(5)
        );
        assert_eq!(
            EmailQueueWorker::calculate_backoff(&cfg, 1, &err),
            Duration::from_secs(10)
        );
        assert_eq!(
            EmailQueueWorker::calculate_backoff(&cfg, 2, &err),
            Duration::from_secs(20)
        );
        // Clamped by `max_retry_delay`.
        assert_eq!(
            EmailQueueWorker::calculate_backoff(&cfg, 20, &err),
            Duration::from_secs(300)
        );
    }

    /// A stable Message-ID is stamped at enqueue time so retries of a send that
    /// a timeout abandoned mid-flight can be deduplicated downstream.
    #[test]
    fn email_job_stamps_a_stable_message_id() {
        let job = EmailJob::new(Email::new().to("a@example.com"));
        let id = job.email.message_id.clone().expect("message id stamped");
        assert!(id.ends_with("@armature"), "unexpected id: {id}");
        assert!(uuid::Uuid::parse_str(id.trim_end_matches("@armature")).is_ok());

        // Distinct jobs get distinct ids.
        let other = EmailJob::new(Email::new().to("b@example.com"));
        assert_ne!(other.email.message_id, job.email.message_id);
    }

    #[test]
    fn email_job_preserves_a_caller_supplied_message_id() {
        let job = EmailJob::new(Email::new().message_id("mine@example.com"));
        assert_eq!(job.email.message_id.as_deref(), Some("mine@example.com"));
    }
}

/// Extension trait for Mailer to add queue support.
pub trait MailerQueueExt {
    /// Create an in-memory email queue.
    ///
    /// The queue itself is transport-agnostic — the transport is bound later,
    /// when a worker is created with [`EmailQueue::worker`], not by this call.
    /// The mailer is therefore not consulted here; this method exists purely as
    /// a discoverable entry point from a `Mailer`.
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
