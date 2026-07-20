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

/// Ceiling on a server-supplied `Retry-After`.
///
/// A provider's `Retry-After` wins over the local backoff — retrying a
/// `Retry-After: 300` after the configured 5s just gets re-throttled — but it
/// arrives from outside the process, and an uncapped one parks a job for as long
/// as it says. `Retry-After: 999999999` is roughly 31 years. One hour is far
/// above any legitimate provider throttle window, so the cap preserves the
/// don't-re-throttle intent while bounding the hostile (or buggy) case.
pub const MAX_SERVER_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// Email queue configuration.
///
/// `#[non_exhaustive]`: this type gains fields (`job_timeout`,
/// `visibility_timeout`, `max_queue_depth`) as the queue grows, and each
/// addition would otherwise break every downstream struct literal. Construct it
/// with `EmailQueueConfig::default()` plus the builders.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// How long a claimed job may sit in the processing set before a sweeper
    /// assumes its worker died and returns it to the queue.
    ///
    /// See [`EmailQueueBackend::reclaim_stale`]. Must be comfortably above
    /// [`Self::job_timeout`]: a job that is merely slow, not lost, would
    /// otherwise be redelivered while its first attempt is still in flight.
    pub visibility_timeout: Duration,
    /// Maximum number of jobs [`InMemoryBackend`] will hold.
    ///
    /// The in-memory backend is unbounded no longer: both its queue and its
    /// dead-letter list retain an `Arc<Email>` with full attachments, and
    /// neither had a cap, a drain, or an eviction policy. Ignored by
    /// [`RedisBackend`], whose bound is Redis's own memory policy.
    pub max_queue_depth: usize,
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
            visibility_timeout: Duration::from_secs(300),
            max_queue_depth: 10_000,
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

    /// Set the visibility timeout for claimed jobs (default: 5 minutes).
    ///
    /// See [`EmailQueueBackend::reclaim_stale`].
    pub fn visibility_timeout(mut self, timeout: Duration) -> Self {
        self.visibility_timeout = timeout;
        self
    }

    /// Set the maximum depth of the in-memory backend (default: 10 000).
    pub fn max_queue_depth(mut self, depth: usize) -> Self {
        self.max_queue_depth = depth.max(1);
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

    /// Mark a job as successfully sent.
    ///
    /// Releases the claim taken by [`Self::pop`], deletes the job body, and
    /// increments [`QueueStats::processed`]. Only call this for a job that was
    /// actually delivered — see [`Self::discard`] for the failure case.
    async fn complete(&self, job_id: &str) -> Result<()>;

    /// Release a claim and delete a job *without* counting it as processed.
    ///
    /// For a job that will never be sent and has nowhere to go — a permanent
    /// failure with the dead-letter queue disabled. `complete` was used for this,
    /// which incremented the success counter with failures, so
    /// [`QueueStats::processed`] could not back a delivery-rate alert: a run
    /// where every send failed reported 100% processed.
    async fn discard(&self, job_id: &str) -> Result<()>;

    /// Mark a job as failed and schedule retry.
    async fn fail(&self, job: EmailJob, error: &str) -> Result<()>;

    /// Move a job to the dead letter queue.
    async fn dead_letter(&self, job: EmailJob) -> Result<()>;

    /// Return jobs claimed longer than `visibility_timeout` ago to the queue.
    ///
    /// A worker that dies between [`Self::pop`] and `complete`/`fail`/
    /// `dead_letter` leaves its job claimed forever: off the pending and retry
    /// sets, never redelivered, with `enqueue` having already told the caller
    /// `Ok(job_id)`. This is the sweeper that recovers those, and
    /// [`EmailQueueWorker::run`] calls it on an interval.
    ///
    /// Returns the number of jobs reclaimed. Redelivery means a job whose worker
    /// was merely slow — not dead — can be sent twice; every retry of a job
    /// reuses the `Message-ID` stamped by [`EmailJob::new`] so a receiving MTA
    /// can deduplicate, and `visibility_timeout` should be set comfortably above
    /// [`EmailQueueConfig::job_timeout`].
    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64>;

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
    /// Jobs handed out by `pop` and not yet completed, failed, or dead-lettered,
    /// keyed by id and carrying the claim time plus the job itself.
    ///
    /// The job is retained so `reclaim_stale` can actually put it back: `pop`
    /// removed it from `queue`, so without a copy here a crashed worker's job is
    /// unrecoverable, exactly as it was in Redis.
    processing: tokio::sync::Mutex<std::collections::HashMap<String, (i64, EmailJob)>>,
    /// Bounded: this was an unbounded `Vec` that was only ever pushed to — no
    /// drain, no cap, no eviction — and every retained job holds an `Arc<Email>`
    /// with full attachments, so a steadily failing queue grew it without limit.
    dead_letter: tokio::sync::Mutex<std::collections::VecDeque<EmailJob>>,
    processed: std::sync::atomic::AtomicU64,
    /// Cap on both `queue` and `dead_letter`.
    max_depth: usize,
}

