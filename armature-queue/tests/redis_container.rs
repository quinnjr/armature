//! Redis-container-backed regression tests for armature-queue.
//!
//! These exercise the real Redis job path (priority ordering, retry/backoff,
//! dead-letter routing, delayed-job promotion, the worker happy path, and the
//! WF3 conformance fixes) against a throwaway Redis container. Every test
//! self-skips when Docker is unavailable, so the default `cargo test` never
//! requires Docker.

use armature_queue::*;
use armature_testkit::containers::RedisContainer;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// ZCARD an arbitrary Redis key on the container (for asserting set membership
/// on keys the public API does not expose, e.g. the dead-letter set).
async fn zcard(url: &str, key: &str) -> usize {
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("ZCARD")
        .arg(key)
        .query_async(&mut conn)
        .await
        .unwrap()
}

/// Poll `get_job` until the job reaches `state`, or panic after `timeout`.
async fn wait_for_state(queue: &Queue, job_id: JobId, state: JobState, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let job = queue.get_job(job_id).await.unwrap().expect("job exists");
        if job.status.state == state {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "job {job_id} did not reach {state:?} within {timeout:?} (last: {:?})",
                job.status.state
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Higher-priority jobs must be dequeued before lower-priority ones regardless
/// of enqueue order.
#[tokio::test]
async fn priority_dequeue_order() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "prio").await.unwrap();
    queue.clear().await.unwrap();

    // Enqueue out of priority order.
    for prio in [
        JobPriority::Low,
        JobPriority::Critical,
        JobPriority::Normal,
        JobPriority::High,
    ] {
        let job = Job::new("prio", "task", json!({})).with_priority(prio);
        queue.enqueue_job(job).await.unwrap();
    }

    let mut seen = Vec::new();
    while let Some(job) = queue.dequeue().await.unwrap() {
        seen.push(job.priority);
    }

    assert_eq!(
        seen,
        vec![
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ],
        "dequeue must drain highest priority first"
    );
}

/// A failed-but-retryable job must be re-scheduled onto the delayed set with a
/// backoff time in the future (retry-with-backoff), not dropped and not left in
/// `processing`.
#[tokio::test]
async fn retry_with_backoff_reschedules_to_delayed() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "retry").await.unwrap();
    queue.clear().await.unwrap();

    let job = Job::new("retry", "task", json!({})).with_max_attempts(3);
    let job_id = queue.enqueue_job(job).await.unwrap();

    // Dequeue (attempt 1) then fail -> should be retryable, so re-scheduled.
    let dequeued = queue.dequeue().await.unwrap().expect("job available");
    assert_eq!(dequeued.id, job_id);
    queue.fail(job_id, "boom".to_string()).await.unwrap();

    let job = queue.get_job(job_id).await.unwrap().unwrap();
    assert_eq!(
        job.status.state,
        JobState::Failed,
        "still retryable => Failed"
    );
    assert!(
        job.scheduled_at.is_some(),
        "retry must be re-scheduled with backoff"
    );
    assert!(
        job.scheduled_at.unwrap() > chrono::Utc::now(),
        "backoff delay must push the retry into the future"
    );
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "failed job must leave the processing set"
    );
    // It must be off the ready queues (delayed, not pending).
    assert!(queue.dequeue().await.unwrap().is_none());
}

/// Once retries are exhausted, a failing job must be routed to the dead-letter
/// set and marked Dead.
#[tokio::test]
async fn dead_letter_routing_on_exhausted_retries() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let url = redis.url();
    let queue = Queue::new(url.clone(), "dead").await.unwrap();
    queue.clear().await.unwrap();

    let job = Job::new("dead", "task", json!({})).with_max_attempts(1);
    let job_id = queue.enqueue_job(job).await.unwrap();

    // Single allowed attempt: dequeue (attempt 1) then fail -> Dead.
    queue.dequeue().await.unwrap().expect("job available");
    queue.fail(job_id, "fatal".to_string()).await.unwrap();

    let job = queue.get_job(job_id).await.unwrap().unwrap();
    assert_eq!(job.status.state, JobState::Dead);
    assert_eq!(queue.processing_len().await.unwrap(), 0);

    let dead_key = "armature:queue:dead:dead";
    assert_eq!(
        zcard(&url, dead_key).await,
        1,
        "exhausted job must land in the dead-letter set"
    );
}

