//! Redis-container-backed regression tests for the armature-mail queue.
//!
//! Covers the WF6 findings on `RedisBackend::pop` (one `GET` per job on the
//! worker hot path -> a single `MGET`), `EmailQueue::enqueue_batch` (N sequential
//! enqueues -> one pipelined batch), and `EmailQueueConfig::job_timeout` (inert ->
//! enforced) against a real Redis. Every test self-skips when Docker is
//! unavailable, so the default `cargo test` never requires Docker.

#![cfg(feature = "redis")]

use armature_mail::{
    Email, EmailJob, EmailQueue, EmailQueueBackend, EmailQueueConfig, Mailer, MailerConfig,
    RedisBackend, Result, Transport,
};
use armature_redis::{RedisConfig, RedisService};
use armature_testkit::containers::RedisContainer;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Skip the calling test when Docker is unavailable — unless
/// `ARMATURE_REQUIRE_DOCKER=1`, in which case fail loudly.
///
/// A thin delegate to the testkit, which now owns this logic (and writes its
/// skip notice straight to stderr, so a skipped test is visible without
/// `--nocapture` instead of reported as a plain green pass). The local copy that
/// used to live here duplicated it and would have drifted.
macro_rules! require_docker {
    () => {
        armature_testkit::skip_if_no_docker!()
    };
}

async fn service(url: &str) -> Arc<RedisService> {
    let config = RedisConfig::builder().url(url).build();
    Arc::new(RedisService::new(config).await.unwrap())
}

fn test_email(n: usize) -> Email {
    Email::new()
        .from("sender@example.com")
        .to("recipient@example.com")
        .subject(format!("Subject {n}"))
        .text("Hello")
}

/// Reset Redis command statistics so the next assertions see only our commands.
async fn reset_stats(url: &str) {
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("CONFIG")
        .arg("RESETSTAT")
        .query_async::<()>(&mut conn)
        .await
        .unwrap();
}

/// Number of calls Redis recorded for `command` since the last `CONFIG RESETSTAT`.
async fn call_count(url: &str, command: &str) -> u64 {
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let info: String = redis::cmd("INFO")
        .arg("commandstats")
        .query_async(&mut conn)
        .await
        .unwrap();

    let prefix = format!("cmdstat_{command}:calls=");
    info.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .and_then(|rest| rest.split(',').next())
        .and_then(|calls| calls.parse().ok())
        .unwrap_or(0)
}

/// WF6 finding 8: `pop` issued a separate `GET job_key` per id in a loop on the
/// worker hot path. It must now fetch every job body with a single `MGET`.
#[tokio::test]
async fn pop_fetches_job_bodies_in_a_single_mget() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let backend = RedisBackend::new(
        redis,
        EmailQueueConfig::default().queue_name("armature:test:pop"),
    );

    for i in 0..5 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }

    // The INFO/CONFIG calls themselves run on a separate connection, so only the
    // backend's own commands are counted in the window below.
    reset_stats(&url).await;
    let jobs = backend.pop(5).await.unwrap();
    assert_eq!(jobs.len(), 5, "all pushed jobs should come back");

    let mget = call_count(&url, "mget").await;
    let get = call_count(&url, "get").await;

    assert_eq!(mget, 1, "job bodies should be fetched in one MGET");
    assert_eq!(get, 0, "no per-job GET should remain (found {get})");
}

/// `pop` on an empty queue must not issue a pointless `MGET`.
#[tokio::test]
async fn pop_on_empty_queue_issues_no_fetch() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let backend = RedisBackend::new(
        redis,
        EmailQueueConfig::default().queue_name("armature:test:pop-empty"),
    );

    reset_stats(&url).await;
    assert!(backend.pop(10).await.unwrap().is_empty());
    assert_eq!(call_count(&url, "mget").await, 0);
    assert_eq!(call_count(&url, "get").await, 0);
}

/// WF6 finding 10: `enqueue_batch` looped single `enqueue`s, paying a connection
/// acquire plus SET plus ZADD per email. It now pipelines — and must still
/// enqueue every job exactly once so that a later `pop` returns all of them.
#[tokio::test]
async fn enqueue_batch_pipelines_and_enqueues_everything() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:batch");
    let queue = EmailQueue::redis(redis.clone(), config.clone());

    let emails: Vec<Email> = (0..20).map(test_email).collect();
    let ids = queue.enqueue_batch(emails).await.unwrap();
    assert_eq!(ids.len(), 20);

    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.pending, 20, "every batched email must be enqueued");

    // And every one of them is retrievable, with distinct ids.
    let backend = RedisBackend::new(redis, config);
    let jobs = backend.pop(20).await.unwrap();
    assert_eq!(jobs.len(), 20);
    let unique: std::collections::HashSet<_> = jobs.iter().map(|j| j.id.clone()).collect();
    assert_eq!(unique.len(), 20);
}

