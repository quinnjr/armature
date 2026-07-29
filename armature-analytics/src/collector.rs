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
use std::time::Instant;

/// Thread-safe metrics collector
pub struct MetricsCollector {
    // Request counters
    total_requests: AtomicU64,
    success_requests: AtomicU64,
    client_errors: AtomicU64,
    server_errors: AtomicU64,
    requests_by_method: DashMap<String, AtomicU64>,
    requests_by_status: DashMap<u16, AtomicU64>,

    // Latency tracking (totals/min/max stored in microseconds to preserve
    // sub-millisecond resolution; sample deque keeps exact millisecond floats).
    latency_samples: RwLock<VecDeque<f64>>,
    total_latency_us: AtomicU64,
    min_latency_us: AtomicU64,
    max_latency_us: AtomicU64,

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
    // Running sum of per-event utilization percentages, used to compute a
    // genuine average utilization rather than the allowed/total ratio.
    rate_limit_utilization_sum: RwLock<f64>,

    // Per-endpoint tracking
    endpoint_metrics: DashMap<String, EndpointData>,

    // Throughput tracking. Requests are downsampled into per-second buckets
    // `(second, count)` rather than one entry per request. This bounds the
    // structure to ~one bucket per second over the retention horizon
    // (~3600 entries) regardless of RPS — instead of RPS*3600 timestamps —
    // and bounds the peak-RPS computation to at most `throughput_window_secs`
    // buckets instead of a full O(requests) scan under the write lock.
    throughput_epoch: Instant,
    request_buckets: RwLock<VecDeque<(u64, u64)>>,
    total_response_bytes: AtomicU64,
    peak_rps: RwLock<f64>,

    // Configuration limits
    max_latency_samples: usize,
    max_recent_errors: usize,
    max_endpoints: usize,
    max_rate_limit_clients: usize,

    // Configuration toggles / windows
    enable_endpoint_metrics: bool,
    enable_rate_limit_tracking: bool,
    throughput_window_secs: u64,
}

struct EndpointData {
    requests: AtomicU64,
    errors: AtomicU64,
    total_latency_us: AtomicU64,
    latency_samples: RwLock<VecDeque<f64>>,
}

impl Default for EndpointData {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
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
            total_latency_us: AtomicU64::new(0),
            min_latency_us: AtomicU64::new(u64::MAX),
            max_latency_us: AtomicU64::new(0),

            total_errors: AtomicU64::new(0),
            errors_by_type: DashMap::new(),
            errors_by_status: DashMap::new(),
            recent_errors: RwLock::new(VecDeque::with_capacity(max_recent_errors)),

            rate_limit_checks: AtomicU64::new(0),
            rate_limit_allowed: AtomicU64::new(0),
            rate_limit_limited: AtomicU64::new(0),
            rate_limited_clients: DashMap::new(),
            rate_limit_utilization_sum: RwLock::new(0.0),

            endpoint_metrics: DashMap::new(),

            throughput_epoch: Instant::now(),
            request_buckets: RwLock::new(VecDeque::with_capacity(3601)),
            total_response_bytes: AtomicU64::new(0),
            peak_rps: RwLock::new(0.0),

            max_latency_samples,
            max_recent_errors,
            max_endpoints,
            max_rate_limit_clients,

