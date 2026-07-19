//! Distributed locks using Redis

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::{Notify, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::RELEASE_SCRIPT;

/// Number of consecutive renewal errors the watchdog tolerates before it
/// treats the lease as lost. Transient Redis blips should not fence the
/// holder, but sustained failure means we can no longer prove we hold the
/// lock, so the critical section must be told to stand down.
const MAX_RENEW_ERRORS: u32 = 3;

const RENEW_SCRIPT: &str = r#"
    if redis.call("get", KEYS[1]) == ARGV[1] then
        redis.call("pexpire", KEYS[1], ARGV[2])
        return 1
    else
        return 0
    end
"#;

/// Distributed lock errors
#[derive(Debug, Error)]
pub enum LockError {
    #[error("Failed to acquire lock: {0}")]
    AcquireFailed(String),

    #[error("Failed to release lock: {0}")]
    ReleaseFailed(String),

    #[error("Lock timeout")]
    Timeout,

    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),

    #[error("Lock not held")]
    NotHeld,
}

/// Distributed lock trait
#[async_trait]
pub trait DistributedLock: Send + Sync {
    /// Acquire the lock
    async fn acquire(&self) -> Result<LockGuard, LockError>;

    /// Try to acquire the lock (non-blocking)
    async fn try_acquire(&self) -> Result<Option<LockGuard>, LockError>;

    /// Acquire with timeout
    async fn acquire_timeout(&self, timeout: Duration) -> Result<LockGuard, LockError>;
}

/// Lock guard that automatically releases on drop.
///
/// While held, a background watchdog task periodically refreshes the key's
/// TTL in Redis (guarded by the lock's token, via [`RENEW_SCRIPT`]) so the
/// lock does not silently expire while the critical section is still
/// running. The watchdog is cancelled when the guard is released or dropped.
///
/// # Fencing
///
/// The watchdog cannot _prevent_ another node from stealing the lease (for
/// example if this node stalls past the TTL and the key expires), but it
/// _detects_ it: when a renewal finds the token no longer matches, or renewal
/// fails repeatedly ([`MAX_RENEW_ERRORS`] consecutive errors), the guard is
/// marked lost. Callers running a mutually-exclusive critical section MUST
/// re-check [`LockGuard::is_held`] (or `select!` on [`LockGuard::lost`])
/// immediately before committing any externally-visible side effect; holding
/// the guard value is not by itself proof the lease is still ours.
pub struct LockGuard {
    key: String,
    token: String,
    conn: redis::aio::ConnectionManager,
    /// Signals the watchdog task to stop; dropping the sender also stops it.
    watchdog_stop: Option<oneshot::Sender<()>>,
    /// Handle to the watchdog task, kept so `drop` can detach it cleanly.
    watchdog: Option<JoinHandle<()>>,
    /// `true` while we can still prove we hold the lease; flipped to `false`
    /// by the watchdog when the lease is lost (token mismatch or sustained
    /// renewal failure). Shared with the watchdog task.
    held: Arc<AtomicBool>,
    /// Notified once when `held` transitions to `false`, so callers can
    /// `select!` on [`LockGuard::lost`] instead of polling.
    lost_notify: Arc<Notify>,
}

