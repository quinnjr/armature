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

/// Simple random float generator (0.0 to 1.0)
fn rand_float() -> f64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos as f64 % 1000.0) / 1000.0
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
}
