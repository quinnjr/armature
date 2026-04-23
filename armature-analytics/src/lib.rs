//! API Analytics Module for Armature Framework
//!
//! Provides comprehensive API usage tracking, rate limit insights, and error monitoring.
//!
//! ## Features
//!
//! - **Request Metrics**: Track requests per endpoint, method, and status code
//! - **Latency Tracking**: P50, P90, P95, P99 latency percentiles
//! - **Error Rates**: Monitor error rates by endpoint and error type
//! - **Rate Limit Insights**: Track rate limit hits, rejections, and usage patterns
//! - **Throughput Monitoring**: Requests per second, minute, hour
//! - **Real-time Dashboard**: JSON endpoint for analytics data
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_analytics::{Analytics, AnalyticsMiddleware};
//! use armature_core::Application;
//!
//! let analytics = Analytics::new(AnalyticsConfig::default());
//!
//! let app = Application::new(container, router)
//!     .middleware(AnalyticsMiddleware::new(analytics.clone()));
//!
//! // Access analytics endpoint
//! // GET /api/_analytics -> JSON dashboard data
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        Requests                              │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//!                           ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                 AnalyticsMiddleware                          │
//! │  - Captures request/response metadata                        │
//! │  - Records timing, status, errors                            │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//!                           ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    MetricsCollector                          │
//! │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐       │
//! │  │ Requests │ │ Latency  │ │  Errors  │ │Rate Limit│       │
//! │  │ Counter  │ │Histogram │ │ Tracker  │ │ Insights │       │
//! │  └──────────┘ └──────────┘ └──────────┘ └──────────┘       │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//!                           ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Export Backends                           │
//! │  ┌──────────┐ ┌──────────┐ ┌──────────┐                     │
//! │  │   JSON   │ │Prometheus│ │  Custom  │                     │
//! │  └──────────┘ └──────────┘ └──────────┘                     │
//! └─────────────────────────────────────────────────────────────┘
//! ```

mod collector;
mod config;
mod error;
mod insights;
mod metrics;
mod middleware;

pub use collector::*;
pub use config::*;
pub use error::*;
pub use insights::*;
pub use metrics::*;
pub use middleware::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Main analytics instance
///
/// Thread-safe analytics collector that can be shared across handlers.
#[derive(Clone)]
pub struct Analytics {
    inner: Arc<AnalyticsInner>,
}

struct AnalyticsInner {
    config: AnalyticsConfig,
    collector: MetricsCollector,
    started_at: DateTime<Utc>,
}

impl Analytics {
    /// Create a new analytics instance
    pub fn new(config: AnalyticsConfig) -> Self {
        Self {
            inner: Arc::new(AnalyticsInner {
                config,
                collector: MetricsCollector::new(),
                started_at: Utc::now(),
            }),
        }
    }

    /// Record a request
    pub fn record_request(&self, record: RequestRecord) {
        self.inner.collector.record_request(record);
    }

    /// Record a rate limit event
    pub fn record_rate_limit(&self, event: RateLimitEvent) {
        self.inner.collector.record_rate_limit(event);
    }

    /// Record an error
    pub fn record_error(&self, error: ErrorRecord) {
        self.inner.collector.record_error(error);
    }

    /// Get current analytics snapshot
    pub fn snapshot(&self) -> AnalyticsSnapshot {
        let collector = &self.inner.collector;

        AnalyticsSnapshot {
            timestamp: Utc::now(),
            uptime_seconds: (Utc::now() - self.inner.started_at).num_seconds() as u64,
            requests: collector.request_metrics(),
            latency: collector.latency_metrics(),
            errors: collector.error_metrics(),
            rate_limits: collector.rate_limit_metrics(),
            endpoints: collector.endpoint_metrics(),
            throughput: collector.throughput_metrics(),
        }
    }

    /// Get JSON dashboard data
    pub fn dashboard_json(&self) -> String {
        serde_json::to_string_pretty(&self.snapshot()).unwrap_or_else(|_| "{}".to_string())
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.inner.collector.reset();
    }

    /// Get the configuration
    pub fn config(&self) -> &AnalyticsConfig {
        &self.inner.config
    }
}

// =============================================================================
// Request Recording
// =============================================================================

