//! Retry pattern with configurable backoff strategies.
//!
//! ## Example
//!
//! ```rust,ignore
//! use armature::resilience::{Retry, RetryConfig, BackoffStrategy};
//! use std::time::Duration;
//!
//! let retry = Retry::new(RetryConfig {
//!     max_attempts: 3,
//!     backoff: BackoffStrategy::exponential(Duration::from_millis(100)),
//!     ..Default::default()
//! });
//!
//! let result = retry.call(|| async {
//!     external_service.fetch().await
//! }).await;
//! ```

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// Type alias for a retry error predicate function.
///
/// Stored as an `Arc` so cloning a config preserves the predicate.
pub type RetryErrorPredicate = Arc<dyn Fn(&dyn std::error::Error) -> bool + Send + Sync>;

/// Backoff strategy for retries.
#[derive(Debug, Clone)]
pub enum BackoffStrategy {
    /// No delay between retries.
    None,
    /// Constant delay between retries.
    Constant(Duration),
    /// Linear backoff: delay increases by a fixed amount.
    Linear {
        /// Initial delay.
        initial: Duration,
        /// Increment per retry.
        increment: Duration,
        /// Maximum delay.
        max: Duration,
    },
    /// Exponential backoff: delay doubles each retry.
    Exponential {
        /// Initial delay.
        initial: Duration,
        /// Multiplier (typically 2.0).
        multiplier: f64,
        /// Maximum delay.
        max: Duration,
    },
    /// Exponential backoff with jitter.
    ExponentialWithJitter {
        /// Initial delay.
        initial: Duration,
        /// Multiplier (typically 2.0).
        multiplier: f64,
        /// Maximum delay.
        max: Duration,
    },
}

impl BackoffStrategy {
    /// Create constant backoff.
    pub fn constant(delay: Duration) -> Self {
        Self::Constant(delay)
    }

    /// Create linear backoff.
    pub fn linear(initial: Duration, increment: Duration) -> Self {
        Self::Linear {
            initial,
            increment,
            max: Duration::from_secs(60),
        }
    }

    /// Create exponential backoff.
    pub fn exponential(initial: Duration) -> Self {
        Self::Exponential {
            initial,
            multiplier: 2.0,
            max: Duration::from_secs(60),
        }
    }

    /// Create exponential backoff with jitter.
    pub fn exponential_with_jitter(initial: Duration) -> Self {
        Self::ExponentialWithJitter {
            initial,
            multiplier: 2.0,
            max: Duration::from_secs(60),
        }
    }

    /// Set maximum delay.
    pub fn with_max(self, max: Duration) -> Self {
        match self {
            Self::Linear {
                initial, increment, ..
            } => Self::Linear {
                initial,
                increment,
                max,
            },
            Self::Exponential {
                initial,
                multiplier,
                ..
            } => Self::Exponential {
                initial,
                multiplier,
                max,
            },
            Self::ExponentialWithJitter {
                initial,
                multiplier,
                ..
            } => Self::ExponentialWithJitter {
                initial,
                multiplier,
                max,
            },
            other => other,
        }
    }

    /// Calculate delay for a given attempt (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self {
            Self::None => Duration::ZERO,
            Self::Constant(d) => *d,
            Self::Linear {
                initial,
                increment,
                max,
            } => {
                let delay = *initial + increment.saturating_mul(attempt);
                delay.min(*max)
            }
            Self::Exponential {
                initial,
                multiplier,
                max,
            } => {
                let factor = multiplier.powi(attempt as i32);
                let millis = (initial.as_millis() as f64 * factor) as u64;
                Duration::from_millis(millis).min(*max)
            }
            Self::ExponentialWithJitter {
                initial,
                multiplier,
                max,
            } => {
                let factor = multiplier.powi(attempt as i32);
                let base_millis = (initial.as_millis() as f64 * factor) as u64;
                // Add jitter: 0-50% of the delay
                let jitter = (base_millis as f64 * rand_factor() * 0.5) as u64;
                Duration::from_millis(base_millis + jitter).min(*max)
            }
        }
    }
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        Self::exponential(Duration::from_millis(100))
    }
}