impl InMemoryBackend {
    /// Create a new in-memory backend with the default depth cap
    /// (`EmailQueueConfig::default().max_queue_depth`).
    ///
    /// See the type-level docs: this backend is intended for testing and
    /// development, and `pop` is O(n) in the queue length.
    pub fn new() -> Self {
        Self::with_max_depth(EmailQueueConfig::default().max_queue_depth)
    }

    /// Create a new in-memory backend holding at most `max_depth` jobs.
    ///
    /// `push` past the cap fails with [`MailError::Queue`] rather than growing
    /// without bound; the dead-letter list evicts its oldest entry instead, since
    /// dropping a *new* permanent failure on the floor would defeat the point of
    /// having one.
    pub fn with_max_depth(max_depth: usize) -> Self {
        Self {
            queue: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
            processing: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            dead_letter: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
            processed: std::sync::atomic::AtomicU64::new(0),
            max_depth: max_depth.max(1),
        }
    }

    /// Snapshot of the dead-letter list, oldest first.
    pub async fn dead_letter_jobs(&self) -> Vec<EmailJob> {
        self.dead_letter.lock().await.iter().cloned().collect()
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
        if queue.len() >= self.max_depth {
            return Err(MailError::Queue(format!(
                "in-memory queue is full ({} jobs, max_queue_depth = {}); \
                 use the Redis backend for a real backlog",
                queue.len(),
                self.max_depth
            )));
        }
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

        // Claim the popped jobs so `QueueStats::processing` is observable, a
        // job in flight is distinguishable from one that was never popped, and
        // `reclaim_stale` has both a claim time and a job body to work from.
        let claimed_at = chrono_now_ms();
        let mut processing = self.processing.lock().await;
        for job in &jobs {
            processing.insert(job.id.clone(), (claimed_at, job.clone()));
        }

        Ok(jobs)
    }

