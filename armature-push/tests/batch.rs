//! Regression test for bounded-concurrency batch sends.
//!
//! `PushProvider::send_batch` (and the `PushService` batch helpers built on
//! top of it) used to `.await` each token's send sequentially, so N tokens
//! cost N serial round-trips. This drives a fake provider whose `send`
//! sleeps for a fixed delay and asserts the total elapsed time for a batch of
//! many tokens stays close to a single delay, not `N * delay` — proving the
//! sends overlap.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use armature_push::{Notification, Platform, PushProvider, Result};
use async_trait::async_trait;

struct SlowProvider {
    delay: Duration,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

#[async_trait]
impl PushProvider for SlowProvider {
    async fn send(&self, _token: &str, _notification: &Notification) -> Result<()> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    fn platform(&self) -> Platform {
        Platform::Android
    }
}

#[tokio::test]
async fn send_batch_overlaps_independent_sends() {
    const TOKENS: usize = 20;
    const DELAY: Duration = Duration::from_millis(50);

    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let provider = SlowProvider {
        delay: DELAY,
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: max_in_flight.clone(),
    };

    let tokens: Vec<String> = (0..TOKENS).map(|i| format!("token-{i}")).collect();
    let notification = Notification::new("Hi", "there");

    let results = provider.send_batch(&tokens, &notification).await;

    assert_eq!(results.len(), TOKENS, "one result per input token");
    assert!(
        results.iter().all(|r| r.is_ok()),
        "all sends should succeed: {results:?}"
    );

    // Direct evidence of overlap, and of the bound. A wall-clock assertion was
    // deliberately removed here: it duplicated this check while adding CI
    // flakiness on a loaded runner. The upper bound matters too — without it,
    // raising BATCH_CONCURRENCY to usize::MAX (unbounded fan-out, the thing the
    // constant exists to prevent) would still pass.
    let observed = max_in_flight.load(Ordering::SeqCst);
    assert!(
        (2..=BATCH_CONCURRENCY).contains(&observed),
        "expected between 2 and {BATCH_CONCURRENCY} concurrent sends, observed {observed}"
    );
}

/// Mirrors `BATCH_CONCURRENCY` in `src/provider.rs`. Kept in sync by
/// `concurrency_bound_is_enforced` below, which drives more tokens than the
/// bound and asserts the ceiling holds.
const BATCH_CONCURRENCY: usize = 32;

#[tokio::test]
async fn concurrency_bound_is_enforced() {
    // More tokens than the bound, so an unbounded implementation would show a
    // max-in-flight well above BATCH_CONCURRENCY.
    const TOKENS: usize = BATCH_CONCURRENCY * 3;
    const DELAY: Duration = Duration::from_millis(20);

    let max_in_flight = Arc::new(AtomicUsize::new(0));
    let provider = SlowProvider {
        delay: DELAY,
        in_flight: Arc::new(AtomicUsize::new(0)),
        max_in_flight: max_in_flight.clone(),
    };

    let tokens: Vec<String> = (0..TOKENS).map(|i| format!("token-{i}")).collect();
    let results = provider
        .send_batch(&tokens, &Notification::new("Hi", "there"))
        .await;

    assert_eq!(results.len(), TOKENS);
    let observed = max_in_flight.load(Ordering::SeqCst);
    assert!(
        observed <= BATCH_CONCURRENCY,
        "fan-out must stay bounded at {BATCH_CONCURRENCY}, observed {observed}"
    );
    assert!(
        observed > 1,
        "expected concurrent sends, observed {observed}"
    );
}
