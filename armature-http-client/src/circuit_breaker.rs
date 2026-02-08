//! Circuit breaker pattern implementation.

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Circuit is closed, requests are allowed.
    Closed,
    /// Circuit is open, requests are rejected.
    Open,
    /// Circuit is half-open, limited requests are allowed for testing.
    HalfOpen,
}

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures to open the circuit.
    pub failure_threshold: u32,
    /// Number of successful requests to close the circuit.
    pub success_threshold: u32,
    /// Time to wait before attempting to close the circuit.
    pub reset_timeout: Duration,
    /// Number of requests to allow in half-open state.
    pub half_open_requests: u32,
    /// Time window for counting failures.
    pub failure_window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            reset_timeout: Duration::from_secs(30),
            half_open_requests: 3,
            failure_window: Duration::from_secs(60),
        }
    }
}

impl CircuitBreakerConfig {
    /// Create a new circuit breaker config.
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failure_threshold,
            reset_timeout,
            ..Default::default()
        }
    }

    /// Set the success threshold to close the circuit.
    pub fn with_success_threshold(mut self, threshold: u32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Set the number of half-open requests.
    pub fn with_half_open_requests(mut self, count: u32) -> Self {
        self.half_open_requests = count;
        self
    }

    /// Set the failure counting window.
    pub fn with_failure_window(mut self, window: Duration) -> Self {
        self.failure_window = window;
        self
    }
}

/// Circuit breaker implementation.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitState>,
    failure_count: AtomicU32,
    success_count: AtomicU32,
    half_open_count: AtomicU32,
    last_failure_time: AtomicU64,
    opened_at: RwLock<Option<Instant>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitState::Closed),
            failure_count: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            half_open_count: AtomicU32::new(0),
            last_failure_time: AtomicU64::new(0),
            opened_at: RwLock::new(None),
        }
    }

    /// Get the current circuit state.
    pub fn state(&self) -> CircuitState {
        self.maybe_transition_to_half_open();
        *self.state.read()
    }

    /// Check if a request is allowed.
    pub fn is_allowed(&self) -> bool {
        self.maybe_transition_to_half_open();

        let state = *self.state.read();
        match state {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                let count = self.half_open_count.fetch_add(1, Ordering::SeqCst);
                count < self.config.half_open_requests
            }
        }
    }

    /// Record a successful request.
    pub fn record_success(&self) {
        let state = *self.state.read();

        match state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count.store(0, Ordering::SeqCst);
            }
            CircuitState::HalfOpen => {
                let successes = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if successes >= self.config.success_threshold {
                    self.close();
                }
            }
            CircuitState::Open => {
                // Should not happen, but reset if it does
                debug!("Success recorded while circuit open, ignoring");
            }
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self) {
        let now = Instant::now();
        let now_millis = now.elapsed().as_millis() as u64;

        let state = *self.state.read();

        match state {
            CircuitState::Closed => {
                // Check if we should reset the failure window
                let last_failure = self.last_failure_time.load(Ordering::SeqCst);
                let window_millis = self.config.failure_window.as_millis() as u64;

                if now_millis.saturating_sub(last_failure) > window_millis {
                    self.failure_count.store(1, Ordering::SeqCst);
                } else {
                    let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                    if failures >= self.config.failure_threshold {
                        self.open();
                    }
                }

                self.last_failure_time.store(now_millis, Ordering::SeqCst);
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
        let mut state = self.state.write();
        if *state != CircuitState::Open {
            warn!("Circuit breaker opening");
            *state = CircuitState::Open;
            *self.opened_at.write() = Some(Instant::now());
            self.half_open_count.store(0, Ordering::SeqCst);
            self.success_count.store(0, Ordering::SeqCst);
        }
    }

    /// Close the circuit.
    fn close(&self) {
        let mut state = self.state.write();
        if *state != CircuitState::Closed {
            info!("Circuit breaker closing");
            *state = CircuitState::Closed;
            *self.opened_at.write() = None;
            self.failure_count.store(0, Ordering::SeqCst);
            self.success_count.store(0, Ordering::SeqCst);
            self.half_open_count.store(0, Ordering::SeqCst);
        }
    }

    /// Transition to half-open if timeout has elapsed.
    fn maybe_transition_to_half_open(&self) {
        let state = *self.state.read();
        if state != CircuitState::Open {
            return;
        }

        let opened_at = *self.opened_at.read();
        if let Some(opened) = opened_at
            && opened.elapsed() >= self.config.reset_timeout
        {
            let mut state = self.state.write();
            if *state == CircuitState::Open {
                debug!("Circuit breaker transitioning to half-open");
                *state = CircuitState::HalfOpen;
                self.half_open_count.store(0, Ordering::SeqCst);
                self.success_count.store(0, Ordering::SeqCst);
            }
        }
    }

    /// Get failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::SeqCst)
    }

    /// Get success count (in half-open state).
    pub fn success_count(&self) -> u32 {
        self.success_count.load(Ordering::SeqCst)
    }

    /// Reset the circuit breaker to closed state.
    pub fn reset(&self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_opens_after_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.is_allowed());

        // Record failures
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.is_allowed());
    }

    #[test]
    fn test_circuit_breaker_success_resets_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        cb.record_failure();
        cb.record_failure();
        cb.record_success();

        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), CircuitState::Closed);
    }
}
