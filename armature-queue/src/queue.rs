//! Queue implementation with Redis backend.

use crate::error::{QueueError, QueueResult};
use crate::job::{Job, JobData, JobId, JobPriority, JobState, JobStatus};
use armature_log::{debug, info, warn};
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client, aio::ConnectionManager};
use std::sync::LazyLock;
use std::time::Duration;

/// Number of keys hinted per `SCAN` iteration when clearing the queue.
const SCAN_COUNT: usize = 500;

/// Lua helper mapping a stored job's serialized `JobPriority` to its
/// `pending:<name>` queue suffix and sort score.
///
/// Prepended to every script that has to re-file a job onto a priority queue
/// (delayed promotion and stale-claim reclaim) so the two can never drift
/// apart from each other or from the Rust-side `-(priority as i64)` scoring.
/// `test_lua_priority_mapping_matches_rust` pins it to the Rust definition.
const PRIORITY_LUA: &str = r#"
local function priority_of(job_json)
    local pname = 'normal'
    local pscore = -1
    local ok, job = pcall(cjson.decode, job_json)
    if ok and type(job) == 'table' and job.priority then
        local p = job.priority
        if p == 'Low' then pname = 'low'; pscore = 0
        elseif p == 'Normal' then pname = 'normal'; pscore = -1
        elseif p == 'High' then pname = 'high'; pscore = -2
        elseif p == 'Critical' then pname = 'critical'; pscore = -3
        end
    end
    return pname, pscore
end
"#;

/// Atomically promote all due delayed jobs to their priority queues.
///
/// `KEYS[1]` = delayed sorted-set key, `ARGV[1]` = queue key prefix,
/// `ARGV[2]` = current unix timestamp. Returns `{promoted, dropped}`.
///
/// The whole script runs atomically on the server, so `ZRANGEBYSCORE` +
/// `ZREM` + `ZADD` happen with zero per-job client round-trips and two
/// concurrent workers can never promote the same job twice (the `ZREM == 1`
/// guard is belt-and-suspenders on top of that atomicity). Priority is read
/// from the stored job JSON server-side via `cjson`, matching the client-side
/// `-(priority as i64)` scoring and `pending:<name>` key layout.
///
/// Due ids whose job body is gone are `ZREM`ed rather than skipped: leaving
/// them in place would leak an entry that is rescanned by every subsequent
/// promotion pass forever and permanently inflates `backlog_size()` (and so
/// eventually trips `max_size`). They are counted separately so the caller can
/// surface them.
const MOVE_DELAYED_BODY: &str = r#"
local delayed_key = KEYS[1]
local prefix = ARGV[1]
local now = ARGV[2]

local job_ids = redis.call('ZRANGEBYSCORE', delayed_key, '-inf', now)
local promoted = 0
local dropped = 0

for _, job_id in ipairs(job_ids) do
    local job_json = redis.call('GET', prefix .. ':job:' .. job_id)
    if job_json then
        -- Claim the job atomically; skip if another pass already took it.
        if redis.call('ZREM', delayed_key, job_id) == 1 then
            local pname, pscore = priority_of(job_json)
            redis.call('ZADD', prefix .. ':pending:' .. pname, pscore, job_id)
            promoted = promoted + 1
        end
    elseif redis.call('ZREM', delayed_key, job_id) == 1 then
        -- Body expired before the job came due: it can never run, so drop the
        -- id instead of rescanning it on every future pass.
        dropped = dropped + 1
    end
end

return {promoted, dropped}
"#;

/// Return in-flight jobs whose claim has outlived the visibility timeout to
/// their priority queues.
///
/// `KEYS[1]` = processing sorted-set key, `ARGV[1]` = queue key prefix,
/// `ARGV[2]` = cutoff unix timestamp (claims scored at or before this are
/// stale). Returns `{reclaimed, dropped}`.
///
/// Mirrors the promotion script: the `ZREM == 1` claim guard means two reapers
/// running concurrently can never re-file the same job twice, and ids whose
/// body has TTL-expired are dropped rather than left to accumulate.
const RECLAIM_STALE_BODY: &str = r#"
local processing_key = KEYS[1]
local prefix = ARGV[1]
local cutoff = ARGV[2]

local job_ids = redis.call('ZRANGEBYSCORE', processing_key, '-inf', cutoff)
local reclaimed = 0
local dropped = 0

for _, job_id in ipairs(job_ids) do
    local job_json = redis.call('GET', prefix .. ':job:' .. job_id)
    if job_json then
        if redis.call('ZREM', processing_key, job_id) == 1 then
            local pname, pscore = priority_of(job_json)
            redis.call('ZADD', prefix .. ':pending:' .. pname, pscore, job_id)
            reclaimed = reclaimed + 1
        end
    elseif redis.call('ZREM', processing_key, job_id) == 1 then
        dropped = dropped + 1
    end
end

return {reclaimed, dropped}
"#;

