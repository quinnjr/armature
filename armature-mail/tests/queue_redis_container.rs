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
    let queue = EmailQueue::redis(redis.clone(), config.clone()).unwrap();

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

    // 3 retry jobs, all already due. `fail` acts only for the caller that still
    // holds the claim, so each is pushed and popped before it is failed — which
    // is also the only sequence a real worker ever performs.
    for i in 3..6 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }
    for mut job in backend.pop(3).await.unwrap() {
        job.next_retry_at = Some(0);
        backend.fail(job, "boom").await.unwrap();
    }

    // 3 pending jobs.
    for i in 0..3 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
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
    let queue = EmailQueue::redis(redis, config).unwrap();

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

    backend
        .complete(&jobs[0].id, jobs[0].claim_token)
        .await
        .unwrap();
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
    let queue = EmailQueue::redis(redis, config.clone()).unwrap();

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
    let queue = EmailQueue::redis(redis, config.clone()).unwrap();

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

/// The retry claim was `ZRANGEBYSCORE` followed by a *separate* `ZREM`, which is
/// not a claim at all. Two concurrent `pop`s both saw the same ids before either
/// `ZREM` landed, both added them to `:processing`, both `MGET`ed the bodies,
/// and both sent the email. The claim must now decide ownership: across N
/// concurrent pops, every job is returned exactly once.
///
/// Runs on a multi-thread runtime, and repeats against a fresh queue name each
/// round. On a current-thread runtime a serialising schedule passes against
/// broken code — the "80 claims for 40 jobs" figure that motivated this only
/// appears when the poppers genuinely interleave.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_pops_claim_each_retry_job_exactly_once() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    const JOBS: usize = 40;
    const ROUNDS: usize = 10;

    for round in 0..ROUNDS {
        let config =
            EmailQueueConfig::default().queue_name(format!("armature:test:claim-race:{round}"));
        let backend = Arc::new(RedisBackend::new(redis.clone(), config));

        // 40 jobs, all in the *retry* set and all already due — the path that
        // had no atomic claim. `fail` acts only for the claim's owner, so each
        // job is pushed and popped before it is failed.
        for i in 0..JOBS {
            backend.push(EmailJob::new(test_email(i))).await.unwrap();
        }
        for mut job in backend.pop(JOBS).await.unwrap() {
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
            "round {round}: a job was claimed by more than one popper — it would have \
             been sent twice ({} claims, {} distinct jobs)",
            all_ids.len(),
            unique.len()
        );
        assert_eq!(
            all_ids.len(),
            JOBS,
            "round {round}: every job must be claimed exactly once across the concurrent pops"
        );
    }
}

/// The same guarantee for the pending path, which `ZPOPMIN` already made atomic
/// — asserted so a future refactor cannot quietly regress it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
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

/// `:processing` documented a sweeper that recovers jobs whose worker died
/// between `pop` and `complete`, and no such sweeper existed — the job stayed
/// claimed and its body stayed at `:job:<id>` forever, after `enqueue` had
/// returned `Ok(job_id)`.
#[tokio::test]
async fn a_job_whose_worker_never_reported_back_is_reclaimed_and_redelivered() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:reclaim");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config).unwrap();

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

    // And it is genuinely redelivered, as the same job, with the reclaim
    // counted as an attempt.
    let again = backend.pop(1).await.unwrap();
    assert_eq!(again.len(), 1, "reclaimed job was not redelivered");
    assert_eq!(again[0].id, claimed[0].id);
    assert_eq!(
        again[0].attempts, 1,
        "a reclaim must count as an attempt or a poison job redelivers forever"
    );
}

