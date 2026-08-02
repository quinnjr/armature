//! Analytics middleware for automatic request tracking

use crate::{Analytics, ErrorRecord, RequestRecord};
use armature_core::{Error, HttpRequest, HttpResponse, Middleware, Next};
use async_trait::async_trait;
use std::time::Instant;

/// Middleware that automatically records analytics for all requests
///
/// # Example
///
/// ```rust,ignore
/// use armature_analytics::{Analytics, AnalyticsMiddleware, AnalyticsConfig};
/// use armature_core::Application;
///
/// let analytics = Analytics::new(AnalyticsConfig::default());
///
/// let app = Application::new(container, router)
///     .middleware(AnalyticsMiddleware::new(analytics.clone()));
/// ```
#[derive(Clone)]
pub struct AnalyticsMiddleware {
    analytics: Analytics,
}

impl AnalyticsMiddleware {
    /// Create a new analytics middleware
    pub fn new(analytics: Analytics) -> Self {
        Self { analytics }
    }

    /// Get a reference to the analytics instance
    pub fn analytics(&self) -> &Analytics {
        &self.analytics
    }
}

/// Request context for tracking within handlers
#[derive(Clone)]
pub struct AnalyticsContext {
    analytics: Analytics,
    start_time: Instant,
    method: String,
    path: String,
}

impl AnalyticsContext {
    /// Create a new analytics context
    pub fn new(analytics: Analytics, method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            analytics,
            start_time: Instant::now(),
            method: method.into(),
            path: path.into(),
        }
    }

    /// Get elapsed time since request start
    pub fn elapsed(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Complete the request tracking
    pub fn complete(self, status: u16, response_size: Option<u64>) {
        let record = RequestRecord::new(
            &self.method,
            normalize_path(&self.path),
            status,
            self.start_time.elapsed(),
        )
        .with_response_size(response_size.unwrap_or(0));

        self.analytics.record_request(record);
    }

    /// Record an error during request processing
    pub fn record_error(&self, error_type: &str, message: &str) {
        let record = ErrorRecord::new(error_type, message)
            .with_endpoint(format!("{} {}", self.method, self.path));

        self.analytics.record_error(record);
    }
}

/// Extract a client identifier from a request for per-client tracking.
///
/// Prefers, in order, the `x-client-id`, `x-forwarded-for` (first hop) and
/// `x-real-ip` headers. Returns `None` when no identifying header is present.
fn extract_client_id(req: &HttpRequest) -> Option<String> {
    if let Some(id) = req.headers.get("x-client-id") {
        return Some(id.to_owned());
    }
    if let Some(fwd) = req.headers.get("x-forwarded-for") {
        // The first entry is the originating client.
        if let Some(first) = fwd.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if let Some(ip) = req.headers.get("x-real-ip") {
        return Some(ip.to_owned());
    }
    None
}

#[async_trait]
impl Middleware for AnalyticsMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let config = self.analytics.config();

        // When analytics is disabled, act as a transparent pass-through.
        if !config.enabled {
            return next(req).await;
        }

        let method = req.method_str().to_owned();
        let raw_path = req.path.clone();

        // Exclusion and sampling gate what we record, but never what we return.
        // Evaluate them first, before any normalization or allocation, so that
        // excluded/unsampled requests skip the expensive path-normalization and
        // query/client-id work entirely.
        if config.should_exclude(&raw_path) || !config.should_sample() {
            return next(req).await;
        }

        // Build the recording path (before `req` is consumed by `next`).
        // Optionally fold query parameters into the tracked path.
        let mut tracked_path = normalize_path(&raw_path);
        if config.include_query_params && !req.query().is_empty() {
            let mut pairs: Vec<(&str, &str)> = req.query().iter().collect();
            pairs.sort_by(|a, b| a.0.cmp(b.0));
            let query: Vec<String> = pairs.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            tracked_path = format!("{}?{}", tracked_path, query.join("&"));
        }

        // Capture the client id only when client tracking is enabled.
        let client_id = if config.track_clients {
            extract_client_id(&req)
        } else {
            None
        };

        let start = Instant::now();
        let result = next(req).await;
        let duration = start.elapsed();

        match &result {
            Ok(response) => {
                let mut record =
                    RequestRecord::new(method, tracked_path, response.status, duration)
                        .with_response_size(response.body.len() as u64);
                if let Some(cid) = client_id {
                    record = record.with_client_id(cid);
                }
                self.analytics.record_request(record);
            }
            Err(err) => {
                // A middleware-level failure never produced a response; record
                // it as a 500 and capture the error for the error metrics.
                let mut record = RequestRecord::new(&method, tracked_path.clone(), 500, duration);
                if let Some(cid) = client_id {
                    record = record.with_client_id(cid);
                }
                self.analytics.record_request(record);
                self.analytics.record_error(
                    ErrorRecord::new("middleware_error", err.to_string())
                        .with_status(500)
                        .with_endpoint(format!("{} {}", method, tracked_path)),
                );
            }
        }

        result
    }
}

