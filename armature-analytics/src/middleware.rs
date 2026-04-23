//! Analytics middleware for automatic request tracking

use crate::{Analytics, ErrorRecord, RequestRecord};
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

    /// Record a request manually (for custom middleware implementations)
    pub fn record_request(
        &self,
        method: &str,
        path: &str,
        status: u16,
        start_time: Instant,
        response_size: Option<u64>,
        authenticated: bool,
    ) {
        // Check if path should be excluded
        if self.analytics.config().should_exclude(path) {
            return;
        }

        // Check sampling
        if !self.analytics.config().should_sample() {
            return;
        }

        let duration = start_time.elapsed();

        let mut record =
            RequestRecord::new(method, path, status, duration).with_authenticated(authenticated);

        if let Some(size) = response_size {
            record = record.with_response_size(size);
        }

        self.analytics.record_request(record);
    }

    /// Record an error manually
    pub fn record_error(
        &self,
        error_type: &str,
        message: &str,
        status: Option<u16>,
        endpoint: Option<&str>,
    ) {
        let mut record = ErrorRecord::new(error_type, message);

        if let Some(s) = status {
            record = record.with_status(s);
        }

        if let Some(ep) = endpoint {
            record = record.with_endpoint(ep);
        }

        self.analytics.record_error(record);
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
        let record =
            RequestRecord::new(&self.method, &self.path, status, self.start_time.elapsed())
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

/// Handler wrapper that automatically tracks analytics
#[allow(dead_code)]
pub struct TrackedHandler<F> {
    handler: F,
    analytics: Analytics,
}

impl<F> TrackedHandler<F> {
    pub fn new(handler: F, analytics: Analytics) -> Self {
        Self { handler, analytics }
    }
}

/// Extension trait for adding analytics to requests
pub trait AnalyticsExt {
    /// Start tracking analytics for this request
    fn start_analytics(&self, analytics: &Analytics) -> AnalyticsContext;
}

/// Helper to normalize request paths for aggregation
///
/// Converts paths like `/users/123/posts/456` to `/users/:id/posts/:id`
pub fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let normalized: Vec<String> = segments
        .into_iter()
        .map(|segment| {
            // Check if segment looks like an ID
            if segment.is_empty() {
                String::new()
            } else if is_likely_id(segment) {
                ":id".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect();

    normalized.join("/")
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
}