/// A job scheduled in the (near) future sits in the delayed set -- not the
/// ready queues -- and is promoted to its priority queue by `move_delayed_jobs`
/// on a dequeue once it comes due.
#[tokio::test]
async fn delayed_job_is_promoted_when_due() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "delayed").await.unwrap();
    queue.clear().await.unwrap();

    // Must be scheduled in the FUTURE at enqueue time, otherwise `is_ready()`
    // routes it straight to the pending queue and the delayed set is bypassed.
    let soon = chrono::Utc::now() + chrono::Duration::seconds(1);
    let job = Job::new("delayed", "task", json!({}))
        .with_priority(JobPriority::High)
        .schedule_at(soon);
    let job_id = queue.enqueue_job(job).await.unwrap();

    // Before it comes due: in the delayed set only -- size() (ready jobs)
    // reports 0, backlog (which counts delayed) reports 1, and a dequeue does
    // not surface it.
    assert_eq!(queue.size().await.unwrap(), 0);
    assert_eq!(queue.backlog_size().await.unwrap(), 1);
    assert!(
        queue.dequeue().await.unwrap().is_none(),
        "not-yet-due job must not be dequeuable"
    );

    // Wait until it is due, then dequeue must promote and return it.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    let dequeued = queue.dequeue().await.unwrap().expect("due job promoted");
    assert_eq!(dequeued.id, job_id);
    assert_eq!(dequeued.priority, JobPriority::High);
}

/// The register -> start -> enqueue -> complete happy path must run the handler
/// and leave ZERO orphaned jobs in `processing`. This is the regression guard
/// for the async `register_handler` fix (previously registration was deferred
/// to a fire-and-forget spawn and workers hit "No handler for job type").
#[tokio::test]
async fn worker_happy_path_no_orphans() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "happy").await.unwrap();
    queue.clear().await.unwrap();

    let ran = Arc::new(AtomicBool::new(false));
    let ran_clone = ran.clone();

    let config = WorkerConfig {
        concurrency: 2,
        poll_interval: Duration::from_millis(10),
        job_timeout: Duration::from_secs(5),
        log_execution: false,
    };
    let mut worker = Worker::with_config(queue.clone(), config);

    // Synchronous registration: the handler is present the instant this awaits.
    worker
        .register_handler("greet", move |_job| {
            let ran = ran_clone.clone();
            async move {
                ran.store(true, Ordering::SeqCst);
                Ok(())
            }
        })
        .await;

    let job_id = queue
        .enqueue("greet", json!({"name": "ada"}))
        .await
        .unwrap();
    worker.start().await.unwrap();

    wait_for_state(&queue, job_id, JobState::Completed, Duration::from_secs(5)).await;
    worker.stop().await.unwrap();

    assert!(ran.load(Ordering::SeqCst), "handler must have executed");
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "completed job must not be orphaned in processing"
    );
}

/// `process_batch` must re-enqueue a dequeued job of a different type instead of
/// orphaning it in `processing` (WF3 data-loss fix).
#[tokio::test]
async fn process_batch_requeues_mismatched_type() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "batch").await.unwrap();
    queue.clear().await.unwrap();

    // A higher-priority job of a DIFFERENT type is dequeued first by the batch
    // loop, which then stops. It must not be lost.
    let other_id = queue
        .enqueue_job(Job::new("batch", "other", json!({})).with_priority(JobPriority::Critical))
        .await
        .unwrap();
    queue.enqueue("target", json!({})).await.unwrap();

    let mut worker = Worker::with_config(
        queue.clone(),
        WorkerConfig {
            log_execution: false,
            ..Default::default()
        },
    );
    worker
        .register_handler("target", |_job| async move { Ok(()) })
        .await;

    let processed = worker.process_batch("target", 10).await.unwrap();
    // The mismatched "other" job aborted the batch before any "target" job was
    // collected, so nothing was processed this round...
    assert!(processed.is_empty());
    // ...and critically, the "other" job is NOT orphaned in processing.
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "mismatched job must be re-enqueued, not left in processing"
    );

    // The requeued job is available again and its attempt count was not burned.
    let requeued = queue.dequeue().await.unwrap().expect("other requeued");
    assert_eq!(requeued.id, other_id);
    assert_eq!(requeued.job_type, "other");
    assert_eq!(
        requeued.attempts, 1,
        "requeue+re-dequeue = exactly one attempt"
    );
}

