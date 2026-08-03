//! Health checking for backend services
//!
//! This module provides health checking capabilities for monitoring
//! backend services behind Ferron proxy.

use crate::error::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

/// Health status of a backend
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Backend is healthy and accepting requests
    Healthy,
    /// Backend is degraded but still functioning
    Degraded,
    /// Backend is unhealthy and should not receive traffic
    Unhealthy,
    /// Health status is unknown (not yet checked)
    Unknown,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

impl HealthStatus {
    /// Check if the status indicates the backend can receive traffic
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

/// Health check result for a backend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    /// Backend URL
    pub url: String,
    /// Health status
    pub status: HealthStatus,
    /// Response time in milliseconds
    pub response_time_ms: Option<u64>,
    /// HTTP status code (if applicable)
    pub http_status: Option<u16>,
    /// Error message (if unhealthy)
    pub error: Option<String>,
    /// Timestamp of the check
    pub checked_at: DateTime<Utc>,
    /// Consecutive failures
    pub consecutive_failures: u32,
    /// Consecutive successes
    pub consecutive_successes: u32,
}

impl HealthCheckResult {
    /// Create a healthy result
    pub fn healthy(url: String, response_time_ms: u64, http_status: u16) -> Self {
        Self {
            url,
            status: HealthStatus::Healthy,
            response_time_ms: Some(response_time_ms),
            http_status: Some(http_status),
            error: None,
            checked_at: Utc::now(),
            consecutive_failures: 0,
            consecutive_successes: 1,
        }
    }

    /// Create an unhealthy result
    pub fn unhealthy(url: String, error: impl Into<String>) -> Self {
        Self {
            url,
            status: HealthStatus::Unhealthy,
            response_time_ms: None,
            http_status: None,
            error: Some(error.into()),
            checked_at: Utc::now(),
            consecutive_failures: 1,
            consecutive_successes: 0,
        }
    }
}

/// Configuration for health checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    /// URL path to check (e.g., "/health", "/healthz")
    pub path: String,
    /// HTTP method to use
    pub method: String,
    /// Expected HTTP status codes for healthy response
    pub expected_status: Vec<u16>,
    /// Request timeout
    pub timeout: Duration,
    /// Interval between health checks
    pub interval: Duration,
    /// Number of failures before marking unhealthy
    pub unhealthy_threshold: u32,
    /// Number of successes before marking healthy
    pub healthy_threshold: u32,
    /// Custom headers to send with health check request
    pub headers: HashMap<String, String>,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            path: "/health".to_string(),
            method: "GET".to_string(),
            expected_status: vec![200],
            timeout: Duration::from_secs(5),
            interval: Duration::from_secs(30),
            unhealthy_threshold: 3,
            healthy_threshold: 2,
            headers: HashMap::new(),
        }
    }
}

impl HealthCheckConfig {
    /// Create a new health check configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the health check path
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// Set the HTTP method
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = method.into();
        self
    }

    /// Set expected status codes
    pub fn expected_status(mut self, codes: Vec<u16>) -> Self {
        self.expected_status = codes;
        self
    }

    /// Set the timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the check interval
    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Set the unhealthy threshold
    pub fn unhealthy_threshold(mut self, threshold: u32) -> Self {
        self.unhealthy_threshold = threshold;
        self
    }

    /// Set the healthy threshold
    pub fn healthy_threshold(mut self, threshold: u32) -> Self {
        self.healthy_threshold = threshold;
        self
    }

    /// Add a custom header
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
}

/// Trait for health check implementations
#[async_trait]
pub trait HealthCheck: Send + Sync {
    /// Check the health of a backend
    async fn check(&self, url: &str, config: &HealthCheckConfig) -> HealthCheckResult;
}

/// HTTP-based health checker
#[derive(Debug, Clone)]
pub struct HttpHealthChecker {
    client: reqwest::Client,
}