impl LockGuard {
    /// Create a new lock guard and spawn its renewal watchdog.
    ///
    /// `ttl` is the lock's TTL; the watchdog renews at `ttl / 3`, mirroring
    /// `LeaderElection`'s refresh cadence.
    fn new(key: String, token: String, conn: redis::aio::ConnectionManager, ttl: Duration) -> Self {
        let (watchdog_stop, mut stop_rx) = oneshot::channel();
        let held = Arc::new(AtomicBool::new(true));
        let lost_notify = Arc::new(Notify::new());

        let watchdog = {
            let key = key.clone();
            let token = token.clone();
            let mut conn = conn.clone();
            let ttl_ms = ttl.as_millis().max(1) as usize;
            let interval = Duration::from_millis((ttl.as_millis() / 3).max(1) as u64);
            let held = held.clone();
            let lost_notify = lost_notify.clone();

            tokio::spawn(async move {
                // Fence the holder: on token mismatch (lease stolen/expired) or
                // sustained renewal failure, flip `held` to false and wake any
                // `lost()` waiter so the critical section can stand down.
                let mark_lost = |held: &AtomicBool, lost_notify: &Notify| {
                    held.store(false, Ordering::Release);
                    lost_notify.notify_waiters();
                };

                let mut consecutive_errors: u32 = 0;

                loop {
                    tokio::select! {
                        _ = &mut stop_rx => break,
                        _ = tokio::time::sleep(interval) => {}
                    }

                    let result: Result<i32, _> = redis::Script::new(RENEW_SCRIPT)
                        .key(&key)
                        .arg(&token)
                        .arg(ttl_ms)
                        .invoke_async(&mut conn)
                        .await;

                    match result {
                        Ok(1) => {
                            consecutive_errors = 0;
                            debug!("Renewed lock: {}", key);
                        }
                        Ok(_) => {
                            warn!(
                                "Lock renewal found lock no longer held (lease lost to another holder): {}",
                                key
                            );
                            mark_lost(&held, &lost_notify);
                            break;
                        }
                        Err(e) => {
                            consecutive_errors += 1;
                            warn!(
                                "Lock renewal failed for {} ({}/{}): {}",
                                key, consecutive_errors, MAX_RENEW_ERRORS, e
                            );
                            if consecutive_errors >= MAX_RENEW_ERRORS {
                                warn!(
                                    "Lock renewal failed {} times in a row; treating lease as lost: {}",
                                    consecutive_errors, key
                                );
                                mark_lost(&held, &lost_notify);
                                break;
                            }
                        }
                    }
                }
            })
        };

        Self {
            key,
            token,
            conn,
            watchdog_stop: Some(watchdog_stop),
            watchdog: Some(watchdog),
            held,
            lost_notify,
        }
    }

    /// Stop the renewal watchdog (best effort; does not wait for it to exit).
    fn stop_watchdog(&mut self) {
        if let Some(stop) = self.watchdog_stop.take() {
            let _ = stop.send(());
        }
        // Detach rather than await: `release`/`drop` must not block on the
        // watchdog's next wake-up.
        self.watchdog.take();
    }

    /// Returns `true` while the lease can still be proven held, and `false`
    /// once the renewal watchdog has observed the lease being lost (token
    /// mismatch, i.e. stolen/expired, or [`MAX_RENEW_ERRORS`] consecutive
    /// renewal failures).
    ///
    /// Critical sections that must be mutually exclusive across nodes should
    /// call this immediately before committing any externally-visible side
    /// effect. A `true` result is not a guarantee for all future time — it
    /// means no loss has been detected as of this call.
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }

    /// Resolves once the lease is lost (see [`LockGuard::is_held`]). If the
    /// lease has already been lost, resolves immediately. Intended for use in
    /// a `tokio::select!` alongside the critical-section work so the holder
    /// can abort the moment fencing is detected:
    ///
    /// ```rust,ignore
    /// tokio::select! {
    ///     _ = guard.lost() => { /* stand down, do not commit */ }
    ///     result = do_work() => { if guard.is_held() { commit(result); } }
    /// }
    /// ```
    pub async fn lost(&self) {
        // Register interest *before* re-checking the flag so a loss that
        // happens between the check and the await is not missed.
        let notified = self.lost_notify.notified();
        if !self.held.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }

    /// Manually release the lock
    pub async fn release(mut self) -> Result<(), LockError> {
        self.stop_watchdog();
        self.release_internal().await
    }

    async fn release_internal(&mut self) -> Result<(), LockError> {
        // Use Lua script for atomic, token-guarded release.
        let result: i32 = redis::Script::new(RELEASE_SCRIPT)
            .key(&self.key)
            .arg(&self.token)
            .invoke_async(&mut self.conn)
            .await?;

        if result == 1 {
            debug!("Released lock: {}", self.key);
            Ok(())
        } else {
            warn!("Failed to release lock (not held or expired): {}", self.key);
            Err(LockError::NotHeld)
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self.stop_watchdog();

        // Best-effort release on drop. This is only safe to spawn when a
        // Tokio runtime is actually running on this thread; dropping a guard
        // outside of one (e.g. during process shutdown after the runtime has
        // been torn down) must not panic, so we skip the release in that
        // case rather than call `tokio::spawn` unconditionally.
        let Ok(handle) = Handle::try_current() else {
            warn!(
                "Lock guard for {} dropped off a Tokio runtime; skipping best-effort release",
                self.key
            );
            return;
        };

        let key = self.key.clone();
        let token = self.token.clone();
        let mut conn = self.conn.clone();

        handle.spawn(async move {
            let _: Result<i32, _> = redis::Script::new(RELEASE_SCRIPT)
                .key(&key)
                .arg(&token)
                .invoke_async(&mut conn)
                .await;
        });
    }
}

