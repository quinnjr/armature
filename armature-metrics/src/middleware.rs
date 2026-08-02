//! Request metrics middleware
//!
//! Automatically collect HTTP request metrics.

use armature_core::{Error, HttpRequest, HttpResponse, Middleware};
use once_cell::sync::Lazy;
use prometheus::{CounterVec, GaugeVec, HistogramVec};
use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::RwLock;
use std::time::Instant;

/// Default upper bound on the number of distinct `path` label values a single
/// middleware instance will emit when path labelling is enabled. Any path
/// beyond this cap is folded into [`OTHER_PATH_LABEL`] so the Prometheus series
/// count for the path-labelled metric families cannot grow without bound.
const DEFAULT_PATH_CARDINALITY_CAP: usize = 1000;

/// Label value used for paths that exceed the per-instance cardinality cap.
const OTHER_PATH_LABEL: &str = "<other>";

/// HTTP request metrics
static HTTP_REQUEST_COUNTER: Lazy<CounterVec> = Lazy::new(|| {
    crate::register_counter_vec(
        "http_requests_total",
        "Total number of HTTP requests",
        &["method", "path", "status"],
    )
    .expect("Failed to register http_requests_total")
});

static HTTP_REQUEST_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    crate::register_histogram_vec_with_buckets(
        "http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["method", "path", "status"],
        vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
        ],
    )
    .expect("Failed to register http_request_duration_seconds")
});

static HTTP_REQUESTS_IN_FLIGHT: Lazy<GaugeVec> = Lazy::new(|| {
    crate::register_gauge_vec(
        "http_requests_in_flight",
        "Number of HTTP requests currently being processed",
        &["method", "path"],
    )
    .expect("Failed to register http_requests_in_flight")
});

static HTTP_REQUEST_SIZE_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    crate::register_histogram_vec_with_buckets(
        "http_request_size_bytes",
        "HTTP request size in bytes",
        &["method", "path"],
        vec![
            100.0,
            1_000.0,
            10_000.0,
            100_000.0,
            1_000_000.0,
            10_000_000.0,
        ],
    )
    .expect("Failed to register http_request_size_bytes")
});

static HTTP_RESPONSE_SIZE_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    crate::register_histogram_vec_with_buckets(
        "http_response_size_bytes",
        "HTTP response size in bytes",
        &["method", "path", "status"],
        vec![
            100.0,
            1_000.0,
            10_000.0,
            100_000.0,
            1_000_000.0,
            10_000_000.0,
        ],
    )
    .expect("Failed to register http_response_size_bytes")
});

/// Request metrics middleware
///
/// Automatically collects the following metrics:
/// - `http_requests_total` - Total number of requests
/// - `http_request_duration_seconds` - Request duration histogram
/// - `http_requests_in_flight` - Active requests gauge
/// - `http_request_size_bytes` - Request size histogram
/// - `http_response_size_bytes` - Response size histogram
///
/// # Cardinality safety
///
/// Prometheus creates a permanent time series for every distinct combination of
/// label values. Because the middleware only sees the concrete request path
/// (not the matched route template), labelling metrics with the raw path lets a
/// remote client mint unbounded series simply by requesting random URLs
/// (`/{random}`), which floods the global registry and can exhaust memory.
///
/// To make this safe:
/// - Path labelling is **disabled by default** ([`RequestMetricsMiddleware::new`]
///   sets `include_path = false`). Opt in explicitly with
///   [`RequestMetricsMiddleware::with_path`].
/// - When path labelling *is* enabled, the number of distinct path label values
///   emitted by an instance is bounded (default
///   [`DEFAULT_PATH_CARDINALITY_CAP`]). Once the cap is reached, every new path
///   is reported as [`OTHER_PATH_LABEL`] (`"<other>"`) so the series count is
///   capped at roughly `cap + 1` regardless of how many unique paths are seen.
///
/// # Examples
///
/// ```no_run
/// use armature_core::*;
/// use armature_metrics::*;
/// use std::sync::Arc;
///
/// // Safe default: no per-path cardinality.
/// let middleware = Arc::new(RequestMetricsMiddleware::new());
///
/// // Opt in to bounded per-path labels.
/// let with_paths = Arc::new(RequestMetricsMiddleware::with_path());
/// ```
pub struct RequestMetricsMiddleware {
    /// Whether to include path in metrics (can lead to high cardinality).
    ///
    /// Defaults to `false`; enable with [`RequestMetricsMiddleware::with_path`].
    include_path: bool,

