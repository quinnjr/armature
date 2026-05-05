//! Bulkhead pattern for resource isolation.
//!
//! The bulkhead pattern limits concurrent access to a resource,
//! preventing a single component from consuming all available resources.
//!
//! ## Example
//!
//! ```rust,ignore
//! use armature::resilience::{Bulkhead, BulkheadConfig};
//!
//! let bulkhead = Bulkhead::new(BulkheadConfig {
//!     max_concurrent: 10,
//!     max_wait: Duration::from_secs(5),
//!     ..Default::default()
//! });
//!
//! let result = bulkhead.call(|| async {
//!     expensive_operation().await
//! }).await;
//! ```

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

/// Bulkhead configuration.
#[derive(Debug, Clone)]
pub struct BulkheadConfig {
    /// Name of the bulkhead (for logging/metrics).
    pub name: String,
    /// Maximum concurrent executions.
    pub max_concurrent: u32,
    /// Maximum time to wait for a permit.
    pub max_wait: Duration,
    /// Queue size (requests waiting for permits).
    pub queue_size: Option<u32>,
}

impl Default for BulkheadConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            max_concurrent: 10,
            max_wait: Duration::from_secs(30),
            queue_size: None,
        }
    }
}

impl BulkheadConfig {
    /// Create a new configuration.
    pub fn new(name: impl Into<String>, max_concurrent: u32) -> Self {
        Self {
            name: name.into(),
            max_concurrent,
            ..Default::default()
        }
    }

    /// Set the maximum wait time.
    pub fn max_wait(mut self, duration: Duration) -> Self {
        self.max_wait = duration;
        self
    }

    /// Set the queue size.
    pub fn queue_size(mut self, size: u32) -> Self {
        self.queue_size = Some(size);
        self
    }
}

/// Bulkhead error.
#[derive(Debug)]
pub enum BulkheadError<E> {
    /// Bulkhead is full, request rejected.
    Full,
    /// Timed out waiting for a permit.
    Timeout,
    /// Execution failed.
    Execution(E),
}

impl<E: std::fmt::Display> std::fmt::Display for BulkheadError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "Bulkhead is full"),
            Self::Timeout => write!(f, "Timed out waiting for bulkhead permit"),
            Self::Execution(e) => write!(f, "Execution failed: {}", e),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for BulkheadError<E> {}

/// Bulkhead for limiting concurrent access.
pub struct Bulkhead {
    config: BulkheadConfig,
    semaphore: Arc<Semaphore>,
    active_count: AtomicU32,
    waiting_count: AtomicU32,
    total_calls: AtomicU64,
    total_rejections: AtomicU64,
    total_timeouts: AtomicU64,
}

impl Bulkhead {
    /// Create a new bulkhead.
    pub fn new(config: BulkheadConfig) -> Arc<Self> {
        tracing::info!(
            name = %config.name,
            max_concurrent = config.max_concurrent,
            "Bulkhead initialized"
        );

        Arc::new(Self {
            semaphore: Arc::new(Semaphore::new(config.max_concurrent as usize)),
            config,
            active_count: AtomicU32::new(0),
            waiting_count: AtomicU32::new(0),
            total_calls: AtomicU64::new(0),
            total_rejections: AtomicU64::new(0),
            total_timeouts: AtomicU64::new(0),
        })
    }