/// `attempts` was bumped only by the worker's `fail` path, which a crashed
/// worker never reaches. A job that kills its worker therefore came back with
/// `attempts` frozen: `should_retry()` never went false, it never reached the
/// dead-letter queue, and it was re-sent every `visibility_timeout` forever — an
/// unbounded stream of duplicates if the send had actually succeeded first.
#[tokio::test]
async fn repeated_reclaims_dead_letter_the_job_instead_of_looping_forever() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:reclaim-poison");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config).unwrap();

    let job = EmailJob::new(test_email(0)).max_retries(3);
    let job_id = job.id.clone();
    backend.push(job).await.unwrap();

    // A worker that dies on this job every single time.
    for attempt in 1..=3 {
        let popped = backend.pop(1).await.unwrap();
        assert_eq!(
            popped.len(),
            1,
            "attempt {attempt}: job was not redelivered"
        );
        assert_eq!(popped[0].id, job_id);
        assert_eq!(popped[0].attempts, attempt - 1);
        assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
    }

    let stats = queue.stats().await.unwrap();
    assert_eq!(
        stats.dead_letter, 1,
        "a job reclaimed max_attempts times must end in the DLQ: {stats:?}"
    );
    assert_eq!(stats.retrying, 0, "it must not still be queued: {stats:?}");
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.processing, 0);
    assert!(
        backend.pop(1).await.unwrap().is_empty(),
        "the dead-lettered job was redelivered anyway"
    );
}

/// A reclaim that races a live `complete` must not manufacture a loss.
///
/// `reclaim_stale` moved the id to `:retry` but left the body; the still-live
/// worker's `complete` then `DEL`ed the body, so the redelivery found
/// `payload == None`, logged an `error!`, `LPUSH`ed to `:lost`, and dead-lettered
/// a stub — for an email that was successfully delivered. This specific race is
/// two-party (the original worker vs. the sweep re-scoring the *same* claim,
/// not yet a new one), so the claim token `CLAIM_STALE` reports back is
/// unchanged from the one `pop` minted — `complete` presenting it still matches
/// and wins normally.
#[tokio::test]
async fn a_reclaim_racing_a_live_complete_does_not_fabricate_a_loss() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:reclaim-vs-complete");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config.clone()).unwrap();

    backend.push(EmailJob::new(test_email(0))).await.unwrap();
    let claimed = backend.pop(1).await.unwrap();
    assert_eq!(claimed.len(), 1);

    // The sweeper decides this claim is stale and takes it back...
    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);

    // ...and only then does the original worker report success. Its claim
    // token is unchanged by the reclaim (see the test doc above), so this
    // still matches and wins normally.
    backend
        .complete(&claimed[0].id, claimed[0].claim_token)
        .await
        .unwrap();

    let stats = queue.stats().await.unwrap();
    assert_eq!(
        stats.dead_letter, 0,
        "a delivered email was dead-lettered as lost: {stats:?}"
    );

    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let lost: Vec<String> = redis::cmd("LRANGE")
        .arg(format!("{}:lost", config.queue_name))
        .arg(0)
        .arg(-1)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert!(lost.is_empty(), "fabricated a lost-job record: {lost:?}");

    // The redelivery is intact — a duplicate send (deduplicable on Message-ID)
    // is the correct trade against a fabricated loss.
    let again = backend.pop(1).await.unwrap();
    assert_eq!(again.len(), 1, "the reclaimed copy lost its body");
    assert_eq!(again[0].id, claimed[0].id);
}

