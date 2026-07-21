//! Retry configuration and strategies.

use std::time::Duration;

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: u32,
    /// Backoff strategy.
    pub backoff: BackoffStrategy,
    /// Status codes that should trigger a retry.
    pub retry_status_codes: Vec<u16>,
    /// Whether to retry on connection errors.
    pub retry_on_connection_error: bool,
    /// Whether to retry on timeout errors.
    pub retry_on_timeout: bool,
    /// Maximum total time for all retries.
    pub max_retry_time: Option<Duration>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: BackoffStrategy::Exponential {
                initial: Duration::from_millis(100),
                max: Duration::from_secs(10),
                multiplier: 2.0,
            },
            retry_status_codes: vec![408, 429, 500, 502, 503, 504],
            retry_on_connection_error: true,
            retry_on_timeout: true,
            max_retry_time: Some(Duration::from_secs(60)),
        }
    }
}

impl RetryConfig {
    /// Create a retry config with exponential backoff.
    pub fn exponential(max_attempts: u32, initial_delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffStrategy::Exponential {
                initial: initial_delay,
                max: Duration::from_secs(30),
                multiplier: 2.0,
            },
            ..Default::default()
        }
    }

    /// Create a retry config with linear backoff.
    pub fn linear(max_attempts: u32, delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffStrategy::Linear {
                delay,
                max: Duration::from_secs(30),
            },
            ..Default::default()
        }
    }

    /// Create a retry config with constant delay.
    pub fn constant(max_attempts: u32, delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffStrategy::Constant(delay),
            ..Default::default()
        }
    }

    /// Create a retry config with no delay.
    pub fn immediate(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            backoff: BackoffStrategy::None,
            ..Default::default()
        }
    }

    /// Set additional status codes to retry on.
    pub fn with_status_codes(mut self, codes: Vec<u16>) -> Self {
        self.retry_status_codes = codes;
        self
    }

    /// Disable retry on connection errors.
    pub fn no_retry_on_connection(mut self) -> Self {
        self.retry_on_connection_error = false;
        self
    }

    /// Disable retry on timeout errors.
    pub fn no_retry_on_timeout(mut self) -> Self {
        self.retry_on_timeout = false;
        self
    }

    /// Set maximum total retry time.
    pub fn with_max_retry_time(mut self, duration: Duration) -> Self {
        self.max_retry_time = Some(duration);
        self
    }

    /// Calculate delay for a given attempt.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        self.backoff.delay_for_attempt(attempt)
    }

    /// Check if a status code should trigger a retry.
    pub fn should_retry_status(&self, status: u16) -> bool {
        self.retry_status_codes.contains(&status)
    }
}

/// Backoff strategy for retries.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BackoffStrategy {
    /// No delay between retries.
    None,
    /// Constant delay between retries.
    Constant(Duration),
    /// Linear backoff: delay increases by a fixed amount.
    Linear {
        /// Delay increment per attempt.
        delay: Duration,
        /// Maximum delay.
        max: Duration,
    },
    /// Exponential backoff: delay doubles each attempt.
    Exponential {
        /// Initial delay.
        initial: Duration,
        /// Maximum delay.
        max: Duration,
        /// Multiplier (typically 2.0).
        multiplier: f64,
    },
}

impl BackoffStrategy {
    /// Calculate delay for a given attempt (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        match self {
            Self::None => Duration::ZERO,
            Self::Constant(d) => *d,
            Self::Linear { delay, max } => {
                let total = delay.saturating_mul(attempt + 1);
                total.min(*max)
            }
            Self::Exponential {
                initial,
                max,
                multiplier,
            } => {
                let factor = multiplier.powi(attempt as i32);
                let millis = (initial.as_millis() as f64 * factor) as u64;
                Duration::from_millis(millis).min(*max)
            }
        }
    }
}

/// Retry decision logic, implemented by [`RetryConfig`].
///
/// This trait exists to separate the "should this be retried, and after
/// what delay" decision from `RetryConfig`'s data, but it is not currently
/// pluggable end-to-end: [`crate::HttpClientConfig::retry`] is concretely
/// typed as `Option<RetryConfig>`, so there is no way for a caller to
/// configure `HttpClient` with a custom `RetryStrategy` implementation
/// today. Treat this as an internal seam (and a hook for possible future
/// pluggability), not a supported extension point.
pub trait RetryStrategy: Send + Sync {
    /// Check if the request should be retried.
    fn should_retry(&self, attempt: u32, error: &crate::HttpClientError) -> bool;

    /// Get the delay before the next retry.
    fn retry_delay(&self, attempt: u32) -> Duration;
}

impl RetryStrategy for RetryConfig {
    fn should_retry(&self, attempt: u32, error: &crate::HttpClientError) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }

        match error {
            crate::HttpClientError::Timeout(_) => self.retry_on_timeout,
            crate::HttpClientError::Connection(_) => self.retry_on_connection_error,
            crate::HttpClientError::Response { status, .. } => {
                self.retry_status_codes.contains(status)
            }
            crate::HttpClientError::Http(e) => {
                if e.is_timeout() {
                    self.retry_on_timeout
                } else if e.is_connect() {
                    self.retry_on_connection_error
                } else if let Some(status) = e.status() {
                    self.retry_status_codes.contains(&status.as_u16())
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn retry_delay(&self, attempt: u32) -> Duration {
        self.delay_for_attempt(attempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let strategy = BackoffStrategy::Exponential {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(10),
            multiplier: 2.0,
        };

        assert_eq!(strategy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(strategy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(strategy.delay_for_attempt(2), Duration::from_millis(400));
        assert_eq!(strategy.delay_for_attempt(3), Duration::from_millis(800));
    }

    #[test]
    fn test_linear_backoff() {
        let strategy = BackoffStrategy::Linear {
            delay: Duration::from_millis(100),
            max: Duration::from_secs(1),
        };

        assert_eq!(strategy.delay_for_attempt(0), Duration::from_millis(100));
        assert_eq!(strategy.delay_for_attempt(1), Duration::from_millis(200));
        assert_eq!(strategy.delay_for_attempt(9), Duration::from_secs(1));
    }

    #[test]
    fn test_constant_backoff() {
        let strategy = BackoffStrategy::Constant(Duration::from_millis(500));

        assert_eq!(strategy.delay_for_attempt(0), Duration::from_millis(500));
        assert_eq!(strategy.delay_for_attempt(5), Duration::from_millis(500));
    }
}
