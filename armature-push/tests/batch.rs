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
use std::time::{Duration, Instant};

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

    let start = Instant::now();
    let results = provider.send_batch(&tokens, &notification).await;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), TOKENS, "one result per input token");
    assert!(
        results.iter().all(|r| r.is_ok()),
        "all sends should succeed: {results:?}"
    );

    // Sequential sends would take TOKENS * DELAY (~1s for 20 * 50ms).
    // Concurrent sends should finish in a couple of DELAY windows.
    assert!(
        elapsed < DELAY * (TOKENS as u32 / 2),
        "expected overlapping sends to finish well under {:?} (sequential bound), took {elapsed:?}",
        DELAY * TOKENS as u32
    );

    // Direct evidence of overlap: more than one send was in flight at once.
    assert!(
        max_in_flight.load(Ordering::SeqCst) > 1,
        "expected multiple sends in flight concurrently, max observed was {}",
        max_in_flight.load(Ordering::SeqCst)
    );
}
