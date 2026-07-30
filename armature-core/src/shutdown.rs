//! Graceful shutdown support for Armature applications
//!
//! This module provides connection draining, shutdown hooks, and coordinated
//! shutdown for web applications.
//!
//! # Features
//!
//! - **Connection Draining** - Wait for in-flight requests to complete
//! - **Shutdown Hooks** - Register custom cleanup functions
//! - **Timeout Support** - Force shutdown after timeout
//! - **Signal Handling** - Respond to SIGTERM, SIGINT
//!
//! Note: this module does not itself update any health-check/readiness
//! status. If your health probes (see [`crate::health`]) should report
//! unhealthy/not-ready during shutdown, mark that status yourself (e.g. via
//! a shutdown hook registered with [`ShutdownManager::add_hook`], or in the
//! same task that calls [`ShutdownManager::initiate_shutdown`]) before or
//! around calling it.
//!
//! # Quick Start
//!
//! ```no_run
//! use armature_core::*;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let shutdown_manager = Arc::new(ShutdownManager::new());
//!
//! // Register shutdown hook
//! shutdown_manager.add_hook(Box::new(|| {
//!     Box::pin(async {
//!         println!("Cleaning up resources...");
//!         Ok(())
//!     })
//! }));
//!
//! // Start shutdown on signal
//! tokio::spawn(async move {
//!     tokio::signal::ctrl_c().await.ok();
//!     shutdown_manager.initiate_shutdown().await;
//! });
//! # Ok(())
//! # }
//! ```

use crate::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{error, info, warn};

/// Shutdown hook function type
///
/// Async function that performs cleanup during shutdown.
pub type ShutdownHook = Box<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>>
        + Send
        + Sync,
>;

/// Connection tracker for draining in-flight requests
#[derive(Debug, Clone)]
pub struct ConnectionTracker {
    /// Number of active connections
    active: Arc<AtomicU64>,
    /// Whether new connections are allowed
    accepting: Arc<AtomicBool>,
}

impl ConnectionTracker {
    /// Create a new connection tracker
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicU64::new(0)),
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Increment active connection count
    ///
    /// Returns None if not accepting new connections.
    pub fn increment(&self) -> Option<ConnectionGuard> {
        if !self.accepting.load(Ordering::Acquire) {
            return None;
        }

        self.active.fetch_add(1, Ordering::SeqCst);
        Some(ConnectionGuard {
            tracker: self.clone(),
        })
    }

    /// Get number of active connections
    pub fn active_count(&self) -> u64 {
        self.active.load(Ordering::Acquire)
    }

    /// Stop accepting new connections
    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Release);
    }

    /// Check if accepting new connections
    pub fn is_accepting(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
    }

    /// Wait for all connections to drain
    ///
    /// Returns true if drained within timeout, false otherwise.
    pub async fn drain(&self, timeout_duration: Duration) -> bool {
        let start = tokio::time::Instant::now();

        while self.active_count() > 0 {
            if start.elapsed() >= timeout_duration {
                warn!(
                    "Connection drain timeout reached with {} active connections",
                    self.active_count()
                );
                return false;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("All connections drained successfully");
        true
    }

    /// Decrement active connection count (internal)
    fn decrement(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard for tracking a connection
///
/// Automatically decrements count when dropped.
#[derive(Debug)]
pub struct ConnectionGuard {
    tracker: ConnectionTracker,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.tracker.decrement();
    }
}

/// Shutdown manager coordinates graceful shutdown
///
/// Handles connection draining and shutdown hooks. It does not update any
/// health-check/readiness status itself; callers who want probes to report
/// unhealthy/not-ready during shutdown must set that status themselves
/// (e.g. in a registered shutdown hook) before or around calling
/// [`ShutdownManager::initiate_shutdown`].
pub struct ShutdownManager {
    /// Connection tracker
    tracker: ConnectionTracker,

    /// Shutdown hooks
    hooks: RwLock<Vec<ShutdownHook>>,

    /// Shutdown initiated flag
    shutdown_initiated: AtomicBool,

    /// Shutdown timeout
    timeout: RwLock<Duration>,
}

impl ShutdownManager {
    /// Create a new shutdown manager
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::*;
    ///
    /// let manager = ShutdownManager::new();
    /// ```
    pub fn new() -> Self {
        Self {
            tracker: ConnectionTracker::new(),
            hooks: RwLock::new(Vec::new()),
            shutdown_initiated: AtomicBool::new(false),
            timeout: RwLock::new(Duration::from_secs(30)),
        }
    }

    /// Set shutdown timeout
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::*;
    /// use std::time::Duration;
    ///
    /// # async fn example() {
    /// let manager = ShutdownManager::new();
    /// manager.set_timeout(Duration::from_secs(60)).await;
    /// # }
    /// ```
    pub async fn set_timeout(&self, duration: Duration) {
        let mut timeout = self.timeout.write().await;
        *timeout = duration;
    }

    /// Get connection tracker
    pub fn tracker(&self) -> &ConnectionTracker {
        &self.tracker
    }

    /// Register a shutdown hook
    ///
    /// Hooks are executed in registration order during shutdown.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::*;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = ShutdownManager::new();
    ///
    /// manager.add_hook(Box::new(|| {
    ///     Box::pin(async {
    ///         println!("Cleaning up database connections...");
    ///         Ok(())
    ///     })
    /// })).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_hook(&self, hook: ShutdownHook) {
        let mut hooks = self.hooks.write().await;
        hooks.push(hook);
    }

    /// Check if shutdown has been initiated
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown_initiated.load(Ordering::Acquire)
    }

    /// Initiate graceful shutdown
    ///
    /// This will:
    /// 1. Stop accepting new connections
    /// 2. Wait for in-flight requests to complete (drain)
    /// 3. Execute shutdown hooks
    ///
    /// This method does not mark any health-check/readiness status as
    /// unhealthy. If your probes should reflect shutdown state, do so
    /// yourself (e.g. in a shutdown hook, or just before calling this
    /// method) — see the module docs for details.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::*;
    /// use std::sync::Arc;
    ///
    /// # async fn example() {
    /// let manager = Arc::new(ShutdownManager::new());
    ///
    /// // Handle SIGTERM
    /// tokio::spawn(async move {
    ///     tokio::signal::ctrl_c().await.ok();
    ///     manager.initiate_shutdown().await;
    /// });
    /// # }
    /// ```
    pub async fn initiate_shutdown(&self) {
        if self.shutdown_initiated.swap(true, Ordering::SeqCst) {
            warn!("Shutdown already initiated");
            return;
        }

        info!("Initiating graceful shutdown...");

        let timeout_duration = *self.timeout.read().await;

        // Phase 1: Stop accepting new connections
        info!("Stopping acceptance of new connections");
        self.tracker.stop_accepting();

        // Phase 2: Drain existing connections
        info!(
            "Draining {} active connections",
            self.tracker.active_count()
        );

        let drain_timeout = timeout_duration / 2; // Use half timeout for draining
        if !self.tracker.drain(drain_timeout).await {
            warn!(
                "Force shutdown: {} connections still active",
                self.tracker.active_count()
            );
        }

        // Phase 3: Execute shutdown hooks
        info!("Executing shutdown hooks");
        let hooks = self.hooks.read().await;

        for (i, hook) in hooks.iter().enumerate() {
            let hook_timeout = Duration::from_secs(5);

            match timeout(hook_timeout, hook()).await {
                Ok(Ok(())) => {
                    info!("Shutdown hook {} completed successfully", i + 1);
                }
                Ok(Err(e)) => {
                    error!("Shutdown hook {} failed: {}", i + 1, e);
                }
                Err(_) => {
                    error!("Shutdown hook {} timed out", i + 1);
                }
            }
        }

        info!("Graceful shutdown complete");
    }

    /// Shutdown with custom phases
    ///
    /// For advanced use cases where you need fine-grained control.
    pub async fn shutdown_with_phases(&self) -> ShutdownPhases<'_> {
        ShutdownPhases { manager: self }
    }
}