/// `register_cpu_intensive_handler` must be callable from an async context
/// (its documented usage). The pre-fix body called `Handle::block_on` inside
/// async and panicked; this test runs the whole flow on the multi-thread
/// runtime and expects the job to complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpu_intensive_handler_registers_in_async_context() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "cpu").await.unwrap();
    queue.clear().await.unwrap();

    let mut worker = Worker::with_config(
        queue.clone(),
        WorkerConfig {
            poll_interval: Duration::from_millis(10),
            log_execution: false,
            ..Default::default()
        },
    );

    // Would panic pre-fix (block_on inside the async test runtime).
    worker
        .register_cpu_intensive_handler("hash", |_job| Ok(()))
        .await;

    let job_id = queue.enqueue("hash", json!({})).await.unwrap();
    worker.start().await.unwrap();
    wait_for_state(&queue, job_id, JobState::Completed, Duration::from_secs(5)).await;
    worker.stop().await.unwrap();
}

/// `max_size` must count delayed/scheduled jobs, not only ready `pending:*`
/// jobs.
#[tokio::test]
async fn max_size_counts_delayed_jobs() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let config = QueueConfig::new(redis.url(), "cap_delayed").with_max_size(1);
    let queue = Queue::with_config(config).await.unwrap();
    queue.clear().await.unwrap();

    // One delayed job fills the single slot.
    queue
        .enqueue_in(chrono::Duration::hours(1), "task", json!({}))
        .await
        .unwrap();

    // A second enqueue must be rejected even though no job is in a ready queue.
    let err = queue.enqueue("task", json!({})).await.unwrap_err();
    assert!(
        matches!(err, QueueError::QueueFull),
        "delayed job must count toward max_size, got {err:?}"
    );
}

/// `max_size` must count in-flight `processing` jobs.
#[tokio::test]
async fn max_size_counts_processing_jobs() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let config = QueueConfig::new(redis.url(), "cap_processing").with_max_size(1);
    let queue = Queue::with_config(config).await.unwrap();
    queue.clear().await.unwrap();

    let _id = queue.enqueue("task", json!({})).await.unwrap();
    // Move it into the processing set (still occupies a slot).
    queue.dequeue().await.unwrap().expect("job available");

    let err = queue.enqueue("task", json!({})).await.unwrap_err();
    assert!(
        matches!(err, QueueError::QueueFull),
        "in-flight processing job must count toward max_size, got {err:?}"
    );
}

/// `complete`/`fail` must drain the `processing` set even when the job body
/// itself is gone (simulating a TTL expiry mid-flight between dequeue and
/// complete/fail): previously both were guarded by `if let Some(job) =
/// get_job(...)`, so a missing job body left the id orphaned in `processing`
/// forever while the call still returned `Ok(())`.
#[tokio::test]
async fn complete_and_fail_drain_orphaned_processing_entry() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "orphan").await.unwrap();
    queue.clear().await.unwrap();

    let client = redis::Client::open(redis.url()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let processing_key = "armature:queue:orphan:processing";
    let fake_id = JobId::new_v4();

    // Put an id in `processing` with no corresponding `job:<id>` key at all
    // (stand-in for "the job key TTL-expired mid-flight").
    let _: () = redis::cmd("ZADD")
        .arg(processing_key)
        .arg(0)
        .arg(fake_id.to_string())
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(queue.processing_len().await.unwrap(), 1);

    queue.complete(fake_id).await.unwrap();
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "complete() must drain processing even when the job body is gone"
    );

    // Same check for fail().
    let _: () = redis::cmd("ZADD")
        .arg(processing_key)
        .arg(0)
        .arg(fake_id.to_string())
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(queue.processing_len().await.unwrap(), 1);

    queue.fail(fake_id, "boom".to_string()).await.unwrap();
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "fail() must drain processing even when the job body is gone"
    );
}

/// `process_batch` happy path: two same-type jobs are dequeued together,
/// processed in parallel, and both complete without leaving anything behind
/// in `processing`.
#[tokio::test]
async fn process_batch_happy_path() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "batch_happy").await.unwrap();
    queue.clear().await.unwrap();

    queue.enqueue("work", json!({"n": 1})).await.unwrap();
    queue.enqueue("work", json!({"n": 2})).await.unwrap();

    let mut worker = Worker::with_config(
        queue.clone(),
        WorkerConfig {
            log_execution: false,
            ..Default::default()
        },
    );
    worker
        .register_handler("work", |_job| async move { Ok(()) })
        .await;

    let processed = worker.process_batch("work", 10).await.unwrap();
    assert_eq!(processed.len(), 2, "both same-type jobs must be processed");
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "batch completion must not leave orphans in processing"
    );
}