/// Helper to normalize request paths for aggregation
///
/// Converts paths like `/users/123/posts/456` to `/users/:id/posts/:id`
pub fn normalize_path(path: &str) -> String {
    // Write directly into a single pre-sized buffer, pushing either the
    // borrowed segment or the `:id` placeholder, instead of allocating an
    // intermediate `Vec<&str>`, a `Vec<String>` (one heap String per segment)
    // and a joined String.
    let mut out = String::with_capacity(path.len());
    for (i, segment) in path.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if segment.is_empty() {
            // Preserve empty segments (leading/trailing/double slashes).
        } else if is_likely_id(segment) {
            out.push_str(":id");
        } else {
            out.push_str(segment);
        }
    }
    out
}

/// Check if a path segment is likely an ID
fn is_likely_id(segment: &str) -> bool {
    // Check for UUID pattern
    if segment.len() == 36 && segment.chars().filter(|c| *c == '-').count() == 4 {
        return true;
    }

    // Check for numeric ID
    if segment.chars().all(|c| c.is_ascii_digit()) && !segment.is_empty() {
        return true;
    }

    // Check for hex IDs (like MongoDB ObjectId)
    if segment.len() == 24 && segment.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AnalyticsConfig;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/users/123/posts"), "/users/:id/posts");
        assert_eq!(normalize_path("/api/v1/users"), "/api/v1/users");
        assert_eq!(
            normalize_path("/users/550e8400-e29b-41d4-a716-446655440000"),
            "/users/:id"
        );
        assert_eq!(
            normalize_path("/items/507f1f77bcf86cd799439011"),
            "/items/:id"
        );
    }

    #[test]
    fn test_is_likely_id() {
        assert!(is_likely_id("123"));
        assert!(is_likely_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_likely_id("507f1f77bcf86cd799439011"));
        assert!(!is_likely_id("users"));
        assert!(!is_likely_id("api"));
    }

    #[test]
    fn test_analytics_context() {
        let analytics = Analytics::new(AnalyticsConfig::default());
        let ctx = AnalyticsContext::new(analytics.clone(), "GET", "/api/users");

        std::thread::sleep(std::time::Duration::from_millis(10));

        assert!(ctx.elapsed().as_millis() >= 10);
    }

    // Regression: AnalyticsMiddleware must implement armature_core::Middleware so
    // that a request flowing through the chain automatically records analytics.
    // Previously it implemented no trait and nothing was recorded unless the
    // caller manually invoked record_*.
    #[tokio::test]
    async fn test_middleware_records_automatically() {
        use std::future::Future;
        use std::pin::Pin;

        let analytics = Analytics::new(AnalyticsConfig::default());
        let mw = AnalyticsMiddleware::new(analytics.clone());

        let req = HttpRequest::new("GET", "/api/users/123".to_string());
        let next: Next = Box::new(|_req: HttpRequest| {
            Box::pin(async { Ok(HttpResponse::ok().with_body(b"hello".to_vec())) })
                as Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        });

        let resp = mw.handle(req, next).await.unwrap();
        assert_eq!(resp.status, 200);

        let snapshot = analytics.snapshot();
        assert_eq!(
            snapshot.requests.total, 1,
            "middleware must record the request"
        );
        assert_eq!(snapshot.requests.success, 1);
        // Path must be normalized for aggregation.
        assert_eq!(snapshot.endpoints.len(), 1);
        assert_eq!(snapshot.endpoints[0].path, "/api/users/:id");
        // Response size captured from the body.
        assert_eq!(snapshot.throughput.total_bytes_transferred, 5);
    }

    #[tokio::test]
    async fn test_middleware_respects_disabled_and_exclusions() {
        use std::future::Future;
        use std::pin::Pin;

        // Disabled: nothing recorded, response still flows.
        let analytics = Analytics::new(AnalyticsConfig::builder().enabled(false).build());
        let mw = AnalyticsMiddleware::new(analytics.clone());
        let req = HttpRequest::new("GET", "/api/x".to_string());
        let next: Next = Box::new(|_req: HttpRequest| {
            Box::pin(async { Ok(HttpResponse::ok()) })
                as Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        });
        let resp = mw.handle(req, next).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(analytics.snapshot().requests.total, 0);

        // Excluded path: not recorded.
        let analytics = Analytics::new(AnalyticsConfig::default());
        let mw = AnalyticsMiddleware::new(analytics.clone());
        let req = HttpRequest::new("GET", "/health".to_string());
        let next: Next = Box::new(|_req: HttpRequest| {
            Box::pin(async { Ok(HttpResponse::ok()) })
                as Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        });
        mw.handle(req, next).await.unwrap();
        assert_eq!(analytics.snapshot().requests.total, 0);
    }
}
