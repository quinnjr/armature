//! Metrics collection and aggregation

use crate::{
    ClientRateLimitInfo, EndpointMetrics, ErrorMetrics, ErrorRecord, ErrorSummary, LatencyMetrics,
    RateLimitEvent, RateLimitEventType, RateLimitMetrics, RequestMetrics, RequestRecord,
    ThroughputMetrics,
};
use chrono::Utc;
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Thread-safe metrics collector
pub struct MetricsCollector {
    // Request counters
    total_requests: AtomicU64,
    success_requests: AtomicU64,
    client_errors: AtomicU64,
    server_errors: AtomicU64,
    requests_by_method: DashMap<String, AtomicU64>,
    requests_by_status: DashMap<u16, AtomicU64>,

    // Latency tracking
    latency_samples: RwLock<VecDeque<f64>>,
    total_latency_ms: AtomicU64,
    min_latency_ms: AtomicU64,
    max_latency_ms: AtomicU64,

    // Error tracking
    total_errors: AtomicU64,
    errors_by_type: DashMap<String, AtomicU64>,
    errors_by_status: DashMap<u16, AtomicU64>,
    recent_errors: RwLock<VecDeque<ErrorSummary>>,

    // Rate limit tracking
    rate_limit_checks: AtomicU64,
    rate_limit_allowed: AtomicU64,
    rate_limit_limited: AtomicU64,
    rate_limited_clients: DashMap<String, ClientRateLimitInfo>,

    // Per-endpoint tracking
    endpoint_metrics: DashMap<String, EndpointData>,

    // Throughput tracking
    request_timestamps: RwLock<VecDeque<Instant>>,
    total_response_bytes: AtomicU64,
    peak_rps: RwLock<f64>,

    // Configuration limits
    max_latency_samples: usize,
    max_recent_errors: usize,
    max_endpoints: usize,
    max_rate_limit_clients: usize,
}

struct EndpointData {
    requests: AtomicU64,
    errors: AtomicU64,
    total_latency_ms: AtomicU64,
    latency_samples: RwLock<VecDeque<f64>>,
}

