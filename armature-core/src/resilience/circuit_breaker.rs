//! Circuit Breaker pattern implementation.
//!
//! The circuit breaker prevents cascade failures by monitoring for failures
//! and "opening" the circuit to reject requests when a failure threshold is reached.
//!
//! ## States
//!
//! - **Closed**: Normal operation, requests pass through
//! - **Open**: Circuit is tripped, requests are rejected immediately
//! - **Half-Open**: Testing if the service has recovered
//!
//! ## Example
//!
//! ```rust,ignore
//! use armature::resilience::{CircuitBreaker, CircuitBreakerConfig};
//! use std::time::Duration;
//!
//! let circuit = CircuitBreaker::new(CircuitBreakerConfig {
//!     failure_threshold: 5,
//!     success_threshold: 2,
//!     reset_timeout: Duration::from_secs(30),
//!     ..Default::default()
//! });
//!
//! // In a controller or service
//! let result = circuit.call(|| async {
//!     external_service.fetch_data().await
//! }).await;
//!
//! match result {
//!     Ok(data) => Ok(Json(data)),
//!     Err(CircuitBreakerError::Open) => {
//!         Err(HttpError::service_unavailable("Service temporarily unavailable"))
//!     }
//!     Err(CircuitBreakerError::Execution(e)) => Err(e.into()),
//! }
//! ```

use parking_lot::RwLock;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests pass through normally.
    Closed,
    /// Circuit is open, requests are rejected.
    Open,
    /// Circuit is half-open, testing recovery.
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "Closed"),
            Self::Open => write!(f, "Open"),
            Self::HalfOpen => write!(f, "HalfOpen"),
        }
    }
}

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Name of the circuit breaker (for logging/metrics).
    pub name: String,
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Number of successful requests needed to close the circuit from half-open.
    pub success_threshold: u32,
    /// Time to wait before transitioning from open to half-open.
    pub reset_timeout: Duration,
    /// Number of requests allowed in half-open state.
    pub half_open_requests: u32,
    /// Time window for counting failures (sliding window).
    pub failure_window: Duration,
    /// Enable automatic state transitions.
    pub automatic_transitions: bool,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            failure_threshold: 5,
            success_threshold: 3,
            reset_timeout: Duration::from_secs(30),
            half_open_requests: 3,
            failure_window: Duration::from_secs(60),
            automatic_transitions: true,
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a new configuration with a name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set the failure threshold.
    pub fn failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// Set the success threshold for recovery.
    pub fn success_threshold(mut self, threshold: u32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Set the reset timeout.
    pub fn reset_timeout(mut self, timeout: Duration) -> Self {
        self.reset_timeout = timeout;
        self
    }

    /// Set the number of half-open requests allowed.
    pub fn half_open_requests(mut self, count: u32) -> Self {
        self.half_open_requests = count;
        self
    }

    /// Set the failure counting window.
    pub fn failure_window(mut self, window: Duration) -> Self {
        self.failure_window = window;
        self
    }
}

/// Circuit breaker error.
#[derive(Debug)]
pub enum CircuitBreakerError<E> {
    /// Circuit is open, request was rejected.
    Open,
    /// Request was executed but failed.
    Execution(E),
    /// Circuit rejected due to half-open limit.
    HalfOpenLimitReached,
}

impl<E: std::fmt::Display> std::fmt::Display for CircuitBreakerError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => write!(f, "Circuit breaker is open"),
            Self::Execution(e) => write!(f, "Execution failed: {}", e),
            Self::HalfOpenLimitReached => write!(f, "Half-open request limit reached"),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for CircuitBreakerError<E> {}

/// Internal circuit breaker state.
struct CircuitBreakerState {
    state: CircuitState,
    opened_at: Option<Instant>,
    failure_timestamps: Vec<Instant>,
}