/// [`MOVE_DELAYED_BODY`] with the shared priority helper prepended.
static MOVE_DELAYED_SCRIPT: LazyLock<String> =
    LazyLock::new(|| format!("{PRIORITY_LUA}{MOVE_DELAYED_BODY}"));

/// [`RECLAIM_STALE_BODY`] with the shared priority helper prepended.
static RECLAIM_STALE_SCRIPT: LazyLock<String> =
    LazyLock::new(|| format!("{PRIORITY_LUA}{RECLAIM_STALE_BODY}"));

/// Pop the highest-priority available job id (and its job JSON) across the
/// priority queues, claiming it in the `processing` set in the same step.
///
/// `KEYS` are the priority queue keys in descending priority order
/// (critical, high, normal, low); `ARGV[1]` is the queue key prefix used to
/// build the `prefix:job:<id>` lookup key and the `prefix:processing` claim
/// key, `ARGV[2]` is the current unix timestamp recorded as the claim time.
/// Returns `{id, job_json}`, or `nil` when every queue is empty. Running
/// server-side makes the "check queues high to low, pop the first non-empty,
/// fetch its job body, and if that body has expired keep popping" sequence a
/// single atomic round-trip: it removes the concurrent double-pop window AND
/// folds the client-side `GET` (plus the expired-job retry loop, which
/// previously re-invoked the whole script) into one call.
///
/// The claim `ZADD` lives here rather than in a follow-up pipeline because a
/// crash between "popped from pending" and "recorded in processing" would
/// otherwise lose the job with no trace anywhere for the reaper to find.
const DEQUEUE_POP_SCRIPT: &str = r#"
local prefix = ARGV[1]
local now = ARGV[2]
for i = 1, #KEYS do
    while true do
        local popped = redis.call('ZPOPMIN', KEYS[i], 1)
        if not popped or not popped[1] then
            break
        end
        local job_id = popped[1]
        local job_json = redis.call('GET', prefix .. ':job:' .. job_id)
        if job_json then
            redis.call('ZADD', prefix .. ':processing', now, job_id)
            return {job_id, job_json}
        end
        -- Job body expired between enqueue and dequeue: the id is discarded
        -- (already popped) and we keep draining this same queue.
    end
end
return nil
"#;

/// Queue configuration.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Redis connection URL
    pub redis_url: String,

    /// Queue name
    pub queue_name: String,

    /// Key prefix for Redis keys
    pub key_prefix: String,

    /// Maximum queue size (0 = unlimited)
    pub max_size: usize,

    /// Job retention time for completed jobs
    pub retention_time: Duration,
}

impl QueueConfig {
    /// Create a new queue configuration.
    pub fn new(redis_url: impl Into<String>, queue_name: impl Into<String>) -> Self {
        let queue_name = queue_name.into();
        Self {
            redis_url: redis_url.into(),
            key_prefix: format!("armature:queue:{}", queue_name),
            queue_name,
            max_size: 0,
            retention_time: Duration::from_secs(86400), // 24 hours
        }
    }

    /// Set the key prefix.
    pub fn with_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Set the maximum queue size.
    pub fn with_max_size(mut self, max_size: usize) -> Self {
        self.max_size = max_size;
        self
    }

    /// Set the retention time for completed jobs.
    pub fn with_retention_time(mut self, retention_time: Duration) -> Self {
        self.retention_time = retention_time;
        self
    }

    /// Build Redis key.
    fn key(&self, suffix: &str) -> String {
        format!("{}:{}", self.key_prefix, suffix)
    }

    /// TTL to store a job body under, given how long from now the job is
    /// scheduled to run.
    ///
    /// A flat `retention_time` TTL is wrong for scheduled jobs: promotion only
    /// happens if the body still exists, so any `enqueue_at`/`enqueue_in`
    /// further out than the retention window (24h by default) would silently
    /// expire before coming due and never run. Scheduled jobs therefore get the
    /// wait itself *plus* the full retention window, so `retention_time` keeps
    /// meaning "how long the terminal record survives after the job ran".
    fn body_ttl_secs(&self, wait: Duration) -> u64 {
        self.retention_time
            .as_secs()
            .saturating_add(wait.as_secs())
    }
}

/// How long from now until `scheduled_at`, clamped at zero for past times.
fn wait_until(scheduled_at: Option<DateTime<Utc>>) -> Duration {
    scheduled_at
        .map(|at| Duration::from_secs((at - Utc::now()).num_seconds().max(0) as u64))
        .unwrap_or(Duration::ZERO)
}

/// Job queue backed by Redis.
#[derive(Clone)]
pub struct Queue {
    connection: ConnectionManager,
    config: QueueConfig,
}

impl Queue {
    /// Create a new queue.
    pub async fn new(
        redis_url: impl Into<String>,
        queue_name: impl Into<String>,
    ) -> QueueResult<Self> {
        let config = QueueConfig::new(redis_url, queue_name);
        Self::with_config(config).await
    }

