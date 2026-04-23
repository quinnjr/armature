//! Analytics insights and alerting

use crate::{AnalyticsSnapshot, EndpointMetrics, LatencyMetrics, RequestMetrics};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Types of insights
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightType {
    HighErrorRate,
    HighLatency,
    RateLimitPressure,
    TrafficSpike,
    SlowEndpoint,
    ErrorSpike,
}

/// Severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// An analytics insight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub insight_type: InsightType,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    pub value: f64,
    pub threshold: f64,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

impl Insight {
    pub fn new(
        insight_type: InsightType,
        severity: Severity,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            insight_type,
            severity,
            title: title.into(),
            description: description.into(),
            value: 0.0,
            threshold: 0.0,
            timestamp: Utc::now(),
            endpoint: None,
            recommendation: None,
        }
    }

    pub fn with_value(mut self, value: f64) -> Self {
        self.value = value;
        self
    }

    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold;
        self
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    pub fn with_recommendation(mut self, rec: impl Into<String>) -> Self {
        self.recommendation = Some(rec.into());
        self
    }
}

/// Configuration for insight detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightConfig {
    /// Error rate threshold (percentage)
    pub error_rate_warning: f64,
    pub error_rate_critical: f64,

    /// Latency thresholds (milliseconds)
    pub p99_latency_warning_ms: f64,
    pub p99_latency_critical_ms: f64,

    /// Rate limit thresholds (percentage of requests limited)
    pub rate_limit_warning: f64,
    pub rate_limit_critical: f64,

    /// Traffic spike threshold (multiplier over average)
    pub traffic_spike_multiplier: f64,

    /// Minimum requests before generating insights
    pub min_requests: u64,
}

impl Default for InsightConfig {
    fn default() -> Self {
        Self {
            error_rate_warning: 1.0,  // 1% error rate
            error_rate_critical: 5.0, // 5% error rate
            p99_latency_warning_ms: 500.0,
            p99_latency_critical_ms: 2000.0,
            rate_limit_warning: 10.0,  // 10% of requests limited
            rate_limit_critical: 25.0, // 25% of requests limited
            traffic_spike_multiplier: 3.0,
            min_requests: 100,
        }
    }
}

/// Insight generator
pub struct InsightGenerator {
    config: InsightConfig,
    baseline_rps: Option<f64>,
}

impl InsightGenerator {
    pub fn new(config: InsightConfig) -> Self {
        Self {
            config,
            baseline_rps: None,
        }
    }

    /// Set baseline RPS for traffic spike detection
    pub fn set_baseline_rps(&mut self, rps: f64) {
        self.baseline_rps = Some(rps);
    }

    /// Generate insights from analytics snapshot
    pub fn generate(&self, snapshot: &AnalyticsSnapshot) -> Vec<Insight> {
        let mut insights = Vec::new();

        // Skip if not enough data
        if snapshot.requests.total < self.config.min_requests {
            return insights;
        }

        // Check error rate
        if let Some(insight) = self.check_error_rate(&snapshot.requests) {
            insights.push(insight);
        }

        // Check latency
        if let Some(insight) = self.check_latency(&snapshot.latency) {
            insights.push(insight);
        }

        // Check rate limits
        if let Some(insight) = self.check_rate_limits(snapshot) {
            insights.push(insight);
        }

        // Check traffic spike
        if let Some(insight) = self.check_traffic_spike(snapshot) {
            insights.push(insight);
        }

        // Check slow endpoints
        for insight in self.check_slow_endpoints(&snapshot.endpoints) {
            insights.push(insight);
        }

        insights
    }

    fn check_error_rate(&self, requests: &RequestMetrics) -> Option<Insight> {
        let error_rate = requests.error_rate();

        if error_rate >= self.config.error_rate_critical {
            Some(
                Insight::new(
                    InsightType::HighErrorRate,
                    Severity::Critical,
                    "Critical Error Rate",
                    format!("Error rate is {:.2}%, above critical threshold of {:.2}%",
                        error_rate, self.config.error_rate_critical),
                )
                .with_value(error_rate)
                .with_threshold(self.config.error_rate_critical)
                .with_recommendation("Investigate error logs immediately. Check for deployment issues or upstream service failures."),
            )
        } else if error_rate >= self.config.error_rate_warning {
            Some(
                Insight::new(
                    InsightType::HighErrorRate,
                    Severity::Warning,
                    "Elevated Error Rate",
                    format!(
                        "Error rate is {:.2}%, above warning threshold of {:.2}%",
                        error_rate, self.config.error_rate_warning
                    ),
                )
                .with_value(error_rate)
                .with_threshold(self.config.error_rate_warning)
                .with_recommendation("Review error logs and monitor for further increase."),
            )
        } else {
            None
        }
    }