/// `enqueue_at` (past) is immediately due and promotes; `enqueue_in` (future)
/// stays delayed and is not yet dequeuable.
#[tokio::test]
async fn enqueue_in_and_enqueue_at() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "sched").await.unwrap();
    queue.clear().await.unwrap();

    // Future delay -> not ready.
    let future_id = queue
        .enqueue_in(chrono::Duration::hours(1), "later", json!({}))
        .await
        .unwrap();
    assert_eq!(queue.backlog_size().await.unwrap(), 1);
    assert!(
        queue.dequeue().await.unwrap().is_none(),
        "future job must not be dequeuable yet"
    );

    // Past instant -> promoted and dequeued.
    let past = chrono::Utc::now() - chrono::Duration::seconds(5);
    let now_id = queue.enqueue_at(past, "now", json!({})).await.unwrap();
    let dequeued = queue.dequeue().await.unwrap().expect("due job promoted");
    assert_eq!(dequeued.id, now_id);
    assert_ne!(dequeued.id, future_id);
}

/// Poll `processing_len` until it reaches `count`, or panic after `timeout`.
/// Used to make sure a job's handler has actually started running -- not just
/// been enqueued -- before racing it against `stop()`.
async fn wait_for_processing_len(queue: &Queue, count: usize, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if queue.processing_len().await.unwrap() == count {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("processing_len never reached {count} within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// `stop()`/`stop_with_timeout()` must let an in-flight job's handler finish
/// before tearing down its task, not cancel it mid-flight. Regression test for
/// the CRITICAL "Worker::stop() aborts in-flight jobs instead of draining
/// gracefully" finding: `stop()` used to call `handle.abort()` unconditionally
/// the instant it was invoked, killing any task currently inside
/// `handler(job.clone()).await` before it could reach `queue.complete()`,
/// permanently orphaning the job "Processing" in Redis. `docs/queue-guide.md`
/// documents `stop()` followed by a 30s sleep to "wait for in-flight jobs to
/// complete" -- a no-op against the old behavior, since the jobs it was meant
/// to let finish were already dead by the time `stop()` returned.
#[tokio::test]
async fn stop_with_timeout_drains_in_flight_job_before_returning() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "graceful_stop").await.unwrap();
    queue.clear().await.unwrap();

    let config = WorkerConfig {
        concurrency: 1,
        poll_interval: Duration::from_millis(10),
        job_timeout: Duration::from_secs(10),
        log_execution: false,
    };
    let mut worker = Worker::with_config(queue.clone(), config);

    worker
        .register_handler("slow", |_job| async move {
            // Long enough that `stop()` is guaranteed to be invoked while this
            // handler is still mid-flight below.
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(())
        })
        .await;

    let job_id = queue.enqueue("slow", json!({})).await.unwrap();
    worker.start().await.unwrap();

    // Don't stop until the handler has actually started, so `stop()` races a
    // live execution rather than an idle poll loop.
    wait_for_processing_len(&queue, 1, Duration::from_secs(5)).await;

    // Grace period comfortably longer than the handler's 2s sleep: the
    // in-flight job must be allowed to run to completion.
    worker
        .stop_with_timeout(Duration::from_secs(5))
        .await
        .unwrap();

    let job = queue.get_job(job_id).await.unwrap().unwrap();
    assert_eq!(
        job.status.state,
        JobState::Completed,
        "a sufficient grace period must let the in-flight handler finish, not be cancelled"
    );
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "a completed job must not be left orphaned in processing"
    );
}

/// The flip side of the previous test: a grace period shorter than the
/// handler's remaining runtime must still force-abort the stuck task once it
/// elapses (the documented last-resort fallback), rather than `stop()` hanging
/// forever waiting for a handler that will never finish in time. The job is
/// left orphaned in `processing` in this case -- the expected trade-off for a
/// timeout too short to let the handler finish, matching the pre-fix behavior
/// for whichever job is still running when the grace period runs out.
#[tokio::test]
async fn stop_with_timeout_force_aborts_after_grace_period_elapses() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "graceful_stop_timeout")
        .await
        .unwrap();
    queue.clear().await.unwrap();

    let config = WorkerConfig {
        concurrency: 1,
        poll_interval: Duration::from_millis(10),
        job_timeout: Duration::from_secs(30),
        log_execution: false,
    };
    let mut worker = Worker::with_config(queue.clone(), config);

    worker
        .register_handler("slow", |_job| async move {
            // Far longer than the 200ms grace period given to `stop_with_timeout`
            // below, so the task is guaranteed to still be running when the
            // deadline elapses.
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        })
        .await;

    let job_id = queue.enqueue("slow", json!({})).await.unwrap();
    worker.start().await.unwrap();
    wait_for_processing_len(&queue, 1, Duration::from_secs(5)).await;

    let started = std::time::Instant::now();
    worker
        .stop_with_timeout(Duration::from_millis(200))
        .await
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "stop_with_timeout must return once its grace period elapses, not block on the stuck handler"
    );

    let job = queue.get_job(job_id).await.unwrap().unwrap();
    assert_ne!(
        job.status.state,
        JobState::Completed,
        "a force-aborted handler must never reach completion"
    );
    assert_eq!(
        queue.processing_len().await.unwrap(),
        1,
        "force-abort after an insufficient grace period is the documented last resort: \
         the job is left orphaned in processing, same as the pre-fix behavior"
    );
}