/// Circuit breaker for protecting against cascade failures.
///
/// The circuit breaker monitors failures and opens the circuit when
/// a threshold is reached, preventing further requests until the
/// service has had time to recover.
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    inner: RwLock<CircuitBreakerState>,
    failure_count: AtomicU32,
    success_count: AtomicU32,
    half_open_count: AtomicU32,
    total_requests: AtomicU64,
    total_failures: AtomicU64,
    total_successes: AtomicU64,
    total_rejections: AtomicU64,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Arc<Self> {
        info!(
            name = %config.name,
            failure_threshold = config.failure_threshold,
            reset_timeout = ?config.reset_timeout,
            "Circuit breaker initialized"
        );

        Arc::new(Self {
            config,
            inner: RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                opened_at: None,
                failure_timestamps: Vec::new(),
            }),
            failure_count: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            half_open_count: AtomicU32::new(0),
            total_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            total_successes: AtomicU64::new(0),
            total_rejections: AtomicU64::new(0),
        })
    }

    /// Create with default configuration.
    pub fn default_circuit() -> Arc<Self> {
        Self::new(CircuitBreakerConfig::default())
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        self.maybe_transition_to_half_open();
        self.inner.read().state
    }

    /// Get the circuit breaker name.
    pub fn name(&self) -> &str {
        &self.config.name
    }

    /// Check if a request is allowed through the circuit.
    pub fn is_allowed(&self) -> bool {
        self.maybe_transition_to_half_open();

        let state = self.inner.read().state;
        match state {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                let count = self.half_open_count.fetch_add(1, Ordering::SeqCst);
                count < self.config.half_open_requests
            }
        }
    }

    /// Execute a function with circuit breaker protection.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, CircuitBreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        // Check if request is allowed
        if !self.is_allowed() {
            self.total_rejections.fetch_add(1, Ordering::Relaxed);
            debug!(
                name = %self.config.name,
                state = %self.state(),
                "Circuit breaker rejected request"
            );
            return Err(CircuitBreakerError::Open);
        }

        // Execute the operation
        match f().await {
            Ok(result) => {
                self.record_success();
                Ok(result)
            }
            Err(e) => {
                self.record_failure();
                Err(CircuitBreakerError::Execution(e))
            }
        }
    }

    /// Execute with a predicate to determine if the result is a failure.
    pub async fn call_with_predicate<F, Fut, T, P>(
        &self,
        f: F,
        is_failure: P,
    ) -> Result<T, CircuitBreakerError<()>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
        P: FnOnce(&T) -> bool,
    {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        if !self.is_allowed() {
            self.total_rejections.fetch_add(1, Ordering::Relaxed);
            return Err(CircuitBreakerError::Open);
        }

        let result = f().await;

        if is_failure(&result) {
            self.record_failure();
            Err(CircuitBreakerError::Execution(()))
        } else {
            self.record_success();
            Ok(result)
        }
    }

    /// Record a successful operation.
    pub fn record_success(&self) {
        self.total_successes.fetch_add(1, Ordering::Relaxed);

        let state = self.inner.read().state;

        match state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);

                // Clear old failure timestamps
                let mut inner = self.inner.write();
                inner.failure_timestamps.clear();
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    self.close();
                }
            }
            CircuitState::Open => {
                // Should not happen in normal flow
                debug!(name = %self.config.name, "Success recorded while circuit open");
            }
        }
    }

    /// Record a failed operation.
    pub fn record_failure(&self) {
        self.total_failures.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();

        let state = self.inner.read().state;

        match state {
            CircuitState::Closed => {
                let mut inner = self.inner.write();

                // Remove old timestamps outside the failure window
                let window_start = now - self.config.failure_window;
                inner.failure_timestamps.retain(|&t| t > window_start);

                // Add new failure
                inner.failure_timestamps.push(now);

                let failure_count = inner.failure_timestamps.len() as u32;
                self.failure_count.store(failure_count, Ordering::SeqCst);

                // Check if we should open the circuit
                if failure_count >= self.config.failure_threshold {
                    drop(inner); // Release lock before calling open()
                    self.open();
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open state reopens the circuit
                self.open();
            }
            CircuitState::Open => {
                // Already open, nothing to do
            }
        }
    }

    /// Open the circuit.
    fn open(&self) {
        let mut inner = self.inner.write();
        if inner.state != CircuitState::Open {
            warn!(
                name = %self.config.name,
                failures = self.failure_count.load(Ordering::SeqCst),
                "Circuit breaker OPENED"
            );
            inner.state = CircuitState::Open;
            inner.opened_at = Some(Instant::now());
            self.half_open_count.store(0, Ordering::SeqCst);
            self.success_count.store(0, Ordering::SeqCst);
        }
    }

    /// Close the circuit.
    fn close(&self) {
        let mut inner = self.inner.write();
        if inner.state != CircuitState::Closed {
            info!(name = %self.config.name, "Circuit breaker CLOSED");
            inner.state = CircuitState::Closed;
            inner.opened_at = None;
            inner.failure_timestamps.clear();
            self.failure_count.store(0, Ordering::SeqCst);
            self.success_count.store(0, Ordering::SeqCst);
            self.half_open_count.store(0, Ordering::SeqCst);
        }
    }

    /// Transition to half-open state if reset timeout has elapsed.
    fn maybe_transition_to_half_open(&self) {
        if !self.config.automatic_transitions {
            return;
        }

        let inner = self.inner.read();
        if inner.state != CircuitState::Open {
            return;
        }

        if let Some(opened_at) = inner.opened_at
            && opened_at.elapsed() >= self.config.reset_timeout
        {
            drop(inner); // Release read lock before acquiring write lock

            let mut inner = self.inner.write();
            if inner.state == CircuitState::Open {
                debug!(name = %self.config.name, "Circuit breaker transitioning to HALF-OPEN");
                inner.state = CircuitState::HalfOpen;
                self.half_open_count.store(0, Ordering::SeqCst);
                self.success_count.store(0, Ordering::SeqCst);
            }
        }
    }

    /// Manually reset the circuit breaker to closed state.
    pub fn reset(&self) {
        self.close();
    }

    /// Manually force the circuit open.
    pub fn force_open(&self) {
        self.open();
    }

    // Metrics

    /// Get the current failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::SeqCst)
    }

    /// Get the current success count (in half-open state).
    pub fn success_count(&self) -> u32 {
        self.success_count.load(Ordering::SeqCst)
    }

    /// Get total requests processed.
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Get total successful requests.
    pub fn total_successes(&self) -> u64 {
        self.total_successes.load(Ordering::Relaxed)
    }

    /// Get total failed requests.
    pub fn total_failures(&self) -> u64 {
        self.total_failures.load(Ordering::Relaxed)
    }

    /// Get total rejected requests (circuit open).
    pub fn total_rejections(&self) -> u64 {
        self.total_rejections.load(Ordering::Relaxed)
    }

    /// Get circuit breaker statistics.
    pub fn stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            name: self.config.name.clone(),
            state: self.state(),
            total_requests: self.total_requests(),
            total_successes: self.total_successes(),
            total_failures: self.total_failures(),
            total_rejections: self.total_rejections(),
            current_failure_count: self.failure_count(),
        }
    }
}

