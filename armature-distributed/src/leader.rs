//! Distributed leader election using Redis

use redis::AsyncCommands;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{Notify, oneshot};
use tokio::task::AbortHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Leader election errors
#[derive(Debug, Error)]
pub enum LeaderError {
    #[error("Election failed: {0}")]
    ElectionFailed(String),

    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),

    #[error("Not the leader")]
    NotLeader,

    /// The detached renewal task spawned by [`LeaderElection::start`]
    /// panicked. `start()` unconditionally clears local leadership state
    /// (see the split-brain note in [`LeaderElection::start`]'s docs)
    /// before returning this, so a caller supervising the task (e.g.
    /// restarting election on error) can rely on
    /// `is_leader()`/[`LeaderFence::is_held`] already being correct without
    /// waiting for its own recovery logic to run.
    #[error("Renewal task panicked: {0}")]
    RenewalTaskPanicked(String),
}

/// Leader election callback
pub type LeaderCallback =
    Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Leader election coordinator
///
/// # Callback execution does not block renewal
///
/// The election loop's Redis renewal cadence (`try_become_leader` every
/// `refresh_interval`) runs on its own task, decoupled from
/// `on_elected`/`on_revoked` dispatch — mirroring how [`crate::lock::LockGuard`]
/// runs lease renewal on a background watchdog independent of the critical
/// section. Callbacks are dispatched via `tokio::spawn` (fire-and-forget) so
/// a slow or blocking callback can never delay the next renewal attempt and
/// cause the Redis key's TTL to lapse underneath a still-believed-held lease.
///
/// Because callbacks run detached from the renewal loop, a long-running
/// `on_elected` callback MUST NOT assume it still holds leadership just
/// because it was invoked — leadership can be lost while the callback is
/// still running. Use [`LeaderElection::fence`] to obtain a cheap, cloneable
/// [`LeaderFence`] handle and capture it in the callback closure; call
/// [`LeaderFence::is_held`] (or `select!` on [`LeaderFence::lost`])
/// immediately before committing any externally-visible side effect, the
/// same way [`crate::lock::LockGuard::is_held`]/[`crate::lock::LockGuard::lost`]
/// are used for distributed locks.
///
/// # Callback concurrency (behavior change)
///
/// Because dispatch is `tokio::spawn`-based rather than awaited in-line,
/// `on_elected`/`on_revoked` callbacks now run *detached* and
/// *concurrently* — with each other (e.g. a rapid leadership flap can leave
/// an `on_revoked` dispatch still running when the next `on_elected`
/// dispatch starts) and with ongoing lease renewal — rather than serialized
/// one at a time on the renewal loop's own task. Callback implementations
/// that assume non-reentrancy, or that share mutable state across
/// invocations without their own synchronization, may behave differently
/// than before as a result and should synchronize explicitly (or use
/// [`LeaderElection::fence`] as described above) if that matters for
/// correctness.
pub struct LeaderElection {
    /// Election key in Redis
    key: String,

    /// Unique node ID
    node_id: String,

    /// TTL for leadership
    ttl: Duration,

    /// Refresh interval (should be less than TTL)
    refresh_interval: Duration,

    /// Redis connection. `ConnectionManager` is `Clone` and internally
    /// multiplexed, so each call site clones it instead of locking.
    conn: redis::aio::ConnectionManager,

    /// Leadership state: whether this node currently believes it holds
    /// leadership, plus the loss-notification primitive that wakes
    /// [`LeaderFence::lost`] waiters. Shared with every [`LeaderFence`]
    /// obtained via [`LeaderElection::fence`] and with the detached renewal
    /// task spawned by [`LeaderElection::start`].
    leadership: Fence,

    /// Callback when becoming leader
    on_elected: Option<LeaderCallback>,

    /// Callback when losing leadership
    on_revoked: Option<LeaderCallback>,

    /// Running flag
    running: Arc<AtomicBool>,