    /// Create a queue with custom configuration.
    pub async fn with_config(config: QueueConfig) -> QueueResult<Self> {
        info!("Initializing job queue: {}", config.queue_name);
        debug!(
            "Queue config - prefix: {}, max_size: {}",
            config.key_prefix, config.max_size
        );

        let client = Client::open(config.redis_url.as_str())
            .map_err(|e| QueueError::Config(e.to_string()))?;

        let connection = ConnectionManager::new(client).await?;

        info!("Job queue '{}' ready", config.queue_name);
        Ok(Self { connection, config })
    }

    /// Enqueue a job.
    pub async fn enqueue(&self, job_type: impl Into<String>, data: JobData) -> QueueResult<JobId> {
        let job_type = job_type.into();
        debug!(
            "Enqueueing job: {} on queue '{}'",
            job_type, self.config.queue_name
        );
        let job = Job::new(&self.config.queue_name, &job_type, data);
        self.enqueue_job(job).await
    }

    /// Enqueue a job to run after a delay.
    ///
    /// Convenience wrapper over [`Job::schedule_after`] + [`enqueue_job`]: the
    /// job lands in the delayed set and is promoted once due.
    ///
    /// [`enqueue_job`]: Self::enqueue_job
    pub async fn enqueue_in(
        &self,
        delay: chrono::Duration,
        job_type: impl Into<String>,
        data: JobData,
    ) -> QueueResult<JobId> {
        let job = Job::new(&self.config.queue_name, job_type, data).schedule_after(delay);
        self.enqueue_job(job).await
    }

    /// Enqueue a job to run at a specific time.
    ///
    /// Convenience wrapper over [`Job::schedule_at`] + [`enqueue_job`]: the job
    /// lands in the delayed set and is promoted once its scheduled time passes.
    ///
    /// [`enqueue_job`]: Self::enqueue_job
    pub async fn enqueue_at(
        &self,
        when: DateTime<Utc>,
        job_type: impl Into<String>,
        data: JobData,
    ) -> QueueResult<JobId> {
        let job = Job::new(&self.config.queue_name, job_type, data).schedule_at(when);
        self.enqueue_job(job).await
    }

    /// Enqueue a job with options.
    pub async fn enqueue_job(&self, job: Job) -> QueueResult<JobId> {
        // Check queue size limit. The cap counts every job occupying the
        // queue -- pending (all priorities), delayed/scheduled, and in-flight
        // `processing` -- not just the ready `pending:*` sets, so scheduled and
        // in-flight jobs cannot silently push the queue past `max_size`.
        if self.config.max_size > 0 {
            let size = self.backlog_size().await?;
            if size >= self.config.max_size {
                return Err(QueueError::QueueFull);
            }
        }

        let job_id = job.id;
        let mut conn = self.connection.clone();

        // Serialize job
        let job_json =
            serde_json::to_string(&job).map_err(|e| QueueError::Serialization(e.to_string()))?;

        // Store job data
        let job_key = self.config.key(&format!("job:{}", job_id));
        let _: () = conn
            .set_ex(
                &job_key,
                job_json,
                self.config.body_ttl_secs(wait_until(job.scheduled_at)),
            )
            .await?;

        // Add to appropriate queue based on priority and schedule
        if job.is_ready() {
            let queue_key = self.priority_queue_key(job.priority);
            let score = -(job.priority as i64); // Negative for high priority first
            let _: () = conn.zadd(&queue_key, job_id.to_string(), score).await?;
        } else {
            // Scheduled job
            let delayed_key = self.config.key("delayed");
            let score = job.scheduled_at.unwrap().timestamp();
            let _: () = conn.zadd(&delayed_key, job_id.to_string(), score).await?;
        }

        Ok(job_id)
    }

    /// Dequeue the next job.
    pub async fn dequeue(&self) -> QueueResult<Option<Job>> {
        self.move_delayed_jobs().await?;

        let mut conn = self.connection.clone();

        // A single Lua pop replaces the previous four sequential ZPOPMIN
        // round-trips PLUS the separate client-side GET: it scans the
        // priority queues high-to-low server-side, atomically pops the first
        // available job id, records the claim in `processing`, and fetches its
        // job JSON in the same round-trip, internally re-draining a queue if a
        // popped id's job body has expired (mirroring the old client-side retry
        // loop with zero extra round-trips).
        let script = redis::Script::new(DEQUEUE_POP_SCRIPT);
        let popped: Option<(String, String)> = script
            .key(self.priority_queue_key(JobPriority::Critical))
            .key(self.priority_queue_key(JobPriority::High))
            .key(self.priority_queue_key(JobPriority::Normal))
            .key(self.priority_queue_key(JobPriority::Low))
            .arg(&self.config.key_prefix)
            .arg(Utc::now().timestamp())
            .invoke_async(&mut conn)
            .await?;

        let Some((job_id_str, job_json)) = popped else {
            return Ok(None);
        };
        // Ids are always UUIDs written by `enqueue_job`; a parse failure here
        // would indicate corrupted queue data, not a normal empty-queue case.
        let job_id = job_id_str
            .parse::<JobId>()
            .map_err(|e| QueueError::Deserialization(e.to_string()))?;
        let mut job: Job = serde_json::from_str(&job_json)
            .map_err(|e| QueueError::Deserialization(e.to_string()))?;

        job.start_processing();
        let job_key = self.config.key(&format!("job:{}", job_id));
        let updated_json =
            serde_json::to_string(&job).map_err(|e| QueueError::Serialization(e.to_string()))?;

        // The `processing` claim was already recorded atomically by the pop
        // script; only the mutated job body has to be written back here. If
        // this write fails (or the process dies before it), the claim is
        // already visible to `reclaim_stale`, so the job is recoverable.
        let _: () = conn
            .set_ex(&job_key, updated_json, self.config.retention_time.as_secs())
            .await?;

        Ok(Some(job))
    }