impl Default for EndpointData {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            total_latency_ms: AtomicU64::new(0),
            latency_samples: RwLock::new(VecDeque::with_capacity(1000)),
        }
    }
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self::with_limits(10_000, 100, 500, 1000)
    }

    pub fn with_limits(
        max_latency_samples: usize,
        max_recent_errors: usize,
        max_endpoints: usize,
        max_rate_limit_clients: usize,
    ) -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            success_requests: AtomicU64::new(0),
            client_errors: AtomicU64::new(0),
            server_errors: AtomicU64::new(0),
            requests_by_method: DashMap::new(),
            requests_by_status: DashMap::new(),

            latency_samples: RwLock::new(VecDeque::with_capacity(max_latency_samples)),
            total_latency_ms: AtomicU64::new(0),
            min_latency_ms: AtomicU64::new(u64::MAX),
            max_latency_ms: AtomicU64::new(0),

            total_errors: AtomicU64::new(0),
            errors_by_type: DashMap::new(),
            errors_by_status: DashMap::new(),
            recent_errors: RwLock::new(VecDeque::with_capacity(max_recent_errors)),

            rate_limit_checks: AtomicU64::new(0),
            rate_limit_allowed: AtomicU64::new(0),
            rate_limit_limited: AtomicU64::new(0),
            rate_limited_clients: DashMap::new(),

            endpoint_metrics: DashMap::new(),

            request_timestamps: RwLock::new(VecDeque::with_capacity(10_000)),
            total_response_bytes: AtomicU64::new(0),
            peak_rps: RwLock::new(0.0),

            max_latency_samples,
            max_recent_errors,
            max_endpoints,
            max_rate_limit_clients,
        }
    }

    /// Record a request
    pub fn record_request(&self, record: RequestRecord) {
        // Update counters
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        if record.is_success() {
            self.success_requests.fetch_add(1, Ordering::Relaxed);
        } else if record.is_client_error() {
            self.client_errors.fetch_add(1, Ordering::Relaxed);
        } else if record.is_server_error() {
            self.server_errors.fetch_add(1, Ordering::Relaxed);
        }

        // Update method counter
        self.requests_by_method
            .entry(record.method.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);

        // Update status counter
        self.requests_by_status
            .entry(record.status)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);

        // Record latency
        let latency_ms = record.duration.as_secs_f64() * 1000.0;
        self.record_latency(latency_ms);

        // Record response size
        if let Some(size) = record.response_size {
            self.total_response_bytes.fetch_add(size, Ordering::Relaxed);
        }

        // Record endpoint metrics
        let endpoint_key = format!("{} {}", record.method, record.path);
        if self.endpoint_metrics.len() < self.max_endpoints {
            let endpoint = self
                .endpoint_metrics
                .entry(endpoint_key)
                .or_insert_with(EndpointData::default);

            endpoint.requests.fetch_add(1, Ordering::Relaxed);
            if !record.is_success() {
                endpoint.errors.fetch_add(1, Ordering::Relaxed);
            }
            endpoint
                .total_latency_ms
                .fetch_add(latency_ms as u64, Ordering::Relaxed);

            let mut samples = endpoint.latency_samples.write();
            if samples.len() >= 1000 {
                samples.pop_front();
            }
            samples.push_back(latency_ms);
        }

        // Record timestamp for throughput calculation
        let mut timestamps = self.request_timestamps.write();
        let now = Instant::now();

        // Remove old timestamps (older than 1 minute)
        while let Some(front) = timestamps.front() {
            if now.duration_since(*front) > Duration::from_secs(60) {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        timestamps.push_back(now);

        // Update peak RPS
        let current_rps = timestamps.len() as f64 / 60.0;
        let mut peak = self.peak_rps.write();
        if current_rps > *peak {
            *peak = current_rps;
        }
    }

    fn record_latency(&self, latency_ms: f64) {
        let latency_u64 = latency_ms as u64;

        // Update total latency
        self.total_latency_ms
            .fetch_add(latency_u64, Ordering::Relaxed);

        // Update min
        let mut current_min = self.min_latency_ms.load(Ordering::Relaxed);
        while latency_u64 < current_min {
            match self.min_latency_ms.compare_exchange_weak(
                current_min,
                latency_u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => current_min = c,
            }
        }

        // Update max
        let mut current_max = self.max_latency_ms.load(Ordering::Relaxed);
        while latency_u64 > current_max {
            match self.max_latency_ms.compare_exchange_weak(
                current_max,
                latency_u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => current_max = c,
            }
        }

        // Add to samples
        let mut samples = self.latency_samples.write();
        if samples.len() >= self.max_latency_samples {
            samples.pop_front();
        }
        samples.push_back(latency_ms);
    }

    /// Record a rate limit event
    pub fn record_rate_limit(&self, event: RateLimitEvent) {
        self.rate_limit_checks.fetch_add(1, Ordering::Relaxed);

        match event.event_type {
            RateLimitEventType::Allowed => {
                self.rate_limit_allowed.fetch_add(1, Ordering::Relaxed);
            }
            RateLimitEventType::Limited => {
                self.rate_limit_limited.fetch_add(1, Ordering::Relaxed);

                // Track limited client
                if self.rate_limited_clients.len() < self.max_rate_limit_clients {
                    self.rate_limited_clients
                        .entry(event.client_id.clone())
                        .and_modify(|info| {
                            info.times_limited += 1;
                            info.last_limited = Utc::now();
                        })
                        .or_insert_with(|| ClientRateLimitInfo {
                            client_id: event.client_id,
                            times_limited: 1,
                            last_limited: Utc::now(),
                        });
                }
            }
            RateLimitEventType::Warning => {
                // Just count as allowed
                self.rate_limit_allowed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record an error
    pub fn record_error(&self, error: ErrorRecord) {
        self.total_errors.fetch_add(1, Ordering::Relaxed);

        // Update error type counter
        self.errors_by_type
            .entry(error.error_type.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);

        // Update error status counter
        if let Some(status) = error.status {
            self.errors_by_status
                .entry(status)
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }

        // Add to recent errors
        let mut recent = self.recent_errors.write();
        if recent.len() >= self.max_recent_errors {
            recent.pop_front();
        }
        recent.push_back(ErrorSummary {
            error_type: error.error_type,
            message: error.message,
            count: 1,
            last_seen: error.timestamp,
        });
    }

    /// Get request metrics
    pub fn request_metrics(&self) -> RequestMetrics {
        let by_method: HashMap<String, u64> = self
            .requests_by_method
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().load(Ordering::Relaxed)))
            .collect();

        let by_status: HashMap<u16, u64> = self
            .requests_by_status
            .iter()
            .map(|entry| (*entry.key(), entry.value().load(Ordering::Relaxed)))
            .collect();

        RequestMetrics {
            total: self.total_requests.load(Ordering::Relaxed),
            success: self.success_requests.load(Ordering::Relaxed),
            client_errors: self.client_errors.load(Ordering::Relaxed),
            server_errors: self.server_errors.load(Ordering::Relaxed),
            by_method,
            by_status,
        }
    }

    /// Get latency metrics
    pub fn latency_metrics(&self) -> LatencyMetrics {
        let samples = self.latency_samples.read();
        let total = self.total_requests.load(Ordering::Relaxed);

        if samples.is_empty() || total == 0 {
            return LatencyMetrics::default();
        }

        let mut sorted: Vec<f64> = samples.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = sorted.len();
        let avg = self.total_latency_ms.load(Ordering::Relaxed) as f64 / total as f64;
        let min = self.min_latency_ms.load(Ordering::Relaxed);
        let max = self.max_latency_ms.load(Ordering::Relaxed);

        LatencyMetrics {
            avg_ms: avg,
            min_ms: if min == u64::MAX { 0.0 } else { min as f64 },
            max_ms: max as f64,
            p50_ms: percentile(&sorted, 50.0),
            p90_ms: percentile(&sorted, 90.0),
            p95_ms: percentile(&sorted, 95.0),
            p99_ms: percentile(&sorted, 99.0),
            samples: len as u64,
        }
    }

    /// Get error metrics
    pub fn error_metrics(&self) -> ErrorMetrics {
        let by_type: HashMap<String, u64> = self
            .errors_by_type
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().load(Ordering::Relaxed)))
            .collect();

        let by_status: HashMap<u16, u64> = self
            .errors_by_status
            .iter()
            .map(|entry| (*entry.key(), entry.value().load(Ordering::Relaxed)))
            .collect();

        let recent: Vec<ErrorSummary> = self.recent_errors.read().iter().cloned().collect();

        ErrorMetrics {
            total: self.total_errors.load(Ordering::Relaxed),
            by_type,
            by_status,
            recent,
        }
    }

    /// Get rate limit metrics
    pub fn rate_limit_metrics(&self) -> RateLimitMetrics {
        let total_checks = self.rate_limit_checks.load(Ordering::Relaxed);
        let allowed = self.rate_limit_allowed.load(Ordering::Relaxed);
        let limited = self.rate_limit_limited.load(Ordering::Relaxed);

        let mut top_limited: Vec<ClientRateLimitInfo> = self
            .rate_limited_clients
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        top_limited.sort_by(|a, b| b.times_limited.cmp(&a.times_limited));
        top_limited.truncate(10);

        let avg_utilization = if total_checks > 0 {
            (allowed as f64 / total_checks as f64) * 100.0
        } else {
            0.0
        };

        RateLimitMetrics {
            total_checks,
            allowed,
            limited,
            unique_clients_limited: self.rate_limited_clients.len() as u64,
            avg_utilization,
            top_limited_clients: top_limited,
        }
    }

    /// Get per-endpoint metrics
    pub fn endpoint_metrics(&self) -> Vec<EndpointMetrics> {
        self.endpoint_metrics
            .iter()
            .map(|entry| {
                let key = entry.key();
                let data = entry.value();
                let requests = data.requests.load(Ordering::Relaxed);
                let errors = data.errors.load(Ordering::Relaxed);
                let total_latency = data.total_latency_ms.load(Ordering::Relaxed);

                let samples = data.latency_samples.read();
                let mut sorted: Vec<f64> = samples.iter().copied().collect();
                sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                let parts: Vec<&str> = key.splitn(2, ' ').collect();
                let (method, path) = if parts.len() == 2 {
                    (parts[0].to_string(), parts[1].to_string())
                } else {
                    ("".to_string(), key.clone())
                };

                EndpointMetrics {
                    path,
                    method,
                    requests,
                    errors,
                    avg_latency_ms: if requests > 0 {
                        total_latency as f64 / requests as f64
                    } else {
                        0.0
                    },
                    p99_latency_ms: percentile(&sorted, 99.0),
                    error_rate: if requests > 0 {
                        (errors as f64 / requests as f64) * 100.0
                    } else {
                        0.0
                    },
                }
            })
            .collect()
    }

    /// Get throughput metrics
    pub fn throughput_metrics(&self) -> ThroughputMetrics {
        let timestamps = self.request_timestamps.read();
        let now = Instant::now();

        // Count requests in last minute
        let requests_last_minute = timestamps
            .iter()
            .filter(|t| now.duration_since(**t) <= Duration::from_secs(60))
            .count() as u64;

        // Count requests in last hour (approximate from minute count)
        let requests_last_hour = requests_last_minute * 60;

        // Calculate current RPS
        let rps = requests_last_minute as f64 / 60.0;

        ThroughputMetrics {
            requests_per_second: rps,
            requests_last_minute,
            requests_last_hour,
            peak_rps: *self.peak_rps.read(),
            avg_response_size: {
                let total = self.total_requests.load(Ordering::Relaxed);
                if total > 0 {
                    self.total_response_bytes.load(Ordering::Relaxed) / total
                } else {
                    0
                }
            },
            total_bytes_transferred: self.total_response_bytes.load(Ordering::Relaxed),
        }
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.success_requests.store(0, Ordering::Relaxed);
        self.client_errors.store(0, Ordering::Relaxed);
        self.server_errors.store(0, Ordering::Relaxed);
        self.requests_by_method.clear();
        self.requests_by_status.clear();

        self.latency_samples.write().clear();
        self.total_latency_ms.store(0, Ordering::Relaxed);
        self.min_latency_ms.store(u64::MAX, Ordering::Relaxed);
        self.max_latency_ms.store(0, Ordering::Relaxed);

        self.total_errors.store(0, Ordering::Relaxed);
        self.errors_by_type.clear();
        self.errors_by_status.clear();
        self.recent_errors.write().clear();

        self.rate_limit_checks.store(0, Ordering::Relaxed);
        self.rate_limit_allowed.store(0, Ordering::Relaxed);
        self.rate_limit_limited.store(0, Ordering::Relaxed);
        self.rate_limited_clients.clear();

        self.endpoint_metrics.clear();

        self.request_timestamps.write().clear();
        self.total_response_bytes.store(0, Ordering::Relaxed);
        *self.peak_rps.write() = 0.0;
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate percentile from sorted array
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_calculation() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        // Using nearest-rank method: idx = round((pct/100) * (n-1))
        // 50th percentile: round(0.5 * 9) = round(4.5) = 5 -> data[5] = 6.0
        assert_eq!(percentile(&data, 50.0), 6.0);
        assert_eq!(percentile(&data, 90.0), 9.0);
        assert_eq!(percentile(&data, 100.0), 10.0);
    }

    #[test]
    fn test_collector_requests() {
        let collector = MetricsCollector::new();

        collector.record_request(RequestRecord::new(
            "GET",
            "/api/users",
            200,
            Duration::from_millis(50),
        ));

        collector.record_request(RequestRecord::new(
            "POST",
            "/api/users",
            201,
            Duration::from_millis(100),
        ));

        collector.record_request(RequestRecord::new(
            "GET",
            "/api/users/1",
            404,
            Duration::from_millis(10),
        ));

        let metrics = collector.request_metrics();
        assert_eq!(metrics.total, 3);
        assert_eq!(metrics.success, 2);
        assert_eq!(metrics.client_errors, 1);
    }
}