/// The two tests above only ever exercise `concurrency: 1`, so a bug that
/// scoped the grace-period deadline per-task instead of sharing a single
/// deadline across the whole `JoinSet` -- e.g. restarting the countdown every
/// time a task finished -- could slip through undetected. This test uses
/// `concurrency: 2` with two jobs of different durations dequeued and
/// executed concurrently, and asserts both complete within a single shared
/// grace period, with the elapsed wall-clock time bounded well under `2x` the
/// longer job's duration (which is what a per-task-reset deadline, or a
/// regression to fully sequential draining, would produce instead).
#[tokio::test]
async fn stop_with_timeout_drains_multiple_concurrent_jobs_before_returning() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let queue = Queue::new(redis.url(), "graceful_stop_multi")
        .await
        .unwrap();
    queue.clear().await.unwrap();

    let config = WorkerConfig {
        concurrency: 2,
        poll_interval: Duration::from_millis(10),
        job_timeout: Duration::from_secs(10),
        log_execution: false,
    };
    let mut worker = Worker::with_config(queue.clone(), config);

    worker
        .register_handler("slow", |job| async move {
            let sleep_ms = job.data["sleep_ms"]
                .as_u64()
                .expect("sleep_ms must be present in job data");
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
            Ok(())
        })
        .await;

    let short_id = queue
        .enqueue("slow", json!({"sleep_ms": 500}))
        .await
        .unwrap();
    let long_id = queue
        .enqueue("slow", json!({"sleep_ms": 1500}))
        .await
        .unwrap();
    worker.start().await.unwrap();

    // Don't stop until both handlers have actually started, so
    // `stop_with_timeout` races two live, concurrently-executing jobs rather
    // than an idle poll loop.
    wait_for_processing_len(&queue, 2, Duration::from_secs(5)).await;

    let started = std::time::Instant::now();
    // Grace period comfortably longer than the longer job (1.5s): both
    // in-flight jobs must be allowed to run to completion.
    let outcome = worker
        .stop_with_timeout(Duration::from_secs(5))
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(
        outcome.gracefully_completed, 2,
        "both concurrent worker tasks must drain gracefully, got {outcome:?}"
    );
    assert_eq!(outcome.panicked, 0, "no task should have panicked");
    assert_eq!(
        outcome.force_aborted, 0,
        "no task should have been force-aborted"
    );

    for (id, label) in [(short_id, "short (500ms)"), (long_id, "long (1.5s)")] {
        let job = queue.get_job(id).await.unwrap().unwrap();
        assert_eq!(
            job.status.state,
            JobState::Completed,
            "{label} job must complete within the shared grace period"
        );
    }
    assert_eq!(
        queue.processing_len().await.unwrap(),
        0,
        "both completed jobs must not be left orphaned in processing"
    );

    // The two jobs ran concurrently against a single shared deadline: elapsed
    // time is bounded by roughly the LONGER job's duration (~1.5s) -- not
    // `2x` it (~3s), which is what a deadline that got reset/restarted per
    // completed task (instead of shared across the whole drain) would allow.
    assert!(
        elapsed < Duration::from_millis(2 * 1_500),
        "stop_with_timeout took {elapsed:?}; expected well under 2x the longer \
         job's 1.5s duration -- the grace-period deadline must be shared across \
         all concurrent worker tasks, not reset per task"
    );
    assert!(
        elapsed >= Duration::from_secs(1),
        "stop_with_timeout returned in {elapsed:?}, too fast for the 1.5s \
         long job to have actually run to completion"
    );
}