    /// Maximum number of distinct `path` label values to emit before folding
    /// further paths into [`OTHER_PATH_LABEL`].
    max_path_cardinality: usize,

    /// Bounded set of path label values already emitted by this instance. Used
    /// to decide whether a newly seen path can get its own label or must be
    /// folded into `<other>`.
    seen_paths: RwLock<HashSet<String>>,
}

impl RequestMetricsMiddleware {
    /// Create new request metrics middleware.
    ///
    /// Path labelling is disabled by default to avoid unbounded Prometheus label
    /// cardinality from attacker-controlled URLs. Use
    /// [`RequestMetricsMiddleware::with_path`] to opt in to bounded per-path
    /// labels.
    pub fn new() -> Self {
        Self {
            include_path: false,
            max_path_cardinality: DEFAULT_PATH_CARDINALITY_CAP,
            seen_paths: RwLock::new(HashSet::new()),
        }
    }

    /// Create middleware with bounded per-path labels enabled.
    ///
    /// Distinct path label values are capped at [`DEFAULT_PATH_CARDINALITY_CAP`];
    /// paths beyond the cap are reported as [`OTHER_PATH_LABEL`].
    pub fn with_path() -> Self {
        Self {
            include_path: true,
            ..Self::new()
        }
    }

    /// Create middleware with bounded per-path labels and a custom cardinality
    /// cap.
    ///
    /// A cap of `0` is treated as `1` to guarantee at least the `<other>` bucket
    /// behaves sensibly.
    pub fn with_path_cardinality(max_path_cardinality: usize) -> Self {
        Self {
            include_path: true,
            max_path_cardinality: max_path_cardinality.max(1),
            ..Self::new()
        }
    }

    /// Create middleware without path labels (to reduce cardinality).
    ///
    /// Equivalent to [`RequestMetricsMiddleware::new`]; retained for clarity and
    /// backwards compatibility.
    pub fn without_path() -> Self {
        Self::new()
    }

    /// Get sanitized, cardinality-bounded path label for metrics.
    ///
    /// Returns `"/"` when path labelling is disabled. Otherwise the path is
    /// length-truncated and then bounded: known paths keep their own label,
    /// newly seen paths are admitted until [`Self::max_path_cardinality`] is
    /// reached, after which they are folded into [`OTHER_PATH_LABEL`].
    ///
    /// Returns `Cow<'static, str>` rather than `String` so the constant-label
    /// branches (path labelling disabled, `<other>` overflow/poisoned-lock
    /// fallback) borrow a `'static` string instead of allocating a fresh copy
    /// on every request; only genuinely new, dynamic paths incur an
    /// allocation.
    fn sanitize_path(&self, path: &str) -> Cow<'static, str> {
        if !self.include_path {
            return Cow::Borrowed("/");
        }

        // Limit raw path length first (defence in depth against huge labels).
        // Truncate on a UTF-8 char boundary: slicing `&path[..97]` directly
        // panics when byte 97 lands in the middle of a multibyte codepoint, so
        // an attacker-controlled multibyte path near the limit could crash the
        // request path.
        let candidate = if path.len() > 100 {
            let mut end = 97;
            while end > 0 && !path.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &path[..end])
        } else {
            path.to_string()
        };

        // Fast path: an already-seen path (the overwhelmingly common case) only
        // needs a shared read lock, so concurrent requests don't serialize on a
        // mutex. Poisoned lock -> bounded label so a panic elsewhere can't turn
        // into unbounded cardinality.
        {
            let seen = match self.seen_paths.read() {
                Ok(guard) => guard,
                Err(_) => return Cow::Borrowed(OTHER_PATH_LABEL),
            };
            if seen.contains(&candidate) {
                return Cow::Owned(candidate);
            }
        }

        // New path: upgrade to a write lock to (maybe) admit it under the cap.
        let mut seen = match self.seen_paths.write() {
            Ok(guard) => guard,
            Err(_) => return Cow::Borrowed(OTHER_PATH_LABEL),
        };

        // Re-check under the write lock: another thread may have admitted this
        // same path between dropping the read lock and taking the write lock.
        if seen.contains(&candidate) {
            return Cow::Owned(candidate);
        }

        if seen.len() < self.max_path_cardinality {
            seen.insert(candidate.clone());
            Cow::Owned(candidate)
        } else {
            Cow::Borrowed(OTHER_PATH_LABEL)
        }
    }
}