/// WF6 audit finding 4: `ZPOPMIN key <count>` returns a flat
/// `member,score,member,score…` array, so deserializing into `Vec<String>` gave
/// `2 * count` entries — every other one a score, which became a bogus
/// `…:job:<score>` key in the MGET. The nils were swallowed, so it "worked" at
/// 2x MGET width. The MGET must request exactly one key per job.
#[tokio::test]
async fn pop_does_not_treat_zpopmin_scores_as_job_ids() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let backend = RedisBackend::new(
        redis,
        EmailQueueConfig::default().queue_name("armature:test:zpopmin"),
    );

    for i in 0..4 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }

    reset_stats(&url).await;
    let jobs = backend.pop(4).await.unwrap();
    assert_eq!(jobs.len(), 4);

    // One MGET, and its key count must equal the job count — not double it.
    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let info: String = redis::cmd("INFO")
        .arg("commandstats")
        .query_async(&mut conn)
        .await
        .unwrap();
    // `usec_per_call` aside, `calls=1` proves a single MGET was issued.
    assert!(
        info.contains("cmdstat_mget:calls=1"),
        "expected exactly one MGET:\n{info}"
    );

    // The decisive assertion: every id popped resolved to a real job body. With
    // scores mixed into the id list, half the MGET keys are nils and the
    // returned job count would still be 4 while 8 keys were requested — so
    // assert on the parsed ids instead.
    let ids: std::collections::HashSet<_> = jobs.iter().map(|j| j.id.clone()).collect();
    assert_eq!(ids.len(), 4, "duplicate or bogus job ids returned");
    for job in &jobs {
        assert!(
            uuid::Uuid::parse_str(&job.id).is_ok(),
            "job id is not a uuid, a score leaked into the id list: {}",
            job.id
        );
    }
}

/// WF6 audit finding 5: `count` was applied independently to the pending set and
/// the retry set and the results concatenated, so `pop(n)` could return `2 * n`.
/// `InMemoryBackend::pop` honors `count` exactly; the two must agree.
#[tokio::test]
async fn pop_never_returns_more_than_count() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:pop-count");
    let backend = RedisBackend::new(redis, config);

    // 3 pending jobs.
    for i in 0..3 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }

    // 3 retry jobs, all already due. `fail` both stores the body and adds the id
    // to the retry set, so these must not also be `push`ed into pending.
    for i in 3..6 {
        let mut job = EmailJob::new(test_email(i));
        job.next_retry_at = Some(0);
        backend.fail(job, "boom").await.unwrap();
    }

    let jobs = backend.pop(2).await.unwrap();
    assert_eq!(jobs.len(), 2, "pop must honor `count` exactly");

    // The remainder is still queued, not dropped.
    let rest = backend.pop(10).await.unwrap();
    assert_eq!(rest.len(), 4, "remaining jobs must still be poppable");
}

/// WF6 audit finding 19: `QueueStats::processing` was hardcoded to 0, so a job
/// lost between `pop` and `complete` was invisible.
#[tokio::test]
async fn processing_is_tracked_between_pop_and_complete() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:processing");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config);

    for i in 0..3 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }
    assert_eq!(queue.stats().await.unwrap().processing, 0);

    let jobs = backend.pop(3).await.unwrap();
    assert_eq!(
        queue.stats().await.unwrap().processing,
        3,
        "popped jobs must be counted as processing"
    );

    backend.complete(&jobs[0].id).await.unwrap();
    backend.fail(jobs[1].clone(), "boom").await.unwrap();
    backend.dead_letter(jobs[2].clone()).await.unwrap();

    let stats = queue.stats().await.unwrap();
    assert_eq!(
        stats.processing, 0,
        "complete/fail/dead_letter must all release the claim: {stats:?}"
    );
}