/// Generate a random factor between 0.0 and 1.0.
///
/// Backed by a per-thread xorshift64 PRNG (same technique as
/// `LoadBalancer::next_random`) rather than raw clock nanoseconds, so that
/// concurrent callers racing through backoff at nearly the same instant
/// (the exact thundering-herd scenario jitter exists to decorrelate) don't
/// end up sampling correlated values off shared clock resolution.
fn rand_factor() -> f64 {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    thread_local! {
        static RNG_STATE: Cell<u64> = Cell::new(seed_rng_state());
    }

    // Monotonically increasing per-process counter mixed into the seed so
    // that threads spun up in quick succession (e.g. a burst of concurrent
    // retry callers) don't derive identical initial states even if their
    // clock reads land in the same tick.
    static SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn seed_rng_state() -> u64 {
        // Mix a wall-clock reading, a thread-local stack address (varies
        // per-thread), and a global atomic counter. Unlike the previous
        // implementation this mixing happens once per thread (at
        // thread-local init) rather than on every call, so subsequent
        // jitter draws advance via xorshift and are decorrelated from
        // call timing even when many threads race through backoff at
        // once.
        let time_bits = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let addr_bits = {
            let local = 0u8;
            &local as *const u8 as u64
        };
        let counter_bits = SEED_COUNTER.fetch_add(1, Ordering::Relaxed);

        let mut seed = time_bits
            ^ addr_bits.rotate_left(17)
            ^ counter_bits.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        if seed == 0 {
            // xorshift cannot recover from a zero state.
            seed = 0x853c_49e6_748f_ea9b;
        }
        seed
    }

    RNG_STATE.with(|state| {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        // Use the top 53 bits for a uniformly distributed f64 in [0, 1).
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of attempts (including initial).
    pub max_attempts: u32,
    /// Backoff strategy.
    pub backoff: BackoffStrategy,
    /// Predicate to determine if an error is retryable.
    pub retryable_errors: RetryableErrors,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: BackoffStrategy::default(),
            retryable_errors: RetryableErrors::All,
        }
    }
}

impl RetryConfig {
    /// Create new retry configuration.
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            ..Default::default()
        }
    }

    /// Set the backoff strategy.
    pub fn backoff(mut self, backoff: BackoffStrategy) -> Self {
        self.backoff = backoff;
        self
    }

    /// Set retryable errors.
    pub fn retryable(mut self, retryable: RetryableErrors) -> Self {
        self.retryable_errors = retryable;
        self
    }

    /// Only retry on specific error types.
    pub fn retry_on<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&dyn std::error::Error) -> bool + Send + Sync + 'static,
    {
        self.retryable_errors = RetryableErrors::Custom(Arc::new(predicate));
        self
    }
}

/// Configuration for which errors are retryable.
pub enum RetryableErrors {
    /// Retry all errors.
    All,
    /// Never retry (fail immediately).
    None,
    /// Use custom predicate.
    Custom(RetryErrorPredicate),
}

impl std::fmt::Debug for RetryableErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "All"),
            Self::None => write!(f, "None"),
            Self::Custom(_) => write!(f, "Custom"),
        }
    }
}

impl Clone for RetryableErrors {
    fn clone(&self) -> Self {
        match self {
            Self::All => Self::All,
            Self::None => Self::None,
            Self::Custom(predicate) => Self::Custom(Arc::clone(predicate)),
        }
    }
}

/// Adapter presenting a `Display`-only error to the `&dyn Error` custom
/// predicate in [`Retry::call`], where the error type is not required to
/// implement [`std::error::Error`]. Preserves the error's display output;
/// use [`Retry::call_if`] for typed inspection of the error.
#[derive(Debug)]
struct DisplayedError(String);

impl std::fmt::Display for DisplayedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DisplayedError {}

/// Retry error.
#[derive(Debug)]
pub struct RetryError<E> {
    /// Last error encountered.
    pub last_error: E,
    /// Number of attempts made.
    pub attempts: u32,
}

impl<E: std::fmt::Display> std::fmt::Display for RetryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed after {} attempts: {}",
            self.attempts, self.last_error
        )
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for RetryError<E> {}

/// Retry executor.
#[derive(Clone)]
pub struct Retry {
    config: RetryConfig,
}

impl Retry {
    /// Create a new retry executor.
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration.
    pub fn default_retry() -> Self {
        Self::new(RetryConfig::default())
    }

    /// Execute with retry logic.
    pub async fn call<F, Fut, T, E>(&self, mut f: F) -> Result<T, RetryError<E>>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut last_error: Option<E> = None;