    async fn complete(&self, job_id: &str) -> Result<()> {
        self.processing.lock().await.remove(job_id);
        self.processed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn discard(&self, job_id: &str) -> Result<()> {
        // Deliberately does not touch `processed`: this job was never sent.
        self.processing.lock().await.remove(job_id);
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
        // Evict the oldest rather than reject the newest: a full DLQ must not
        // start silently dropping the failures it exists to record.
        while dl.len() >= self.max_depth {
            let evicted = dl.pop_front();
            if let Some(job) = evicted {
                warn!(
                    job_id = %job.id,
                    "Dead letter queue full; evicting the oldest entry"
                );
            }
        }
        dl.push_back(job);
        Ok(())
    }

    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64> {
        let cutoff = chrono_now_ms() - visibility_timeout.as_millis() as i64;

        let mut processing = self.processing.lock().await;
        let stale: Vec<String> = processing
            .iter()
            .filter(|(_, (claimed_at, _))| *claimed_at <= cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        if stale.is_empty() {
            return Ok(0);
        }

        let mut queue = self.queue.lock().await;
        let mut reclaimed = 0u64;
        for id in stale {
            let Some((_, mut job)) = processing.remove(&id) else {
                continue;
            };
            // Due immediately: the previous attempt already served as the delay.
            job.next_retry_at = None;
            queue.push_back(job);
            reclaimed += 1;
        }

        warn!(
            count = reclaimed,
            timeout = ?visibility_timeout,
            "Reclaimed email jobs whose worker never reported back"
        );
        Ok(reclaimed)
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

    /// List recording ids whose body was missing or corrupt at `pop` time.
    ///
    /// Written to unconditionally, unlike the dead-letter list: `discard_lost`
    /// used to `DEL` the body and record nothing when `dead_letter_queue` was
    /// off, destroying the last forensic trace of a job the caller was told had
    /// been enqueued.
    fn lost_key(&self) -> String {
        format!("{}:lost", self.config.queue_name)
    }

    /// Sorted set of jobs claimed by `pop` but not yet completed or failed.
    ///
    /// Scored by claim time, which is what [`RedisBackend::reclaim_stale`] keys
    /// off to recover jobs whose worker crashed between `pop` and `complete` —
    /// without it, such a job is simply lost and `QueueStats::processing` can
    /// only ever report 0.
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
    /// recording the id goes to the `:lost` list — and, when the dead-letter
    /// queue is enabled, to the dead-letter list too.
    ///
    /// Issued as a single pipelined round-trip. This used to be three sequential
    /// commands per lost job in a loop, unbounded in `batch_size`, and it fires
    /// exactly when Redis is under memory pressure — the worst possible moment
    /// to make `3 * batch_size` extra round-trips.
    async fn discard_lost(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        ids: &[String],
    ) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let mut pipe = redis::pipe();
        for id in ids {
            let stub = serde_json::json!({
                "id": id,
                "error": "job body missing or corrupt in Redis",
                "lost_at": chrono_now_ms(),
            })
            .to_string();

            pipe.cmd("ZREM").arg(self.processing_key()).arg(id);
            // Recorded regardless of `dead_letter_queue`: the body is about to
            // be deleted, so this stub is the only remaining trace of a job the
            // caller was told had been enqueued.
            pipe.cmd("LPUSH").arg(self.lost_key()).arg(&stub);
            if self.config.dead_letter_queue {
                pipe.cmd("LPUSH").arg(self.dead_letter_key()).arg(&stub);
            }
            pipe.cmd("DEL").arg(self.job_key(id));

            // An email the caller believes is in flight is being destroyed; that
            // is not a warning.
            error!(
                job_id = %id,
                dead_lettered = self.config.dead_letter_queue,
                "Email job body missing or corrupt; job recorded as lost and discarded"
            );
        }

        pipe.query_async::<()>(conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))
    }

    /// Atomically claim due retry jobs.
    ///
    /// `ZRANGEBYSCORE` followed by a separate `ZREM` is not a claim: two workers
    /// (or one worker with `concurrency > 1`) both saw the same ids before
    /// either `ZREM` landed, both added them to `:processing`, both `MGET`ed the
    /// bodies, and both sent the email. The tests passed only because they ran a
    /// single worker. Doing the range, the removal, and the processing-set claim
    /// in one `EVAL` makes the script the single point that decides ownership:
    /// Redis runs it to completion before any other client's command.
    async fn claim_retry_ids(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        now: f64,
        count: usize,
    ) -> Result<Vec<String>> {
        const CLAIM_RETRY: &str = r#"
            local ids = redis.call('ZRANGEBYSCORE', KEYS[1], 0, ARGV[1], 'LIMIT', 0, ARGV[2])
            for i = 1, #ids do
                redis.call('ZREM', KEYS[1], ids[i])
                redis.call('ZADD', KEYS[2], ARGV[3], ids[i])
            end
            return ids
        "#;

        redis::cmd("EVAL")
            .arg(CLAIM_RETRY)
            .arg(2)
            .arg(self.retry_key())
            .arg(self.processing_key())
            .arg(now)
            .arg(count)
            .arg(now)
            .query_async(conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))
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
        //
        // The retry claim is one atomic `EVAL` — range, remove, and claim into
        // `:processing` — so exactly one caller can win a given id. It also
        // means these ids are *already* in the processing set below.
        let retry_ids: Vec<String> = self.claim_retry_ids(&mut conn, now, count).await?;

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

        let pending_ids: Vec<String> = pending.into_iter().map(|(id, _score)| id).collect();

        // Claim the ids popped from pending: they are now off that set, so
        // without this a crash between here and `complete` loses them with no
        // trace. The score is the claim time, which `reclaim_stale` keys off.
        // The retry ids were claimed atomically inside `claim_retry_ids`.
        if !pending_ids.is_empty() {
            redis::cmd("ZADD")
                .arg(self.processing_key())
                .arg(
                    pending_ids
                        .iter()
                        .flat_map(|id| [now.to_string(), id.clone()])
                        .collect::<Vec<_>>(),
                )
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;
        }

        let ids: Vec<String> = retry_ids.into_iter().chain(pending_ids).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

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
            // `discard_lost` below emits the `error!` for each of these — an
            // email the caller believes is in flight is about to be destroyed,
            // which the previous `warn!` understated. Logging the specific cause
            // at debug here keeps "missing" distinguishable from "corrupt"
            // without duplicating the error line.
            let Some(json) = payload else {
                debug!(
                    job_id = %id,
                    "Email job body missing from Redis (expired or evicted)"
                );
                lost.push(id.clone());
                continue;
            };
            match serde_json::from_str(&json) {
                Ok(job) => jobs.push(job),
                Err(e) => {
                    debug!(job_id = %id, error = %e, "Email job body is not deserializable");
                    lost.push(id.clone());
                }
            }
        }