impl HttpHealthChecker {
    /// Create a new HTTP health checker
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Create with custom client configuration
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl Default for HttpHealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HealthCheck for HttpHealthChecker {
    async fn check(&self, url: &str, config: &HealthCheckConfig) -> HealthCheckResult {
        let full_url = format!("{}{}", url.trim_end_matches('/'), config.path);
        let start = std::time::Instant::now();

        let mut request = match config.method.to_uppercase().as_str() {
            "POST" => self.client.post(&full_url),
            "HEAD" => self.client.head(&full_url),
            _ => self.client.get(&full_url),
        };

        request = request.timeout(config.timeout);

        for (name, value) in &config.headers {
            request = request.header(name, value);
        }

        match request.send().await {
            Ok(response) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let status = response.status().as_u16();

                if config.expected_status.contains(&status) {
                    debug!(
                        "Health check passed for {}: {} in {}ms",
                        url, status, elapsed
                    );
                    HealthCheckResult::healthy(url.to_string(), elapsed, status)
                } else {
                    warn!(
                        "Health check failed for {}: unexpected status {}",
                        url, status
                    );
                    HealthCheckResult {
                        url: url.to_string(),
                        status: HealthStatus::Unhealthy,
                        response_time_ms: Some(elapsed),
                        http_status: Some(status),
                        error: Some(format!("Unexpected status code: {}", status)),
                        checked_at: Utc::now(),
                        consecutive_failures: 1,
                        consecutive_successes: 0,
                    }
                }
            }
            Err(e) => {
                error!("Health check failed for {}: {}", url, e);
                HealthCheckResult::unhealthy(url.to_string(), e.to_string())
            }
        }
    }
}

/// Backend health state tracker
pub struct HealthState {
    /// Current health results for each backend
    results: RwLock<HashMap<String, HealthCheckResult>>,
    /// Health check configuration
    config: HealthCheckConfig,
    /// Health checker implementation
    checker: Arc<dyn HealthCheck>,
}

impl HealthState {
    /// Create a new health state tracker
    pub fn new(config: HealthCheckConfig) -> Self {
        Self {
            results: RwLock::new(HashMap::new()),
            config,
            checker: Arc::new(HttpHealthChecker::new()),
        }
    }

    /// Create with custom health checker
    pub fn with_checker(config: HealthCheckConfig, checker: Arc<dyn HealthCheck>) -> Self {
        Self {
            results: RwLock::new(HashMap::new()),
            config,
            checker,
        }
    }

    /// Check health of a backend and update state
    ///
    /// A backend only flips to [`HealthStatus::Unhealthy`] once consecutive
    /// failures reach `unhealthy_threshold` (staying `Degraded` below that),
    /// and only flips back to [`HealthStatus::Healthy`] once consecutive
    /// successes reach `healthy_threshold` (staying `Degraded` while
    /// recovering). Both thresholds are compared directly against the
    /// running counters, independent of the previous status.
    pub async fn check_backend(&self, url: &str) -> HealthCheckResult {
        let mut result = self.checker.check(url, &self.config).await;

        let mut results = self.results.write().await;
        let prev_failures = results
            .get(url)
            .map(|p| p.consecutive_failures)
            .unwrap_or(0);
        let prev_successes = results
            .get(url)
            .map(|p| p.consecutive_successes)
            .unwrap_or(0);

        if result.status == HealthStatus::Healthy {
            result.consecutive_successes = prev_successes + 1;
            result.consecutive_failures = 0;

            result.status = if result.consecutive_successes >= self.config.healthy_threshold {
                HealthStatus::Healthy
            } else {
                HealthStatus::Degraded
            };
        } else {
            result.consecutive_failures = prev_failures + 1;
            result.consecutive_successes = 0;

            result.status = if result.consecutive_failures >= self.config.unhealthy_threshold {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Degraded
            };
        }

        results.insert(url.to_string(), result.clone());
        result
    }

    /// Get current health status of a backend
    pub async fn get_status(&self, url: &str) -> Option<HealthCheckResult> {
        let results = self.results.read().await;
        results.get(url).cloned()
    }

    /// Get all health results
    pub async fn get_all_results(&self) -> HashMap<String, HealthCheckResult> {
        self.results.read().await.clone()
    }

    /// Get all healthy backends
    pub async fn get_healthy_backends(&self) -> Vec<String> {
        let results = self.results.read().await;
        results
            .iter()
            .filter(|(_, r)| r.status.is_available())
            .map(|(url, _)| url.clone())
            .collect()
    }

    /// Start background health checking for multiple backends
    pub async fn start_background_checks(
        self: Arc<Self>,
        backends: Vec<String>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let state = self.clone();
        let interval = self.config.interval;

        let handle = tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);