        for attempt in 0..self.config.max_attempts {
            match f().await {
                Ok(result) => {
                    if attempt > 0 {
                        debug!(attempt = attempt + 1, "Retry succeeded");
                    }
                    return Ok(result);
                }
                Err(e) => {
                    // Consult the configured retry policy before scheduling
                    // another attempt
                    let is_retryable = match &self.config.retryable_errors {
                        RetryableErrors::All => true,
                        RetryableErrors::None => false,
                        RetryableErrors::Custom(predicate) => {
                            predicate(&DisplayedError(e.to_string()))
                        }
                    };

                    if !is_retryable {
                        debug!(
                            attempt = attempt + 1,
                            error = %e,
                            "Error not retryable, failing immediately"
                        );
                        return Err(RetryError {
                            last_error: e,
                            attempts: attempt + 1,
                        });
                    }

                    let is_last_attempt = attempt + 1 >= self.config.max_attempts;

                    if is_last_attempt {
                        warn!(
                            attempt = attempt + 1,
                            max_attempts = self.config.max_attempts,
                            error = %e,
                            "Final retry attempt failed"
                        );
                        last_error = Some(e);
                    } else {
                        let delay = self.config.backoff.delay_for_attempt(attempt);
                        debug!(
                            attempt = attempt + 1,
                            delay = ?delay,
                            error = %e,
                            "Retry attempt failed, waiting before retry"
                        );

                        if delay > Duration::ZERO {
                            tokio::time::sleep(delay).await;
                        }

                        last_error = Some(e);
                    }
                }
            }
        }

        Err(RetryError {
            last_error: last_error.unwrap(),
            attempts: self.config.max_attempts,
        })
    }

    /// Execute with a custom retry predicate.
    pub async fn call_if<F, Fut, T, E, P>(
        &self,
        mut f: F,
        should_retry: P,
    ) -> Result<T, RetryError<E>>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: std::fmt::Display,
        P: Fn(&E) -> bool,
    {
        let mut last_error: Option<E> = None;

        for attempt in 0..self.config.max_attempts {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let should_continue = should_retry(&e);
                    let is_last_attempt = attempt + 1 >= self.config.max_attempts;

                    if !should_continue || is_last_attempt {
                        return Err(RetryError {
                            last_error: e,
                            attempts: attempt + 1,
                        });
                    }

                    let delay = self.config.backoff.delay_for_attempt(attempt);
                    if delay > Duration::ZERO {
                        tokio::time::sleep(delay).await;
                    }

                    last_error = Some(e);
                }
            }
        }

        Err(RetryError {
            last_error: last_error.unwrap(),
            attempts: self.config.max_attempts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let retry = Retry::new(RetryConfig::new(3));

        let result: Result<i32, RetryError<&str>> = retry.call(|| async { Ok(42) }).await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_on_second_try() {
        let attempts = AtomicU32::new(0);
        let retry = Retry::new(RetryConfig {
            max_attempts: 3,
            backoff: BackoffStrategy::None,
            ..Default::default()
        });

        let result: Result<i32, RetryError<&str>> = retry
            .call(|| {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Err("first failure")
                    } else {
                        Ok(42)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_exhausted() {
        let retry = Retry::new(RetryConfig {
            max_attempts: 3,
            backoff: BackoffStrategy::None,
            ..Default::default()
        });

        let result: Result<i32, RetryError<&str>> =
            retry.call(|| async { Err("always fails") }).await;

        let err = result.unwrap_err();
        assert_eq!(err.attempts, 3);
        assert_eq!(err.last_error, "always fails");
    }

    #[tokio::test]
    async fn test_retry_none_fails_immediately() {
        let attempts = AtomicU32::new(0);
        let retry = Retry::new(RetryConfig {
            max_attempts: 3,
            backoff: BackoffStrategy::None,
            retryable_errors: RetryableErrors::None,
        });

        let result: Result<i32, RetryError<&str>> = retry
            .call(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("always fails") }
            })
            .await;

        let err = result.unwrap_err();
        assert_eq!(err.attempts, 1);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_custom_predicate_respected() {
        let config = RetryConfig::new(3)
            .backoff(BackoffStrategy::None)
            .retry_on(|e| e.to_string().contains("transient"));

        // Non-matching error: no retries
        let attempts = AtomicU32::new(0);
        let retry = Retry::new(config.clone());
        let result: Result<i32, RetryError<&str>> = retry
            .call(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("fatal error") }
            })
            .await;
        assert_eq!(result.unwrap_err().attempts, 1);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        // Matching error: retried until exhaustion (the clone above must
        // also have preserved the custom predicate)
        let attempts = AtomicU32::new(0);
        let retry = Retry::new(config);
        let result: Result<i32, RetryError<&str>> = retry
            .call(|| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("transient error") }
            })
            .await;
        assert_eq!(result.unwrap_err().attempts, 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retryable_errors_clone_preserves_custom() {
        let custom = RetryableErrors::Custom(Arc::new(|_| true));
        assert!(matches!(custom.clone(), RetryableErrors::Custom(_)));
    }

    #[test]
    fn test_exponential_backoff() {
        let backoff = BackoffStrategy::exponential(Duration::from_millis(100));

        assert_eq!(backoff.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(backoff.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(backoff.delay_for_attempt(2), Duration::from_millis(400));
    }
}