impl Default for ShutdownManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for custom shutdown phases
pub struct ShutdownPhases<'a> {
    manager: &'a ShutdownManager,
}

impl<'a> ShutdownPhases<'a> {
    /// Stop accepting new connections
    pub async fn stop_accepting(&self) {
        info!("Stopping acceptance of new connections");
        self.manager.tracker.stop_accepting();
    }

    /// Drain connections with timeout
    pub async fn drain_connections(&self, timeout_duration: Duration) -> bool {
        info!(
            "Draining {} active connections",
            self.manager.tracker.active_count()
        );
        self.manager.tracker.drain(timeout_duration).await
    }

    /// Execute shutdown hooks
    pub async fn execute_hooks(&self) {
        info!("Executing shutdown hooks");
        let hooks = self.manager.hooks.read().await;

        for (i, hook) in hooks.iter().enumerate() {
            match timeout(Duration::from_secs(5), hook()).await {
                Ok(Ok(())) => info!("Hook {} completed", i + 1),
                Ok(Err(e)) => error!("Hook {} failed: {}", i + 1, e),
                Err(_) => error!("Hook {} timed out", i + 1),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_tracker_new() {
        let tracker = ConnectionTracker::new();
        assert_eq!(tracker.active_count(), 0);
        assert!(tracker.is_accepting());
    }

    #[test]
    fn test_connection_tracker_increment() {
        let tracker = ConnectionTracker::new();

        let _guard1 = tracker.increment().unwrap();
        assert_eq!(tracker.active_count(), 1);

        let _guard2 = tracker.increment().unwrap();
        assert_eq!(tracker.active_count(), 2);
    }

    #[test]
    fn test_connection_tracker_decrement() {
        let tracker = ConnectionTracker::new();

        {
            let _guard = tracker.increment().unwrap();
            assert_eq!(tracker.active_count(), 1);
        } // guard dropped here

        assert_eq!(tracker.active_count(), 0);
    }

    #[test]
    fn test_connection_tracker_stop_accepting() {
        let tracker = ConnectionTracker::new();

        assert!(tracker.is_accepting());

        tracker.stop_accepting();

        assert!(!tracker.is_accepting());
        assert!(tracker.increment().is_none());
    }

    #[tokio::test]
    async fn test_connection_tracker_drain() {
        let tracker = ConnectionTracker::new();

        let _guard = tracker.increment().unwrap();
        tracker.stop_accepting();

        // Should timeout
        let drained = tracker.drain(Duration::from_millis(100)).await;
        assert!(!drained);
        assert_eq!(tracker.active_count(), 1);
    }

    #[tokio::test]
    async fn test_shutdown_manager_new() {
        let manager = ShutdownManager::new();
        assert!(!manager.is_shutting_down());
        assert_eq!(manager.tracker().active_count(), 0);
    }

    #[tokio::test]
    async fn test_shutdown_manager_add_hook() {
        let manager = ShutdownManager::new();

        manager
            .add_hook(Box::new(|| Box::pin(async { Ok(()) })))
            .await;

        assert_eq!(manager.hooks.read().await.len(), 1);
    }

    #[tokio::test]
    async fn test_shutdown_manager_set_timeout() {
        let manager = ShutdownManager::new();

        manager.set_timeout(Duration::from_secs(60)).await;

        assert_eq!(*manager.timeout.read().await, Duration::from_secs(60));
    }
}
