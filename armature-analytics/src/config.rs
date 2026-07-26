//! Analytics configuration

use serde::{Deserialize, Serialize};

/// Configuration for the analytics module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsConfig {
    /// Enable analytics collection
    pub enabled: bool,
    /// Maximum number of latency samples to keep for percentile calculation
    pub max_latency_samples: usize,
    /// Maximum number of recent errors to keep
    pub max_recent_errors: usize,
    /// Time window for throughput calculation (in seconds)
    pub throughput_window_secs: u64,
    /// Enable per-endpoint metrics
    pub enable_endpoint_metrics: bool,
    /// Maximum number of endpoints to track
    pub max_endpoints: usize,
    /// Enable rate limit tracking
    pub enable_rate_limit_tracking: bool,
    /// Paths to exclude from analytics
    pub exclude_paths: Vec<String>,
    /// Whether to include query parameters in path tracking
    pub include_query_params: bool,
    /// Sampling rate (0.0 to 1.0, 1.0 = 100% of requests)
    pub sampling_rate: f64,
    /// Enable client identification tracking
    pub track_clients: bool,
    /// Maximum number of unique clients to track for rate limits
    pub max_rate_limit_clients: usize,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_latency_samples: 10_000,
            max_recent_errors: 100,
            throughput_window_secs: 60,
            enable_endpoint_metrics: true,
            max_endpoints: 500,
            enable_rate_limit_tracking: true,
            exclude_paths: vec![
                "/health".to_string(),
                "/healthz".to_string(),
                "/ready".to_string(),
                "/metrics".to_string(),
            ],
            include_query_params: false,
            sampling_rate: 1.0,
            track_clients: true,
            max_rate_limit_clients: 1000,
        }
    }
}

impl AnalyticsConfig {
    /// Create a new configuration builder
    pub fn builder() -> AnalyticsConfigBuilder {
        AnalyticsConfigBuilder::default()
    }

    /// Create configuration for development (verbose tracking)
    pub fn development() -> Self {
        Self {
            enabled: true,
            max_latency_samples: 50_000,
            max_recent_errors: 500,
            throughput_window_secs: 60,
            enable_endpoint_metrics: true,
            max_endpoints: 1000,
            enable_rate_limit_tracking: true,
            exclude_paths: vec![],
            include_query_params: true,
            sampling_rate: 1.0,
            track_clients: true,
            max_rate_limit_clients: 5000,
        }
    }

    /// Create configuration for production (optimized)
    pub fn production() -> Self {
        Self {
            enabled: true,
            max_latency_samples: 10_000,
            max_recent_errors: 100,
            throughput_window_secs: 60,
            enable_endpoint_metrics: true,
            max_endpoints: 500,
            enable_rate_limit_tracking: true,
            exclude_paths: vec![
                "/health".to_string(),
                "/healthz".to_string(),
                "/ready".to_string(),
                "/metrics".to_string(),
                "/favicon.ico".to_string(),
            ],
            include_query_params: false,
            sampling_rate: 1.0,
            track_clients: true,
            max_rate_limit_clients: 1000,
        }
    }

    /// Create minimal configuration (low overhead)
    pub fn minimal() -> Self {
        Self {
            enabled: true,
            max_latency_samples: 1_000,
            max_recent_errors: 20,
            throughput_window_secs: 60,
            enable_endpoint_metrics: false,
            max_endpoints: 100,
            enable_rate_limit_tracking: false,
            exclude_paths: vec![
                "/health".to_string(),
                "/healthz".to_string(),
                "/ready".to_string(),
                "/metrics".to_string(),
            ],
            include_query_params: false,
            sampling_rate: 0.1, // 10% sampling
            track_clients: false,
            max_rate_limit_clients: 100,
        }
    }

    /// Check if a path should be excluded
    pub fn should_exclude(&self, path: &str) -> bool {
        self.exclude_paths.iter().any(|p| path.starts_with(p))
    }

    /// Check if this request should be sampled
    pub fn should_sample(&self) -> bool {
        if self.sampling_rate >= 1.0 {
            return true;
        }
        if self.sampling_rate <= 0.0 {
            return false;
        }
        rand_float() < self.sampling_rate
    }
}