/// Record of a single request for analytics
#[derive(Debug, Clone)]
pub struct RequestRecord {
    /// HTTP method
    pub method: String,
    /// Request path (normalized)
    pub path: String,
    /// HTTP status code
    pub status: u16,
    /// Request duration
    pub duration: Duration,
    /// Request timestamp
    pub timestamp: DateTime<Utc>,
    /// Client identifier (IP, user ID, etc.)
    pub client_id: Option<String>,
    /// Response body size in bytes
    pub response_size: Option<u64>,
    /// Whether the request was authenticated
    pub authenticated: bool,
    /// Custom tags for filtering
    pub tags: HashMap<String, String>,
}

impl RequestRecord {
    /// Create a new request record
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        status: u16,
        duration: Duration,
    ) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            status,
            duration,
            timestamp: Utc::now(),
            client_id: None,
            response_size: None,
            authenticated: false,
            tags: HashMap::new(),
        }
    }

    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    pub fn with_response_size(mut self, size: u64) -> Self {
        self.response_size = Some(size);
        self
    }

    pub fn with_authenticated(mut self, authenticated: bool) -> Self {
        self.authenticated = authenticated;
        self
    }

    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Check if request was successful (2xx)
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// Check if request was a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        self.status >= 400 && self.status < 500
    }

    /// Check if request was a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        self.status >= 500
    }
}

// =============================================================================
// Rate Limit Events
// =============================================================================

/// Rate limit event types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateLimitEventType {
    /// Request was allowed
    Allowed,
    /// Request was rate limited
    Limited,
    /// Near the limit (warning threshold)
    Warning,
}

/// Record of a rate limit event
#[derive(Debug, Clone)]
pub struct RateLimitEvent {
    /// Client identifier
    pub client_id: String,
    /// Event type
    pub event_type: RateLimitEventType,
    /// Current request count
    pub current_count: u64,
    /// Maximum allowed requests
    pub limit: u64,
    /// Time window in seconds
    pub window_seconds: u64,
    /// Endpoint affected
    pub endpoint: Option<String>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

impl RateLimitEvent {
    pub fn allowed(client_id: impl Into<String>, current: u64, limit: u64, window: u64) -> Self {
        Self {
            client_id: client_id.into(),
            event_type: RateLimitEventType::Allowed,
            current_count: current,
            limit,
            window_seconds: window,
            endpoint: None,
            timestamp: Utc::now(),
        }
    }

    pub fn limited(client_id: impl Into<String>, current: u64, limit: u64, window: u64) -> Self {
        Self {
            client_id: client_id.into(),
            event_type: RateLimitEventType::Limited,
            current_count: current,
            limit,
            window_seconds: window,
            endpoint: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Calculate utilization percentage
    pub fn utilization(&self) -> f64 {
        if self.limit == 0 {
            0.0
        } else {
            (self.current_count as f64 / self.limit as f64) * 100.0
        }
    }
}

// =============================================================================
// Error Recording
// =============================================================================

/// Record of an error for analytics
#[derive(Debug, Clone)]
pub struct ErrorRecord {
    /// Error type/code
    pub error_type: String,
    /// Error message
    pub message: String,
    /// HTTP status code
    pub status: Option<u16>,
    /// Endpoint where error occurred
    pub endpoint: Option<String>,
    /// Stack trace (if available)
    pub stack_trace: Option<String>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Additional context
    pub context: HashMap<String, String>,
}

impl ErrorRecord {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error_type: error_type.into(),
            message: message.into(),
            status: None,
            endpoint: None,
            stack_trace: None,
            timestamp: Utc::now(),
            context: HashMap::new(),
        }
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }
}

// =============================================================================
// Analytics Snapshot
// =============================================================================

/// Complete analytics snapshot for dashboard/export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyticsSnapshot {
    /// Snapshot timestamp
    pub timestamp: DateTime<Utc>,
    /// Uptime in seconds
    pub uptime_seconds: u64,
    /// Request metrics
    pub requests: RequestMetrics,
    /// Latency metrics
    pub latency: LatencyMetrics,
    /// Error metrics
    pub errors: ErrorMetrics,
    /// Rate limit metrics
    pub rate_limits: RateLimitMetrics,
    /// Per-endpoint metrics
    pub endpoints: Vec<EndpointMetrics>,
    /// Throughput metrics
    pub throughput: ThroughputMetrics,
}