    fn check_latency(&self, latency: &LatencyMetrics) -> Option<Insight> {
        if latency.p99_ms >= self.config.p99_latency_critical_ms {
            Some(
                Insight::new(
                    InsightType::HighLatency,
                    Severity::Critical,
                    "Critical Latency",
                    format!(
                        "P99 latency is {:.0}ms, above critical threshold of {:.0}ms",
                        latency.p99_ms, self.config.p99_latency_critical_ms
                    ),
                )
                .with_value(latency.p99_ms)
                .with_threshold(self.config.p99_latency_critical_ms)
                .with_recommendation(
                    "Check database queries, external service calls, and resource utilization.",
                ),
            )
        } else if latency.p99_ms >= self.config.p99_latency_warning_ms {
            Some(
                Insight::new(
                    InsightType::HighLatency,
                    Severity::Warning,
                    "Elevated Latency",
                    format!(
                        "P99 latency is {:.0}ms, above warning threshold of {:.0}ms",
                        latency.p99_ms, self.config.p99_latency_warning_ms
                    ),
                )
                .with_value(latency.p99_ms)
                .with_threshold(self.config.p99_latency_warning_ms)
                .with_recommendation("Profile slow requests and consider caching or optimization."),
            )
        } else {
            None
        }
    }

    fn check_rate_limits(&self, snapshot: &AnalyticsSnapshot) -> Option<Insight> {
        let rate_limits = &snapshot.rate_limits;
        if rate_limits.total_checks == 0 {
            return None;
        }

        let limited_rate = (rate_limits.limited as f64 / rate_limits.total_checks as f64) * 100.0;

        if limited_rate >= self.config.rate_limit_critical {
            Some(
                Insight::new(
                    InsightType::RateLimitPressure,
                    Severity::Critical,
                    "Critical Rate Limit Pressure",
                    format!("{:.2}% of requests are being rate limited", limited_rate),
                )
                .with_value(limited_rate)
                .with_threshold(self.config.rate_limit_critical)
                .with_recommendation("Consider increasing rate limits, adding capacity, or implementing request queuing."),
            )
        } else if limited_rate >= self.config.rate_limit_warning {
            Some(
                Insight::new(
                    InsightType::RateLimitPressure,
                    Severity::Warning,
                    "Rate Limit Pressure",
                    format!("{:.2}% of requests are being rate limited", limited_rate),
                )
                .with_value(limited_rate)
                .with_threshold(self.config.rate_limit_warning)
                .with_recommendation("Monitor rate limit usage and consider adjusting limits for legitimate traffic."),
            )
        } else {
            None
        }
    }

    fn check_traffic_spike(&self, snapshot: &AnalyticsSnapshot) -> Option<Insight> {
        let baseline = self.baseline_rps?;
        let current_rps = snapshot.throughput.requests_per_second;

        if current_rps > baseline * self.config.traffic_spike_multiplier {
            Some(
                Insight::new(
                    InsightType::TrafficSpike,
                    Severity::Warning,
                    "Traffic Spike Detected",
                    format!(
                        "Current RPS ({:.1}) is {:.1}x higher than baseline ({:.1})",
                        current_rps,
                        current_rps / baseline,
                        baseline
                    ),
                )
                .with_value(current_rps)
                .with_threshold(baseline * self.config.traffic_spike_multiplier)
                .with_recommendation(
                    "Investigate traffic source. Consider enabling auto-scaling if available.",
                ),
            )
        } else {
            None
        }
    }

    fn check_slow_endpoints(&self, endpoints: &[EndpointMetrics]) -> Vec<Insight> {
        let mut insights = Vec::new();

        for endpoint in endpoints {
            // Skip endpoints with few requests
            if endpoint.requests < 10 {
                continue;
            }

            // Check for slow endpoints
            if endpoint.p99_latency_ms >= self.config.p99_latency_critical_ms {
                insights.push(
                    Insight::new(
                        InsightType::SlowEndpoint,
                        Severity::Warning,
                        "Slow Endpoint",
                        format!(
                            "{} {} has P99 latency of {:.0}ms",
                            endpoint.method, endpoint.path, endpoint.p99_latency_ms
                        ),
                    )
                    .with_value(endpoint.p99_latency_ms)
                    .with_threshold(self.config.p99_latency_critical_ms)
                    .with_endpoint(format!("{} {}", endpoint.method, endpoint.path))
                    .with_recommendation(
                        "Profile this specific endpoint for optimization opportunities.",
                    ),
                );
            }

            // Check for high error rate endpoints
            if endpoint.error_rate >= self.config.error_rate_critical {
                insights.push(
                    Insight::new(
                        InsightType::ErrorSpike,
                        Severity::Warning,
                        "High Error Rate Endpoint",
                        format!(
                            "{} {} has error rate of {:.2}%",
                            endpoint.method, endpoint.path, endpoint.error_rate
                        ),
                    )
                    .with_value(endpoint.error_rate)
                    .with_threshold(self.config.error_rate_critical)
                    .with_endpoint(format!("{} {}", endpoint.method, endpoint.path))
                    .with_recommendation("Investigate errors specific to this endpoint."),
                );
            }
        }

        insights
    }
}

impl Default for InsightGenerator {
    fn default() -> Self {
        Self::new(InsightConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insight_creation() {
        let insight = Insight::new(
            InsightType::HighErrorRate,
            Severity::Critical,
            "Test",
            "Description",
        )
        .with_value(5.0)
        .with_threshold(1.0)
        .with_recommendation("Fix it");

        assert_eq!(insight.insight_type, InsightType::HighErrorRate);
        assert_eq!(insight.severity, Severity::Critical);
        assert_eq!(insight.value, 5.0);
    }

    #[test]
    fn test_insight_config_defaults() {
        let config = InsightConfig::default();
        assert_eq!(config.error_rate_warning, 1.0);
        assert_eq!(config.error_rate_critical, 5.0);
    }
}