/// A claimed job whose body Redis evicted is a *genuine* loss and must be
/// recorded, not released silently.
///
/// The sweeper sees the same symptom — a stale claim with no body — in two
/// opposite situations, and the naive reading ("the worker must have completed
/// it") turns an evicted email into a `debug!` line. The discriminator is
/// `RELEASE_IF_OWNED`'s reply: it releases the claim only if the token it is
/// given still matches the current one, so an owner that finished has already
/// taken (and cleared) the claim and the sweeper's release returns 0. Here
/// nobody completed it, the claim is still ours, and the caller was told this
/// email was enqueued — so it belongs in `:lost`.
#[tokio::test]
async fn an_evicted_body_under_a_live_claim_is_recorded_as_lost() {
    require_docker!();
    let container = RedisContainer::start().await;
    let url = container.url();
    let redis = service(&url).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:evicted-body");
    let backend = RedisBackend::new(redis.clone(), config.clone());

    backend.push(EmailJob::new(test_email(0))).await.unwrap();
    let claimed = backend.pop(1).await.unwrap();
    assert_eq!(claimed.len(), 1);

    // Simulate Redis evicting the body while the claim is still held. No worker
    // ever calls `complete`, so the `:processing` entry remains ours.
    let client = redis::Client::open(url.as_str()).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    let deleted: i64 = redis::cmd("DEL")
        .arg(format!("{}:job:{}", config.queue_name, claimed[0].id))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "the body should have been there to evict");

    backend.reclaim_stale(Duration::ZERO).await.unwrap();

    let lost: Vec<String> = redis::cmd("LRANGE")
        .arg(format!("{}:lost", config.queue_name))
        .arg(0)
        .arg(-1)
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(
        lost.len(),
        1,
        "an evicted body under a live claim must be recorded as lost, not released silently"
    );
    assert!(
        lost[0].contains(&claimed[0].id),
        "the lost record must name the job: {lost:?}"
    );

    // And the claim is released either way, so `:processing` does not leak.
    let still_claimed: i64 = redis::cmd("ZCARD")
        .arg(format!("{}:processing", config.queue_name))
        .query_async(&mut conn)
        .await
        .unwrap();
    assert_eq!(still_claimed, 0, "the stale claim was not released");
}

/// The reverse order: the owner completes first, so the sweeper's finalize is
/// the no-op and nothing is resurrected.
#[tokio::test]
async fn a_completed_job_is_not_resurrected_by_a_later_sweep() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:complete-then-sweep");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config).unwrap();

    backend.push(EmailJob::new(test_email(0))).await.unwrap();
    let claimed = backend.pop(1).await.unwrap();
    backend
        .complete(&claimed[0].id, claimed[0].claim_token)
        .await
        .unwrap();

    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 0);
    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.processed, 1);
    assert_eq!(stats.retrying, 0, "a delivered job was requeued: {stats:?}");
    assert_eq!(stats.dead_letter, 0);
    assert!(backend.pop(1).await.unwrap().is_empty());
}

/// The `RECLAIM` script had no `LIMIT`, unlike `CLAIM_RETRY`. After a fleet
/// restart `:processing` holds the whole backlog, all past the cutoff, and one
/// `EVAL` does two writes per id while single-threaded Redis is blocked — and as
/// a *write* script `SCRIPT KILL` refuses it, leaving `SHUTDOWN NOSAVE` (which
/// discards unpersisted jobs) as the only recovery. The sweep is now batched,
/// and the caller loops until a batch comes back short, so a backlog larger than
/// one batch is still fully reclaimed.
#[tokio::test]
async fn a_backlog_larger_than_one_batch_is_swept_in_batches() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    // Comfortably more than the 100-id batch the sweeper uses.
    const JOBS: usize = 250;

    let config = EmailQueueConfig::default().queue_name("armature:test:reclaim-batching");
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config).unwrap();

    for i in 0..JOBS {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }
    assert_eq!(backend.pop(JOBS).await.unwrap().len(), JOBS);
    assert_eq!(queue.stats().await.unwrap().processing, JOBS as u64);

    assert_eq!(
        backend.reclaim_stale(Duration::ZERO).await.unwrap(),
        JOBS as u64,
        "the sweeper must keep going past the first batch"
    );

    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.processing, 0, "claims not fully released: {stats:?}");
    assert_eq!(stats.retrying, JOBS as u64, "{stats:?}");
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
    let queue = EmailQueue::redis(redis, config).unwrap();

    for i in 0..2 {
        backend.push(EmailJob::new(test_email(i))).await.unwrap();
    }
    let jobs = backend.pop(2).await.unwrap();

    backend
        .complete(&jobs[0].id, jobs[0].claim_token)
        .await
        .unwrap();
    backend
        .discard(&jobs[1].id, jobs[1].claim_token)
        .await
        .unwrap();

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
    let queue = EmailQueue::redis(redis, config.clone()).unwrap();

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