impl Default for RequestMetricsMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Middleware for RequestMetricsMiddleware {
    async fn handle(
        &self,
        request: HttpRequest,
        next: armature_core::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        let start = Instant::now();
        let method = request.method.clone();
        let path = self.sanitize_path(&request.path);
        let request_size = request.body.len() as f64;

        // Record request size
        HTTP_REQUEST_SIZE_BYTES
            .with_label_values(&[method.as_str(), path.as_ref()])
            .observe(request_size);

        // Increment in-flight requests
        HTTP_REQUESTS_IN_FLIGHT
            .with_label_values(&[method.as_str(), path.as_ref()])
            .inc();

        // Process request
        let result = next(request).await;

        // Decrement in-flight requests
        HTTP_REQUESTS_IN_FLIGHT
            .with_label_values(&[method.as_str(), path.as_ref()])
            .dec();

        // Record metrics
        let duration = start.elapsed().as_secs_f64();

        match &result {
            Ok(response) => {
                let status = response.status.to_string();
                let response_size = response.body.len() as f64;

                // Record request count
                HTTP_REQUEST_COUNTER
                    .with_label_values(&[method.as_str(), path.as_ref(), status.as_str()])
                    .inc();

                // Record request duration
                HTTP_REQUEST_DURATION
                    .with_label_values(&[method.as_str(), path.as_ref(), status.as_str()])
                    .observe(duration);

                // Record response size
                HTTP_RESPONSE_SIZE_BYTES
                    .with_label_values(&[method.as_str(), path.as_ref(), status.as_str()])
                    .observe(response_size);
            }
            Err(err) => {
                // Record error
                let status = err.status_code().to_string();

                HTTP_REQUEST_COUNTER
                    .with_label_values(&[method.as_str(), path.as_ref(), status.as_str()])
                    .inc();

                HTTP_REQUEST_DURATION
                    .with_label_values(&[method.as_str(), path.as_ref(), status.as_str()])
                    .observe(duration);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_metrics_middleware_new_defaults_to_no_path() {
        // Safe default: no per-path cardinality unless explicitly opted in.
        let middleware = RequestMetricsMiddleware::new();
        assert!(!middleware.include_path);
    }

    #[test]
    fn test_default_does_not_label_per_path() {
        // With the default config every path collapses to a single "/" label,
        // so the registry cannot accumulate one series per unique path.
        let middleware = RequestMetricsMiddleware::default();
        assert_eq!(middleware.sanitize_path("/api/users"), "/");
        assert_eq!(middleware.sanitize_path("/random-9f83a"), "/");
        assert_eq!(middleware.sanitize_path("/another/path"), "/");
    }

    #[test]
    fn test_with_path_enables_path_labels() {
        let middleware = RequestMetricsMiddleware::with_path();
        assert!(middleware.include_path);
        assert_eq!(middleware.sanitize_path("/api/users"), "/api/users");
    }

    #[test]
    fn test_request_metrics_middleware_without_path() {
        let middleware = RequestMetricsMiddleware::without_path();
        assert!(!middleware.include_path);
        assert_eq!(middleware.sanitize_path("/api/users"), "/");
    }

    #[test]
    fn test_sanitize_path_truncates_long_paths() {
        let middleware = RequestMetricsMiddleware::with_path();
        assert_eq!(middleware.sanitize_path("/api/users"), "/api/users");

        let long_path = "/".to_string() + &"a".repeat(150);
        let sanitized = middleware.sanitize_path(&long_path);
        assert!(sanitized.len() <= 103); // 100 + "..."
    }

    #[test]
    fn test_path_cardinality_is_capped() {
        // Enable path labels with a tiny cap, then feed many unique paths.
        let cap = 4;
        let middleware = RequestMetricsMiddleware::with_path_cardinality(cap);

        let mut labels = std::collections::HashSet::new();
        for i in 0..1000 {
            labels.insert(middleware.sanitize_path(&format!("/item/{i}")));
        }

        // Distinct labels must never exceed cap + 1 (the "<other>" bucket),
        // regardless of how many unique paths were observed.
        assert!(
            labels.len() <= cap + 1,
            "distinct labels {} exceeded cap+1 ({})",
            labels.len(),
            cap + 1
        );
        // Overflow paths are folded into the shared "<other>" bucket.
        assert!(labels.contains(OTHER_PATH_LABEL));
    }

    #[tokio::test]
    async fn test_handle_records_counter_duration_and_in_flight() {
        use armature_core::HttpResponse;

        // Distinct method/path labels so this test's series don't collide with
        // any other test hitting the shared static metric families.
        let method = "QZHANDLETEST";
        let path = "/handle-metrics-regression";
        let middleware = RequestMetricsMiddleware::with_path();

        let mut request = HttpRequest::new(method.to_string(), path.to_string());
        request.body = armature_core::Bytes::from_static(b"hello"); // 5-byte request body

        let count_before = HTTP_REQUEST_COUNTER
            .with_label_values(&[method, path, "200"])
            .get();
        let dur_before = HTTP_REQUEST_DURATION
            .with_label_values(&[method, path, "200"])
            .get_sample_count();
        let size_before = HTTP_REQUEST_SIZE_BYTES
            .with_label_values(&[method, path])
            .get_sample_count();

        let next: armature_core::middleware::Next = Box::new(|_req| {
            Box::pin(async move { Ok(HttpResponse::ok().with_body(b"response-body".to_vec())) })
        });

        let response = middleware.handle(request, next).await.unwrap();
        assert_eq!(response.status, 200);

        // Request counter incremented exactly once for this label set.
        let count_after = HTTP_REQUEST_COUNTER
            .with_label_values(&[method, path, "200"])
            .get();
        assert_eq!(count_after - count_before, 1.0);

        // Duration histogram recorded one sample.
        let dur_after = HTTP_REQUEST_DURATION
            .with_label_values(&[method, path, "200"])
            .get_sample_count();
        assert_eq!(dur_after - dur_before, 1);

        // Request-size histogram recorded the body.
        let size_after = HTTP_REQUEST_SIZE_BYTES
            .with_label_values(&[method, path])
            .get_sample_count();
        assert_eq!(size_after - size_before, 1);

        // In-flight gauge is balanced back to zero after the request completes.
        let in_flight = HTTP_REQUESTS_IN_FLIGHT
            .with_label_values(&[method, path])
            .get();
        assert_eq!(in_flight, 0.0);
    }

    #[test]
    fn test_sanitize_path_multibyte_near_limit_does_not_panic() {
        let middleware = RequestMetricsMiddleware::with_path();
        // 40 x 4-byte emoji = 160 bytes; byte index 97 falls in the middle of a
        // codepoint, so the old `&path[..97]` slice panicked outright.
        let path = "\u{1F389}".repeat(40);
        let sanitized = middleware.sanitize_path(&path);
        assert!(sanitized.ends_with("..."));
        // Truncation stays within the byte budget and never splits a codepoint.
        assert!(sanitized.len() <= 100);
        assert!(std::str::from_utf8(sanitized.as_bytes()).is_ok());
    }

    #[test]
    fn test_seen_paths_get_stable_labels() {
        let middleware = RequestMetricsMiddleware::with_path_cardinality(2);
        // First two distinct paths are admitted with their own labels.
        assert_eq!(middleware.sanitize_path("/a"), "/a");
        assert_eq!(middleware.sanitize_path("/b"), "/b");
        // A repeat of an admitted path keeps its own label.
        assert_eq!(middleware.sanitize_path("/a"), "/a");
        // A new path beyond the cap folds into "<other>".
        assert_eq!(middleware.sanitize_path("/c"), OTHER_PATH_LABEL);
    }
}