            loop {
                interval_timer.tick().await;

                // Check every backend concurrently. Serially, one backend
                // sitting on `config.timeout` delayed the check of every
                // backend behind it, and with enough backends a single slow
                // one pushed the whole sweep past the tick interval - so the
                // health data went stale exactly when a backend was in
                // trouble.
                let mut set = tokio::task::JoinSet::new();
                for backend in &backends {
                    let state = state.clone();
                    let backend = backend.clone();
                    set.spawn(async move {
                        let result = state.check_backend(&backend).await;
                        (backend, result.status)
                    });
                }

                while let Some(joined) = set.join_next().await {
                    match joined {
                        Ok((backend, status)) => {
                            debug!("Background health check for {}: {:?}", backend, status);
                        }
                        Err(e) => {
                            debug!("Background health check task failed: {}", e);
                        }
                    }
                }
            }
        });

        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_availability() {
        assert!(HealthStatus::Healthy.is_available());
        assert!(HealthStatus::Degraded.is_available());
        assert!(!HealthStatus::Unhealthy.is_available());
        assert!(!HealthStatus::Unknown.is_available());
    }

    #[test]
    fn test_health_check_config_builder() {
        let config = HealthCheckConfig::new()
            .path("/healthz")
            .method("HEAD")
            .timeout(Duration::from_secs(10))
            .interval(Duration::from_secs(60))
            .unhealthy_threshold(5)
            .healthy_threshold(3);

        assert_eq!(config.path, "/healthz");
        assert_eq!(config.method, "HEAD");
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert_eq!(config.interval, Duration::from_secs(60));
        assert_eq!(config.unhealthy_threshold, 5);
        assert_eq!(config.healthy_threshold, 3);
    }

    #[test]
    fn test_health_check_result() {
        let healthy = HealthCheckResult::healthy("http://localhost:3000".into(), 50, 200);
        assert_eq!(healthy.status, HealthStatus::Healthy);
        assert_eq!(healthy.response_time_ms, Some(50));
        assert_eq!(healthy.http_status, Some(200));

        let unhealthy =
            HealthCheckResult::unhealthy("http://localhost:3000".into(), "Connection refused");
        assert_eq!(unhealthy.status, HealthStatus::Unhealthy);
        assert!(unhealthy.error.is_some());
    }

    #[tokio::test]
    async fn test_health_state() {
        let state = HealthState::new(HealthCheckConfig::default());

        // Initially no results
        let results = state.get_all_results().await;
        assert!(results.is_empty());
    }

    /// A deterministic, offline [`HealthCheck`] whose outcomes are scripted
    /// up front, so `HealthState::check_backend` transitions can be tested
    /// without any real network calls.
    struct ScriptedChecker {
        outcomes: std::sync::Mutex<std::collections::VecDeque<bool>>,
    }

    impl ScriptedChecker {
        fn new(outcomes: Vec<bool>) -> Self {
            Self {
                outcomes: std::sync::Mutex::new(outcomes.into_iter().collect()),
            }
        }
    }

    #[async_trait]
    impl HealthCheck for ScriptedChecker {
        async fn check(&self, url: &str, _config: &HealthCheckConfig) -> HealthCheckResult {
            let healthy = self
                .outcomes
                .lock()
                .expect("mutex poisoned")
                .pop_front()
                .unwrap_or(false);

            if healthy {
                HealthCheckResult::healthy(url.to_string(), 1, 200)
            } else {
                HealthCheckResult::unhealthy(url.to_string(), "scripted failure")
            }
        }
    }

    #[tokio::test]
    async fn test_health_transitions_at_exact_thresholds() {
        // unhealthy_threshold=3: the backend must stay Degraded through the
        // first two consecutive failures and only flip to Unhealthy on the
        // third. healthy_threshold=2 mirrors this for recovery.
        let config = HealthCheckConfig::new()
            .unhealthy_threshold(3)
            .healthy_threshold(2);
        let checker: Arc<dyn HealthCheck> =
            Arc::new(ScriptedChecker::new(vec![false, false, false, true, true]));
        let state = HealthState::with_checker(config, checker);

        let r1 = state.check_backend("http://backend").await;
        assert_eq!(r1.status, HealthStatus::Degraded, "1st failure");
        assert_eq!(r1.consecutive_failures, 1);

        let r2 = state.check_backend("http://backend").await;
        assert_eq!(
            r2.status,
            HealthStatus::Degraded,
            "2nd failure must stay Degraded, not jump to Unhealthy"
        );
        assert_eq!(r2.consecutive_failures, 2);

        let r3 = state.check_backend("http://backend").await;
        assert_eq!(
            r3.status,
            HealthStatus::Unhealthy,
            "3rd failure reaches unhealthy_threshold"
        );
        assert_eq!(r3.consecutive_failures, 3);

        let r4 = state.check_backend("http://backend").await;
        assert_eq!(
            r4.status,
            HealthStatus::Degraded,
            "1st success while recovering must stay Degraded"
        );
        assert_eq!(r4.consecutive_successes, 1);

        let r5 = state.check_backend("http://backend").await;
        assert_eq!(
            r5.status,
            HealthStatus::Healthy,
            "2nd success reaches healthy_threshold"
        );
        assert_eq!(r5.consecutive_successes, 2);
    }
}