/// Uniform random float generator in `[0.0, 1.0)`.
///
/// Backed by a per-thread `xorshift64*` PRNG seeded from a high-resolution
/// clock and a monotonic counter. The previous implementation derived the
/// value from `subsec_nanos() % 1000`, which is neither uniform (nanosecond
/// timers are quantized on most platforms) nor well distributed, so sampling
/// decisions were badly biased. This produces 53 bits of uniform entropy.
fn rand_float() -> f64 {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Distinct, ever-changing contribution to each thread's seed so that
    // threads spawned within the same clock tick do not share a stream.
    static SEED_COUNTER: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static STATE: Cell<u64> = Cell::new({
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let counter = SEED_COUNTER.fetch_add(1, Ordering::Relaxed);
            // Mix the clock and the counter; force a non-zero state.
            let mut s = nanos
                ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ 0xD1B5_4A32_D192_ED03;
            if s == 0 {
                s = 0x9E37_79B9_7F4A_7C15;
            }
            s
        });
    }

    STATE.with(|state| {
        let mut x = state.get();
        // xorshift64*
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        state.set(x);
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Top 53 bits -> uniform f64 in [0, 1).
        ((v >> 11) as f64) / ((1u64 << 53) as f64)
    })
}

/// Builder for AnalyticsConfig
#[derive(Default)]
pub struct AnalyticsConfigBuilder {
    config: AnalyticsConfig,
}

impl AnalyticsConfigBuilder {
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    pub fn max_latency_samples(mut self, max: usize) -> Self {
        self.config.max_latency_samples = max;
        self
    }

    pub fn max_recent_errors(mut self, max: usize) -> Self {
        self.config.max_recent_errors = max;
        self
    }

    pub fn throughput_window(mut self, secs: u64) -> Self {
        self.config.throughput_window_secs = secs;
        self
    }

    pub fn enable_endpoint_metrics(mut self, enabled: bool) -> Self {
        self.config.enable_endpoint_metrics = enabled;
        self
    }

    pub fn max_endpoints(mut self, max: usize) -> Self {
        self.config.max_endpoints = max;
        self
    }

    pub fn enable_rate_limit_tracking(mut self, enabled: bool) -> Self {
        self.config.enable_rate_limit_tracking = enabled;
        self
    }

    pub fn exclude_path(mut self, path: impl Into<String>) -> Self {
        self.config.exclude_paths.push(path.into());
        self
    }

    pub fn exclude_paths(mut self, paths: Vec<String>) -> Self {
        self.config.exclude_paths = paths;
        self
    }

    pub fn include_query_params(mut self, include: bool) -> Self {
        self.config.include_query_params = include;
        self
    }

    pub fn sampling_rate(mut self, rate: f64) -> Self {
        self.config.sampling_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn track_clients(mut self, track: bool) -> Self {
        self.config.track_clients = track;
        self
    }

    pub fn max_rate_limit_clients(mut self, max: usize) -> Self {
        self.config.max_rate_limit_clients = max;
        self
    }

    pub fn build(self) -> AnalyticsConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AnalyticsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.sampling_rate, 1.0);
    }

    #[test]
    fn test_exclude_paths() {
        let config = AnalyticsConfig::default();
        assert!(config.should_exclude("/health"));
        assert!(config.should_exclude("/healthz"));
        assert!(!config.should_exclude("/api/users"));
    }

    #[test]
    fn test_builder() {
        let config = AnalyticsConfig::builder()
            .enabled(true)
            .sampling_rate(0.5)
            .max_latency_samples(5000)
            .exclude_path("/internal")
            .build();

        assert!(config.enabled);
        assert_eq!(config.sampling_rate, 0.5);
        assert_eq!(config.max_latency_samples, 5000);
        assert!(config.should_exclude("/internal"));
    }

    // Regression: the old rand_float derived from `subsec_nanos() % 1000` was
    // heavily biased in a tight loop, so a 50% sampling rate did not sample
    // anywhere near half of requests. This asserts rough uniformity.
    #[test]
    fn test_sampling_is_roughly_uniform() {
        let config = AnalyticsConfig::builder().sampling_rate(0.5).build();
        let n = 20_000;
        let sampled = (0..n).filter(|_| config.should_sample()).count();
        let ratio = sampled as f64 / n as f64;
        assert!(
            (0.45..=0.55).contains(&ratio),
            "expected ~50% sampling, got {:.3}",
            ratio
        );
    }

    #[test]
    fn test_rand_float_in_range_and_varies() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let v = rand_float();
            assert!((0.0..1.0).contains(&v));
            seen.insert(v.to_bits());
        }
        // A biased/quantized generator would collapse to a handful of values.
        assert!(
            seen.len() > 500,
            "rand_float not varied enough: {}",
            seen.len()
        );
    }
}
