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

    let queue = EmailQueue::in_memory(config.clone());
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

    let queue = EmailQueue::in_memory(config);
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
    let queue = EmailQueue::in_memory(EmailQueueConfig::default());

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
    let queue = EmailQueue::in_memory(EmailQueueConfig::default());
    assert!(queue.enqueue_batch(Vec::new()).await.unwrap().is_empty());
    assert_eq!(queue.stats().await.unwrap().pending, 0);
}
