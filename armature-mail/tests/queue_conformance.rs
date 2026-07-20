//! Queue conformance regression tests that need no external services.
//!
//! Covers the WF6 findings on `EmailQueueConfig::job_timeout` (previously inert)
//! and `EmailQueue::enqueue_batch` (previously N sequential enqueues).

#![cfg(feature = "queue")]

use armature_mail::{
    Email, EmailJob, EmailQueue, EmailQueueConfig, Mailer, MailerConfig, Result, Transport,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A transport whose send never returns within the life of a test.
struct HangingTransport {
    started: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Transport for HangingTransport {
    async fn send(&self, _email: &Email) -> Result<()> {
        self.started.fetch_add(1, Ordering::SeqCst);
        // Far longer than any test deadline: stands in for a dead SMTP socket or
        // a provider that accepts the connection and never answers.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        Ok(())
    }
}

fn test_email() -> Email {
    Email::new()
        .from("sender@example.com")
        .to("recipient@example.com")
        .subject("Test")
        .text("Hello")
}

/// WF6 finding 7: `job_timeout` was configured but never applied, so a hung send
/// occupied its worker slot forever. The send must now be abandoned after
/// `job_timeout` and the job routed to the dead-letter queue (no retries left).
#[tokio::test]
async fn hung_send_is_abandoned_after_job_timeout() {
    let started = Arc::new(AtomicUsize::new(0));
    let mailer = Arc::new(
        Mailer::new(HangingTransport {
            started: started.clone(),
        })
        .with_config(MailerConfig::default().retries(0)),
    );

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:timeout")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(20))
        .job_timeout(Duration::from_millis(200));

    let queue = EmailQueue::in_memory(config.clone()).unwrap();
    queue
        .enqueue_job(EmailJob::new(test_email()).max_retries(0))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let worker = queue.worker(mailer).with_shutdown(shutdown_rx);
    let handle = tokio::spawn(worker.run());

    // The job must reach the dead-letter queue promptly. Without the timeout the
    // worker blocks in `send` forever and this deadline expires.
    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(10);
    loop {
        let stats = queue.stats().await.unwrap();
        if stats.dead_letter == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hung send was never timed out (stats: {stats:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "send should be attempted"
    );
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "timeout took far longer than job_timeout"
    );

    let _ = shutdown_tx.send(());
    handle.abort();
}

/// A timed-out send is retryable: with retries left the job goes back to the
/// queue rather than straight to the dead-letter queue.
#[tokio::test]
async fn timed_out_send_is_retried_when_retries_remain() {
    let started = Arc::new(AtomicUsize::new(0));
    let mailer = Arc::new(
        Mailer::new(HangingTransport {
            started: started.clone(),
        })
        .with_config(MailerConfig::default().retries(0)),
    );

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:timeout-retry")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(20))
        .retry_delay(Duration::from_secs(30))
        .job_timeout(Duration::from_millis(200));

    let queue = EmailQueue::in_memory(config).unwrap();
    queue
        .enqueue_job(EmailJob::new(test_email()).max_retries(3))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let handle = tokio::spawn(queue.worker(mailer).with_shutdown(shutdown_rx).run());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stats = queue.stats().await.unwrap();
        if stats.retrying == 1 {
            assert_eq!(stats.dead_letter, 0, "retryable timeout was dead-lettered");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed-out job never scheduled for retry (stats: {stats:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let _ = shutdown_tx.send(());
    handle.abort();
}

/// `enqueue_batch` now goes through `EmailQueueBackend::push_batch`; every email
/// must still be enqueued exactly once and get a distinct id.
#[tokio::test]
async fn enqueue_batch_enqueues_every_email() {
    let queue = EmailQueue::in_memory(EmailQueueConfig::default()).unwrap();

    let emails: Vec<Email> = (0..10)
        .map(|i| test_email().subject(format!("Subject {i}")))
        .collect();

    let ids = queue.enqueue_batch(emails).await.unwrap();

    assert_eq!(ids.len(), 10);
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 10, "job ids must be unique");

    let stats = queue.stats().await.unwrap();
    assert_eq!(stats.pending, 10);
}

#[tokio::test]
async fn enqueue_batch_of_nothing_is_a_no_op() {
    let queue = EmailQueue::in_memory(EmailQueueConfig::default()).unwrap();
    assert!(queue.enqueue_batch(Vec::new()).await.unwrap().is_empty());
    assert_eq!(queue.stats().await.unwrap().pending, 0);
}

/// A transport that always fails permanently, recording each attempt's Message-ID.
struct RecordingTransport {
    seen: Arc<std::sync::Mutex<Vec<Option<String>>>>,
    fail: bool,
}

#[async_trait::async_trait]
impl Transport for RecordingTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        self.seen.lock().unwrap().push(email.message_id.clone());
        if self.fail {
            // Retryable, so the job goes back to the queue and is retried.
            Err(armature_mail::MailError::Network("boom".into()))
        } else {
            Ok(())
        }
    }
}

/// WF6 audit finding 6: `Email::message_id` defaulted to `None`, so `to_lettre`
/// let lettre mint a *new* Message-ID per attempt. A send abandoned by
/// `job_timeout` but actually delivered was then redelivered under a different
/// identity, which nothing downstream could deduplicate. The id must be stamped
/// once at enqueue time and reused by every retry.
#[tokio::test]
async fn every_retry_reuses_the_same_message_id() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mailer = Arc::new(
        Mailer::new(RecordingTransport {
            seen: seen.clone(),
            fail: true,
        })
        .with_config(MailerConfig::default().retries(0)),
    );

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:message-id")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(10))
        .retry_delay(Duration::from_millis(10))
        .job_timeout(Duration::from_secs(5));

    let queue = EmailQueue::in_memory(config).unwrap();
    queue
        .enqueue_job(EmailJob::new(test_email()).max_retries(3))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let handle = tokio::spawn(queue.worker(mailer).with_shutdown(shutdown_rx).run());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if seen.lock().unwrap().len() >= 3 {
            break;
        }
        assert!(Instant::now() < deadline, "job was never retried");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let _ = shutdown_tx.send(());
    handle.abort();

    let ids = seen.lock().unwrap().clone();
    let first = ids[0]
        .clone()
        .expect("a Message-ID must be stamped at enqueue");
    assert!(first.ends_with("@armature"), "unexpected id: {first}");
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(
            id.as_ref(),
            Some(&first),
            "attempt {i} used a different Message-ID: {id:?}"
        );
    }
}

/// A caller-supplied Message-ID is never overwritten by the queue.
#[tokio::test]
async fn an_explicit_message_id_is_preserved() {
    let job = EmailJob::new(test_email().message_id("mine@example.com"));
    assert_eq!(job.email.message_id.as_deref(), Some("mine@example.com"));
}

/// WF6 audit finding 11: with the DLQ disabled the `else if` had no `else`, so a
/// permanently failing email vanished — no log, no metric, no `complete`.
#[tokio::test]
async fn permanent_failure_with_the_dlq_disabled_is_not_silently_lost() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mailer = Arc::new(
        Mailer::new(RecordingTransport {
            seen: seen.clone(),
            fail: true,
        })
        .with_config(MailerConfig::default().retries(0)),
    );

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:no-dlq")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(10))
        .dead_letter_queue(false);

    let queue = EmailQueue::in_memory(config).unwrap();
    queue
        // No retries left, so the first failure is permanent.
        .enqueue_job(EmailJob::new(test_email()).max_retries(0))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let handle = tokio::spawn(queue.worker(mailer).with_shutdown(shutdown_rx).run());

    // The job must leave the "processing" state rather than being abandoned
    // there: the worker accounts for it via `discard` and logs an error.
    //
    // `discard`, not `complete`. `complete` increments `processed`, so routing
    // permanent failures through it inflated the success counter with failures
    // and made `QueueStats::processed` unusable as the numerator of a
    // delivery-rate alert — a run where every send failed reported 100%.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if seen.lock().unwrap().len() == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "send was never attempted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let stats = queue.stats().await.unwrap();
        if stats.processing == 0 && stats.pending == 0 {
            assert_eq!(stats.dead_letter, 0, "DLQ is disabled");
            assert_eq!(
                stats.processed, 0,
                "a job that was never sent must not count as processed: {stats:?}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "permanently failed job was left in flight (stats: {stats:?})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "send should be attempted once"
    );

    let _ = shutdown_tx.send(());
    handle.abort();
}

/// WF6 audit finding 19: `QueueStats::processing` was hardcoded to 0 in both
/// backends, so a job lost between `pop` and `complete` was invisible.
#[tokio::test]
async fn in_memory_processing_is_tracked_between_pop_and_complete() {
    use armature_mail::{EmailQueueBackend, InMemoryBackend};

    let backend = InMemoryBackend::new();
    for _ in 0..3 {
        backend.push(EmailJob::new(test_email())).await.unwrap();
    }
    assert_eq!(backend.stats().await.unwrap().processing, 0);

    let jobs = backend.pop(3).await.unwrap();
    assert_eq!(jobs.len(), 3);
    assert_eq!(
        backend.stats().await.unwrap().processing,
        3,
        "popped jobs must be counted as processing"
    );

    backend
        .complete(&jobs[0].id, jobs[0].claim_token)
        .await
        .unwrap();
    backend.fail(jobs[1].clone(), "boom").await.unwrap();
    backend.dead_letter(jobs[2].clone()).await.unwrap();

    let stats = backend.stats().await.unwrap();
    assert_eq!(
        stats.processing, 0,
        "complete/fail/dead_letter must all release the claim: {stats:?}"
    );
}

/// `pop(count)` must honor `count` exactly — the contract `RedisBackend::pop`
/// now shares.
#[tokio::test]
async fn in_memory_pop_honors_count_exactly() {
    use armature_mail::{EmailQueueBackend, InMemoryBackend};

    let backend = InMemoryBackend::new();
    for _ in 0..6 {
        backend.push(EmailJob::new(test_email())).await.unwrap();
    }

    assert_eq!(backend.pop(2).await.unwrap().len(), 2);
    assert_eq!(backend.pop(10).await.unwrap().len(), 4);
}

/// Delegating wrapper so a test can hold the backend *and* hand it to a queue.
///
/// `EmailQueue::with_backend` takes ownership, but the sweep test needs to claim
/// a job out of band, behind the worker's back.
struct Shared(Arc<armature_mail::InMemoryBackend>);

#[async_trait::async_trait]
impl armature_mail::EmailQueueBackend for Shared {
    async fn push(&self, job: EmailJob) -> Result<()> {
        self.0.push(job).await
    }
    async fn pop(&self, count: usize) -> Result<Vec<EmailJob>> {
        self.0.pop(count).await
    }
    async fn complete(&self, job_id: &str, claim_token: u64) -> Result<()> {
        self.0.complete(job_id, claim_token).await
    }
    async fn discard(&self, job_id: &str, claim_token: u64) -> Result<()> {
        self.0.discard(job_id, claim_token).await
    }
    async fn fail(&self, job: EmailJob, error: &str) -> Result<()> {
        self.0.fail(job, error).await
    }
    async fn dead_letter(&self, job: EmailJob) -> Result<()> {
        self.0.dead_letter(job).await
    }
    async fn reclaim_stale(&self, visibility_timeout: Duration) -> Result<u64> {
        self.0.reclaim_stale(visibility_timeout).await
    }
    async fn stats(&self) -> Result<armature_mail::QueueStats> {
        self.0.stats().await
    }
}

/// The worker's sweep loop — the actual production wiring of `reclaim_stale` —
/// had no test at all. An off-by-one in `sweep_interval`, or the `Err(e) =>
/// error!` arm becoming a `break`, was invisible.
///
/// A job is claimed out of band and never reported on, standing in for a worker
/// that died between `pop` and `complete`. The running worker must sweep it up
/// and deliver it without any further help.
#[tokio::test]
async fn the_worker_sweep_loop_reclaims_and_redelivers_an_abandoned_job() {
    use armature_mail::{EmailQueue, EmailQueueBackend, InMemoryBackend};

    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mailer = Arc::new(
        Mailer::new(RecordingTransport {
            seen: seen.clone(),
            fail: false,
        })
        .with_config(MailerConfig::default().retries(0)),
    );

    let backend = Arc::new(InMemoryBackend::new());
    backend.push(EmailJob::new(test_email())).await.unwrap();

    // Claim it and "die": no complete, no fail, no dead_letter. The worker below
    // has no other way to learn about this job.
    let claimed = backend.pop(1).await.unwrap();
    assert_eq!(claimed.len(), 1);

    let config = EmailQueueConfig::default()
        .queue_name("armature:test:worker-sweep")
        .concurrency(1)
        .batch_size(1)
        .poll_interval(Duration::from_millis(10))
        .job_timeout(Duration::from_millis(20))
        .visibility_timeout(Duration::from_millis(100));

    let queue = EmailQueue::with_backend(Shared(backend.clone()), config).unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
    let handle = tokio::spawn(queue.worker(mailer).with_shutdown(shutdown_rx).run());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker never swept the abandoned job (stats: {:?})",
            backend.stats().await.unwrap()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(
        seen.lock().unwrap()[0],
        claimed[0].email.message_id,
        "a different email was delivered"
    );

    let _ = shutdown_tx.send(());
    handle.abort();
}

/// `visibility_timeout` "must be comfortably above `job_timeout`" was a field
/// doc and nothing else. `job_timeout(600s)` against the 300s default is a
/// one-line plausible misconfiguration that systematically redelivers every slow
/// email; an unsupportable configuration must be an error, not a hope.
#[tokio::test]
async fn a_queue_rejects_a_visibility_timeout_that_does_not_clear_job_timeout() {
    let bad = EmailQueueConfig::default()
        .queue_name("armature:test:bad-config")
        .job_timeout(Duration::from_secs(600));

    match EmailQueue::in_memory(bad) {
        Err(armature_mail::MailError::Config(_)) => {}
        Err(other) => panic!("expected a Config error, got {other}"),
        Ok(_) => panic!("an unsupportable configuration was accepted"),
    }

    // The default is fine, and stays fine.
    assert!(EmailQueue::in_memory(EmailQueueConfig::default()).is_ok());
}