        // Deliberately not `?`: `jobs` here are already claimed in `:processing`
        // and already off the pending/retry sets. Propagating would discard
        // them — perfectly good emails, permanently stuck until the sweeper
        // notices — because *other* ids could not be cleaned up. Log and carry
        // on; the lost ids stay claimed and `reclaim_stale` will retry them.
        if !lost.is_empty()
            && let Err(e) = self.discard_lost(&mut conn, &lost).await
        {
            error!(
                error = %e,
                lost = lost.len(),
                claimed = jobs.len(),
                "Failed to record lost email jobs; returning the jobs that were readable"
            );
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

    async fn discard(&self, job_id: &str) -> Result<()> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Same as `complete` minus the `HINCRBY processed`: this job was never
        // delivered, and counting it would inflate the success counter with
        // failures.
        redis::pipe()
            .cmd("ZREM")
            .arg(self.processing_key())
            .arg(job_id)
            .cmd("DEL")
            .arg(self.job_key(job_id))
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job_id, "Email job discarded without being sent");
        Ok(())
    }

    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64> {
        // Range, remove from `:processing`, and re-add to `:retry` in one
        // `EVAL`, for the same reason the retry claim is one: two sweepers doing
        // this as separate commands would both reclaim the same id and enqueue
        // it twice.
        //
        // Reclaimed jobs go back to the *retry* set, scored `now`, rather than
        // to pending: the retry set is scanned first and a timestamp score makes
        // them immediately due, which is the behavior a recovered job wants.
        const RECLAIM: &str = r#"
            local ids = redis.call('ZRANGEBYSCORE', KEYS[1], 0, ARGV[1])
            for i = 1, #ids do
                redis.call('ZREM', KEYS[1], ids[i])
                redis.call('ZADD', KEYS[2], ARGV[2], ids[i])
            end
            return #ids
        "#;

        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let now = chrono_now_ms();
        let cutoff = now - visibility_timeout.as_millis() as i64;

        let reclaimed: u64 = redis::cmd("EVAL")
            .arg(RECLAIM)
            .arg(2)
            .arg(self.processing_key())
            .arg(self.retry_key())
            .arg(cutoff)
            .arg(now)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        if reclaimed > 0 {
            warn!(
                count = reclaimed,
                timeout = ?visibility_timeout,
                "Reclaimed email jobs whose worker never reported back"
            );
        }

        Ok(reclaimed)
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
            backend: Arc::new(InMemoryBackend::with_max_depth(config.max_queue_depth)),
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

        // How often the poll loop sweeps for jobs whose worker died mid-send.
        // A quarter of the visibility timeout bounds the extra latency such a
        // job sees at ~25% of the timeout, without sweeping on every poll.
        let sweep_interval = (self.config.visibility_timeout / 4).max(self.config.poll_interval);
        let mut last_sweep = std::time::Instant::now();

        // Main loop: fetch jobs and distribute to workers
        loop {
            if let Some(ref mut shutdown) = self.shutdown
                && shutdown.try_recv().is_ok()
            {
                info!("Email queue worker shutting down");
                break;
            }

            // The `:processing` set is only useful if something drains it: a
            // worker that dies between `pop` and `complete` otherwise leaves its
            // job claimed forever, after `enqueue` returned `Ok(job_id)`.
            if last_sweep.elapsed() >= sweep_interval {
                last_sweep = std::time::Instant::now();
                match self
                    .queue
                    .reclaim_stale(self.config.visibility_timeout)
                    .await
                {
                    Ok(0) => {}
                    Ok(n) => info!(count = n, "Reclaimed stale email jobs"),
                    // Never fatal to the loop: a failed sweep must not stop the
                    // worker from draining the queue.
                    Err(e) => error!(error = %e, "Failed to sweep stale email jobs"),
                }
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
                        // `discard`, not `complete`: this email was never sent,
                        // and `complete` increments `QueueStats::processed`,
                        // which would inflate the success counter with failures
                        // and make a delivery-rate alert unbuildable.
                        if let Err(err) = queue.discard(&job.id).await {
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
    /// deliberately *not* capped by `max_retry_delay`: capping it there would
    /// reintroduce exactly the re-throttling this avoids.
    ///
    /// It *is* capped by [`MAX_SERVER_RETRY_AFTER`]. The value comes from
    /// outside the process, and uncapped a `Retry-After: 999999999` parked the
    /// job for roughly 31 years. One hour is far above any legitimate throttle
    /// window, so the ceiling only ever bites on a hostile or broken header.
    fn calculate_backoff(config: &EmailQueueConfig, attempts: u32, error: &MailError) -> Duration {
        if let Some(retry_after) = error.retry_after() {
            if retry_after > MAX_SERVER_RETRY_AFTER {
                warn!(
                    ?retry_after,
                    cap = ?MAX_SERVER_RETRY_AFTER,
                    "Provider Retry-After exceeds the ceiling; capping"
                );
            }
            return retry_after.min(MAX_SERVER_RETRY_AFTER);
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

    /// A server `Retry-After` was honored verbatim and uncapped, so
    /// `Retry-After: 999999999` parked a job for ~31 years.
    #[test]
    fn a_hostile_retry_after_is_capped() {
        let delay =
            EmailQueueWorker::calculate_backoff(&config(), 0, &MailError::RateLimited(999_999_999));
        assert_eq!(delay, MAX_SERVER_RETRY_AFTER);

        // Exactly at the ceiling is untouched, and anything below it still wins
        // over the local backoff.
        assert_eq!(
            EmailQueueWorker::calculate_backoff(&config(), 0, &MailError::RateLimited(3600)),
            Duration::from_secs(3600)
        );
        assert_eq!(
            EmailQueueWorker::calculate_backoff(&config(), 0, &MailError::RateLimited(1800)),
            Duration::from_secs(1800)
        );
    }

    fn queued_email() -> Email {
        Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello")
    }

    /// `discard` releases the claim and drops the job without counting it as a
    /// success. `complete` was used for this, so a run in which every send
    /// failed still reported `processed == n`.
    #[tokio::test]
    async fn discard_does_not_inflate_the_processed_counter() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let jobs = backend.pop(2).await.unwrap();
        backend.complete(&jobs[0].id).await.unwrap();
        backend.discard(&jobs[1].id).await.unwrap();

        let stats = backend.stats().await.unwrap();
        assert_eq!(stats.processed, 1, "discard must not count as processed");
        assert_eq!(stats.processing, 0, "discard must release the claim");
        assert_eq!(stats.pending, 0);
    }

    /// A job claimed by `pop` and never reported on must be recoverable: the
    /// `:processing` set documented a sweeper that did not exist.
    #[tokio::test]
    async fn a_claimed_job_is_reclaimed_after_the_visibility_timeout() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let jobs = backend.pop(1).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(backend.stats().await.unwrap().processing, 1);

        // Not yet stale: a long timeout must not steal an in-flight job.
        assert_eq!(
            backend
                .reclaim_stale(Duration::from_secs(300))
                .await
                .unwrap(),
            0
        );
        assert_eq!(backend.stats().await.unwrap().pending, 0);

        // Past the timeout (zero, so the claim already qualifies) it comes back.
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);

        let stats = backend.stats().await.unwrap();
        assert_eq!(stats.processing, 0, "claim must be released: {stats:?}");
        assert_eq!(stats.pending, 1, "job must be re-queued: {stats:?}");

        // And it is genuinely re-deliverable, with the same id.
        let again = backend.pop(1).await.unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, jobs[0].id);
    }

    /// A completed job is not resurrected by a later sweep.
    #[tokio::test]
    async fn reclaim_ignores_jobs_that_already_reported_back() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let jobs = backend.pop(1).await.unwrap();
        backend.complete(&jobs[0].id).await.unwrap();

        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 0);
        assert_eq!(backend.stats().await.unwrap().pending, 0);
    }

    /// The in-memory queue and dead-letter list were unbounded `Vec`s that were
    /// only ever pushed to, each entry retaining an `Arc<Email>` with full
    /// attachments.
    #[tokio::test]
    async fn the_in_memory_backend_is_bounded() {
        let backend = InMemoryBackend::with_max_depth(2);

        backend.push(EmailJob::new(queued_email())).await.unwrap();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let err = backend
            .push(EmailJob::new(queued_email()))
            .await
            .expect_err("push past max_queue_depth must fail");
        assert!(matches!(err, MailError::Queue(_)), "{err}");

        // The dead-letter list evicts rather than rejects, so a new permanent
        // failure is never dropped in favour of an older one.
        for _ in 0..5 {
            backend
                .dead_letter(EmailJob::new(queued_email()))
                .await
                .unwrap();
        }
        assert_eq!(backend.stats().await.unwrap().dead_letter, 2);
        assert_eq!(backend.dead_letter_jobs().await.len(), 2);
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