/// WF6 audit finding 10: the ids are already off the pending/retry sets by the
/// time the body is fetched, so a missing body dropped the email permanently
/// with no log and no dead-letter entry.
#[tokio::test]
async fn a_job_with_a_missing_body_is_dead_lettered_not_dropped() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:lost-body");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config.clone());

    let job = EmailJob::new(test_email(0));
    let job_id = job.id.clone();
    backend.push(job).await.unwrap();

    // Simulate an expired/evicted body.
    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("DEL")
        .arg(format!("{}:job:{}", config.queue_name, job_id))
        .query_async::<()>(&mut conn)
        .await
        .unwrap();

    let jobs = backend.pop(10).await.unwrap();
    assert!(jobs.is_empty(), "a body-less job must not be returned");

    let stats = queue.stats().await.unwrap();
    assert_eq!(
        stats.dead_letter, 1,
        "lost job must be dead-lettered, not silently dropped: {stats:?}"
    );
    assert_eq!(stats.processing, 0, "claim must be released: {stats:?}");
}

/// A corrupt (undeserializable) body takes the same path as a missing one.
#[tokio::test]
async fn a_job_with_a_corrupt_body_is_dead_lettered() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:corrupt-body");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config.clone());

    let job = EmailJob::new(test_email(0));
    let job_id = job.id.clone();
    backend.push(job).await.unwrap();

    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("SET")
        .arg(format!("{}:job:{}", config.queue_name, job_id))
        .arg("{not valid json")
        .query_async::<()>(&mut conn)
        .await
        .unwrap();

    assert!(backend.pop(10).await.unwrap().is_empty());
    assert_eq!(queue.stats().await.unwrap().dead_letter, 1);
}

/// WF6 audit: the retry claim was `ZRANGEBYSCORE` followed by a *separate*
/// `ZREM`, which is not a claim at all. Two concurrent `pop`s both saw the same
/// ids before either `ZREM` landed, both added them to `:processing`, both
/// `MGET`ed the bodies, and both sent the email. The claim must now decide
/// ownership: across N concurrent pops, every job is returned exactly once.
#[tokio::test]
async fn concurrent_pops_claim_each_retry_job_exactly_once() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:claim-race");
    let backend = Arc::new(RedisBackend::new(redis, config));

    // 40 jobs, all in the *retry* set and all already due — the path that had
    // no atomic claim. `fail` both stores the body and adds the id to the retry
    // set, so they must not also be pushed to pending.
    const JOBS: usize = 40;
    for i in 0..JOBS {
        let mut job = EmailJob::new(test_email(i));
        job.next_retry_at = Some(0);
        backend.fail(job, "boom").await.unwrap();
    }

    // 8 concurrent poppers, each asking for the whole set: without an atomic
    // claim every one of them gets every id.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let backend = backend.clone();
        handles.push(tokio::spawn(
            async move { backend.pop(JOBS).await.unwrap() },
        ));
    }

    let mut all_ids = Vec::new();
    for handle in handles {
        all_ids.extend(handle.await.unwrap().into_iter().map(|j| j.id));
    }

    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(
        all_ids.len(),
        unique.len(),
        "a job was claimed by more than one popper — it would have been sent twice \
         ({} claims, {} distinct jobs)",
        all_ids.len(),
        unique.len()
    );
    assert_eq!(
        all_ids.len(),
        JOBS,
        "every job must be claimed exactly once across the concurrent pops"
    );
}

/// The same guarantee for the pending path, which `ZPOPMIN` already made atomic
/// — asserted so a future refactor cannot quietly regress it.
#[tokio::test]
async fn concurrent_pops_claim_each_pending_job_exactly_once() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:claim-race-pending");
    let backend = Arc::new(RedisBackend::new(redis, config));

    const JOBS: usize = 40;
    for i in 0..JOBS {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }

    let mut handles = Vec::new();
    for _ in 0..8 {
        let backend = backend.clone();
        handles.push(tokio::spawn(
            async move { backend.pop(JOBS).await.unwrap() },
        ));
    }

    let mut all_ids = Vec::new();
    for handle in handles {
        all_ids.extend(handle.await.unwrap().into_iter().map(|j| j.id));
    }

    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(
        all_ids.len(),
        unique.len(),
        "a pending job was claimed twice"
    );
    assert_eq!(all_ids.len(), JOBS);
}