    /// Complete a job.
    pub async fn complete(&self, job_id: JobId) -> QueueResult<()> {
        // `remove_from_processing` runs unconditionally, even when the job
        // body itself is gone (TTL-expired mid-flight between dequeue and
        // complete): otherwise the id would be orphaned forever in
        // `processing` while this fn still returned `Ok(())`.
        if let Some(mut job) = self.get_job(job_id).await? {
            job.complete();
            let job_key = self.config.key(&format!("job:{}", job_id));
            let processing_key = self.config.key("processing");
            let job_json = serde_json::to_string(&job)
                .map_err(|e| QueueError::Serialization(e.to_string()))?;

            let mut conn = self.connection.clone();
            let _: () = redis::pipe()
                .set_ex(&job_key, job_json, self.config.retention_time.as_secs())
                .ignore()
                .zrem(&processing_key, job_id.to_string())
                .ignore()
                .query_async(&mut conn)
                .await?;
        } else {
            self.remove_from_processing(job_id).await?;
        }
        Ok(())
    }

    /// Fail a job.
    pub async fn fail(&self, job_id: JobId, error: String) -> QueueResult<()> {
        // As in `complete`, `remove_from_processing` must run even when the
        // job body has TTL-expired mid-flight, so the id is never left
        // orphaned in `processing`.
        if let Some(mut job) = self.get_job(job_id).await? {
            job.fail(error);

            let job_key = self.config.key(&format!("job:{}", job_id));
            let processing_key = self.config.key("processing");
            let job_json = serde_json::to_string(&job)
                .map_err(|e| QueueError::Serialization(e.to_string()))?;
            let mut conn = self.connection.clone();

            if job.status.state == JobState::Failed && job.can_retry() {
                // Retry with backoff
                let retry_at = Utc::now() + job.backoff_delay();
                job.scheduled_at = Some(retry_at);
                let job_json = serde_json::to_string(&job)
                    .map_err(|e| QueueError::Serialization(e.to_string()))?;

                // Add to delayed queue. The body TTL has to cover the backoff
                // wait as well as retention, or a long backoff would expire the
                // job before it comes due and it would never be retried.
                let delayed_key = self.config.key("delayed");
                let _: () = redis::pipe()
                    .set_ex(
                        &job_key,
                        job_json,
                        self.config.body_ttl_secs(wait_until(Some(retry_at))),
                    )
                    .ignore()
                    .zadd(&delayed_key, job_id.to_string(), retry_at.timestamp())
                    .ignore()
                    .zrem(&processing_key, job_id.to_string())
                    .ignore()
                    .query_async(&mut conn)
                    .await?;
            } else {
                // Move to dead letter queue
                let dead_key = self.config.key("dead");
                let _: () = redis::pipe()
                    .set_ex(&job_key, job_json, self.config.retention_time.as_secs())
                    .ignore()
                    .zadd(&dead_key, job_id.to_string(), Utc::now().timestamp())
                    .ignore()
                    .zrem(&processing_key, job_id.to_string())
                    .ignore()
                    .query_async(&mut conn)
                    .await?;
            }
        } else {
            self.remove_from_processing(job_id).await?;
        }
        Ok(())
    }

    /// Return a dequeued-but-unprocessed job to its pending priority queue.
    ///
    /// A job popped by [`dequeue`] has already been marked processing (attempt
    /// incremented) and placed in the `processing` set. When the caller cannot
    /// run it after all (e.g. a batch consumer that dequeued the wrong job
    /// type), this puts it back on its priority queue and removes it from
    /// `processing`, undoing the `start_processing` bookkeeping so the requeue
    /// does not burn a retry attempt. Without this the job would be orphaned in
    /// `processing` forever (data loss).
    ///
    /// [`dequeue`]: Self::dequeue
    pub async fn requeue(&self, job: &Job) -> QueueResult<()> {
        let mut job = job.clone();

        // Undo the `start_processing` side effects so the job looks untouched.
        job.status = JobStatus::pending();
        job.started_at = None;
        job.attempts = job.attempts.saturating_sub(1);
        self.save_job(&job).await?;

        let mut conn = self.connection.clone();
        let queue_key = self.priority_queue_key(job.priority);
        let score = -(job.priority as i64);
        let _: () = conn.zadd(&queue_key, job.id.to_string(), score).await?;

        self.remove_from_processing(job.id).await?;
        Ok(())
    }