/// Request metrics summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestMetrics {
    /// Total requests
    pub total: u64,
    /// Successful requests (2xx)
    pub success: u64,
    /// Client errors (4xx)
    pub client_errors: u64,
    /// Server errors (5xx)
    pub server_errors: u64,
    /// Requests by method
    pub by_method: HashMap<String, u64>,
    /// Requests by status code
    pub by_status: HashMap<u16, u64>,
}

impl RequestMetrics {
    /// Calculate success rate as percentage
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            100.0
        } else {
            (self.success as f64 / self.total as f64) * 100.0
        }
    }

    /// Calculate error rate as percentage
    pub fn error_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            ((self.client_errors + self.server_errors) as f64 / self.total as f64) * 100.0
        }
    }
}

/// Latency metrics with percentiles
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LatencyMetrics {
    /// Average latency in milliseconds
    pub avg_ms: f64,
    /// Minimum latency in milliseconds
    pub min_ms: f64,
    /// Maximum latency in milliseconds
    pub max_ms: f64,
    /// 50th percentile (median)
    pub p50_ms: f64,
    /// 90th percentile
    pub p90_ms: f64,
    /// 95th percentile
    pub p95_ms: f64,
    /// 99th percentile
    pub p99_ms: f64,
    /// Sample count
    pub samples: u64,
}

/// Error metrics summary
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorMetrics {
    /// Total errors
    pub total: u64,
    /// Errors by type
    pub by_type: HashMap<String, u64>,
    /// Errors by status code
    pub by_status: HashMap<u16, u64>,
    /// Recent errors (last N)
    pub recent: Vec<ErrorSummary>,
}

/// Summary of a recent error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSummary {
    pub error_type: String,
    pub message: String,
    pub count: u64,
    pub last_seen: DateTime<Utc>,
}

/// Rate limit metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RateLimitMetrics {
    /// Total rate limit checks
    pub total_checks: u64,
    /// Requests that were allowed
    pub allowed: u64,
    /// Requests that were limited
    pub limited: u64,
    /// Unique clients rate limited
    pub unique_clients_limited: u64,
    /// Average utilization percentage
    pub avg_utilization: f64,
    /// Top rate-limited clients
    pub top_limited_clients: Vec<ClientRateLimitInfo>,
}

/// Rate limit info for a specific client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRateLimitInfo {
    pub client_id: String,
    pub times_limited: u64,
    pub last_limited: DateTime<Utc>,
}

/// Per-endpoint metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointMetrics {
    /// Endpoint path
    pub path: String,
    /// HTTP method
    pub method: String,
    /// Total requests
    pub requests: u64,
    /// Error count
    pub errors: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
    /// P99 latency in milliseconds
    pub p99_latency_ms: f64,
    /// Error rate percentage
    pub error_rate: f64,
}

/// Throughput metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThroughputMetrics {
    /// Requests per second (current)
    pub requests_per_second: f64,
    /// Requests in the last minute
    pub requests_last_minute: u64,
    /// Requests in the last hour
    pub requests_last_hour: u64,
    /// Peak requests per second
    pub peak_rps: f64,
    /// Average response size in bytes
    pub avg_response_size: u64,
    /// Total data transferred in bytes
    pub total_bytes_transferred: u64,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_record() {
        let record = RequestRecord::new("GET", "/api/users", 200, Duration::from_millis(50))
            .with_client_id("user-123")
            .with_response_size(1024)
            .with_authenticated(true)
            .with_tag("version", "v1");

        assert!(record.is_success());
        assert!(!record.is_client_error());
        assert!(!record.is_server_error());
        assert_eq!(record.client_id, Some("user-123".to_string()));
    }

    #[test]
    fn test_rate_limit_event() {
        let event = RateLimitEvent::limited("client-1", 100, 100, 60);
        assert_eq!(event.utilization(), 100.0);

        let event = RateLimitEvent::allowed("client-2", 50, 100, 60);
        assert_eq!(event.utilization(), 50.0);
    }

    #[test]
    fn test_request_metrics() {
        let metrics = RequestMetrics {
            total: 100,
            success: 90,
            client_errors: 8,
            server_errors: 2,
            ..Default::default()
        };

        assert_eq!(metrics.success_rate(), 90.0);
        assert_eq!(metrics.error_rate(), 10.0);
    }

    #[test]
    fn test_analytics_creation() {
        let analytics = Analytics::new(AnalyticsConfig::default());
        let snapshot = analytics.snapshot();

        assert_eq!(snapshot.requests.total, 0);
        assert_eq!(snapshot.errors.total, 0);
    }
}