impl Clone for CircuitBreaker {
    fn clone(&self) -> Self {
        // Clone creates a new independent circuit breaker
        Self {
            config: self.config.clone(),
            inner: RwLock::new(CircuitBreakerState {
                state: self.inner.read().state,
                opened_at: self.inner.read().opened_at,
                failure_timestamps: self.inner.read().failure_timestamps.clone(),
            }),
            failure_count: AtomicU32::new(self.failure_count.load(Ordering::SeqCst)),
            success_count: AtomicU32::new(self.success_count.load(Ordering::SeqCst)),
            half_open_count: AtomicU32::new(self.half_open_count.load(Ordering::SeqCst)),
            total_requests: AtomicU64::new(self.total_requests.load(Ordering::Relaxed)),
            total_failures: AtomicU64::new(self.total_failures.load(Ordering::Relaxed)),
            total_successes: AtomicU64::new(self.total_successes.load(Ordering::Relaxed)),
            total_rejections: AtomicU64::new(self.total_rejections.load(Ordering::Relaxed)),
        }
    }
}

/// Circuit breaker statistics.
#[derive(Debug, Clone)]
pub struct CircuitBreakerStats {
    /// Circuit breaker name.
    pub name: String,
    /// Current state.
    pub state: CircuitState,
    /// Total requests.
    pub total_requests: u64,
    /// Total successes.
    pub total_successes: u64,
    /// Total failures.
    pub total_failures: u64,
    /// Total rejections.
    pub total_rejections: u64,
    /// Current failure count in window.
    pub current_failure_count: u32,
}

impl CircuitBreakerStats {
    /// Calculate success rate (0.0 - 1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            1.0
        } else {
            self.total_successes as f64 / self.total_requests as f64
        }
    }

    /// Calculate failure rate (0.0 - 1.0).
    pub fn failure_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_failures as f64 / self.total_requests as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_secs(30),
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        assert_eq!(cb.state(), CircuitState::Closed);

        // Record failures
        for _ in 0..3 {
            let _: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Err("error") }).await;
        }

        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_rejects_when_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Trip the circuit
        let _: Result<(), _> = cb.call(|| async { Err::<(), _>("error") }).await;

        // Next call should be rejected
        let result: Result<(), CircuitBreakerError<&str>> = cb.call(|| async { Ok(()) }).await;

        assert!(matches!(result, Err(CircuitBreakerError::Open)));
    }

    #[tokio::test]
    async fn test_circuit_breaker_success_resets_count() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Record 2 failures
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        // Record success
        cb.record_success();
        assert_eq!(cb.failure_count(), 0);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 2,
            reset_timeout: Duration::from_millis(50),
            half_open_requests: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Trip the circuit
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        // Wait for reset timeout (use longer sleep to avoid timing issues)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should be half-open now
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // Record successes to close
        cb.record_success();
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }
}