    /// Get a job by ID.
    pub async fn get_job(&self, job_id: JobId) -> QueueResult<Option<Job>> {
        let mut conn = self.connection.clone();
        let job_key = self.config.key(&format!("job:{}", job_id));

        let job_json: Option<String> = conn.get(&job_key).await?;

        if let Some(json) = job_json {
            let job: Job = serde_json::from_str(&json)
                .map_err(|e| QueueError::Deserialization(e.to_string()))?;
            Ok(Some(job))
        } else {
            Ok(None)
        }
    }

    /// Save a job.
    async fn save_job(&self, job: &Job) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let job_key = self.config.key(&format!("job:{}", job.id));
        let job_json =
            serde_json::to_string(job).map_err(|e| QueueError::Serialization(e.to_string()))?;

        let _: () = conn
            .set_ex(&job_key, job_json, self.config.retention_time.as_secs())
            .await?;
        Ok(())
    }

    /// Get queue size.
    pub async fn size(&self) -> QueueResult<usize> {
        let mut conn = self.connection.clone();

        // Pipeline the four ZCARDs into one round-trip instead of issuing them
        // sequentially.
        let mut pipe = redis::pipe();
        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            pipe.zcard(self.priority_queue_key(priority));
        }

        let counts: Vec<usize> = pipe.query_async(&mut conn).await?;
        Ok(counts.iter().sum())
    }

    /// Total number of jobs occupying the queue, for `max_size` enforcement.
    ///
    /// Unlike [`size`], which counts only ready `pending:*` jobs, this also
    /// counts delayed/scheduled jobs and in-flight `processing` jobs -- every
    /// job that holds a slot against the configured cap. Pipelined into one
    /// round-trip.
    ///
    /// [`size`]: Self::size
    pub async fn backlog_size(&self) -> QueueResult<usize> {
        let mut conn = self.connection.clone();

        let mut pipe = redis::pipe();
        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            pipe.zcard(self.priority_queue_key(priority));
        }
        pipe.zcard(self.config.key("delayed"));
        pipe.zcard(self.config.key("processing"));

        let counts: Vec<usize> = pipe.query_async(&mut conn).await?;
        Ok(counts.iter().sum())
    }

    /// Number of jobs currently in the in-flight `processing` set.
    pub async fn processing_len(&self) -> QueueResult<usize> {
        let mut conn = self.connection.clone();
        let processing_key = self.config.key("processing");
        let count: usize = conn.zcard(&processing_key).await?;
        Ok(count)
    }

    /// Move delayed jobs to ready queue.
    ///
    /// Runs entirely server-side via a single atomic Lua script: the previous
    /// implementation was an N+1 (one ZRANGEBYSCORE plus a GET + ZREM + ZADD
    /// per due job) and could double-promote a job when two workers dequeued
    /// concurrently. The script promotes all due jobs in one round-trip with
    /// zero per-job client traffic and no double-promotion race.
    async fn move_delayed_jobs(&self) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let delayed_key = self.config.key("delayed");
        let now = Utc::now().timestamp();

        // Cheap O(log N) guard on the hot dequeue path: peek the earliest
        // delayed job's score and skip the promotion round-trip entirely unless
        // something is actually due. Previously the full ZRANGEBYSCORE script
        // ran on every single `dequeue()` even when the delayed set was empty
        // or entirely in the future.
        let earliest: Vec<(String, i64)> = conn.zrange_withscores(&delayed_key, 0, 0).await?;
        match earliest.first() {
            Some((_, score)) if *score <= now => {}
            _ => return Ok(()),
        }

        let script = redis::Script::new(&MOVE_DELAYED_SCRIPT);
        let (_promoted, dropped): (i64, i64) = script
            .key(&delayed_key)
            .arg(&self.config.key_prefix)
            .arg(now)
            .invoke_async(&mut conn)
            .await?;

        if dropped > 0 {
            warn!(
                "Dropped {} delayed job(s) from queue '{}' whose body expired before their scheduled time",
                dropped, self.config.queue_name
            );
        }

        Ok(())
    }

    /// Return jobs whose in-flight claim is older than `visibility_timeout` to
    /// their pending priority queues, and report how many were reclaimed.
    ///
    /// `dequeue` records a claim timestamp in the `processing` set, but nothing
    /// else ever reads it back: if a worker crashes, is SIGKILLed, or its
    /// handler task panics, its job is in no pending queue and no retry path
    /// will ever pick it up. This is the reaper that closes that hole, and
    /// [`Worker::start`] runs it periodically in the background.
    ///
    /// `visibility_timeout` must exceed the longest a job may legitimately stay
    /// in flight (i.e. at least `WorkerConfig::job_timeout`), otherwise a job
    /// that is merely slow will be re-filed and run twice. Handlers should be
    /// idempotent regardless, since a crash after the handler's side effects
    /// but before `complete()` is indistinguishable from a crash before them.
    ///
    /// Reclaimed jobs re-enter the queue with their `attempts` counter as the
    /// crashed worker left it, so a job that reliably kills its worker still
    /// exhausts `max_attempts` and lands in the dead-letter set rather than
    /// looping forever.
    ///
    /// [`Worker::start`]: crate::Worker::start
    pub async fn reclaim_stale(&self, visibility_timeout: Duration) -> QueueResult<usize> {
        let mut conn = self.connection.clone();
        let processing_key = self.config.key("processing");
        let cutoff = Utc::now().timestamp() - visibility_timeout.as_secs() as i64;

        let script = redis::Script::new(&RECLAIM_STALE_SCRIPT);
        let (reclaimed, dropped): (i64, i64) = script
            .key(&processing_key)
            .arg(&self.config.key_prefix)
            .arg(cutoff)
            .invoke_async(&mut conn)
            .await?;

        if reclaimed > 0 {
            warn!(
                "Reclaimed {} stale in-flight job(s) on queue '{}' (claim older than {:?})",
                reclaimed, self.config.queue_name, visibility_timeout
            );
        }
        if dropped > 0 {
            warn!(
                "Dropped {} stale in-flight job(s) on queue '{}' whose body had already expired",
                dropped, self.config.queue_name
            );
        }

        Ok(reclaimed as usize)
    }

    /// Remove job from processing set.
    async fn remove_from_processing(&self, job_id: JobId) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let processing_key = self.config.key("processing");
        let _: () = conn.zrem(&processing_key, job_id.to_string()).await?;
        Ok(())
    }

    /// Get the priority queue key.
    fn priority_queue_key(&self, priority: JobPriority) -> String {
        self.config
            .key(&format!("pending:{:?}", priority).to_lowercase())
    }

    /// Clear all jobs from the queue.
    pub async fn clear(&self) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let pattern = format!("{}:*", self.config.key_prefix);

        // Cursored SCAN + UNLINK instead of the blocking KEYS + DEL, so a large
        // queue does not stall the Redis event loop while being cleared.
        let mut cursor: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(SCAN_COUNT)
                .query_async(&mut conn)
                .await?;

            if !keys.is_empty() {
                let _: () = redis::cmd("UNLINK")
                    .arg(&keys)
                    .query_async(&mut conn)
                    .await?;
            }

            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_config() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.queue_name, "test");
        assert!(config.key_prefix.contains("test"));
    }

    /// The Lua scripts hard-code the priority -> (queue name, score) mapping so
    /// they can re-file jobs without a client round-trip. If the Rust
    /// `JobPriority` scoring or the `pending:<name>` key layout ever changes,
    /// this test fails, flagging the now-stale script.
    #[test]
    fn test_lua_priority_mapping_matches_rust() {
        for priority in [
            JobPriority::Low,
            JobPriority::Normal,
            JobPriority::High,
            JobPriority::Critical,
        ] {
            // Rust-side scoring used by enqueue/dequeue.
            let rust_score = -(priority as i64);
            // Serde/Debug variant name the job JSON carries (e.g. "Normal").
            let variant = format!("{priority:?}");
            // Lowercased suffix used by `priority_queue_key`.
            let name = variant.to_lowercase();

            let expected = format!("'{variant}' then pname = '{name}'; pscore = {rust_score}");
            assert!(
                PRIORITY_LUA.contains(&expected),
                "script missing mapping for {priority:?}: expected `{expected}`"
            );
        }
    }

    /// Both re-filing scripts must actually pull in the shared helper, or the
    /// mapping test above would pin a fragment nothing uses.
    #[test]
    fn test_refiling_scripts_share_priority_helper() {
        for script in [&*MOVE_DELAYED_SCRIPT, &*RECLAIM_STALE_SCRIPT] {
            assert!(script.contains("local function priority_of"));
            assert!(script.contains("priority_of(job_json)"));
        }
    }

    /// The delayed set must not accumulate ids whose body has expired: they can
    /// never be promoted, are rescanned by every future pass, and inflate
    /// `backlog_size()` against `max_size` forever.
    #[test]
    fn test_move_delayed_drops_bodyless_ids() {
        assert!(MOVE_DELAYED_BODY.contains("elseif redis.call('ZREM', delayed_key, job_id) == 1"));
    }

    /// The reaper must re-file stale claims, not merely count them.
    #[test]
    fn test_reclaim_script_refiles_to_pending() {
        assert!(RECLAIM_STALE_BODY.contains("ZRANGEBYSCORE"));
        assert!(RECLAIM_STALE_BODY.contains("ZADD', prefix .. ':pending:' .. pname"));
        assert!(RECLAIM_STALE_BODY.contains("ZREM', processing_key"));
    }

    /// Pop and claim have to happen in the same script: a crash in between
    /// would leave the job in no queue and no processing set at all, with
    /// nothing anywhere for the reaper to find.
    #[test]
    fn test_dequeue_script_claims_atomically() {
        assert!(DEQUEUE_POP_SCRIPT.contains("ZADD', prefix .. ':processing', now, job_id"));
    }

    #[test]
    fn test_priority_queue_key_layout() {
        // Confirms the key layout the Lua script reconstructs (`prefix:pending:<name>`).
        let config = QueueConfig::new("redis://localhost:6379", "jobs");
        for (priority, name) in [
            (JobPriority::Low, "low"),
            (JobPriority::Normal, "normal"),
            (JobPriority::High, "high"),
            (JobPriority::Critical, "critical"),
        ] {
            let key = config.key(&format!("pending:{priority:?}").to_lowercase());
            assert_eq!(key, format!("{}:pending:{}", config.key_prefix, name));
        }
    }

    #[test]
    fn test_dequeue_script_scans_all_keys() {
        // The pop script must consider every priority queue passed as KEYS.
        assert!(DEQUEUE_POP_SCRIPT.contains("for i = 1, #KEYS do"));
        assert!(DEQUEUE_POP_SCRIPT.contains("ZPOPMIN"));
    }

    // Backend-dependent: requires a live Redis at redis://localhost:6379.
    #[tokio::test]
    #[ignore = "requires a running Redis instance"]
    async fn test_move_delayed_promotes_due_jobs() {
        use crate::job::Job;

        let queue = Queue::new("redis://localhost:6379", "test_move_delayed")
            .await
            .unwrap();
        queue.clear().await.unwrap();

        // Enqueue a job scheduled in the past -> lands in the delayed set.
        let past = Utc::now() - chrono::Duration::seconds(30);
        let job = Job::new("test_move_delayed", "task", serde_json::json!({}))
            .with_priority(JobPriority::High)
            .schedule_at(past);
        queue.enqueue_job(job).await.unwrap();

        // Delayed jobs are not counted in size() until promoted.
        assert_eq!(queue.size().await.unwrap(), 0);

        // Dequeue triggers move_delayed_jobs; the due job should come back out.
        let dequeued = queue.dequeue().await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().priority, JobPriority::High);

        queue.clear().await.unwrap();
    }

    /// A job scheduled beyond `retention_time` must get a body TTL that
    /// outlives its own schedule, otherwise it expires before it comes due and
    /// silently never runs.
    #[test]
    fn test_body_ttl_covers_long_horizon_schedule() {
        let config = QueueConfig::new("redis://localhost:6379", "test"); // 24h retention
        let retention = config.retention_time.as_secs();

        // Immediate job: plain retention.
        assert_eq!(config.body_ttl_secs(Duration::ZERO), retention);

        // Scheduled 40 days out: TTL must exceed the wait, not fall inside it.
        let wait = Duration::from_secs(40 * 86_400);
        let ttl = config.body_ttl_secs(wait);
        assert!(
            ttl > wait.as_secs(),
            "body would expire {} s before its scheduled time",
            wait.as_secs() - ttl
        );
        assert_eq!(ttl, wait.as_secs() + retention);
    }

    #[test]
    fn test_wait_until_clamps_past_times() {
        // Past-scheduled jobs are already due; they must not get a negative
        // (i.e. wrapping) wait.
        let past = Utc::now() - chrono::Duration::days(3);
        assert_eq!(wait_until(Some(past)), Duration::ZERO);
        assert_eq!(wait_until(None), Duration::ZERO);
        assert!(wait_until(Some(Utc::now() + chrono::Duration::hours(2))) > Duration::from_secs(0));
    }

    // Backend-dependent: requires a live Redis at redis://localhost:6379.
    #[tokio::test]
    #[ignore = "requires a running Redis instance"]
    async fn test_long_horizon_scheduled_job_body_survives() {
        use crate::job::Job;

        let queue = Queue::new("redis://localhost:6379", "test_long_horizon")
            .await
            .unwrap();
        queue.clear().await.unwrap();

        // Scheduled well beyond the 24h default retention time.
        let far_future = Utc::now() + chrono::Duration::days(40);
        let job = Job::new("test_long_horizon", "task", serde_json::json!({}))
            .schedule_at(far_future);
        let job_id = queue.enqueue_job(job).await.unwrap();

        // The body must still be readable and its TTL must outlast the wait,
        // or the promotion script will never find anything to promote.
        assert!(queue.get_job(job_id).await.unwrap().is_some());

        let mut conn = queue.connection.clone();
        let ttl: i64 = conn
            .ttl(queue.config.key(&format!("job:{job_id}")))
            .await
            .unwrap();
        assert!(
            ttl > 40 * 86_400,
            "body TTL {ttl}s expires before the job's scheduled time"
        );

        queue.clear().await.unwrap();
    }

    // Backend-dependent: requires a live Redis at redis://localhost:6379.
    #[tokio::test]
    #[ignore = "requires a running Redis instance"]
    async fn test_reclaim_stale_returns_orphaned_job() {
        use crate::job::Job;

        let queue = Queue::new("redis://localhost:6379", "test_reclaim")
            .await
            .unwrap();
        queue.clear().await.unwrap();

        let job = Job::new("test_reclaim", "task", serde_json::json!({}))
            .with_priority(JobPriority::High);
        let job_id = queue.enqueue_job(job).await.unwrap();

        // Simulate a worker that dequeued the job and then died: the claim is
        // in `processing` and the job is in no pending queue.
        let dequeued = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(dequeued.id, job_id);
        assert_eq!(queue.size().await.unwrap(), 0);
        assert_eq!(queue.processing_len().await.unwrap(), 1);

        // A timeout longer than the claim's age reclaims nothing...
        assert_eq!(
            queue.reclaim_stale(Duration::from_secs(3600)).await.unwrap(),
            0
        );
        assert_eq!(queue.processing_len().await.unwrap(), 1);

        // ...but once the claim is considered stale the job goes back to its
        // own priority queue and becomes dequeueable again.
        assert_eq!(queue.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
        assert_eq!(queue.processing_len().await.unwrap(), 0);
        assert_eq!(queue.size().await.unwrap(), 1);

        let again = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(again.id, job_id);
        assert_eq!(again.priority, JobPriority::High);

        queue.clear().await.unwrap();
    }

    #[test]
    fn test_priority_queue_key() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert!(config.key("pending:high").contains("high"));
    }

    #[test]
    fn test_queue_config_with_custom_prefix() {
        let config = QueueConfig::new("redis://localhost:6379", "myqueue").with_key_prefix("app");
        assert!(config.key_prefix.contains("app"));
    }

    #[test]
    fn test_queue_config_default_retention() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.retention_time, Duration::from_secs(86400)); // 1 day
    }

    #[test]
    fn test_queue_config_custom_retention() {
        let retention = Duration::from_secs(3600);
        let config =
            QueueConfig::new("redis://localhost:6379", "test").with_retention_time(retention);
        assert_eq!(config.retention_time, retention);
    }

    #[test]
    fn test_queue_config_default_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.max_size, 0); // 0 means unlimited
    }

    #[test]
    fn test_queue_config_custom_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test").with_max_size(1000);
        assert_eq!(config.max_size, 1000);
    }

    #[test]
    fn test_queue_key_generation() {
        let config = QueueConfig::new("redis://localhost:6379", "jobs");

        let pending_key = config.key("pending:normal");
        let processing_key = config.key("processing");
        let completed_key = config.key("completed");

        assert!(pending_key.contains("jobs"));
        assert!(processing_key.contains("jobs"));
        assert!(completed_key.contains("jobs"));
    }

    #[test]
    fn test_queue_config_clone() {
        let config1 = QueueConfig::new("redis://localhost:6379", "test");
        let config2 = config1.clone();

        assert_eq!(config1.queue_name, config2.queue_name);
        assert_eq!(config1.redis_url, config2.redis_url);
    }

    #[test]
    fn test_queue_config_different_queues() {
        let config1 = QueueConfig::new("redis://localhost:6379", "queue1");
        let config2 = QueueConfig::new("redis://localhost:6379", "queue2");

        assert_ne!(config1.key_prefix, config2.key_prefix);
    }

    #[test]
    fn test_queue_config_key_consistency() {
        let config = QueueConfig::new("redis://localhost:6379", "test");

        let key1 = config.key("pending");
        let key2 = config.key("pending");

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_queue_config_builder_pattern() {
        let config = QueueConfig::new("redis://localhost:6379", "test")
            .with_key_prefix("app")
            .with_retention_time(Duration::from_secs(7200))
            .with_max_size(500);

        assert!(config.key_prefix.contains("app"));
        assert_eq!(config.retention_time, Duration::from_secs(7200));
        assert_eq!(config.max_size, 500);
    }

    #[test]
    fn test_queue_config_redis_url() {
        let url = "redis://user:pass@host:6380/2";
        let config = QueueConfig::new(url, "test");
        assert_eq!(config.redis_url, url);
    }

    #[test]
    fn test_queue_config_key_with_empty_suffix() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        let key = config.key("");
        assert!(key.contains("test"));
    }

    #[test]
    fn test_queue_config_key_with_special_characters() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        let key = config.key("pending:high:priority");
        assert!(key.contains("pending:high:priority"));
    }

    #[test]
    fn test_queue_config_multiple_prefixes() {
        let config1 =
            QueueConfig::new("redis://localhost:6379", "app1").with_key_prefix("production");
        let config2 =
            QueueConfig::new("redis://localhost:6379", "app2").with_key_prefix("development");

        let key1 = config1.key("jobs");
        let key2 = config2.key("jobs");

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_queue_config_unlimited_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test").with_max_size(0);
        assert_eq!(config.max_size, 0);
    }

    #[test]
    fn test_queue_config_large_retention() {
        let week = Duration::from_secs(7 * 24 * 3600);
        let config = QueueConfig::new("redis://localhost:6379", "test").with_retention_time(week);
        assert_eq!(config.retention_time, week);
    }
}