    /// Stop-signal sender and abort handle for the detached renewal task
    /// spawned by [`LeaderElection::start`], stored so `Drop` can request a
    /// clean shutdown (falling back to a hard abort) if this instance is
    /// dropped while that task is still running independently of `start`'s
    /// own future — e.g. a caller that only aborts the thin wrapper task
    /// returned by `tokio::spawn(async move { election.start().await })`
    /// without calling [`LeaderElection::stop`] first. `None` whenever no
    /// renewal task is currently running. Mirrors [`crate::lock::LockGuard`]'s
    /// `watchdog`/`watchdog_stop` fields.
    renewal: Mutex<Option<RenewalHandle>>,
}

/// See the `renewal` field on [`LeaderElection`].
struct RenewalHandle {
    stop: oneshot::Sender<()>,
    abort: AbortHandle,
}

/// Shared "register-before-check" fencing primitive: an `AtomicBool` +
/// `Notify` pair that can be marked lost exactly once, and lets waiters
/// `select!` on the loss without racing the check-then-await window (the
/// `Notify` is subscribed *before* the flag is re-checked, so a loss that
/// happens in between is never missed).
///
/// [`LeaderElection`]'s own leadership state and [`LeaderFence`] both
/// compose this type rather than duplicating the pattern by hand.
///
/// This is defined `pub(crate)` here, rather than in its own module, because
/// at the time of writing this crate's module list (`lib.rs`) and
/// `Cargo.toml` are owned by a parallel change in the same review batch as
/// this one. [`crate::lock::LockGuard`]'s `held`/`lost_notify` fields (see
/// its `is_held`/`lost`) implement the identical pattern by hand and could
/// be migrated onto this same type in a future pass once that's no longer a
/// concern.
#[derive(Clone)]
pub(crate) struct Fence {
    held: Arc<AtomicBool>,
    lost_notify: Arc<Notify>,
}

impl Fence {
    fn new(initially_held: bool) -> Self {
        Self {
            held: Arc::new(AtomicBool::new(initially_held)),
            lost_notify: Arc::new(Notify::new()),
        }
    }

    fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    /// Unconditionally mark the fence lost and wake any `lost()` waiters.
    fn mark_lost(&self) {
        self.held.store(false, Ordering::Release);
        self.lost_notify.notify_waiters();
    }

    /// Mark the fence lost only if it was currently held, waking `lost()`
    /// waiters if so. Returns whether it was held (and thus just
    /// transitioned) — useful when the caller doesn't already know the
    /// prior state and needs that to decide whether to fire a one-time
    /// "just lost" side effect (e.g. dispatching an `on_revoked` callback).
    fn mark_lost_if_held(&self) -> bool {
        let was_held = self.held.swap(false, Ordering::Release);
        if was_held {
            self.lost_notify.notify_waiters();
        }
        was_held
    }

    /// Mark the fence held. No notification is sent; only `lost()` waiters
    /// exist.
    fn mark_held(&self) {
        self.held.store(true, Ordering::Release);
    }