/// WF6 audit: `:processing` documented a sweeper that recovers jobs whose worker
/// died between `pop` and `complete`. `grep -rn "sweep\|reclaim"` found only the
/// comment — the job stayed claimed and its body stayed at `:job:<id>` forever,
/// after `enqueue` had returned `Ok(job_id)`.
#[tokio::test]
async fn a_job_whose_worker_never_reported_back_is_reclaimed_and_redelivered() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:reclaim");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config);

    backend.push(EmailJob::new(test_email(0))).await.unwrap();

    // Claim it and then "die": no complete, no fail, no dead_letter.
    let claimed = backend.pop(1).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(queue.stats().await.unwrap().processing, 1);

    // A live claim is not stolen from a worker that is merely slow.
    assert_eq!(
        backend
            .reclaim_stale(Duration::from_secs(300))
            .await
            .unwrap(),
        0
    );
    assert_eq!(queue.stats().await.unwrap().processing, 1);

    // Past the visibility timeout it comes back.
    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);

    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.processing, 0, "claim must be released: {stats:?}");
    assert_eq!(stats.retrying, 1, "job must be re-queued: {stats:?}");

    // And it is genuinely redelivered, as the same job.
    let again = backend.pop(1).await.unwrap();
    assert_eq!(again.len(), 1, "reclaimed job was not redelivered");
    assert_eq!(again[0].id, claimed[0].id);
}

/// A permanently failed email must not inflate the success counter: `complete`
/// does `HINCRBY processed 1`, so using it for a failure made
/// `QueueStats::processed` useless as the denominator of a delivery-rate alert.
#[tokio::test]
async fn discard_releases_the_claim_without_counting_a_success() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:discard");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config);

    for i in 0..2 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }
    let jobs = backend.pop(2).await.unwrap();

    backend.complete(&jobs[0].id).await.unwrap();
    backend.discard(&jobs[1].id).await.unwrap();

    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.processed, 1, "discard must not count as processed");
    assert_eq!(stats.processing, 0, "discard must release the claim");
    assert_eq!(stats.pending, 0);
}

/// A lost job must leave a forensic trace even with the dead-letter queue
/// disabled: `discard_lost` used to `DEL` the body and record nothing, so the
/// last evidence of a job the caller was told had been enqueued was destroyed.
#[tokio::test]
async fn a_lost_job_is_recorded_even_with_the_dlq_disabled() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:lost-no-dlq")
        .dead_letter_queue(false);
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config.clone());

    let job = EmailJob::new(test_email(0));
    let job_id = job.id.clone();
    backend.push(job).await.unwrap();

    // Simulate an evicted body.
    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("DEL")
        .arg(format!("{}:job:{}", config.queue_name, job_id))
        .query_async::<()>(&mut conn)
        .await
        .unwrap();

    assert!(backend.pop(10).await.unwrap().is_empty());

    // No DLQ, by configuration...
    assert_eq!(queue.stats().await.unwrap().dead_letter, 0);
    assert_eq!(queue.stats().await.unwrap().processing, 0);

    // ...but the `:lost` list still records what happened.
    let lost: Vec<String> = redis::cmd("LRANGE")
        .arg(format!("{}:lost", config.queue_name))
        .arg(0)
        .arg(-1)
        .query_async(&mut conn)
        .await
        .unwrap();

    assert_eq!(lost.len(), 1, "the lost job left no trace at all");
    assert!(
        lost[0].contains(&job_id),
        "stub does not name the job: {lost:?}"
    );
}

struct HangingTransport;

#[async_trait::async_trait]
impl Transport for HangingTransport {
    async fn send(&self, _email: &Email) -> Result<()> {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(())
    }
}

/// WF6 finding 7, against the Redis backend: a hung send must be abandoned after
/// `job_timeout` and the job dead-lettered rather than pinning the worker slot.
#[tokio::test]
async fn job_timeout_is_enforced_against_redis_backend() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:job-timeout")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(20))
        .job_timeout(Duration::from_millis(200));

    let queue = EmailQueue::redis(redis, config);
    queue
        .enqueue_job(EmailJob::new(test_email(0)).max_retries(0))
        .await
        .unwrap();

    let mailer =
        Arc::new(Mailer::new(HangingTransport).with_config(MailerConfig::default().retries(0)));
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let handle = tokio::spawn(queue.worker(mailer).with_shutdown(shutdown_rx).run());

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let stats = queue.stats().await.unwrap();
        if stats.dead_letter == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hung send was never timed out (stats: {stats:?})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let _ = shutdown_tx.send(());
    handle.abort();
}