            enable_endpoint_metrics: true,
            enable_rate_limit_tracking: true,
            throughput_window_secs: 60,
        }
    }

    /// Build a collector from a full [`crate::AnalyticsConfig`], honoring both
    /// the capacity knobs and the behavioral toggles/windows.
    pub fn from_config(config: &crate::AnalyticsConfig) -> Self {
        let mut collector = Self::with_limits(
            config.max_latency_samples,
            config.max_recent_errors,
            config.max_endpoints,
            config.max_rate_limit_clients,
        );
        collector.enable_endpoint_metrics = config.enable_endpoint_metrics;
        collector.enable_rate_limit_tracking = config.enable_rate_limit_tracking;
        collector.throughput_window_secs = config.throughput_window_secs.max(1);
        collector
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

        // Record endpoint metrics (only when enabled). The cap must gate the
        // insertion of *new* endpoint keys only — an already-tracked endpoint
        // has to keep incrementing after the cap is reached, otherwise its
        // counters freeze the moment the map fills up.
        if self.enable_endpoint_metrics {
            let endpoint_key = format!("{} {}", record.method, record.path);
            let tracked = self.endpoint_metrics.contains_key(&endpoint_key)
                || self.endpoint_metrics.len() < self.max_endpoints;
            if tracked {
                let endpoint = self.endpoint_metrics.entry(endpoint_key).or_default();

                endpoint.requests.fetch_add(1, Ordering::Relaxed);
                if !record.is_success() {
                    endpoint.errors.fetch_add(1, Ordering::Relaxed);
                }
                endpoint
                    .total_latency_us
                    .fetch_add((latency_ms * 1000.0) as u64, Ordering::Relaxed);

                let mut samples = endpoint.latency_samples.write();
                if samples.len() >= 1000 {
                    samples.pop_front();
                }
                samples.push_back(latency_ms);
            }
        }

        // Record throughput into per-second buckets. Downsampling to
        // `(second, count)` keeps the structure count-bounded (~one bucket per
        // second over the one-hour horizon) even under sustained high RPS,
        // where a per-request timestamp deque would grow to RPS*3600 entries.
        // Retain up to one hour so `requests_last_hour` reflects real
        // observations rather than a `requests_last_minute * 60` extrapolation.
        //
        // The slight exactness tradeoff: windows are now second-granular (each
        // request is attributed to the 1-second bucket it fell in) rather than
        // exact-instant. `requests_last_minute`/`requests_last_hour` sum bucket
        // counts and remain correct to within sub-second boundary rounding.
        let now = Instant::now();
        let sec = now
            .saturating_duration_since(self.throughput_epoch)
            .as_secs();
        let window = self.throughput_window_secs.max(1);

        let mut buckets = self.request_buckets.write();

        // Append to the current second's bucket, opening a new one on rollover.
        match buckets.back_mut() {
            Some((s, count)) if *s == sec => *count += 1,
            _ => buckets.push_back((sec, 1)),
        }

        // Drop buckets older than the one-hour retention horizon.
        let horizon = sec.saturating_sub(3600);
        while let Some((s, _)) = buckets.front() {
            if *s < horizon {
                buckets.pop_front();
            } else {
                break;
            }
        }

        // Peak RPS over the configured window. Summing only the buckets inside
        // the window (at most `window` + 1 of them, since there is one bucket
        // per second) keeps this O(window) rather than O(total requests) — the
        // in-window count is maintained by the incremental bucket counts, so no
        // full scan of individual requests is ever performed under the lock.
        let window_start = sec.saturating_sub(window - 1);
        let in_window: u64 = sum_buckets_since(&buckets, window_start);
        drop(buckets);

        let current_rps = in_window as f64 / window as f64;
        let mut peak = self.peak_rps.write();
        if current_rps > *peak {
            *peak = current_rps;
        }
    }

    fn record_latency(&self, latency_ms: f64) {
        // Store in microseconds so sub-millisecond requests are not truncated
        // to zero (which previously zeroed min/avg for fast endpoints).
        let latency_us = (latency_ms * 1000.0) as u64;

        // Update total latency
        self.total_latency_us
            .fetch_add(latency_us, Ordering::Relaxed);

        // Update min
        let mut current_min = self.min_latency_us.load(Ordering::Relaxed);
        while latency_us < current_min {
            match self.min_latency_us.compare_exchange_weak(
                current_min,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(c) => current_min = c,
            }
        }

        // Update max
        let mut current_max = self.max_latency_us.load(Ordering::Relaxed);
        while latency_us > current_max {
            match self.max_latency_us.compare_exchange_weak(
                current_max,
                latency_us,
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
        // Respect the rate-limit tracking toggle.
        if !self.enable_rate_limit_tracking {
            return;
        }

        self.rate_limit_checks.fetch_add(1, Ordering::Relaxed);

        // Accumulate the event's utilization so `avg_utilization` reports the
        // mean of how close clients were to their limits, not the allow ratio.
        *self.rate_limit_utilization_sum.write() += event.utilization();

        match event.event_type {
            RateLimitEventType::Allowed => {
                self.rate_limit_allowed.fetch_add(1, Ordering::Relaxed);
            }
            RateLimitEventType::Limited => {
                self.rate_limit_limited.fetch_add(1, Ordering::Relaxed);

                // Track limited client. The cap gates insertion of *new*
                // clients only; an already-tracked client must keep counting.
                let tracked = self.rate_limited_clients.contains_key(&event.client_id)
                    || self.rate_limited_clients.len() < self.max_rate_limit_clients;
                if tracked {
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
        let mut values: Vec<f64> = {
            let samples = self.latency_samples.read();
            samples.iter().copied().collect()
        };
        let total = self.total_requests.load(Ordering::Relaxed);

        if values.is_empty() || total == 0 {
            return LatencyMetrics::default();
        }

        let len = values.len();
        let avg = self.total_latency_us.load(Ordering::Relaxed) as f64 / total as f64 / 1000.0;
        let min = self.min_latency_us.load(Ordering::Relaxed);
        let max = self.max_latency_us.load(Ordering::Relaxed);

        // Use quickselect (O(n) per percentile, only partially reordering the
        // buffer) instead of a full O(n log n) sort of the whole sample buffer
        // on every snapshot — matching `endpoint_metrics`.
        LatencyMetrics {
            avg_ms: avg,
            min_ms: if min == u64::MAX {
                0.0
            } else {
                min as f64 / 1000.0
            },
            max_ms: max as f64 / 1000.0,
            p50_ms: percentile_select(&mut values, 50.0),
            p90_ms: percentile_select(&mut values, 90.0),
            p95_ms: percentile_select(&mut values, 95.0),
            p99_ms: percentile_select(&mut values, 99.0),
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

        top_limited.sort_by_key(|e| std::cmp::Reverse(e.times_limited));
        top_limited.truncate(10);

        // Mean of the per-event utilization percentages. The previous formula
        // (allowed / total_checks) measured the allow ratio, not utilization.
        let avg_utilization = if total_checks > 0 {
            *self.rate_limit_utilization_sum.read() / total_checks as f64
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
                let total_latency_us = data.total_latency_us.load(Ordering::Relaxed);

                // Only the p99 is needed here, so use quickselect (O(n)) instead
                // of a full sort (O(n log n)) of every endpoint's sample buffer
                // on each snapshot.
                let mut values: Vec<f64> = data.latency_samples.read().iter().copied().collect();
                let p99 = percentile_select(&mut values, 99.0);

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
                        total_latency_us as f64 / requests as f64 / 1000.0
                    } else {
                        0.0
                    },
                    p99_latency_ms: p99,
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
        let buckets = self.request_buckets.read();
        let now = Instant::now();
        let sec = now
            .saturating_duration_since(self.throughput_epoch)
            .as_secs();

        // Sum the counts of all per-second buckets at or after `cutoff`. Buckets
        // are stored in ascending second order, so a reverse `take_while` visits
        // only the relevant tail.
        let sum_since = |cutoff: u64| -> u64 { sum_buckets_since(&buckets, cutoff) };

        // Second-granular windows (each request counted in its 1-second bucket).
        let requests_last_minute = sum_since(sec.saturating_sub(59));
        let requests_last_hour = sum_since(sec.saturating_sub(3599));

        // Current RPS is measured over the configured throughput window.
        let window = self.throughput_window_secs.max(1);
        let requests_in_window = sum_since(sec.saturating_sub(window - 1));
        let rps = requests_in_window as f64 / window as f64;

        ThroughputMetrics {
            requests_per_second: rps,
            requests_last_minute,
            requests_last_hour,
            peak_rps: *self.peak_rps.read(),
            avg_response_size: {
                let total = self.total_requests.load(Ordering::Relaxed);
                self.total_response_bytes
                    .load(Ordering::Relaxed)
                    .checked_div(total)
                    .unwrap_or(0)
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
        self.total_latency_us.store(0, Ordering::Relaxed);
        self.min_latency_us.store(u64::MAX, Ordering::Relaxed);
        self.max_latency_us.store(0, Ordering::Relaxed);

        self.total_errors.store(0, Ordering::Relaxed);
        self.errors_by_type.clear();
        self.errors_by_status.clear();
        self.recent_errors.write().clear();

        self.rate_limit_checks.store(0, Ordering::Relaxed);
        self.rate_limit_allowed.store(0, Ordering::Relaxed);
        self.rate_limit_limited.store(0, Ordering::Relaxed);
        self.rate_limited_clients.clear();
        *self.rate_limit_utilization_sum.write() = 0.0;

        self.endpoint_metrics.clear();

        self.request_buckets.write().clear();
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
///
/// Retained as the reference implementation used to validate `percentile_select`
/// in tests; production snapshots use the O(n) quickselect variant below.
#[cfg(test)]
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }

    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Calculate a percentile from an *unsorted* slice using quickselect.
///
/// Equivalent result to `percentile` on the sorted slice, but runs in O(n)
/// average time and only partially reorders `data`, avoiding a full sort when
/// a single percentile is required.
fn percentile_select(data: &mut [f64], pct: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let idx = (((pct / 100.0) * (data.len() - 1) as f64).round() as usize).min(data.len() - 1);
    let (_, nth, _) = data.select_nth_unstable_by(idx, |a, b| {
        a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
    });
    *nth
}

/// Sum the counts of all `(second, count)` buckets at or after `cutoff`.
///
/// Buckets are stored in ascending second order, so a reverse `take_while`
/// visits only the relevant tail and stops as soon as it passes `cutoff`.
fn sum_buckets_since(buckets: &std::collections::VecDeque<(u64, u64)>, cutoff: u64) -> u64 {
    buckets
        .iter()
        .rev()
        .take_while(|(s, _)| *s >= cutoff)
        .map(|(_, c)| c)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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

    #[test]
    fn test_percentile_select_matches_sorted() {
        let mut data = vec![10.0, 2.0, 7.0, 1.0, 9.0, 3.0, 8.0, 4.0, 6.0, 5.0];
        let mut sorted = data.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pct in [50.0, 90.0, 95.0, 99.0, 100.0] {
            assert_eq!(percentile_select(&mut data, pct), percentile(&sorted, pct));
        }
    }

    // Regression: once the endpoint cap is reached, an *already-tracked*
    // endpoint must keep incrementing. The old code wrapped the whole update in
    // `if len < max`, freezing every counter as soon as the map filled.
    #[test]
    fn test_endpoint_cap_does_not_freeze_existing() {
        let collector = MetricsCollector::from_config(&crate::AnalyticsConfig {
            max_endpoints: 1,
            ..crate::AnalyticsConfig::default()
        });

        // First endpoint fills the map to its cap.
        collector.record_request(RequestRecord::new(
            "GET",
            "/a",
            200,
            Duration::from_millis(5),
        ));
        // A second, different endpoint must be rejected (cap reached).
        collector.record_request(RequestRecord::new(
            "GET",
            "/b",
            200,
            Duration::from_millis(5),
        ));
        // But the already-tracked endpoint must keep counting.
        collector.record_request(RequestRecord::new(
            "GET",
            "/a",
            200,
            Duration::from_millis(5),
        ));

        let endpoints = collector.endpoint_metrics();
        assert_eq!(endpoints.len(), 1);
        let a = endpoints.iter().find(|e| e.path == "/a").unwrap();
        assert_eq!(
            a.requests, 2,
            "existing endpoint must keep incrementing at cap"
        );
    }

    // Regression: same freeze bug for the rate-limited client map.
    #[test]
    fn test_rate_limit_client_cap_does_not_freeze_existing() {
        let collector = MetricsCollector::from_config(&crate::AnalyticsConfig {
            max_rate_limit_clients: 1,
            ..crate::AnalyticsConfig::default()
        });

        collector.record_rate_limit(RateLimitEvent::limited("c1", 100, 100, 60));
        collector.record_rate_limit(RateLimitEvent::limited("c2", 100, 100, 60)); // rejected (cap)
        collector.record_rate_limit(RateLimitEvent::limited("c1", 100, 100, 60)); // must count

        let metrics = collector.rate_limit_metrics();
        assert_eq!(metrics.unique_clients_limited, 1);
        let c1 = metrics
            .top_limited_clients
            .iter()
            .find(|c| c.client_id == "c1")
            .unwrap();
        assert_eq!(
            c1.times_limited, 2,
            "existing client must keep counting at cap"
        );
    }

    // Regression: sub-millisecond latencies used to truncate to 0, zeroing the
    // min and dragging the average to 0 for fast endpoints.
    #[test]
    fn test_submillisecond_latency_not_truncated() {
        let collector = MetricsCollector::new();
        collector.record_request(RequestRecord::new(
            "GET",
            "/fast",
            200,
            Duration::from_micros(400), // 0.4 ms
        ));

        let latency = collector.latency_metrics();
        assert!(latency.avg_ms > 0.0, "avg latency must not truncate to 0");
        assert!(latency.min_ms > 0.0, "min latency must not truncate to 0");
        assert!((latency.avg_ms - 0.4).abs() < 0.05);
    }

    // Regression: avg_utilization used to report allowed/total, ignoring how
    // close each client actually was to its limit.
    #[test]
    fn test_avg_utilization_uses_event_utilization() {
        let collector = MetricsCollector::new();
        // Two events at 100% and 50% utilization -> mean 75%.
        collector.record_rate_limit(RateLimitEvent::limited("c1", 100, 100, 60)); // 100%
        collector.record_rate_limit(RateLimitEvent::allowed("c2", 50, 100, 60)); // 50%

        let metrics = collector.rate_limit_metrics();
        assert!(
            (metrics.avg_utilization - 75.0).abs() < 1e-6,
            "avg_utilization should be mean of per-event utilization, got {}",
            metrics.avg_utilization
        );
    }

    // Regression: requests_last_hour was requests_last_minute * 60. A single
    // request must therefore report 1, not 60.
    #[test]
    fn test_requests_last_hour_is_real_count() {
        let collector = MetricsCollector::new();
        collector.record_request(RequestRecord::new(
            "GET",
            "/x",
            200,
            Duration::from_millis(5),
        ));
        let throughput = collector.throughput_metrics();
        assert_eq!(throughput.requests_last_hour, 1);
        assert_eq!(throughput.requests_last_minute, 1);
    }

    // Config toggles must reach the collector: disabling endpoint metrics and
    // rate-limit tracking must suppress the corresponding data.
    #[test]
    fn test_config_toggles_honored() {
        let collector = MetricsCollector::from_config(&crate::AnalyticsConfig {
            enable_endpoint_metrics: false,
            enable_rate_limit_tracking: false,
            ..crate::AnalyticsConfig::default()
        });

        collector.record_request(RequestRecord::new(
            "GET",
            "/x",
            200,
            Duration::from_millis(5),
        ));
        collector.record_rate_limit(RateLimitEvent::limited("c1", 100, 100, 60));

        assert!(
            collector.endpoint_metrics().is_empty(),
            "endpoint metrics disabled -> no endpoints"
        );
        assert_eq!(
            collector.rate_limit_metrics().total_checks,
            0,
            "rate limit tracking disabled -> no checks recorded"
        );
        // Core request counters still work.
        assert_eq!(collector.request_metrics().total, 1);
    }

    // Regression (item 1): peak/current RPS must still be computed correctly
    // after switching from a per-request timestamp scan to incremental
    // per-second bucket counts. All requests recorded back-to-back land in the
    // same 1-second bucket; over the default 60s window that is count/60 rps,
    // and peak must equal the highest observed current rps.
    #[test]
    fn test_peak_rps_from_incremental_buckets() {
        let collector = MetricsCollector::new(); // 60s throughput window
        for _ in 0..120 {
            collector.record_request(RequestRecord::new(
                "GET",
                "/x",
                200,
                Duration::from_millis(1),
            ));
        }
        let t = collector.throughput_metrics();
        assert_eq!(t.requests_last_minute, 120);
        assert_eq!(t.requests_last_hour, 120);
        // 120 requests over a 60s window = 2.0 rps.
        assert!(
            (t.requests_per_second - 2.0).abs() < 1e-9,
            "current rps = {}",
            t.requests_per_second
        );
        assert!((t.peak_rps - 2.0).abs() < 1e-9, "peak rps = {}", t.peak_rps);
    }

    // Regression (item 2): the throughput structure must be count-bounded. A
    // per-request timestamp deque would hold one entry per request (10k here);
    // per-second bucketing collapses same-second requests into a single
    // `(second, count)` entry, so the deque stays tiny while totals are exact.
    #[test]
    fn test_request_buckets_are_count_bounded() {
        let collector = MetricsCollector::new();
        for _ in 0..10_000 {
            collector.record_request(RequestRecord::new(
                "GET",
                "/x",
                200,
                Duration::from_millis(1),
            ));
        }
        let buckets = collector.request_buckets.read();
        assert!(
            buckets.len() <= 2,
            "10k same-second requests must downsample to <=2 buckets, got {}",
            buckets.len()
        );
        let total: u64 = buckets.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 10_000, "bucket counts must preserve the exact total");
    }
}