    /// Resolves once the fence is lost (see [`Fence::is_held`]). If already
    /// lost, resolves immediately.
    async fn lost(&self) {
        // Register interest *before* re-checking the flag so a loss that
        // happens between the check and the await is not missed.
        let notified = self.lost_notify.notified();
        if !self.held.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

/// Lightweight, cloneable fencing handle for a [`LeaderElection`].
///
/// Mirrors [`crate::lock::LockGuard`]'s `is_held`/`lost` fencing pair. Obtain
/// one via [`LeaderElection::fence`] *before* wrapping the election in an
/// `Arc` and starting it, then capture it in an `on_elected`/`on_revoked`
/// closure so long-running callback work can detect a stale leadership
/// belief instead of assuming the callback firing means leadership is still
/// held for the callback's entire duration.
///
/// ```rust,ignore
/// let election = LeaderElection::new(key, ttl, conn);
/// let fence = election.fence();
/// let election = election.on_elected(move || {
///     let fence = fence.clone();
///     async move {
///         tokio::select! {
///             _ = fence.lost() => { /* stand down, do not commit */ }
///             result = do_work() => { if fence.is_held() { commit(result); } }
///         }
///     }
/// });
/// ```
#[derive(Clone)]
pub struct LeaderFence {
    fence: Fence,
}

impl LeaderFence {
    /// Returns `true` while this node can still be believed to hold
    /// leadership, and `false` once a loss has been observed (lost the CAS
    /// race, or sustained renewal errors revoked leadership). A `true`
    /// result is not a guarantee for all future time — it means no loss has
    /// been detected as of this call, mirroring
    /// [`crate::lock::LockGuard::is_held`].
    pub fn is_held(&self) -> bool {
        self.fence.is_held()
    }

    /// Resolves once leadership is lost (see [`LeaderFence::is_held`]). If
    /// leadership has already been lost, resolves immediately. Intended for
    /// use in a `tokio::select!` alongside callback work, mirroring
    /// [`crate::lock::LockGuard::lost`].
    pub async fn lost(&self) {
        self.fence.lost().await
    }
}

impl LeaderElection {
    /// Create new leader election coordinator
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_distributed::LeaderElection;
    /// use std::time::Duration;
    ///
    /// let client = redis::Client::open("redis://127.0.0.1/")?;
    /// let conn = client.get_connection_manager().await?;
    ///
    /// let election = LeaderElection::new(
    ///     "my-service-leader",
    ///     Duration::from_secs(30),
    ///     conn,
    /// );
    /// ```
    pub fn new(key: impl Into<String>, ttl: Duration, conn: redis::aio::ConnectionManager) -> Self {
        let refresh_interval = Self::refresh_interval_for(ttl);

        Self {
            key: key.into(),
            node_id: Uuid::new_v4().to_string(),
            ttl,
            refresh_interval,
            conn,
            leadership: Fence::new(false),
            on_elected: None,
            on_revoked: None,
            running: Arc::new(AtomicBool::new(false)),
            renewal: Mutex::new(None),
        }
    }

    /// Compute the renewal cadence for a given `ttl`: a third of the TTL,
    /// mirroring `LockGuard::new`'s watchdog interval, clamped to at least
    /// 1ms so a sub-3ms ttl can't truncate the interval to 0 and busy-poll.
    fn refresh_interval_for(ttl: Duration) -> Duration {
        Duration::from_millis((ttl.as_millis() / 3).max(1) as u64)
    }

    /// Set callback for when this node becomes leader
    pub fn on_elected<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_elected = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Set callback for when this node loses leadership
    pub fn on_revoked<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_revoked = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Check if this node is the leader
    pub fn is_leader(&self) -> bool {
        self.leadership.is_held()
    }

    /// Obtain a cheap, cloneable fencing handle for this election.
    ///
    /// Call this *before* moving `self` into `Arc::new`/`start`, so the
    /// resulting [`LeaderFence`] can be captured in an `on_elected`/
    /// `on_revoked` closure. See [`LeaderFence`] for why this matters: since
    /// callback execution is decoupled from lease renewal, a long-running
    /// callback needs its own way to detect that leadership was lost while
    /// it was running.
    pub fn fence(&self) -> LeaderFence {
        LeaderFence {
            fence: self.leadership.clone(),
        }
    }

    /// Get the node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Start participating in leader election
    ///
    /// The Redis renewal cadence runs on its own task (`run_renewal_loop`)
    /// so that `on_elected`/`on_revoked` callbacks — dispatched via
    /// `tokio::spawn` rather than awaited in-line — can never delay the next
    /// renewal attempt. This mirrors [`crate::lock::LockGuard`]'s dedicated
    /// renewal watchdog, which is likewise fully decoupled from
    /// critical-section work. See the type-level docs on [`LeaderElection`]
    /// and [`LeaderFence`] for details.
    ///
    /// # Shutdown
    ///
    /// If the returned future runs to completion (i.e. it is awaited
    /// normally until [`LeaderElection::stop`] is called), the renewal task
    /// is joined and any held leadership is resigned before this returns.
    /// If instead the *caller's own wrapper task* around this future is
    /// aborted (e.g. `let h = tokio::spawn(async move { election.start().await });`
    /// later followed by `h.abort()`) without calling `stop()` first, this
    /// future never gets the chance to run that cleanup itself — but
    /// dropping this `LeaderElection`'s last `Arc` (which happens as part of
    /// tearing down the aborted wrapper future, since it owns the `Arc<Self>`
    /// receiver) still stops the detached renewal task, via `Drop`.
    ///
    /// # Errors
    ///
    /// Returns `Err(LeaderError::RenewalTaskPanicked)` if the renewal task
    /// panics. Local leadership state is unconditionally cleared before this
    /// returns in that case (see the split-brain note there), independent of
    /// whether the subsequent best-effort `resign()` succeeds.
    pub async fn start(self: Arc<Self>) -> Result<(), LeaderError> {
        self.running.store(true, Ordering::Release);

        info!(
            "Starting leader election for key: {} (node: {})",
            self.key, self.node_id
        );

        // Run the renewal loop on its own task, independent of this future,
        // so nothing about how `start` is awaited/polled can affect renewal
        // cadence either.
        //
        // The task is handed a snapshot of the individual fields it needs
        // rather than an `Arc<Self>` clone. Every field snapshotted below is
        // itself `Arc`-shared (or trivially `Clone`/`Copy`), so this
        // preserves identical shared state to `self.clone()` — but
        // critically it does NOT keep `self`'s own strong count elevated.
        // If the loop held its own `Arc<Self>` instead, `self`'s strong
        // count could never reach zero while the loop was running, so
        // `Drop` below (which is what stops the loop once nothing else
        // references this `LeaderElection`) could never fire: exactly
        // backwards.
        let (stop_tx, stop_rx) = oneshot::channel();
        let renewal_state = Self {
            key: self.key.clone(),
            node_id: self.node_id.clone(),
            ttl: self.ttl,
            refresh_interval: self.refresh_interval,
            conn: self.conn.clone(),
            leadership: self.leadership.clone(),
            on_elected: self.on_elected.clone(),
            on_revoked: self.on_revoked.clone(),
            running: self.running.clone(),
            renewal: Mutex::new(None),
        };
        let renewal_task =
            tokio::spawn(async move { renewal_state.run_renewal_loop(stop_rx).await });

        // Stash the stop signal and an abort handle so `Drop` can request a
        // clean shutdown (falling back to a hard abort) if this instance is
        // dropped while the renewal task is still running independently of
        // this future. See the "Shutdown" section above and the `renewal`
        // field's doc comment.
        *self.renewal.lock().unwrap() = Some(RenewalHandle {
            stop: stop_tx,
            abort: renewal_task.abort_handle(),
        });

        let renewal_result = renewal_task.await;

        // The task finished on its own before this instance was dropped, so
        // there's nothing left for `Drop` to do.
        self.renewal.lock().unwrap().take();

        let panicked = match &renewal_result {
            Ok(_) => false,
            Err(join_err) => {
                error!("Leader election renewal task panicked: {}", join_err);
                // Unconditionally force local leadership state to false
                // BEFORE attempting any cleanup below, so a panic can never
                // leave leadership state stuck "held" — and therefore
                // `LeaderFence::is_held` stuck `true` — even if the
                // `resign()` call below also fails (e.g. Redis unreachable
                // at the same time). Without this, a panic concurrent with
                // a Redis outage would leave this node believing forever
                // that it is still leader while another node can
                // legitimately win the lease: split-brain.
                self.leadership.mark_lost();
                true
            }
        };

        // Clean up on stop or panic: best-effort resign so the Redis key is
        // released promptly instead of waiting out its TTL. This is cleanup
        // only — the local leadership state above is already correct
        // regardless of whether this call succeeds.
        if panicked || self.leadership.is_held() {
            let _ = self.resign().await;
        }

        match renewal_result {
            Ok(result) => result,
            Err(join_err) => Err(LeaderError::RenewalTaskPanicked(join_err.to_string())),
        }
    }

    /// Redis renewal loop: attempts/maintains leadership via
    /// [`Self::try_become_leader`] every [`Self::refresh_interval`] and
    /// dispatches `on_elected`/`on_revoked` callbacks as detached
    /// (`tokio::spawn`) tasks on leadership transitions. Runs until
    /// [`Self::stop`] clears the running flag or `stop_rx` fires (see
    /// [`LeaderElection::start`]'s "Shutdown" section).
    ///
    /// Crucially, callback dispatch never blocks this loop: a slow or
    /// blocking callback delays neither the next `try_become_leader` call
    /// nor the next `is_leader` transition, so the Redis key's TTL cannot
    /// lapse just because a callback is still running.
    async fn run_renewal_loop(
        &self,
        mut stop_rx: oneshot::Receiver<()>,
    ) -> Result<(), LeaderError> {
        loop {
            if !self.running.load(Ordering::Acquire) {
                break;
            }

            // Try to become leader
            match self.try_become_leader().await {
                Ok(became_leader) => {
                    let was_leader = self.leadership.is_held();

                    if became_leader && !was_leader {
                        // Newly elected
                        self.leadership.mark_held();
                        info!("Node {} became leader for {}", self.node_id, self.key);

                        Self::dispatch(&self.on_elected);
                    } else if !became_leader && was_leader {
                        // Lost leadership
                        self.leadership.mark_lost();
                        warn!("Node {} lost leadership for {}", self.node_id, self.key);

                        Self::dispatch(&self.on_revoked);
                    } else if became_leader {
                        // Still leader, just refreshed
                        debug!(
                            "Node {} refreshed leadership for {}",
                            self.node_id, self.key
                        );
                    }
                }
                Err(e) => {
                    error!("Leader election error: {}", e);

                    // If we were leader but encountered an error, we're no longer leader
                    if self.leadership.mark_lost_if_held() {
                        Self::dispatch(&self.on_revoked);
                    }
                }
            }

            // Wait before the next attempt, but wake immediately if `Drop`
            // (on the original `LeaderElection` this loop was spawned from)
            // signals a stop — see `LeaderElection::start`'s "Shutdown"
            // section for why the loop can't simply hold its own
            // `Arc<Self>` back-reference to notice that instead.
            tokio::select! {
                _ = &mut stop_rx => break,
                _ = tokio::time::sleep(self.refresh_interval) => {}
            }
        }

        Ok(())
    }

    /// Dispatch a leadership callback, if set, as a detached task so its
    /// duration cannot delay the renewal loop that triggered it. The
    /// callback's own panics are caught and logged (rather than being
    /// silently swallowed by a dropped `JoinHandle`) by awaiting it inside a
    /// second, supervisory `tokio::spawn`.
    fn dispatch(callback: &Option<LeaderCallback>) {
        if let Some(callback) = callback {
            let callback = callback.clone();
            tokio::spawn(async move {
                if let Err(join_err) = tokio::spawn(async move { callback().await }).await {
                    error!(error = %join_err, "Leader election callback panicked");
                }
            });
        }
    }

    /// Stop participating in leader election
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    /// Try to become or maintain leadership
    async fn try_become_leader(&self) -> Result<bool, LeaderError> {
        let mut conn = self.conn.clone();
        let ttl_ms = self.ttl.as_millis() as usize;

        // Use Lua script for atomic operation
        let script = r#"
            local current = redis.call("get", KEYS[1])
            if current == false or current == ARGV[1] then
                redis.call("set", KEYS[1], ARGV[1], "PX", ARGV[2])
                return 1
            else
                return 0
            end
        "#;

        let result: i32 = redis::Script::new(script)
            .key(&self.key)
            .arg(&self.node_id)
            .arg(ttl_ms)
            .invoke_async(&mut conn)
            .await?;

        Ok(result == 1)
    }

    /// Resign from leadership
    async fn resign(&self) -> Result<(), LeaderError> {
        let mut conn = self.conn.clone();

        // Only delete if we're still the leader (token-guarded release,
        // shared with the distributed-lock release path).
        let _: i32 = redis::Script::new(crate::RELEASE_SCRIPT)
            .key(&self.key)
            .arg(&self.node_id)
            .invoke_async(&mut conn)
            .await?;

        // Wake any in-flight callback `select!`ing on a `LeaderFence::lost`
        // it captured, so voluntary resignation (e.g. via `stop`) is
        // observed promptly even though callback dispatch is detached from
        // the renewal loop.
        self.leadership.mark_lost();
        info!("Node {} resigned from leadership", self.node_id);

        Ok(())
    }

    /// Get current leader node ID
    pub async fn get_leader(&self) -> Result<Option<String>, LeaderError> {
        let mut conn = self.conn.clone();
        let leader: Option<String> = conn.get(&self.key).await?;
        Ok(leader)
    }
}

impl Drop for LeaderElection {
    fn drop(&mut self) {
        // If `start()`'s wrapper future is aborted rather than run to
        // completion (see its "Shutdown" section), only the thin wrapper is
        // cancelled — the renewal loop it spawned is a fully independent
        // task and would otherwise keep renewing the lease forever. Once
        // the last `Arc<Self>` referencing this instance is dropped (which
        // happens as part of unwinding the aborted wrapper future, since it
        // owns an `Arc<Self>` receiver), signal the loop to stop cleanly,
        // with a hard abort as a fallback in case it's currently stuck
        // somewhere other than the stop-signal `select!` (e.g. mid-Redis
        // call). Mirrors `LockGuard::stop_watchdog`, invoked from
        // `LockGuard`'s own `Drop`.
        if let Some(renewal) = self.renewal.lock().unwrap().take() {
            let _ = renewal.stop.send(());
            renewal.abort.abort();
        }
    }
}

/// Leader election builder
pub struct LeaderElectionBuilder {
    key: String,
    ttl: Duration,
    on_elected: Option<LeaderCallback>,
    on_revoked: Option<LeaderCallback>,
}

impl LeaderElectionBuilder {
    /// Create new builder
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ttl: Duration::from_secs(30),
            on_elected: None,
            on_revoked: None,
        }
    }