    /// Get the bulkhead name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Get current number of active executions.
    pub fn active_count(&self) -> u32 {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Get current number of waiting requests.
    pub fn waiting_count(&self) -> u32 {
        self.waiting_count.load(Ordering::SeqCst)
    }

    /// Get available permits.
    pub fn available_permits(&self) -> u32 {
        self.semaphore.available_permits() as u32
    }

    /// Check if the bulkhead has capacity.
    pub fn has_capacity(&self) -> bool {
        self.semaphore.available_permits() > 0
    }

    /// Execute with bulkhead protection.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, BulkheadError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        // Check queue size limit
        if let Some(queue_size) = self.config.queue_size
            && self.waiting_count.load(Ordering::SeqCst) >= queue_size
        {
            self.total_rejections.fetch_add(1, Ordering::Relaxed);
            debug!(name = %self.config.name, "Bulkhead queue full, rejecting request");
            return Err(BulkheadError::Full);
        }

        self.waiting_count.fetch_add(1, Ordering::SeqCst);

        // Try to acquire permit with timeout
        let permit =
            match tokio::time::timeout(self.config.max_wait, self.semaphore.acquire()).await {
                Ok(Ok(permit)) => {
                    self.waiting_count.fetch_sub(1, Ordering::SeqCst);
                    permit
                }
                Ok(Err(_)) => {
                    // Semaphore closed (shouldn't happen)
                    self.waiting_count.fetch_sub(1, Ordering::SeqCst);
                    self.total_rejections.fetch_add(1, Ordering::Relaxed);
                    return Err(BulkheadError::Full);
                }
                Err(_) => {
                    // Timeout
                    self.waiting_count.fetch_sub(1, Ordering::SeqCst);
                    self.total_timeouts.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        name = %self.config.name,
                        max_wait = ?self.config.max_wait,
                        "Bulkhead timeout waiting for permit"
                    );
                    return Err(BulkheadError::Timeout);
                }
            };

        self.active_count.fetch_add(1, Ordering::SeqCst);

        // Execute the operation
        let result = f().await;

        // Release permit
        self.active_count.fetch_sub(1, Ordering::SeqCst);
        drop(permit);

        result.map_err(BulkheadError::Execution)
    }

    /// Try to execute immediately without waiting.
    pub async fn try_call<F, Fut, T, E>(&self, f: F) -> Result<T, BulkheadError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.total_calls.fetch_add(1, Ordering::Relaxed);

        let permit = match self.semaphore.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                self.total_rejections.fetch_add(1, Ordering::Relaxed);
                return Err(BulkheadError::Full);
            }
        };

        self.active_count.fetch_add(1, Ordering::SeqCst);

        let result = f().await;

        self.active_count.fetch_sub(1, Ordering::SeqCst);
        drop(permit);

        result.map_err(BulkheadError::Execution)
    }

    /// Get bulkhead statistics.
    pub fn stats(&self) -> BulkheadStats {
        BulkheadStats {
            name: self.config.name.clone(),
            max_concurrent: self.config.max_concurrent,
            active_count: self.active_count(),
            waiting_count: self.waiting_count(),
            available_permits: self.available_permits(),
            total_calls: self.total_calls.load(Ordering::Relaxed),
            total_rejections: self.total_rejections.load(Ordering::Relaxed),
            total_timeouts: self.total_timeouts.load(Ordering::Relaxed),
        }
    }
}

/// Bulkhead statistics.
#[derive(Debug, Clone)]
pub struct BulkheadStats {
    /// Bulkhead name.
    pub name: String,
    /// Maximum concurrent executions.
    pub max_concurrent: u32,
    /// Current active executions.
    pub active_count: u32,
    /// Current waiting requests.
    pub waiting_count: u32,
    /// Available permits.
    pub available_permits: u32,
    /// Total calls.
    pub total_calls: u64,
    /// Total rejections.
    pub total_rejections: u64,
    /// Total timeouts.
    pub total_timeouts: u64,
}

impl BulkheadStats {
    /// Calculate utilization (0.0 - 1.0).
    pub fn utilization(&self) -> f64 {
        self.active_count as f64 / self.max_concurrent as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bulkhead_allows_concurrent() {
        let bulkhead = Bulkhead::new(BulkheadConfig::new("test", 2));

        let result: Result<i32, BulkheadError<&str>> = bulkhead.call(|| async { Ok(42) }).await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_bulkhead_rejects_when_full() {
        let bulkhead = Bulkhead::new(BulkheadConfig {
            name: "test".to_string(),
            max_concurrent: 1,
            max_wait: Duration::from_millis(10),
            queue_size: Some(0),
        });

        // Acquire the only permit
        let _permit = bulkhead.semaphore.acquire().await.unwrap();

        // Try to execute - should be rejected
        let result: Result<i32, BulkheadError<&str>> = bulkhead.try_call(|| async { Ok(42) }).await;

        assert!(matches!(result, Err(BulkheadError::Full)));
    }
}
