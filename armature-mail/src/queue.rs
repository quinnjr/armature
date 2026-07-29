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
//! // The config is validated here: `visibility_timeout` must clear
//! // `job_timeout` by more than 2x, or every slow send is redelivered.
//! let queue = EmailQueue::in_memory(EmailQueueConfig::default())?;
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
//! // mailer so retries do not deep-copy the email. Construct it with
//! // `.retries(0)`: the queue owns retry/backoff decisions via
//! // `EmailJob::attempts` and the provider's `Retry-After`, and a mailer with
//! // its own internal retry loop stacks those retries *inside* `job_timeout`,
//! // discarding `Retry-After` fidelity and multiplying real provider attempts
//! // per queue-level attempt.
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
    /// Shared rather than owned so a retry is a refcount bump, not a deep copy
    /// of every attachment.
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
    /// Fencing token for the claim currently held on this job, if any.
    ///
    /// Minted fresh by whichever backend operation claims the job (`pop`, or a
    /// backend's internal re-claim during `reclaim_stale`) and required back by
    /// `complete`/`discard`/`fail`/`dead_letter` before they will release it.
    /// Without this, ownership was decided by id membership alone: a job
    /// reclaimed and re-popped by a second worker reused the same id, so a
    /// finalize call from the first worker's now-stale claim was
    /// indistinguishable from one for the second worker's live claim and could
    /// destroy it. Never persisted — the authoritative value lives with the
    /// backend (the Redis `:claim` hash, or the in-memory processing map), and
    /// a claim always overwrites whatever value a job happened to carry.
    #[serde(skip)]
    pub claim_token: u64,
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
            claim_token: 0,
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
    /// See [`EmailQueueBackend::reclaim_stale`]. Must be more than twice
    /// [`Self::job_timeout`]: a job that is merely slow, not lost, would
    /// otherwise be redelivered while its first attempt is still in flight.
    /// [`Self::validate`] enforces this, and every queue constructor calls it.
    pub visibility_timeout: Duration,
    /// Maximum number of jobs [`InMemoryBackend`] will hold in its queue, and
    /// separately in its dead-letter list.
    ///
    /// Both retain an `Arc<Email>` with full attachments, so neither may grow
    /// without bound. Ignored by [`RedisBackend`], whose bound is Redis's own
    /// memory policy.
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

    /// Reject a configuration the queue cannot honor.
    ///
    /// [`Self::visibility_timeout`] must be more than twice
    /// [`Self::job_timeout`]. The sweeper
    /// ([`EmailQueueBackend::reclaim_stale`]) assumes a claim older than the
    /// visibility timeout belongs to a dead worker; if a *live* worker is still
    /// allowed to hold a job for `job_timeout`, a visibility timeout at or below
    /// that redelivers every slow email while its first attempt is still in
    /// flight, and each redelivery burns an attempt until the job dead-letters.
    /// The factor of two leaves room for the poll, the dispatch, and the sweep
    /// interval on top of the send itself.
    ///
    /// Called by [`EmailQueue::in_memory`], [`EmailQueue::redis`],
    /// [`EmailQueue::with_backend`], and [`EmailQueueWorker::run`], so an
    /// unsupportable configuration surfaces as an error rather than as
    /// duplicate mail.
    pub fn validate(&self) -> Result<()> {
        if self.visibility_timeout <= self.job_timeout * 2 {
            return Err(MailError::Config(format!(
                "visibility_timeout ({:?}) must be more than twice job_timeout ({:?}); \
                 otherwise every send slower than the visibility timeout is redelivered \
                 while its first attempt is still in flight",
                self.visibility_timeout, self.job_timeout
            )));
        }
        // `batch_size == 0` makes every `pop` a silent no-op forever: the
        // worker looks alive (it polls and sleeps on schedule) but never
        // fetches a job. `concurrency == 0` caps the send semaphore at zero
        // permits, so a job that *is* popped claims a slot and then blocks
        // forever waiting for a permit that will never be issued — stuck
        // exactly where the sweep would otherwise reclaim it after
        // `visibility_timeout`, except the queue never stops "running" to
        // surface the problem. Both are past the point this check exists to
        // catch bad config instead of a queue that silently never drains.
        if self.batch_size == 0 {
            return Err(MailError::Config(
                "batch_size must be at least 1; a queue that never fetches jobs cannot make \
                 progress"
                    .to_string(),
            ));
        }
        if self.concurrency == 0 {
            return Err(MailError::Config(
                "concurrency must be at least 1; a queue with no workers claims jobs it can \
                 never finish, and the sweep loop that would recover them never runs either"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// Email queue backend trait.
///
/// # Claim fencing
///
/// [`Self::pop`] and a backend's own re-claim during [`Self::reclaim_stale`]
/// each mint a fresh claim token into the returned/reclaimed
/// [`EmailJob::claim_token`]. [`Self::complete`] and [`Self::discard`] must be
/// called with that same token, and [`Self::fail`]/[`Self::dead_letter`] read
/// it off the `EmailJob` they are given; every one of the four MUST verify the
/// presented token still matches the current claim before releasing it, and
/// treat a mismatch — not just a missing claim — as "not owned" and no-op.
///
/// Id membership alone is not enough: a job reclaimed by the sweeper and
/// re-popped by a second worker reuses the same id for its new claim, so a
/// finalize call arriving late from the *first* worker's now-stale claim is
/// indistinguishable, by id alone, from one for the second worker's live
/// claim, and can destroy it — losing the job if the destroying call is
/// `fail`/`dead_letter`, since the live claim's own eventual finalize then
/// finds nothing to act on.
#[async_trait::async_trait]
pub trait EmailQueueBackend: Send + Sync {
    /// Push a job to the queue.
    async fn push(&self, job: EmailJob) -> Result<()>;

    /// Push several jobs at once, all-or-nothing.
    ///
    /// The default implementation pushes them one at a time — NOT
    /// all-or-nothing, since an error partway through leaves a prefix already
    /// durably pushed — and is only correct for a backend where `push` cannot
    /// partially fail (there is no such backend in this crate; both
    /// [`InMemoryBackend`] and [`RedisBackend`] override this with a real
    /// atomic implementation). Backends that can batch (e.g. Redis pipelining)
    /// should override this to avoid paying the per-job round-trip cost N
    /// times, and must keep the all-or-nothing property: a caller that retries
    /// a whole batch after seeing `Err` must not risk double-enqueuing a
    /// prefix that actually landed.
    async fn push_batch(&self, jobs: Vec<EmailJob>) -> Result<()> {
        for job in jobs {
            self.push(job).await?;
        }
        Ok(())
    }

    /// Pop jobs from the queue.
    ///
    /// Each returned [`EmailJob`] carries a freshly minted
    /// [`EmailJob::claim_token`] — see the trait-level "Claim fencing" section.
    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>>;

    /// Mark a job as successfully sent.
    ///
    /// Releases the claim taken by [`Self::pop`], deletes the job body, and
    /// increments [`QueueStats::processed`]. Only call this for a job that was
    /// actually delivered — see [`Self::discard`] for the failure case.
    /// `claim_token` must be the value [`Self::pop`] (or a reclaim) minted for
    /// this job; a stale or mismatched token is a no-op, not an error — see the
    /// trait-level "Claim fencing" section.
    async fn complete(&self, job_id: &str, claim_token: u64) -> Result<()>;

    /// Release a claim and delete a job *without* counting it as processed.
    ///
    /// For a job that will never be sent and has nowhere to go — a permanent
    /// failure with the dead-letter queue disabled. `complete` was used for this,
    /// which incremented the success counter with failures, so
    /// [`QueueStats::processed`] could not back a delivery-rate alert: a run
    /// where every send failed reported 100% processed. `claim_token` is
    /// verified the same way as in [`Self::complete`].
    async fn discard(&self, job_id: &str, claim_token: u64) -> Result<()>;

    /// Mark a job as failed and schedule retry.
    ///
    /// `job.claim_token` is verified the same way as in [`Self::complete`].
    async fn fail(&self, job: EmailJob, error: &str) -> Result<()>;

    /// Move a job to the dead letter queue.
    ///
    /// `job.claim_token` is verified the same way as in [`Self::complete`].
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
    /// can deduplicate, and [`EmailQueueConfig::validate`] rejects a
    /// `visibility_timeout` that is not comfortably above
    /// [`EmailQueueConfig::job_timeout`].
    ///
    /// # A reclaim counts as an attempt
    ///
    /// Implementations MUST increment [`EmailJob::attempts`] on the reclaimed
    /// job and route it to the dead-letter queue once
    /// [`EmailJob::should_retry`] is false — **and must respect
    /// [`EmailQueueConfig::dead_letter_queue`] when doing so**, discarding
    /// (dropping without a durable record) exactly as the normal worker
    /// exhaustion path does when the dead-letter queue is disabled, rather than
    /// writing to it unconditionally. A job that kills its worker — OOM on a
    /// large attachment, a transport panic, a `job_timeout` outliving the
    /// visibility timeout — is otherwise redelivered with `attempts` frozen, so
    /// it never reaches the dead-letter queue and is re-sent every
    /// `visibility_timeout` forever. If the send had actually succeeded before
    /// the crash, that is an unbounded stream of duplicate email.
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
///
/// # Locking
///
/// All three collections — the pending queue, the processing claims, and the
/// dead-letter list — live behind **one** `Mutex`. They were three, and the
/// operations disagreed about the order to take them in: `pop` took
/// `queue → processing`, `reclaim_stale` took them the other way round (`fail`
/// never held both at once — its `processing` lock was an unnamed temporary,
/// released at the end of the statement that took it, before `queue` was ever
/// locked). `tokio::sync::Mutex` is FIFO and non-reentrant, so a worker with
/// `concurrency > 1` whose sweep interval landed while the poll loop sat in
/// `pop` parked forever — no error, no log, no panic. A single lock makes that
/// class of bug unrepresentable; do not split it back apart.
pub struct InMemoryBackend {
    state: tokio::sync::Mutex<InMemoryState>,
    processed: std::sync::atomic::AtomicU64,
    /// Cap on the queue and, separately, on the dead-letter list.
    max_depth: usize,
    /// Mirrors [`EmailQueueConfig::dead_letter_queue`]; `reclaim_stale` must
    /// honor it the same way the normal worker-exhaustion path does, rather
    /// than dead-lettering unconditionally.
    dead_letter_queue_enabled: bool,
}

/// Everything `InMemoryBackend` guards, under a single lock.
#[derive(Default)]
struct InMemoryState {
    queue: std::collections::VecDeque<EmailJob>,
    /// Jobs handed out by `pop` and not yet completed, failed, or dead-lettered,
    /// keyed by id and carrying the claim time, the fencing token minted for
    /// this claim, and the job itself.
    ///
    /// The job is retained so `reclaim_stale` can actually put it back: `pop`
    /// removed it from `queue`, so without a copy here a crashed worker's job is
    /// unrecoverable. The token is compared against whatever `complete`/
    /// `discard`/`fail`/`dead_letter` are called with — see
    /// [`EmailQueueBackend`]'s "Claim fencing" section — so a finalize call
    /// from a claim this entry has since superseded is rejected as stale
    /// rather than tearing down the current one.
    processing: std::collections::HashMap<String, (i64, u64, EmailJob)>,
    /// Bounded — every retained job holds an `Arc<Email>` with full
    /// attachments, so a steadily failing queue must not grow it without limit.
    dead_letter: std::collections::VecDeque<EmailJob>,
    /// Monotonic source for claim fencing tokens. `0` is reserved to mean
    /// "unclaimed" ([`EmailJob::new`]'s default), so the first real token is 1.
    claim_seq: u64,
}

impl InMemoryState {
    /// Mint the next claim fencing token.
    fn next_claim_token(&mut self) -> u64 {
        self.claim_seq += 1;
        self.claim_seq
    }
}

impl InMemoryState {
    /// Append to the dead-letter list, evicting the oldest entry at the cap.
    ///
    /// Evict rather than reject: a full DLQ must not start silently dropping the
    /// failures it exists to record.
    fn push_dead_letter(&mut self, job: EmailJob, max_depth: usize) {
        while self.dead_letter.len() >= max_depth {
            if let Some(evicted) = self.dead_letter.pop_front() {
                warn!(
                    job_id = %evicted.id,
                    "Dead letter queue full; evicting the oldest entry"
                );
            }
        }
        self.dead_letter.push_back(job);
    }
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
            state: tokio::sync::Mutex::new(InMemoryState::default()),
            processed: std::sync::atomic::AtomicU64::new(0),
            max_depth: max_depth.max(1),
            dead_letter_queue_enabled: true,
        }
    }

    /// Set whether `reclaim_stale` dead-letters exhausted jobs (default: `true`,
    /// matching [`EmailQueueConfig::default`]'s `dead_letter_queue`).
    ///
    /// When `false`, a job that exhausts its retries through repeated reclaims
    /// is discarded — dropped without a durable record — exactly as the normal
    /// worker-exhaustion path already does with the dead-letter queue disabled,
    /// rather than being written to it regardless of this setting.
    pub fn dead_letter_queue(mut self, enabled: bool) -> Self {
        self.dead_letter_queue_enabled = enabled;
        self
    }

    /// Snapshot of the dead-letter list, oldest first.
    pub async fn dead_letter_jobs(&self) -> Vec<EmailJob> {
        self.state
            .lock()
            .await
            .dead_letter
            .iter()
            .cloned()
            .collect()
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
        let mut state = self.state.lock().await;
        if state.queue.len() >= self.max_depth {
            return Err(MailError::Queue(format!(
                "in-memory queue is full ({} jobs, max_queue_depth = {}); \
                 use the Redis backend for a real backlog",
                state.queue.len(),
                self.max_depth
            )));
        }
        state.queue.push_back(job);
        Ok(())
    }

    /// All-or-nothing: checked against the whole batch under one lock
    /// acquisition, rather than the default trait implementation's per-job
    /// loop, which could push a prefix of the batch before hitting
    /// `max_queue_depth` partway through and returning `Err` — a caller that
    /// retries the whole batch after that would double-enqueue the prefix.
    async fn push_batch(&self, jobs: Vec<EmailJob>) -> Result<()> {
        if jobs.is_empty() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        if state.queue.len() + jobs.len() > self.max_depth {
            return Err(MailError::Queue(format!(
                "in-memory queue cannot hold {} more jobs ({} already queued, \
                 max_queue_depth = {}); push_batch is all-or-nothing, so none of this batch was \
                 enqueued",
                jobs.len(),
                state.queue.len(),
                self.max_depth
            )));
        }
        state.queue.extend(jobs);
        Ok(())
    }

    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>> {
        let mut state = self.state.lock().await;
        let now = chrono_now_ms();
        let mut jobs = Vec::with_capacity(count);

        let mut i = 0;
        while i < state.queue.len() && jobs.len() < count {
            if let Some(next_retry) = state.queue[i].next_retry_at
                && next_retry > now
            {
                i += 1;
                continue;
            }
            if let Some(job) = state.queue.remove(i) {
                jobs.push(job);
            }
        }

        // Claim the popped jobs so `QueueStats::processing` is observable, a
        // job in flight is distinguishable from one that was never popped, and
        // `reclaim_stale` has both a claim time and a job body to work from.
        // Each claim mints a fresh fencing token — see [`EmailQueueBackend`]'s
        // "Claim fencing" section for why id membership alone is not enough.
        let claimed_at = chrono_now_ms();
        for job in &mut jobs {
            let token = state.next_claim_token();
            job.claim_token = token;
            state
                .processing
                .insert(job.id.clone(), (claimed_at, token, job.clone()));
        }

        Ok(jobs)
    }

    async fn complete(&self, job_id: &str, claim_token: u64) -> Result<()> {
        // Only count a job the caller still owns: present in `processing`
        // *and* under the same claim token it was handed. A token mismatch
        // means a reclaim-then-repop already gave this id to a newer claim,
        // which this call must not tear down — see the trait's "Claim
        // fencing" section.
        let mut state = self.state.lock().await;
        let still_owned = matches!(
            state.processing.get(job_id),
            Some((_, token, _)) if *token == claim_token
        );
        if !still_owned {
            debug!(
                job_id = %job_id,
                "Completed an email job whose claim was stale (already reclaimed or superseded); \
                 the current claim will be redelivered"
            );
            return Ok(());
        }
        state.processing.remove(job_id);
        self.processed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn discard(&self, job_id: &str, claim_token: u64) -> Result<()> {
        // Deliberately does not touch `processed`: this job was never sent.
        let mut state = self.state.lock().await;
        let still_owned = matches!(
            state.processing.get(job_id),
            Some((_, token, _)) if *token == claim_token
        );
        if still_owned {
            state.processing.remove(job_id);
        }
        Ok(())
    }

    async fn fail(&self, mut job: EmailJob, error: &str) -> Result<()> {
        let mut state = self.state.lock().await;
        // Ownership check, as in `complete`: a job whose claim has since been
        // superseded must not be re-queued a second time here, or one crashed
        // send becomes two live copies of the same email.
        let still_owned = matches!(
            state.processing.get(&job.id),
            Some((_, token, _)) if *token == job.claim_token
        );
        if !still_owned {
            debug!(job_id = %job.id, "Failed an email job whose claim was stale");
            return Ok(());
        }
        state.processing.remove(&job.id);
        job.last_error = Some(error.to_string());
        state.queue.push_back(job);
        Ok(())
    }

    async fn dead_letter(&self, job: EmailJob) -> Result<()> {
        let mut state = self.state.lock().await;
        let still_owned = matches!(
            state.processing.get(&job.id),
            Some((_, token, _)) if *token == job.claim_token
        );
        if !still_owned {
            debug!(
                job_id = %job.id,
                "Dead-lettered an email job whose claim was stale"
            );
            return Ok(());
        }
        state.processing.remove(&job.id);
        state.push_dead_letter(job, self.max_depth);
        Ok(())
    }

    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64> {
        let cutoff = chrono_now_ms() - visibility_timeout.as_millis() as i64;

        let mut state = self.state.lock().await;
        let mut stale: Vec<String> = state
            .processing
            .iter()
            .filter(|(_, (claimed_at, _, _))| *claimed_at <= cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        if stale.is_empty() {
            return Ok(0);
        }
        // `HashMap` iteration order is arbitrary; sort so a deferral at the cap
        // reclaims oldest-claim-first and is deterministic.
        stale.sort_unstable_by_key(|id| state.processing[id].0);

        let mut reclaimed = 0u64;
        let mut deferred = 0u64;
        for id in stale {
            let Some((claimed_at, old_token, job)) = state.processing.remove(&id) else {
                continue;
            };

            // The reclaim IS the attempt. Without this a job that kills its
            // worker comes back with `attempts` frozen, so `should_retry` never
            // goes false and it is re-sent every visibility timeout forever.
            let mut reclaimed_job = job.clone();
            reclaimed_job.attempts += 1;
            // Due immediately: the previous attempt already served as the delay.
            reclaimed_job.next_retry_at = None;
            // Going back to `queue` unclaimed; the next `pop` mints a fresh
            // token. This job's old claim is fully released the moment it
            // leaves `processing` below, so a finalize call that arrives late
            // from the worker that held it is rejected as stale by the token
            // check in `complete`/`discard`/`fail`/`dead_letter`.
            reclaimed_job.claim_token = 0;

            if !reclaimed_job.should_retry() {
                if self.dead_letter_queue_enabled {
                    warn!(
                        job_id = %reclaimed_job.id,
                        attempts = reclaimed_job.attempts,
                        "Email job exhausted its attempts through repeated reclaims; dead-lettering"
                    );
                    // The dead-letter list evicts at the cap rather than
                    // rejecting, so this path is never blocked by `max_depth`.
                    state.push_dead_letter(reclaimed_job, self.max_depth);
                } else {
                    error!(
                        job_id = %reclaimed_job.id,
                        attempts = reclaimed_job.attempts,
                        "Email job exhausted its attempts through repeated reclaims and \
                         dead_letter_queue is disabled; dropping without a durable record"
                    );
                }
                reclaimed += 1;
                continue;
            }

            // `push` refuses past the cap, and so must this: the bound would
            // otherwise not be a bound. A reclaimed job is a failure record, so
            // it goes back into the claim map untouched — attempt not spent —
            // rather than being dropped, and the next sweep picks it up once the
            // queue has drained.
            if state.queue.len() >= self.max_depth {
                state.processing.insert(id, (claimed_at, old_token, job));
                deferred += 1;
                continue;
            }

            state.queue.push_back(reclaimed_job);
            reclaimed += 1;
        }

        if deferred > 0 {
            warn!(
                deferred,
                max_queue_depth = self.max_depth,
                "Deferred reclaiming stale email jobs: the queue is at max_queue_depth"
            );
        }
        if reclaimed > 0 {
            warn!(
                count = reclaimed,
                timeout = ?visibility_timeout,
                "Reclaimed email jobs whose worker never reported back"
            );
        }
        Ok(reclaimed)
    }

    async fn stats(&self) -> Result<QueueStats> {
        let state = self.state.lock().await;
        let queue = &state.queue;
        let processing = state.processing.len() as u64;
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
            dead_letter: state.dead_letter.len() as u64,
            processed: self.processed.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}

/// Release the claim and delete the body, but only for the caller presenting
/// the token that currently owns the claim.
///
/// `KEYS = [processing, job, stats, claim]`, `ARGV = [job_id, claim_token]`.
/// Returns 1 when the presented token matched (this caller owned the claim),
/// 0 otherwise — including when `reclaim_stale` re-popped this id under a
/// newer claim in between, which id-only membership in `processing` cannot
/// distinguish from "still ours". See [`EmailQueueBackend`]'s "Claim fencing"
/// section.
#[cfg(feature = "redis")]
const COMPLETE_IF_OWNED: &str = r#"
    if redis.call('HGET', KEYS[4], ARGV[1]) == ARGV[2] then
        redis.call('ZREM', KEYS[1], ARGV[1])
        redis.call('HDEL', KEYS[4], ARGV[1])
        redis.call('DEL', KEYS[2])
        redis.call('HINCRBY', KEYS[3], 'processed', 1)
        return 1
    end
    return 0
"#;

/// As [`COMPLETE_IF_OWNED`] without the `processed` increment.
///
/// `KEYS = [processing, job, claim]`, `ARGV = [job_id, claim_token]`.
#[cfg(feature = "redis")]
const DISCARD_IF_OWNED: &str = r#"
    if redis.call('HGET', KEYS[3], ARGV[1]) == ARGV[2] then
        redis.call('ZREM', KEYS[1], ARGV[1])
        redis.call('HDEL', KEYS[3], ARGV[1])
        redis.call('DEL', KEYS[2])
        return 1
    end
    return 0
"#;

/// Store the updated body and schedule a retry, for the current claim's owner
/// only.
///
/// `KEYS = [processing, job, retry, claim]`, `ARGV = [job_id, body, score,
/// claim_token]`.
#[cfg(feature = "redis")]
const FAIL_IF_OWNED: &str = r#"
    if redis.call('HGET', KEYS[4], ARGV[1]) == ARGV[4] then
        redis.call('ZREM', KEYS[1], ARGV[1])
        redis.call('HDEL', KEYS[4], ARGV[1])
        redis.call('SET', KEYS[2], ARGV[2])
        redis.call('ZADD', KEYS[3], ARGV[3], ARGV[1])
        return 1
    end
    return 0
"#;

/// Move the job to the dead-letter list, for the current claim's owner only.
///
/// `KEYS = [processing, dead_letter, job, claim]`, `ARGV = [job_id, body,
/// claim_token]`.
#[cfg(feature = "redis")]
const DEAD_LETTER_IF_OWNED: &str = r#"
    if redis.call('HGET', KEYS[4], ARGV[1]) == ARGV[3] then
        redis.call('ZREM', KEYS[1], ARGV[1])
        redis.call('HDEL', KEYS[4], ARGV[1])
        redis.call('LPUSH', KEYS[2], ARGV[2])
        redis.call('DEL', KEYS[3])
        return 1
    end
    return 0
"#;

/// As [`DISCARD_IF_OWNED`], for [`RedisBackend::reclaim_stale`]'s missing-body
/// release path: releases the claim without a job body to delete.
///
/// `KEYS = [processing, claim]`, `ARGV = [job_id, claim_token]`.
#[cfg(feature = "redis")]
const RELEASE_IF_OWNED: &str = r#"
    if redis.call('HGET', KEYS[2], ARGV[1]) == ARGV[2] then
        redis.call('ZREM', KEYS[1], ARGV[1])
        redis.call('HDEL', KEYS[2], ARGV[1])
        return 1
    end
    return 0
"#;

/// Take ownership of at most `ARGV[2]` claims older than `ARGV[1]`.
///
/// Re-scoring to `ARGV[3]` (now) *is* the claim: a concurrent sweeper's range
/// query no longer sees them, and the id stays in `:processing` so a crash
/// before the sweeper finalizes simply defers it to the next sweep rather than
/// stranding it. Deliberately does NOT mint a new claim token — the original
/// claim (worker vs. this sweep) is still being arbitrated between exactly two
/// parties at this point, which the existing token continues to do correctly.
/// A fresh token is only needed once the job is later re-popped from `:retry`
/// or `:pending` into a genuinely new claim (see [`CLAIM_RETRY`] /
/// [`CLAIM_PENDING`]), which is the three-party case id-only membership could
/// not distinguish.
///
/// Also returns each id's current claim token (or `"0"`, the reserved
/// "unclaimed" sentinel, if none is recorded — which only happens if a
/// concurrent finalize already released the claim between the `ZRANGEBYSCORE`
/// above and here) so the caller can finalize through the same
/// `*_IF_OWNED` gate every other release path uses, rather than assuming it
/// still owns whatever this script just touched.
///
/// `KEYS = [processing, claim]`, `ARGV = [cutoff, limit, now]`. Returns a flat
/// array `id1, token1, id2, token2, …`. The `LIMIT` is not optional: as a
/// *write* script this cannot be `SCRIPT KILL`ed, so an unbounded batch after
/// a fleet restart blocks single-threaded Redis for the whole backlog with
/// `SHUTDOWN NOSAVE` as the only way out.
#[cfg(feature = "redis")]
const CLAIM_STALE: &str = r#"
    local ids = redis.call('ZRANGEBYSCORE', KEYS[1], 0, ARGV[1], 'LIMIT', 0, ARGV[2])
    local result = {}
    for i = 1, #ids do
        redis.call('ZADD', KEYS[1], ARGV[3], ids[i])
        local token = redis.call('HGET', KEYS[2], ids[i])
        table.insert(result, ids[i])
        if token then
            table.insert(result, token)
        else
            table.insert(result, "0")
        end
    end
    return result
"#;

/// Atomically claim due jobs from the main pending queue.
///
/// `ZPOPMIN` followed by a separate `ZADD` into `:processing` is not a claim:
/// a crash, or any transient error, between the two leaves the id in neither
/// set, unrecoverable by `reclaim_stale` (which only scans `:processing`) —
/// silent, permanent loss with no concurrency required. Doing the pop, the
/// processing-set claim, and the token mint in one `EVAL` closes that window
/// the same way [`CLAIM_RETRY`] already does for the retry side.
///
/// `KEYS = [pending, processing, claim, claim_seq]`, `ARGV = [count, now]`.
/// Returns a flat array `id1, token1, id2, token2, …`.
#[cfg(feature = "redis")]
const CLAIM_PENDING: &str = r#"
    local popped = redis.call('ZPOPMIN', KEYS[1], ARGV[1])
    local result = {}
    local i = 1
    while i <= #popped do
        local id = popped[i]
        local token = redis.call('INCR', KEYS[4])
        redis.call('ZADD', KEYS[2], ARGV[2], id)
        redis.call('HSET', KEYS[3], id, token)
        table.insert(result, id)
        table.insert(result, token)
        i = i + 2
    end
    return result
"#;

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

    /// Hash mapping a claimed job id to the fencing token minted for its
    /// current claim.
    ///
    /// The authoritative source of "who owns this claim right now" — see
    /// [`EmailQueueBackend`]'s "Claim fencing" section. A field is present iff
    /// the id is currently claimed; every claim (`CLAIM_PENDING`,
    /// `CLAIM_RETRY`) `HSET`s it and every release (`COMPLETE_IF_OWNED`,
    /// `DISCARD_IF_OWNED`, `FAIL_IF_OWNED`, `DEAD_LETTER_IF_OWNED`,
    /// `RELEASE_IF_OWNED`) `HDEL`s it in the same script as the `processing`
    /// `ZREM`, so the two never disagree about which ids are claimed.
    fn claim_key(&self) -> String {
        format!("{}:claim", self.config.queue_name)
    }

    /// Monotonic counter minting claim fencing tokens via `INCR`.
    ///
    /// A counter rather than a random value: it is simpler, needs no entropy
    /// source inside a Lua script, and gives claims a natural generation order
    /// for free.
    fn claim_seq_key(&self) -> String {
        format!("{}:claim_seq", self.config.queue_name)
    }

    /// Number of stale claims one `reclaim_stale` `EVAL` may touch.
    ///
    /// The script is a *write* script, so once it is running `SCRIPT KILL`
    /// refuses it and the only recovery is `SHUTDOWN NOSAVE` — which discards
    /// unpersisted jobs. Unbounded, a fleet restart puts the entire backlog past
    /// the cutoff and one `EVAL` blocks single-threaded Redis for the whole of
    /// it: the sweeper added to recover lost mail becomes the mechanism that
    /// loses more. The caller loops until a batch comes back short.
    const RECLAIM_BATCH: usize = 100;

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
            // Every claim release cleans up its `:claim` entry (see
            // `claim_key`'s doc) so the hash never disagrees with `processing`
            // about which ids are claimed, and so it does not grow forever.
            pipe.cmd("HDEL").arg(self.claim_key()).arg(id);
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
    ///
    /// Also mints a fresh claim fencing token per id — see
    /// [`EmailQueueBackend`]'s "Claim fencing" section — since this is a
    /// genuinely new claim, distinct from whatever claim this id may have held
    /// before landing on `:retry`.
    ///
    /// Returns `(id, claim_token)` pairs.
    async fn claim_retry_ids(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        now: f64,
        count: usize,
    ) -> Result<Vec<(String, u64)>> {
        const CLAIM_RETRY: &str = r#"
            local ids = redis.call('ZRANGEBYSCORE', KEYS[1], 0, ARGV[1], 'LIMIT', 0, ARGV[2])
            local result = {}
            for i = 1, #ids do
                redis.call('ZREM', KEYS[1], ids[i])
                redis.call('ZADD', KEYS[2], ARGV[3], ids[i])
                local token = redis.call('INCR', KEYS[4])
                redis.call('HSET', KEYS[3], ids[i], token)
                table.insert(result, ids[i])
                table.insert(result, token)
            end
            return result
        "#;

        let flat: Vec<redis::Value> = redis::cmd("EVAL")
            .arg(CLAIM_RETRY)
            .arg(4)
            .arg(self.retry_key())
            .arg(self.processing_key())
            .arg(self.claim_key())
            .arg(self.claim_seq_key())
            .arg(now)
            .arg(count)
            .arg(now)
            .query_async(conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        Self::pairs_from_flat(flat)
    }

    /// Atomically claim due jobs from the main pending queue.
    ///
    /// See [`CLAIM_PENDING`] for why this must be one script rather than the
    /// `ZPOPMIN` + separate `ZADD` it replaces. Returns `(id, claim_token)`
    /// pairs.
    async fn claim_pending_ids(
        &self,
        conn: &mut impl redis::aio::ConnectionLike,
        now: f64,
        count: usize,
    ) -> Result<Vec<(String, u64)>> {
        let flat: Vec<redis::Value> = redis::cmd("EVAL")
            .arg(CLAIM_PENDING)
            .arg(4)
            .arg(self.pending_key())
            .arg(self.processing_key())
            .arg(self.claim_key())
            .arg(self.claim_seq_key())
            .arg(count)
            .arg(now)
            .query_async(conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        Self::pairs_from_flat(flat)
    }

    /// Decode a flat `[id1, token1, id2, token2, …]` `EVAL` reply into pairs.
    fn pairs_from_flat(flat: Vec<redis::Value>) -> Result<Vec<(String, u64)>> {
        use redis::FromRedisValue;

        let mut pairs = Vec::with_capacity(flat.len() / 2);
        let mut iter = flat.into_iter();
        while let (Some(id), Some(token)) = (iter.next(), iter.next()) {
            let id = String::from_redis_value(id).map_err(|e| MailError::Queue(e.to_string()))?;
            // Tokens travel through Lua/RESP as bulk strings; parse explicitly
            // rather than relying on numeric RESP coercion.
            let token_str =
                String::from_redis_value(token).map_err(|e| MailError::Queue(e.to_string()))?;
            let token: u64 = token_str
                .parse()
                .map_err(|_| MailError::Queue(format!("non-numeric claim token: {token_str}")))?;
            pairs.push((id, token));
        }
        Ok(pairs)
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
    ///
    /// Wrapped in `MULTI`/`EXEC` (`.atomic()`) so the batch is genuinely
    /// all-or-nothing: a plain pipeline applies each command as it arrives, so
    /// a connection reset partway through could leave a prefix of jobs durably
    /// enqueued while this call still returns `Err` — a caller that
    /// reasonably retries the whole batch after that would double-enqueue the
    /// jobs that had already landed.
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
        pipe.atomic();
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
        // Both claims are a single atomic `EVAL` each — range/pop, processing-set
        // claim, and fencing-token mint together — so exactly one caller can win
        // a given id, and a crash between the pop and the claim cannot strand it
        // in neither set. See `CLAIM_RETRY` and `CLAIM_PENDING`.
        let retry_pairs = self.claim_retry_ids(&mut conn, now, count).await?;

        let remaining = count.saturating_sub(retry_pairs.len());
        let pending_pairs = if remaining > 0 {
            self.claim_pending_ids(&mut conn, now, remaining).await?
        } else {
            Vec::new()
        };

        let pairs: Vec<(String, u64)> = retry_pairs.into_iter().chain(pending_pairs).collect();
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch every job body in a single MGET rather than one GET per id: this
        // runs on the worker's hot path, and N round-trips per poll dominates the
        // poll cost once batch_size grows.
        let keys: Vec<String> = pairs.iter().map(|(id, _)| self.job_key(id)).collect();
        let payloads: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let mut jobs = Vec::with_capacity(pairs.len());
        let mut lost: Vec<String> = Vec::new();
        for ((id, token), payload) in pairs.iter().zip(payloads) {
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
            match serde_json::from_str::<EmailJob>(&json) {
                Ok(mut job) => {
                    // Authoritative: the token this claim just minted, not
                    // whatever this job's body happened to carry from a
                    // previous claim (`claim_token` is `#[serde(skip)]`, so a
                    // freshly-pushed job deserializes it as the `0` sentinel
                    // regardless).
                    job.claim_token = *token;
                    jobs.push(job);
                }
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

    async fn complete(&self, job_id: &str, claim_token: u64) -> Result<()> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Everything is conditional on the presented token still matching the
        // current claim. A mismatch means `reclaim_stale` already took this job
        // back *and* it has since been re-popped into a newer live claim;
        // deleting the body here would destroy that claim's payload out from
        // under it, which `pop`'s next reader would misread as a job lost to
        // eviction rather than one already in flight elsewhere.
        let owned: i64 = redis::cmd("EVAL")
            .arg(COMPLETE_IF_OWNED)
            .arg(4)
            .arg(self.processing_key())
            .arg(self.job_key(job_id))
            .arg(self.stats_key())
            .arg(self.claim_key())
            .arg(job_id)
            .arg(claim_token)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        if owned == 0 {
            warn!(
                job_id = %job_id,
                "Email job was delivered but its claim was stale (already reclaimed or \
                 superseded); the current claim will be redelivered (deduplicate on Message-ID)"
            );
            return Ok(());
        }

        debug!(job_id = %job_id, "Email job completed");
        Ok(())
    }

    async fn discard(&self, job_id: &str, claim_token: u64) -> Result<()> {
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        // Same ownership gate as `complete`, minus the `HINCRBY processed`: this
        // job was never delivered, and counting it would inflate the success
        // counter with failures.
        redis::cmd("EVAL")
            .arg(DISCARD_IF_OWNED)
            .arg(3)
            .arg(self.processing_key())
            .arg(self.job_key(job_id))
            .arg(self.claim_key())
            .arg(job_id)
            .arg(claim_token)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        debug!(job_id = %job_id, "Email job discarded without being sent");
        Ok(())
    }

    /// Sweep stale claims in bounded batches.
    ///
    /// Two phases, because the attempt counter lives inside the JSON body and
    /// re-encoding that body in Lua is not safe: Redis's `cjson` cannot tell an
    /// empty object from an empty array, so a round-trip through
    /// `cjson.decode`/`cjson.encode` silently rewrites an `Email`'s empty `cc`,
    /// `bcc`, or `metadata` and the job stops deserializing.
    ///
    /// 1. `CLAIM_STALE` re-scores the batch to *now* inside `:processing`. That
    ///    is the claim: a concurrent sweeper's `ZRANGEBYSCORE … cutoff` no
    ///    longer sees them, and a crash here just leaves them to the next sweep
    ///    one visibility timeout later.
    /// 2. The bodies are read, `attempts` is incremented, and each job is
    ///    finalized by a script that acts only if `ZREM :processing` returns 1 —
    ///    the same ownership test `complete`/`fail`/`dead_letter` use, so
    ///    whichever of the sweeper and the original worker gets there first
    ///    wins and the other becomes a no-op.
    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64> {
        let mut total = 0u64;

        loop {
            let mut conn = self
                .redis
                .get()
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;

            let now = chrono_now_ms();
            let cutoff = now - visibility_timeout.as_millis() as i64;

            let flat: Vec<redis::Value> = redis::cmd("EVAL")
                .arg(CLAIM_STALE)
                .arg(2)
                .arg(self.processing_key())
                .arg(self.claim_key())
                .arg(cutoff)
                .arg(Self::RECLAIM_BATCH)
                .arg(now)
                .query_async(&mut conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;
            let pairs = Self::pairs_from_flat(flat)?;

            if pairs.is_empty() {
                break;
            }

            let batch = pairs.len();
            let keys: Vec<String> = pairs.iter().map(|(id, _)| self.job_key(id)).collect();
            let payloads: Vec<Option<String>> = redis::cmd("MGET")
                .arg(&keys)
                .query_async(&mut conn)
                .await
                .map_err(|e| MailError::Queue(e.to_string()))?;

            let mut reclaimed = 0u64;
            for ((id, token), payload) in pairs.iter().zip(payloads) {
                let Some(json) = payload else {
                    // A missing body has two causes and they need opposite
                    // handling, so do not guess: release the claim and let the
                    // reply say which happened.
                    //
                    // `complete`/`discard` are gated on the presented token
                    // still matching the current claim, so if the owning
                    // worker finished between the re-score and this `MGET` it
                    // already took the claim — our release returns 0 and the
                    // email was *delivered*. If the claim is still ours
                    // (returns 1) nobody completed it, so the body was evicted
                    // under memory pressure and the email is genuinely lost.
                    // Reporting the first as a loss is the fabricated failure
                    // this sweeper was fixed to stop emitting; staying silent
                    // about the second re-hides the loss that `discard_lost`
                    // exists to record.
                    let owned: i64 = redis::cmd("EVAL")
                        .arg(RELEASE_IF_OWNED)
                        .arg(2)
                        .arg(self.processing_key())
                        .arg(self.claim_key())
                        .arg(id)
                        .arg(token)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| MailError::Queue(e.to_string()))?;

                    if owned == 1 {
                        let stub = serde_json::json!({
                            "id": id,
                            "error": "job body evicted from Redis while claimed",
                            "lost_at": chrono_now_ms(),
                        })
                        .to_string();

                        let mut pipe = redis::pipe();
                        pipe.cmd("LPUSH").arg(self.lost_key()).arg(&stub);
                        if self.config.dead_letter_queue {
                            pipe.cmd("LPUSH").arg(self.dead_letter_key()).arg(&stub);
                        }
                        pipe.query_async::<()>(&mut conn)
                            .await
                            .map_err(|e| MailError::Queue(e.to_string()))?;

                        error!(
                            job_id = %id,
                            dead_lettered = self.config.dead_letter_queue,
                            "Claimed email job's body is gone and no worker completed it; \
                             recording as lost"
                        );
                    } else {
                        debug!(
                            job_id = %id,
                            "Stale claim released; the owning worker had already completed it, \
                             or a newer claim has since superseded it"
                        );
                    }
                    continue;
                };

                let mut job: EmailJob = match serde_json::from_str(&json) {
                    Ok(job) => job,
                    Err(e) => {
                        error!(job_id = %id, error = %e, "Stale email job body is not deserializable");
                        continue;
                    }
                };

                // The reclaim IS the attempt. Without this a job that kills its
                // worker returns with `attempts` frozen, never reaches the
                // dead-letter queue, and is re-sent every visibility timeout
                // forever — an unbounded stream of duplicates if the original
                // send had in fact succeeded.
                job.attempts += 1;
                job.next_retry_at = Some(now);
                let body = serde_json::to_string(&job)?;

                let acted: i64 = if job.should_retry() {
                    redis::cmd("EVAL")
                        .arg(FAIL_IF_OWNED)
                        .arg(4)
                        .arg(self.processing_key())
                        .arg(self.job_key(id))
                        .arg(self.retry_key())
                        .arg(self.claim_key())
                        .arg(id)
                        .arg(&body)
                        .arg(now)
                        .arg(token)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| MailError::Queue(e.to_string()))?
                } else if self.config.dead_letter_queue {
                    warn!(
                        job_id = %id,
                        attempts = job.attempts,
                        "Email job exhausted its attempts through repeated reclaims; dead-lettering"
                    );
                    redis::cmd("EVAL")
                        .arg(DEAD_LETTER_IF_OWNED)
                        .arg(4)
                        .arg(self.processing_key())
                        .arg(self.dead_letter_key())
                        .arg(self.job_key(id))
                        .arg(self.claim_key())
                        .arg(id)
                        .arg(&body)
                        .arg(token)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| MailError::Queue(e.to_string()))?
                } else {
                    error!(
                        job_id = %id,
                        attempts = job.attempts,
                        "Email job exhausted its attempts through repeated reclaims and \
                         dead_letter_queue is disabled; dropping without a durable record"
                    );
                    redis::cmd("EVAL")
                        .arg(DISCARD_IF_OWNED)
                        .arg(3)
                        .arg(self.processing_key())
                        .arg(self.job_key(id))
                        .arg(self.claim_key())
                        .arg(id)
                        .arg(token)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| MailError::Queue(e.to_string()))?
                };

                if acted == 1 {
                    reclaimed += 1;
                }
            }

            total += reclaimed;

            if batch < Self::RECLAIM_BATCH {
                break;
            }
            // Give the runtime — and Redis — room between batches rather than
            // monopolising both until the whole backlog is swept.
            tokio::task::yield_now().await;
        }

        if total > 0 {
            warn!(
                count = total,
                timeout = ?visibility_timeout,
                "Reclaimed email jobs whose worker never reported back"
            );
        }

        Ok(total)
    }

    async fn fail(&self, mut job: EmailJob, error: &str) -> Result<()> {
        job.last_error = Some(error.to_string());

        let job_json = serde_json::to_string(&job)?;
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        let score = job.next_retry_at.unwrap_or_else(chrono_now_ms);

        // Ownership-gated, like `complete`: if the presented token no longer
        // matches the current claim, either the sweeper already reclaimed this
        // id (it is on `:retry` with its own body) or a newer claim now holds
        // it, and writing ours over the top would resurrect a stale attempt
        // count or clobber a live claim.
        let owned: i64 = redis::cmd("EVAL")
            .arg(FAIL_IF_OWNED)
            .arg(4)
            .arg(self.processing_key())
            .arg(self.job_key(&job.id))
            .arg(self.retry_key())
            .arg(self.claim_key())
            .arg(&job.id)
            .arg(&job_json)
            .arg(score)
            .arg(job.claim_token)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        if owned == 0 {
            debug!(job_id = %job.id, "Failed an email job whose claim was stale");
            return Ok(());
        }

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

        let owned: i64 = redis::cmd("EVAL")
            .arg(DEAD_LETTER_IF_OWNED)
            .arg(4)
            .arg(self.processing_key())
            .arg(self.dead_letter_key())
            .arg(self.job_key(&job.id))
            .arg(self.claim_key())
            .arg(&job.id)
            .arg(&job_json)
            .arg(job.claim_token)
            .query_async(&mut conn)
            .await
            .map_err(|e| MailError::Queue(e.to_string()))?;

        if owned == 0 {
            debug!(job_id = %job.id, "Dead-lettered an email job whose claim was stale");
            return Ok(());
        }

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
    ///
    /// Returns [`MailError::Config`] when `config` fails
    /// [`EmailQueueConfig::validate`].
    pub fn in_memory(config: EmailQueueConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            backend: Arc::new(
                InMemoryBackend::with_max_depth(config.max_queue_depth)
                    .dead_letter_queue(config.dead_letter_queue),
            ),
            config,
        })
    }

    /// Create a new email queue with a Redis backend.
    ///
    /// Returns [`MailError::Config`] when `config` fails
    /// [`EmailQueueConfig::validate`].
    #[cfg(feature = "redis")]
    pub fn redis(
        redis: Arc<armature_redis::RedisService>,
        config: EmailQueueConfig,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            backend: Arc::new(RedisBackend::new(redis, config.clone())),
            config,
        })
    }

    /// Create with a custom backend.
    ///
    /// Returns [`MailError::Config`] when `config` fails
    /// [`EmailQueueConfig::validate`].
    pub fn with_backend(
        backend: impl EmailQueueBackend + 'static,
        config: EmailQueueConfig,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            backend: Arc::new(backend),
            config,
        })
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
    ///
    /// Returns immediately, having logged an error, when the configuration
    /// fails [`EmailQueueConfig::validate`] — a worker whose
    /// `visibility_timeout` does not clear its `job_timeout` redelivers every
    /// slow email, and refusing to start is far louder than shipping duplicates.
    pub async fn run(mut self) {
        if let Err(e) = self.config.validate() {
            error!(
                error = %e,
                queue = %self.config.queue_name,
                "Refusing to start the email queue worker: invalid configuration"
            );
            return;
        }

        info!(
            concurrency = self.config.concurrency,
            queue = %self.config.queue_name,
            "Email queue worker started"
        );

        // Caps how many jobs are actually sending at once; claimed-but-waiting
        // jobs (below) are not counted against it. `acquire_owned` rather than
        // `acquire`: the permit is moved into a spawned task and held for the
        // job's full duration, so it must not borrow from this stack frame.
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.concurrency));
        // One task per job rather than a fixed pool of `concurrency` long-lived
        // loops reading off a shared channel: a panic inside a spawned task
        // (e.g. `mailer.send_shared`, or a template/transport bug) is contained
        // by tokio's task boundary and surfaces as a `JoinError` here, instead
        // of unwinding a whole worker loop and silently, permanently shrinking
        // effective concurrency by one with nothing to log it.
        let mut tasks = tokio::task::JoinSet::new();

        // How often the poll loop sweeps for jobs whose worker died mid-send.
        // A quarter of the visibility timeout bounds the extra latency such a
        // job sees at ~25% of the timeout, without sweeping on every poll.
        let sweep_interval = (self.config.visibility_timeout / 4).max(self.config.poll_interval);
        let mut last_sweep = std::time::Instant::now();

        // Main loop: fetch jobs and spawn a task per job
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

            // Drain finished tasks promptly (rather than only at shutdown) so a
            // panic is logged as soon as it happens and `tasks` does not grow
            // without bound across a long-running worker.
            while let Some(result) = tasks.try_join_next() {
                Self::log_task_outcome(result);
            }

            match self.queue.pop(self.config.batch_size).await {
                Ok(jobs) => {
                    if jobs.is_empty() {
                        tokio::time::sleep(self.config.poll_interval).await;
                    } else {
                        for job in jobs {
                            let semaphore = semaphore.clone();
                            let queue = self.queue.clone();
                            let mailer = self.mailer.clone();
                            let config = self.config.clone();
                            tasks.spawn(async move {
                                // Acquire before doing any work, so at most
                                // `concurrency` sends run at once; held until
                                // the job is fully finalized, so a slot only
                                // frees once complete/fail/dead_letter/discard
                                // has actually run. The semaphore is never
                                // closed, so `.await` here cannot fail.
                                let _permit = semaphore.acquire_owned().await;
                                Self::process_one_job(job, queue, mailer, config).await;
                            });
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to fetch jobs from queue");
                    tokio::time::sleep(self.config.poll_interval).await;
                }
            }
        }

        // Drain remaining in-flight jobs rather than abandoning them mid-send;
        // any panic surfaces here instead of being silently discarded.
        while let Some(result) = tasks.join_next().await {
            Self::log_task_outcome(result);
        }

        info!("Email queue worker stopped");
    }

    fn log_task_outcome(result: std::result::Result<(), tokio::task::JoinError>) {
        if let Err(join_err) = result
            && join_err.is_panic()
        {
            error!(
                error = %join_err,
                "An email job's send task panicked; its claim will be recovered by the next \
                 sweep once the visibility timeout elapses"
            );
        }
    }

    async fn process_one_job(
        mut job: EmailJob,
        queue: Arc<dyn EmailQueueBackend>,
        mailer: Arc<Mailer>,
        config: EmailQueueConfig,
    ) {
        debug!(job_id = %job.id, "Processing email job");

        // Bound every send by `job_timeout`: without this a transport that
        // hangs (dead SMTP socket, provider that never responds) holds this
        // permit forever. An elapsed timeout is a retryable failure.
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
        let outcome =
            match tokio::time::timeout(config.job_timeout, mailer.send_shared(job.email.clone()))
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    warn!(
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
                if let Err(e) = queue.complete(&job.id, job.claim_token).await {
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
                    if let Err(err) = queue.discard(&job.id, job.claim_token).await {
                        error!(job_id = %job.id, error = %err, "Failed to discard job");
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

/// Extension trait for Mailer to add queue support.
pub trait MailerQueueExt {
    /// Create an in-memory email queue.
    ///
    /// The queue itself is transport-agnostic — the transport is bound later,
    /// when a worker is created with [`EmailQueue::worker`], not by this call.
    /// The mailer is therefore not consulted here; this method exists purely as
    /// a discoverable entry point from a `Mailer`.
    fn queue(&self, config: EmailQueueConfig) -> Result<EmailQueue>;

    /// Create a Redis-backed email queue.
    #[cfg(feature = "redis")]
    fn queue_redis(
        &self,
        redis: Arc<armature_redis::RedisService>,
        config: EmailQueueConfig,
    ) -> Result<EmailQueue>;
}

impl MailerQueueExt for Mailer {
    fn queue(&self, config: EmailQueueConfig) -> Result<EmailQueue> {
        EmailQueue::in_memory(config)
    }

    #[cfg(feature = "redis")]
    fn queue_redis(
        &self,
        redis: Arc<armature_redis::RedisService>,
        config: EmailQueueConfig,
    ) -> Result<EmailQueue> {
        EmailQueue::redis(redis, config)
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

    /// `MailError::retry_after` was parsed from the provider's `Retry-After`
    /// header and then ignored by both retry paths, so a `Retry-After: 300`
    /// was retried after the local delay and re-throttled.
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
        backend
            .complete(&jobs[0].id, jobs[0].claim_token)
            .await
            .unwrap();
        backend
            .discard(&jobs[1].id, jobs[1].claim_token)
            .await
            .unwrap();

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

        // And it is genuinely re-deliverable, with the same id — and the reclaim
        // counted as an attempt.
        let again = backend.pop(1).await.unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, jobs[0].id);
        assert_eq!(
            again[0].attempts, 1,
            "a reclaim must count as an attempt or a poison job redelivers forever"
        );
    }

    /// A job that kills its worker every time must reach the dead-letter queue.
    ///
    /// `attempts` was bumped only by the worker's `fail` path, which a crashed
    /// worker never reaches, so a reclaimed job came back with `attempts` frozen:
    /// `should_retry()` never went false and the job was re-sent every
    /// visibility timeout forever. If its send had actually succeeded before the
    /// crash, that is an unbounded stream of duplicate email.
    #[tokio::test]
    async fn repeated_reclaims_dead_letter_the_job_instead_of_looping_forever() {
        let backend = InMemoryBackend::new();
        let job = EmailJob::new(queued_email()).max_retries(3);
        let job_id = job.id.clone();
        backend.push(job).await.unwrap();

        // Pop and never report back, over and over — a worker that dies on this
        // job every single time.
        for attempt in 1..=3 {
            let popped = backend.pop(1).await.unwrap();
            assert_eq!(
                popped.len(),
                1,
                "attempt {attempt}: job was not redelivered"
            );
            assert_eq!(popped[0].id, job_id);
            assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
        }

        // Fourth reclaim exhausts `max_retries` and must dead-letter, not requeue.
        let stats = backend.stats().await.unwrap();
        assert_eq!(
            stats.dead_letter, 1,
            "a job reclaimed max_attempts times must end in the DLQ: {stats:?}"
        );
        assert_eq!(
            stats.pending, 0,
            "it must not still be re-queued: {stats:?}"
        );
        assert_eq!(stats.processing, 0);
        assert!(
            backend.pop(1).await.unwrap().is_empty(),
            "the dead-lettered job was redelivered anyway"
        );

        let dead = backend.dead_letter_jobs().await;
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].id, job_id);
        assert_eq!(dead[0].attempts, 3);
    }

    /// `push` refuses past `max_queue_depth`; so must the sweeper. It pushed
    /// unconditionally, so the bound was not a bound.
    #[tokio::test]
    async fn reclaim_respects_max_queue_depth_and_defers_the_rest() {
        let backend = InMemoryBackend::with_max_depth(2);

        backend.push(EmailJob::new(queued_email())).await.unwrap();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        // Claim both, then push two fresh jobs so the queue is back at the cap.
        let claimed = backend.pop(2).await.unwrap();
        assert_eq!(claimed.len(), 2);
        backend.push(EmailJob::new(queued_email())).await.unwrap();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        // Nothing can be reclaimed into a full queue, and nothing may be
        // dropped: a reclaimed job is a failure record.
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 0);
        let stats = backend.stats().await.unwrap();
        assert_eq!(stats.pending, 2, "the cap was breached: {stats:?}");
        assert_eq!(stats.processing, 2, "deferred jobs must be retained");
        assert_eq!(stats.dead_letter, 0);

        // Once the queue drains, the next sweep picks them up.
        for job in backend.pop(2).await.unwrap() {
            backend.complete(&job.id, job.claim_token).await.unwrap();
        }
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 2);
        assert_eq!(backend.stats().await.unwrap().pending, 2);
    }

    /// `batch_size`/`concurrency` of 0 make the queue silently unable to
    /// progress rather than throwing a clear config error at startup.
    #[test]
    fn a_zero_batch_size_or_concurrency_is_rejected() {
        let err = EmailQueueConfig::default()
            .batch_size(0)
            .validate()
            .expect_err("batch_size=0 must be rejected");
        assert!(matches!(err, MailError::Config(_)), "{err}");

        let err = EmailQueueConfig::default()
            .concurrency(0)
            .validate()
            .expect_err("concurrency=0 must be rejected");
        assert!(matches!(err, MailError::Config(_)), "{err}");

        // The default is unaffected.
        EmailQueueConfig::default().validate().unwrap();
    }

    /// A `visibility_timeout` that does not clear `job_timeout` redelivers every
    /// slow email; the invariant was documented on the field and never checked.
    #[test]
    fn a_visibility_timeout_below_the_job_timeout_is_rejected() {
        let bad = EmailQueueConfig::default()
            .job_timeout(Duration::from_secs(600))
            .visibility_timeout(Duration::from_secs(300));
        let err = bad.validate().expect_err("must be rejected");
        assert!(matches!(err, MailError::Config(_)), "{err}");

        // Exactly 2x is still rejected: the sweep interval and the poll sit on
        // top of the send itself.
        assert!(
            EmailQueueConfig::default()
                .job_timeout(Duration::from_secs(150))
                .visibility_timeout(Duration::from_secs(300))
                .validate()
                .is_err()
        );

        // The default is sane, and so is anything comfortably above 2x.
        EmailQueueConfig::default().validate().unwrap();
        EmailQueueConfig::default()
            .job_timeout(Duration::from_secs(100))
            .visibility_timeout(Duration::from_secs(300))
            .validate()
            .unwrap();
    }

    /// Every constructor rejects it, not just `validate` in isolation.
    #[test]
    fn the_constructors_reject_an_unsupportable_configuration() {
        let bad = EmailQueueConfig::default()
            .job_timeout(Duration::from_secs(600))
            .visibility_timeout(Duration::from_secs(300));

        assert!(matches!(
            EmailQueue::in_memory(bad.clone()),
            Err(MailError::Config(_))
        ));
        assert!(matches!(
            EmailQueue::with_backend(InMemoryBackend::new(), bad),
            Err(MailError::Config(_))
        ));
    }

    /// `pop` took `queue → processing` while `reclaim_stale` took them the
    /// other way round (`fail` never held both at once — see
    /// [`InMemoryBackend`]'s locking docs). `tokio::sync::Mutex` is FIFO and
    /// non-reentrant, so with `concurrency > 1` a poll sitting in `pop` and a
    /// concurrent sweep could park each other forever — silently. The three
    /// locks are now one; this test hangs against the old code and is why the
    /// collapse must not be undone. `fail` is still exercised here as a
    /// realistic concurrent caller, even though it was never the operation
    /// that inverted the lock order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_pop_fail_and_reclaim_never_deadlock() {
        let backend = Arc::new(InMemoryBackend::with_max_depth(10_000));
        for _ in 0..200 {
            backend.push(EmailJob::new(queued_email())).await.unwrap();
        }

        let mut handles = Vec::new();
        for _ in 0..8 {
            let backend = backend.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..200 {
                    let jobs = backend.pop(4).await.unwrap();
                    for job in jobs {
                        backend.fail(job, "boom").await.unwrap();
                    }
                    tokio::task::yield_now().await;
                }
            }));
        }
        for _ in 0..4 {
            let backend = backend.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..400 {
                    backend.reclaim_stale(Duration::ZERO).await.unwrap();
                    tokio::task::yield_now().await;
                }
            }));
        }

        tokio::time::timeout(Duration::from_secs(20), async {
            for handle in handles {
                handle.await.unwrap();
            }
        })
        .await
        .expect("pop/fail/reclaim_stale deadlocked on a lock-order inversion");
    }

    /// A completed job is not resurrected by a later sweep.
    #[tokio::test]
    async fn reclaim_ignores_jobs_that_already_reported_back() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let jobs = backend.pop(1).await.unwrap();
        backend
            .complete(&jobs[0].id, jobs[0].claim_token)
            .await
            .unwrap();

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

        // Drain the two queued jobs so the pushes below fit under the cap.
        for job in backend.pop(2).await.unwrap() {
            backend.complete(&job.id, job.claim_token).await.unwrap();
        }

        // The dead-letter list evicts rather than rejects, so a new permanent
        // failure is never dropped in favour of an older one. Each job is
        // pushed and popped first: `dead_letter` acts only for the caller that
        // still holds the claim.
        for _ in 0..5 {
            backend.push(EmailJob::new(queued_email())).await.unwrap();
            let claimed = backend.pop(1).await.unwrap();
            backend.dead_letter(claimed[0].clone()).await.unwrap();
        }
        assert_eq!(backend.stats().await.unwrap().dead_letter, 2);
        assert_eq!(backend.dead_letter_jobs().await.len(), 2);
    }

    /// The core of the claim-fencing fix: a stale claim's finalize call must
    /// not destroy a newer, live claim on the same job id.
    ///
    /// Without a fencing token, ownership was decided by id membership alone.
    /// Worker A's claim is reclaimed by the sweeper and the job is re-popped
    /// by worker B — a fresh claim, same id. If A's `complete` then fires,
    /// id-only membership cannot tell A's now-stale claim apart from B's live
    /// one: it would remove B's entry, and if B's own `fail`/`dead_letter`
    /// follows, its gate would find nothing to act on and the job would
    /// vanish with no trace.
    #[tokio::test]
    async fn a_stale_claims_finalize_does_not_destroy_a_newer_live_claim() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        // Worker A claims it.
        let first_claim = backend.pop(1).await.unwrap();
        assert_eq!(first_claim.len(), 1);
        let stale_token = first_claim[0].claim_token;
        assert_ne!(stale_token, 0, "a popped job must carry a real claim token");

        // The sweeper reclaims it (A was too slow) and worker B re-pops it,
        // minting a fresh claim under the same job id.
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
        let second_claim = backend.pop(1).await.unwrap();
        assert_eq!(second_claim.len(), 1);
        assert_eq!(second_claim[0].id, first_claim[0].id, "same job id");
        assert_ne!(
            second_claim[0].claim_token, stale_token,
            "the re-claim must mint a fresh token, not reuse the stale one"
        );

        // Worker A, unaware it was reclaimed, finally reports back with its
        // now-stale token.
        backend
            .complete(&first_claim[0].id, stale_token)
            .await
            .unwrap();

        // B's live claim must be untouched: still claimed, not completed.
        let stats = backend.stats().await.unwrap();
        assert_eq!(
            stats.processing, 1,
            "A's stale complete must not release B's live claim: {stats:?}"
        );
        assert_eq!(
            stats.processed, 0,
            "A's stale complete must not be counted as a delivery: {stats:?}"
        );

        // B's own finalize, with the correct token, must succeed normally.
        backend
            .complete(&second_claim[0].id, second_claim[0].claim_token)
            .await
            .unwrap();
        let stats = backend.stats().await.unwrap();
        assert_eq!(stats.processing, 0);
        assert_eq!(
            stats.processed, 1,
            "B's genuine completion must count: {stats:?}"
        );
    }

    /// As above, but the stale caller's terminal call is `fail` — the
    /// scenario that previously lost mail outright: A's stale `fail` would
    /// destroy B's live claim, and B's own later finalize would then find
    /// nothing to act on, leaving the job in no set at all.
    #[tokio::test]
    async fn a_stale_claims_fail_does_not_destroy_a_newer_live_claim() {
        let backend = InMemoryBackend::new();
        backend.push(EmailJob::new(queued_email())).await.unwrap();

        let first_claim = backend.pop(1).await.unwrap();
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
        let second_claim = backend.pop(1).await.unwrap();

        // A's stale `fail` must be a no-op, not a resurrection of a dead claim.
        backend
            .fail(first_claim[0].clone(), "stale worker A timed out")
            .await
            .unwrap();

        let stats = backend.stats().await.unwrap();
        assert_eq!(
            stats.processing, 1,
            "A's stale fail must not touch B's live claim: {stats:?}"
        );
        assert_eq!(
            stats.pending, 0,
            "must not requeue a second copy: {stats:?}"
        );

        // B can still legitimately finalize its own live claim afterward.
        backend
            .complete(&second_claim[0].id, second_claim[0].claim_token)
            .await
            .unwrap();
        assert_eq!(backend.stats().await.unwrap().processing, 0);
    }

    /// `push_batch` is all-or-nothing: a batch that would breach
    /// `max_queue_depth` must enqueue none of it, not a prefix a caller's
    /// retry-the-whole-batch would then double up.
    #[tokio::test]
    async fn push_batch_is_all_or_nothing_at_the_depth_cap() {
        let backend = InMemoryBackend::with_max_depth(3);
        let jobs: Vec<EmailJob> = (0..5).map(|_| EmailJob::new(queued_email())).collect();

        let err = backend
            .push_batch(jobs)
            .await
            .expect_err("a batch larger than the cap must be rejected entirely");
        assert!(matches!(err, MailError::Queue(_)), "{err}");

        assert_eq!(
            backend.stats().await.unwrap().pending,
            0,
            "none of the rejected batch may have been enqueued"
        );
    }

    /// `reclaim_stale` must respect `dead_letter_queue(false)` the same way
    /// the normal worker-exhaustion path does: discard without a durable
    /// record, rather than writing to the dead-letter list regardless of the
    /// setting.
    #[tokio::test]
    async fn reclaim_stale_respects_a_disabled_dead_letter_queue() {
        let backend = InMemoryBackend::new().dead_letter_queue(false);
        let job = EmailJob::new(queued_email()).max_retries(0);
        backend.push(job).await.unwrap();

        // `max_retries(0)`: the reclaim's own `attempts += 1` is immediately
        // exhausting, so a single sweep is enough to hit the DLQ-disabled path.
        let popped = backend.pop(1).await.unwrap();
        assert_eq!(popped.len(), 1);
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);

        let stats = backend.stats().await.unwrap();
        assert_eq!(
            stats.dead_letter, 0,
            "dead_letter_queue(false) must not accumulate a durable record: {stats:?}"
        );
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.processing, 0);
    }
}
