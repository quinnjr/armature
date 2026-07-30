//! Per-connection deadlines.
//!
//! One [`tokio::time::Sleep`] lives per connection and is reused across every
//! phase — header read, body read, write, idle — via `Sleep::reset`. That is
//! what removes the per-request timer allocation a `tokio::time::timeout` call
//! around each read would incur.
//!
//! A bespoke timing wheel is deliberately *not* built here: tokio's timer driver
//! already is a hierarchical wheel, so reimplementing one would add code without
//! adding capability. Coarsening each deadline up to a tick boundary is what
//! keeps that wheel's slot churn low.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::{Instant, Sleep};

/// How far into the future a disarmed deadline is pushed.
///
/// Long enough never to fire in practice, short enough to stay well inside
/// tokio's representable timer range.
const FAR_FUTURE: Duration = Duration::from_secs(86_400 * 365);

/// A reusable, coarsened deadline for one connection.
pub struct ConnDeadline {
    sleep: Pin<Box<Sleep>>,
    tick: Duration,
}

impl ConnDeadline {
    /// Create a disarmed deadline coarsened to `tick`.
    pub fn new(tick: Duration) -> Self {
        let tick = if tick.is_zero() {
            Duration::from_millis(1)
        } else {
            tick
        };
        Self {
            sleep: Box::pin(tokio::time::sleep(FAR_FUTURE)),
            tick,
        }
    }

    /// Arm the deadline for `after` from now, rounded up to the next tick.
    ///
    /// Rounding up rather than down matters: a deadline must never fire early,
    /// or a slow-but-legitimate client gets a 408 it did not earn.
    pub fn arm(&mut self, after: Duration) {
        let coarse = round_up(after, self.tick);
        self.sleep.as_mut().reset(Instant::now() + coarse);
    }

    /// Push the deadline far enough out that it will not fire.
    pub fn disarm(&mut self) {
        self.sleep.as_mut().reset(Instant::now() + FAR_FUTURE);
    }

    /// Poll the deadline.
    #[inline]
    pub fn poll_expired(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.sleep.as_mut().poll(cx)
    }

    /// Wait for the armed deadline to pass.
    ///
    /// Borrows mutably rather than consuming, so the same timer serves the next
    /// phase.
    pub async fn expired(&mut self) {
        Expired { d: self }.await
    }
}

impl std::fmt::Debug for ConnDeadline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnDeadline")
            .field("tick", &self.tick)
            .field("deadline", &self.sleep.deadline())
            .finish()
    }
}

/// Future returned by [`ConnDeadline::expired`].
struct Expired<'a> {
    d: &'a mut ConnDeadline,
}

impl Future for Expired<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.d.poll_expired(cx)
    }
}

/// Round `d` up to the next multiple of `tick`.
fn round_up(d: Duration, tick: Duration) -> Duration {
    let n = tick.as_nanos();
    if n == 0 {
        return d;
    }
    let target = d.as_nanos();
    let rounded = target.div_ceil(n).saturating_mul(n);
    Duration::from_nanos(u64::try_from(rounded).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::poll_fn;

    const TICK: Duration = Duration::from_millis(100);

    /// Whether the deadline has already fired, without waiting for it.
    async fn is_expired(d: &mut ConnDeadline) -> bool {
        poll_fn(|cx| Poll::Ready(d.poll_expired(cx).is_ready())).await
    }

    #[tokio::test(start_paused = true)]
    async fn expires_after_the_armed_duration() {
        let mut d = ConnDeadline::new(TICK);
        d.arm(Duration::from_secs(1));
        tokio::time::advance(Duration::from_millis(1001)).await;
        assert!(is_expired(&mut d).await);
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_expire_early() {
        let mut d = ConnDeadline::new(TICK);
        d.arm(Duration::from_secs(1));
        tokio::time::advance(Duration::from_millis(900)).await;
        assert!(!is_expired(&mut d).await);
    }

    #[tokio::test(start_paused = true)]
    async fn rearming_extends_the_deadline() {
        let mut d = ConnDeadline::new(TICK);
        d.arm(Duration::from_secs(1));
        tokio::time::advance(Duration::from_millis(500)).await;
        d.arm(Duration::from_secs(1));
        // The original deadline moment passes without expiry.
        tokio::time::advance(Duration::from_millis(600)).await;
        assert!(!is_expired(&mut d).await);
        tokio::time::advance(Duration::from_millis(500)).await;
        assert!(is_expired(&mut d).await);
    }

    #[tokio::test(start_paused = true)]
    async fn disarm_prevents_expiry() {
        let mut d = ConnDeadline::new(TICK);
        d.arm(Duration::from_millis(100));
        d.disarm();
        tokio::time::advance(Duration::from_secs(3600)).await;
        assert!(!is_expired(&mut d).await);
    }

    /// Coarsening rounds up, never down: a deadline that fires early would give
    /// a slow-but-legitimate client a 408 it did not earn.
    #[tokio::test(start_paused = true)]
    async fn coarsens_up_to_the_tick_boundary() {
        let mut d = ConnDeadline::new(TICK);
        d.arm(Duration::from_millis(10));
        tokio::time::advance(Duration::from_millis(11)).await;
        assert!(!is_expired(&mut d).await, "must not fire at 10ms");
        tokio::time::advance(Duration::from_millis(95)).await;
        assert!(is_expired(&mut d).await, "must fire by the 100ms boundary");
    }

    #[tokio::test(start_paused = true)]
    async fn reuses_one_timer_across_many_arms() {
        let mut d = ConnDeadline::new(TICK);
        for _ in 0..1000 {
            d.arm(Duration::from_millis(100));
            d.expired().await;
        }
        assert!(is_expired(&mut d).await);
    }

    #[test]
    fn round_up_is_exact_on_boundaries() {
        assert_eq!(round_up(Duration::from_millis(100), TICK), TICK);
        assert_eq!(
            round_up(Duration::from_millis(101), TICK),
            Duration::from_millis(200)
        );
        assert_eq!(round_up(Duration::ZERO, TICK), Duration::ZERO);
        assert_eq!(
            round_up(Duration::from_millis(1), TICK),
            Duration::from_millis(100)
        );
    }
}
