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

macro_rules! require_docker {
    () => {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
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