    /// Set TTL
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Set elected callback
    pub fn on_elected<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_elected = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Set revoked callback
    pub fn on_revoked<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_revoked = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Build the leader election coordinator
    pub fn build(self, conn: redis::aio::ConnectionManager) -> LeaderElection {
        let mut election = LeaderElection::new(self.key, self.ttl, conn);
        election.on_elected = self.on_elected;
        election.on_revoked = self.on_revoked;
        election
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leader_election_builder() {
        let builder = LeaderElectionBuilder::new("test-leader").with_ttl(Duration::from_secs(60));

        assert_eq!(builder.key, "test-leader");
        assert_eq!(builder.ttl, Duration::from_secs(60));
    }

    // --- Finding 2: refresh_interval lower-bound clamp ---
    //
    // Pure computation, no Redis connection needed, so this runs unconditionally
    // (unlike the Docker-gated tests below).

    #[test]
    fn refresh_interval_clamps_to_at_least_one_millisecond() {
        // ttl / 3 would truncate to 0ms for any ttl < 3ms; the clamp must
        // keep the sleep from becoming a zero-duration busy-poll.
        assert_eq!(
            LeaderElection::refresh_interval_for(Duration::from_millis(0)),
            Duration::from_millis(1)
        );
        assert_eq!(
            LeaderElection::refresh_interval_for(Duration::from_millis(1)),
            Duration::from_millis(1)
        );
        assert_eq!(
            LeaderElection::refresh_interval_for(Duration::from_millis(2)),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn refresh_interval_is_a_third_of_ttl_above_the_clamp() {
        assert_eq!(
            LeaderElection::refresh_interval_for(Duration::from_millis(9)),
            Duration::from_millis(3)
        );
        assert_eq!(
            LeaderElection::refresh_interval_for(Duration::from_secs(3)),
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn single_leader_among_two_contenders() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let client = redis::Client::open(container.url()).expect("open redis client");
        let conn1 = client
            .get_connection_manager()
            .await
            .expect("get connection manager");
        let conn2 = client
            .get_connection_manager()
            .await
            .expect("get connection manager");

        let ttl = Duration::from_secs(3);
        let e1 = Arc::new(LeaderElection::new("wf3-single-leader", ttl, conn1));
        let e2 = Arc::new(LeaderElection::new("wf3-single-leader", ttl, conn2));

        let e1_run = e1.clone();
        let h1 = tokio::spawn(async move { e1_run.start().await });
        let e2_run = e2.clone();
        let h2 = tokio::spawn(async move { e2_run.start().await });

        // Give both contenders several rounds to converge on a single leader.
        tokio::time::sleep(Duration::from_millis(800)).await;

        assert_ne!(
            e1.is_leader(),
            e2.is_leader(),
            "exactly one of the two contenders should hold leadership"
        );

        e1.stop().await;
        e2.stop().await;
        let _ = h1.await;
        let _ = h2.await;
    }

    // --- Finding 1: callback execution must not block lease renewal ---

    #[tokio::test]
    async fn slow_on_elected_callback_does_not_block_lease_renewal() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let client = redis::Client::open(container.url()).expect("open redis client");
        let conn = client
            .get_connection_manager()
            .await
            .expect("get connection manager");

        // A short ttl so a coupled implementation would let the key expire
        // well within the test's timeout budget.
        let ttl = Duration::from_millis(300);
        let callback_started = Arc::new(Notify::new());
        let callback_started_tx = callback_started.clone();

        let election = Arc::new(
            LeaderElection::new("wf3-slow-callback", ttl, conn).on_elected(move || {
                let callback_started_tx = callback_started_tx.clone();
                async move {
                    callback_started_tx.notify_one();
                    // Sleep far longer than the ttl. If renewal were coupled
                    // to callback completion (the pre-fix behavior), the
                    // lease would lapse while this sleep is in flight.
                    tokio::time::sleep(ttl * 10).await;
                }
            }),
        );

        let election_run = election.clone();
        let handle = tokio::spawn(async move { election_run.start().await });

        tokio::time::timeout(Duration::from_secs(2), callback_started.notified())
            .await
            .expect("on_elected callback should start promptly after election");

        // Sleep past several ttl windows while the slow callback is still
        // running (it sleeps for 10x ttl). A coupled implementation would
        // have missed every renewal during this window and lost the key.
        tokio::time::sleep(ttl * 4).await;

        assert!(
            election.is_leader(),
            "node must still believe it is leader while the slow on_elected callback \
             is still running: renewal must not be blocked by callback execution"
        );

        // Verify directly against Redis, not just the in-process flag: the
        // key must still be present and still owned by this node.
        let leader = election
            .get_leader()
            .await
            .expect("get_leader should not error");
        assert_eq!(
            leader.as_deref(),
            Some(election.node_id()),
            "the Redis key must still be renewed under this node's id while the \
             callback runs"
        );

        election.stop().await;
        let _ = handle.await;
    }

    // --- LeaderFence: fencing/loss-detection for long-running callbacks ---

    #[tokio::test]
    async fn fence_lost_resolves_when_leadership_is_lost() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let client = redis::Client::open(container.url()).expect("open redis client");
        let conn = client
            .get_connection_manager()
            .await
            .expect("get connection manager");
        let mut stealer = client
            .get_connection_manager()
            .await
            .expect("get connection manager");

        let ttl = Duration::from_millis(300);
        let election = Arc::new(LeaderElection::new("wf3-fence-lost", ttl, conn));
        let fence = election.fence();
        assert!(
            !fence.is_held(),
            "fence should start unheld before election runs"
        );

        let election_run = election.clone();
        let handle = tokio::spawn(async move { election_run.start().await });

        tokio::time::timeout(Duration::from_secs(2), async {
            while !election.is_leader() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("node should become leader promptly (sole contender)");
        assert!(fence.is_held(), "fence must reflect acquired leadership");

        // Steal the key with a foreign token so the node's next renewal
        // attempt observes a mismatch and treats leadership as lost.
        let _: () = redis::cmd("SET")
            .arg("wf3-fence-lost")
            .arg("foreign-token")
            .query_async(&mut stealer)
            .await
            .expect("steal the leader key");

        tokio::time::timeout(Duration::from_secs(2), fence.lost())
            .await
            .expect("fence should observe leadership loss within the timeout");
        assert!(
            !fence.is_held(),
            "fence must flip to unheld once leadership has been lost"
        );

        election.stop().await;
        let _ = handle.await;
    }

    // --- Finding 4: LeaderFence's doc-example select! usage pattern ---
    //
    // The regression test above (`fence_lost_resolves_when_leadership_is_lost`)
    // only covers that `fence.lost()` resolves; it doesn't exercise the
    // actual pattern `LeaderFence`'s docs sell it for: a long-running
    // callback racing `fence.lost()` against its own work in a `select!` so
    // it stands down instead of committing after leadership was lost mid-flight.

    #[tokio::test]
    async fn fence_select_pattern_stands_down_instead_of_committing_when_leadership_is_lost_mid_flight()
     {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let client = redis::Client::open(container.url()).expect("open redis client");
        let conn = client
            .get_connection_manager()
            .await
            .expect("get connection manager");
        let mut stealer = client
            .get_connection_manager()
            .await
            .expect("get connection manager");

        let key = "wf3-fence-select-pattern";
        let ttl = Duration::from_millis(300);
        let election = Arc::new(LeaderElection::new(key, ttl, conn));
        let fence = election.fence();

        let election_run = election.clone();
        let handle = tokio::spawn(async move { election_run.start().await });

        tokio::time::timeout(Duration::from_secs(2), async {
            while !election.is_leader() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("node should become leader promptly (sole contender)");

        // Steal the key from a second connection shortly after the "work"
        // below starts, so the loss is discovered mid-flight rather than
        // before the select even starts — same steal technique as
        // `fence_lost_resolves_when_leadership_is_lost`.
        let key_owned = key.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _: () = redis::cmd("SET")
                .arg(&key_owned)
                .arg("foreign-token")
                .query_async(&mut stealer)
                .await
                .expect("steal the leader key");
        });

        // Mirrors the doc example on `LeaderFence`: an `on_elected`-style
        // callback captures `fence.clone()` and races `fence.lost()`
        // against its own "do the work" future, so it can detect a stale
        // leadership belief instead of blindly committing after whatever
        // leadership check let it start running in the first place.
        #[derive(Debug, PartialEq, Eq)]
        enum Outcome {
            StoodDown,
            Committed,
        }

        let outcome = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                _ = fence.lost() => Outcome::StoodDown,
                _ = tokio::time::sleep(Duration::from_millis(600)) => Outcome::Committed,
            }
        })
        .await
        .expect("select should resolve within the timeout");

        assert_eq!(
            outcome,
            Outcome::StoodDown,
            "fence.lost() must win the select once leadership is stolen mid-flight, so \
             the caller stands down instead of committing work performed under a \
             leadership belief that was no longer current"
        );

        election.stop().await;
        let _ = handle.await;
    }
}