/// Redis-based distributed lock
pub struct RedisLock {
    key: String,
    ttl: Duration,
    conn: redis::aio::ConnectionManager,
}

impl RedisLock {
    /// Create new Redis lock
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_distributed::RedisLock;
    /// use std::time::Duration;
    ///
    /// let client = redis::Client::open("redis://127.0.0.1/")?;
    /// let conn = client.get_connection_manager().await?;
    /// let lock = RedisLock::new("my-resource", Duration::from_secs(30), conn);
    /// ```
    pub fn new(key: impl Into<String>, ttl: Duration, conn: redis::aio::ConnectionManager) -> Self {
        Self {
            key: key.into(),
            ttl,
            conn,
        }
    }

    /// Get the lock key
    pub fn key(&self) -> &str {
        &self.key
    }
}

#[async_trait]
impl DistributedLock for RedisLock {
    async fn acquire(&self) -> Result<LockGuard, LockError> {
        loop {
            match self.try_acquire().await? {
                Some(guard) => return Ok(guard),
                None => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn try_acquire(&self) -> Result<Option<LockGuard>, LockError> {
        let token = Uuid::new_v4().to_string();
        let ttl_ms = self.ttl.as_millis() as usize;

        let mut conn = self.conn.clone();

        // Use SET NX PX for atomic acquire with TTL
        let result: Option<String> = redis::cmd("SET")
            .arg(&self.key)
            .arg(&token)
            .arg("NX") // Only set if not exists
            .arg("PX") // Set expiry in milliseconds
            .arg(ttl_ms)
            .query_async(&mut conn)
            .await?;

        if result.is_some() {
            info!("Acquired lock: {}", self.key);
            Ok(Some(LockGuard::new(
                self.key.clone(),
                token,
                conn,
                self.ttl,
            )))
        } else {
            debug!("Failed to acquire lock (already held): {}", self.key);
            Ok(None)
        }
    }

    async fn acquire_timeout(&self, timeout: Duration) -> Result<LockGuard, LockError> {
        let start = tokio::time::Instant::now();

        loop {
            match self.try_acquire().await? {
                Some(guard) => return Ok(guard),
                None => {
                    if start.elapsed() >= timeout {
                        return Err(LockError::Timeout);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
}

/// Distributed lock builder
pub struct LockBuilder {
    key: String,
    ttl: Duration,
}

impl LockBuilder {
    /// Create new lock builder
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ttl: Duration::from_secs(30),
        }
    }

    /// Set TTL
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Build the lock
    pub fn build(self, conn: redis::aio::ConnectionManager) -> RedisLock {
        RedisLock::new(self.key, self.ttl, conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_builder() {
        let builder = LockBuilder::new("test-lock").with_ttl(Duration::from_secs(60));

        assert_eq!(builder.key, "test-lock");
        assert_eq!(builder.ttl, Duration::from_secs(60));
    }

    // --- Redis-backed semantics tests ---
    //
    // These exercise real lock semantics against a containerized Redis and
    // are skipped (not failed) when Docker is unavailable, matching the
    // pattern used elsewhere in the workspace (e.g. `armature-redis`).

    async fn connection_manager(url: &str) -> redis::aio::ConnectionManager {
        let client = redis::Client::open(url).expect("open redis client");
        client
            .get_connection_manager()
            .await
            .expect("get connection manager")
    }

    #[tokio::test]
    async fn mutual_exclusion_second_acquire_fails_while_first_held() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        let container = armature_testkit::containers::RedisContainer::start().await;
        let conn = connection_manager(&container.url()).await;

        let lock = RedisLock::new("wf3-mutex", Duration::from_secs(30), conn);

        let guard = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error")
            .expect("first acquire should succeed");

        let second = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error");
        assert!(
            second.is_none(),
            "second acquire should fail while the first guard is held"
        );

        drop(guard);
    }

    #[tokio::test]
    async fn token_guarded_release_does_not_delete_lock_reacquired_by_someone_else() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        let container = armature_testkit::containers::RedisContainer::start().await;
        let conn1 = connection_manager(&container.url()).await;
        let conn2 = connection_manager(&container.url()).await;

        let short_ttl = Duration::from_millis(200);
        let lock = RedisLock::new("wf3-token-guard", short_ttl, conn1);

        let mut guard = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error")
            .expect("first acquire should succeed");
        // Disable the renewal watchdog so the key can expire naturally,
        // simulating a stale holder whose lease lapsed.
        guard.stop_watchdog();

        tokio::time::sleep(short_ttl * 3).await;

        // A different holder re-acquires the now-expired key.
        let lock2 = RedisLock::new("wf3-token-guard", Duration::from_secs(30), conn2);
        let guard2 = lock2
            .try_acquire()
            .await
            .expect("try_acquire should not error")
            .expect("second holder should be able to acquire the expired lock");

        // The stale guard's release must be a no-op (token mismatch), not a
        // deletion of the new holder's lock.
        let release_result = guard.release().await;
        assert!(
            matches!(release_result, Err(LockError::NotHeld)),
            "stale release should fail with NotHeld, got {release_result:?}"
        );

        // The new holder's lock must still be intact.
        let still_held = lock2
            .try_acquire()
            .await
            .expect("try_acquire should not error");
        assert!(
            still_held.is_none(),
            "new holder's lock must still be held after the stale release"
        );

        drop(guard2);
    }

    #[test]
    fn dropping_guard_off_a_tokio_runtime_does_not_panic() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        // Construct a real guard inside a runtime (acquiring the lock and
        // spawning its watchdog both need one), then move it out and drop it
        // from a plain OS thread that has no Tokio runtime, exercising the
        // `Handle::try_current()` fallback in `Drop` (which must skip the
        // best-effort release rather than panic).
        let rt = tokio::runtime::Runtime::new().expect("build runtime");
        let guard = rt.block_on(async {
            let container = armature_testkit::containers::RedisContainer::start().await;
            let conn = connection_manager(&container.url()).await;
            let lock = RedisLock::new("wf3-drop-off-runtime", Duration::from_secs(30), conn);
            lock.try_acquire()
                .await
                .expect("try_acquire should not error")
                .expect("acquire should succeed")
            // `container` is dropped here; the off-runtime drop below performs
            // no Redis I/O, so the stopped container does not matter.
        });
        assert!(guard.is_held(), "freshly acquired guard should report held");

        let handle = std::thread::spawn(move || {
            drop(guard);
        });
        handle
            .join()
            .expect("dropping a LockGuard off any Tokio runtime must not panic");
    }

    #[tokio::test]
    async fn is_held_flips_false_when_the_lease_is_stolen() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        let container = armature_testkit::containers::RedisContainer::start().await;
        let conn = connection_manager(&container.url()).await;
        let mut stealer = connection_manager(&container.url()).await;

        let ttl = Duration::from_millis(300);
        let lock = RedisLock::new("wf3-fencing", ttl, conn);

        let guard = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error")
            .expect("first acquire should succeed");
        assert!(guard.is_held(), "guard should start held");

        // Steal the lease: overwrite the key with a foreign token so the
        // holder's next renewal sees a token mismatch (Lua returns 0).
        let _: () = redis::cmd("SET")
            .arg("wf3-fencing")
            .arg("foreign-token")
            .query_async(&mut stealer)
            .await
            .expect("steal the lock key");

        // The watchdog should observe the mismatch on its next renewal and
        // wake `lost()`.
        tokio::time::timeout(Duration::from_secs(2), guard.lost())
            .await
            .expect("watchdog should mark the lease lost within the timeout");

        assert!(
            !guard.is_held(),
            "is_held must flip to false once the lease has been stolen"
        );
    }

    #[tokio::test]
    async fn renewal_watchdog_keeps_lock_held_past_original_ttl() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        let container = armature_testkit::containers::RedisContainer::start().await;
        let conn = connection_manager(&container.url()).await;

        let ttl = Duration::from_millis(300);
        let lock = RedisLock::new("wf3-renewal", ttl, conn);

        let guard = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error")
            .expect("first acquire should succeed");

        // Wait past the original TTL; without a renewal watchdog the key
        // would have expired by now.
        tokio::time::sleep(ttl * 2).await;

        let second = lock
            .try_acquire()
            .await
            .expect("try_acquire should not error");
        assert!(
            second.is_none(),
            "lock should still be held past its original TTL thanks to the renewal watchdog"
        );

        drop(guard);
    }
}
