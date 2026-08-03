//! Shared bounded-concurrency dispatch helpers.
//!
//! `aws.rs`, `nats.rs`, `rabbitmq.rs`, and `mq_bridge.rs` all add
//! `SubscribeOptions::concurrency` support the same way: acquire a permit
//! from a `Semaphore` before spawning each per-message handler task, and
//! release it (via `Drop`) when that task finishes. This module centralizes
//! that idiom, plus tracking spawned tasks in a `JoinSet` so shutdown paths
//! (`Subscription::unsubscribe`) can drain outstanding handlers within a
//! bounded timeout instead of abandoning them.
//!
//! Kafka is deliberately excluded: its consume loop uses `FuturesOrdered` to
//! preserve dispatch order for offset commits (see the ordering comment in
//! `kafka.rs::consume_messages`), which is a structurally different dispatch
//! mechanism than the "fire off a tracked task per message" pattern here.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tracing::warn;

/// Normalize a `SubscribeOptions::concurrency` into a usable permit count:
/// `None` defaults to `1` (fully sequential dispatch, matching the behavior
/// before per-message concurrency existed), and `Some(0)` is clamped up to
/// `1` since a zero-permit semaphore would deadlock the consume loop forever
/// (every `acquire` would block with no way to ever be granted a permit).
pub(crate) fn concurrency_or_default(concurrency: Option<usize>) -> usize {
    concurrency.unwrap_or(1).max(1)
}

/// Default grace period `unsubscribe` waits for in-flight handler tasks to
/// finish on their own before forcibly aborting them.
pub(crate) const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Acquire a permit from `semaphore` (bounding how many handler tasks may run
/// concurrently) and spawn `fut` as a task tracked in `tasks`, releasing the
/// permit when `fut` completes.
///
/// The permit is acquired *before* `tasks` is locked, so a slow or stuck
/// handler holding up the semaphore never prevents [`drain_with_timeout`]
/// from acquiring the lock and starting its grace-period wait -- otherwise a
/// single stuck handler could make `unsubscribe`'s "bounded" timeout
/// unbounded in practice.
///
/// The semaphore is never closed for the life of the consume loops that call
/// this, so `acquire_owned` can only fail if it were dropped out from under
/// us, which cannot happen here.
pub(crate) async fn spawn_bounded<F>(
    semaphore: &Arc<Semaphore>,
    tasks: &Arc<Mutex<JoinSet<()>>>,
    fut: F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    let permit = semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("semaphore is not closed");

    let mut tasks = tasks.lock().await;

    // Reap finished tasks before adding another. A `JoinSet` retains every
    // completed entry until it is joined, and the only other drain is
    // `drain_with_timeout`, which runs solely from `Subscription::unsubscribe`.
    // Without this, a long-lived subscription accumulates one entry per message
    // handled for its entire lifetime -- the semaphore bounds how many handlers
    // run at once, not how large the set grows.
    while tasks.try_join_next().is_some() {}

    tasks.spawn(async move {
        fut.await;
        drop(permit);
    });
}

/// Drain `tasks`, waiting up to `timeout` total for outstanding handler
/// tasks to finish on their own before forcibly aborting anything still
/// running. Mirrors `armature_queue::Worker::stop_with_timeout`'s grace
/// period / abort pattern: each completed task is drained from the
/// `JoinSet` as it finishes, against a single shared deadline (not a
/// per-task timeout). `backend` names the caller in the warning logged when
/// tasks had to be forcibly abandoned.
pub(crate) async fn drain_with_timeout(
    tasks: &Arc<Mutex<JoinSet<()>>>,
    timeout: Duration,
    backend: &str,
) {
    let mut tasks = tasks.lock().await;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, tasks.join_next()).await {
            // A task finished on its own; keep waiting for the rest.
            Ok(Some(_)) => continue,
            // All tasks finished on their own within the grace period.
            Ok(None) => break,
            // Grace period elapsed with tasks still outstanding.
            Err(_) => break,
        }
    }

    // Last resort: force-abort anything still running once the grace period
    // has elapsed. `abort_all` + draining is a no-op if everything above
    // already finished.
    let abandoned = tasks.len();
    if abandoned > 0 {
        warn!(
            backend,
            abandoned,
            "Timed out waiting for in-flight message handler tasks to finish; aborting remaining tasks"
        );
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrency_or_default_normalizes() {
        assert_eq!(concurrency_or_default(None), 1);
        assert_eq!(concurrency_or_default(Some(0)), 1);
        assert_eq!(concurrency_or_default(Some(5)), 5);
    }

    #[tokio::test]
    async fn spawn_bounded_tracks_and_runs_every_task() {
        let semaphore = Arc::new(Semaphore::new(2));
        let tasks = Arc::new(Mutex::new(JoinSet::new()));
        let counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let counter = counter.clone();
            spawn_bounded(&semaphore, &tasks, async move {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        }

        drain_with_timeout(&tasks, Duration::from_secs(5), "test").await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        assert_eq!(tasks.lock().await.len(), 0);
    }

    /// A subscription handles far more messages over its lifetime than it ever
    /// runs concurrently. The `JoinSet` must not retain an entry per message
    /// until `unsubscribe`, so spawning many short tasks through a narrow
    /// semaphore has to leave the set near-empty rather than near-`n`.
    #[tokio::test]
    async fn spawn_bounded_reaps_completed_tasks() {
        let semaphore = Arc::new(Semaphore::new(1));
        let tasks = Arc::new(Mutex::new(JoinSet::new()));

        for _ in 0..200 {
            spawn_bounded(&semaphore, &tasks, async {}).await;
            // Let the just-spawned task actually run to completion, so the next
            // call has something to reap.
            tokio::task::yield_now().await;
        }

        // A couple of entries may legitimately still be in flight; the failure
        // this guards against is unbounded growth toward 200.
        let outstanding = tasks.lock().await.len();
        assert!(
            outstanding < 10,
            "JoinSet retained {outstanding} completed tasks instead of reaping them"
        );

        drain_with_timeout(&tasks, Duration::from_secs(5), "test").await;
    }

    #[tokio::test]
    async fn drain_with_timeout_waits_for_quick_tasks_without_aborting() {
        let semaphore = Arc::new(Semaphore::new(4));
        let tasks = Arc::new(Mutex::new(JoinSet::new()));
        let completed = Arc::new(AtomicUsize::new(0));

        for _ in 0..4 {
            let completed = completed.clone();
            spawn_bounded(&semaphore, &tasks, async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                completed.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        }

        drain_with_timeout(&tasks, Duration::from_secs(5), "test").await;
        assert_eq!(completed.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn drain_with_timeout_aborts_stuck_tasks_after_deadline() {
        let semaphore = Arc::new(Semaphore::new(1));
        let tasks = Arc::new(Mutex::new(JoinSet::new()));

        spawn_bounded(&semaphore, &tasks, async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        })
        .await;

        let start = tokio::time::Instant::now();
        drain_with_timeout(&tasks, Duration::from_millis(50), "test").await;

        // Must return promptly once the deadline elapses, not wait for the
        // stuck task's own 60s sleep to finish.
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(tasks.lock().await.len(), 0);
    }
}