/// The three-party race the fencing token exists to close: a claim reclaimed
/// by the sweeper *and re-popped by a second worker* before the original
/// worker's stale finalize arrives. Without a token, `complete`/`discard`/
/// `fail`/`dead_letter` decided ownership by id membership alone — the second
/// worker's fresh claim reuses the same id, so the first worker's stale
/// `complete` looked identical to a legitimate one, released the *second*
/// worker's claim, and a subsequent `fail`/`dead_letter` from the second
/// worker then found nothing to act on: the job vanished from every set with
/// no trace, despite `enqueue` having told the caller it succeeded.
#[tokio::test]
async fn a_stale_claims_finalize_does_not_destroy_a_newer_live_claim() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:claim-fencing");
    let backend = RedisBackend::new(redis, config);

    backend.push(EmailJob::new(test_email(0))).await.unwrap();

    // Worker A claims it.
    let first_claim = backend.pop(1).await.unwrap();
    assert_eq!(first_claim.len(), 1);
    let stale_token = first_claim[0].claim_token;
    assert_ne!(stale_token, 0, "a popped job must carry a real claim token");

    // The sweeper reclaims it (A was too slow) and worker B re-pops it from
    // `:retry`, minting a fresh claim under the same job id.
    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
    let second_claim = backend.pop(1).await.unwrap();
    assert_eq!(second_claim.len(), 1);
    assert_eq!(second_claim[0].id, first_claim[0].id, "same job id");
    assert_ne!(
        second_claim[0].claim_token, stale_token,
        "re-popping from :retry must mint a fresh token, not reuse the stale one"
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

    // B's own finalize, with the correct token, must succeed normally — and
    // the job must not have vanished in between.
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

/// As above, but the stale caller's terminal call is `fail` — the shape that
/// previously lost mail outright, since a `fail` that wins the id-only gate
/// resurrects a stale attempt count on top of destroying the live claim.
#[tokio::test]
async fn a_stale_claims_fail_does_not_destroy_a_newer_live_claim() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default().queue_name("armature:test:claim-fencing-fail");
    let backend = RedisBackend::new(redis, config);

    backend.push(EmailJob::new(test_email(0))).await.unwrap();

    let first_claim = backend.pop(1).await.unwrap();
    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);
    let second_claim = backend.pop(1).await.unwrap();

    // A's stale `fail` must be a no-op.
    backend
        .fail(first_claim[0].clone(), "stale worker A timed out")
        .await
        .unwrap();

    let stats = backend.stats().await.unwrap();
    assert_eq!(
        stats.processing, 1,
        "A's stale fail must not touch B's live claim: {stats:?}"
    );

    // B's live claim can still be legitimately finalized afterward.
    backend
        .complete(&second_claim[0].id, second_claim[0].claim_token)
        .await
        .unwrap();
    assert_eq!(backend.stats().await.unwrap().processing, 0);
}

/// `reclaim_stale` must respect `dead_letter_queue(false)` the same way the
/// normal worker-exhaustion path already does: discard without a durable
/// record when a job exhausts its retries through repeated reclaims, rather
/// than writing to `:dead` unconditionally.
#[tokio::test]
async fn reclaim_stale_respects_a_disabled_dead_letter_queue() {
    require_docker!();
    let container = RedisContainer::start().await;
    let redis = service(&container.url()).await;

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:reclaim-no-dlq")
        .dead_letter_queue(false);
    let backend = RedisBackend::new(redis.clone(), config.clone());
    let queue = EmailQueue::redis(redis, config).unwrap();

    let job = EmailJob::new(test_email(0)).max_retries(0);
    backend.push(job).await.unwrap();

    // `max_retries(0)`: the reclaim's own `attempts += 1` is immediately
    // exhausting, so a single sweep is enough to hit the DLQ-disabled path.
    assert_eq!(backend.pop(1).await.unwrap().len(), 1);
    assert_eq!(backend.reclaim_stale(Duration::ZERO).await.unwrap(), 1);

    let stats = queue.stats().await.unwrap();
    assert_eq!(
        stats.dead_letter, 0,
        "dead_letter_queue(false) must not accumulate a durable record: {stats:?}"
    );
    assert_eq!(stats.retrying, 0);
    assert_eq!(stats.processing, 0);
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

    let queue = EmailQueue::redis(redis, config).unwrap();
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
