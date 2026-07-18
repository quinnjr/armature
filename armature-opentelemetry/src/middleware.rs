//! OpenTelemetry middleware for automatic instrumentation

use armature_core::{Error, HttpRequest, HttpResponse, Middleware, Next};
use async_trait::async_trait;
use opentelemetry::{
    Context as OtelContext, KeyValue, global,
    trace::{SpanKind, TraceContextExt, Tracer},
};
use std::sync::Arc;
use std::time::Instant;

/// OpenTelemetry middleware for automatic tracing and metrics
pub struct TelemetryMiddleware {
    service_name: String,
    metrics: Option<Arc<crate::metrics::HttpMetrics>>,
}

impl TelemetryMiddleware {
    /// Create a new telemetry middleware
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            metrics: None,
        }
    }

    /// Create with metrics collection
    pub fn with_metrics(mut self, metrics: Arc<crate::metrics::HttpMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Extract span attributes from request
    fn extract_attributes(&self, req: &HttpRequest) -> Vec<KeyValue> {
        let mut attributes = vec![
            KeyValue::new("http.method", req.method.to_string()),
            KeyValue::new("http.target", req.path.clone()),
            KeyValue::new("http.scheme", "http"),
        ];

        // Add host header if present
        if let Some(host) = req.headers.get("host") {
            attributes.push(KeyValue::new("http.host", host.clone()));
        }

        // Add user agent if present
        if let Some(ua) = req.headers.get("user-agent") {
            attributes.push(KeyValue::new("http.user_agent", ua.clone()));
        }

        attributes
    }

    /// Extract response attributes
    fn extract_response_attributes(&self, res: &HttpResponse) -> Vec<KeyValue> {
        vec![
            KeyValue::new("http.status_code", res.status.to_string()),
            KeyValue::new("http.response_content_length", res.body.len() as i64),
        ]
    }
}

#[async_trait]
impl Middleware for TelemetryMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let start_time = Instant::now();

        // Increment active requests
        if let Some(ref metrics) = self.metrics {
            metrics.increment_active();
        }

        // Extract span context from headers (for distributed tracing)
        let parent_context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(&req.headers))
        });

        // Create a span for this request
        let service_name = self.service_name.clone();
        let tracer = global::tracer(service_name);
        let request_attrs = self.extract_attributes(&req);
        let span = tracer
            .span_builder(format!("{} {}", req.method, req.path))
            .with_kind(SpanKind::Server)
            .with_attributes(request_attrs)
            .start_with_context(&tracer, &parent_context);

        let cx = OtelContext::current_with_span(span);

        // Execute request with tracing context
        let response_result = next(req).await;

        // Add response attributes to span
        match &response_result {
            Ok(res) => {
                let span = cx.span();
                for attr in self.extract_response_attributes(res) {
                    span.set_attribute(attr);
                }

                // Set span status based on HTTP status
                if res.status >= 400 {
                    span.set_status(opentelemetry::trace::Status::error(format!(
                        "HTTP {}",
                        res.status
                    )));
                } else {
                    span.set_status(opentelemetry::trace::Status::Ok);
                }
            }
            Err(e) => {
                // Record error
                let span = cx.span();
                span.set_status(opentelemetry::trace::Status::error(e.to_string()));
                span.record_error(e);
            }
        }

        let result = response_result;

        // Record metrics
        let duration = start_time.elapsed().as_secs_f64();

        if let Some(ref metrics) = self.metrics {
            metrics.decrement_active();

            if let Ok(ref res) = result {
                metrics.record_request(
                    result.as_ref().ok().map(|_| "method").unwrap_or("UNKNOWN"),
                    "path",
                    res.status,
                    duration,
                );
            }
        }

        result
    }
}

/// Helper to extract trace context from HTTP headers
struct HeaderExtractor<'a>(&'a armature_core::headers::HeaderMap);

impl<'a> opentelemetry::propagation::Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|k| k.as_str()).collect()
    }
}

/// Helper to inject trace context into HTTP headers
pub struct HeaderInjector<'a>(pub &'a mut std::collections::HashMap<String, String>);

impl<'a> opentelemetry::propagation::Injector for HeaderInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
}

/// Create a span for a function
#[macro_export]
macro_rules! trace_span {
    ($name:expr) => {{
        use opentelemetry::trace::Tracer;
        let tracer = opentelemetry::global::tracer("");
        tracer.start($name)
    }};
    ($name:expr, $($key:expr => $value:expr),*) => {{
        use opentelemetry::trace::Tracer;
        use opentelemetry::KeyValue;
        let tracer = opentelemetry::global::tracer("");
        let mut span = tracer.start($name);
        $(
            span.set_attribute(KeyValue::new($key, $value));
        )*
        span
    }};
}

/// Add an attribute to the current span
#[macro_export]
macro_rules! span_attribute {
    ($key:expr, $value:expr) => {{
        use opentelemetry::{Context, KeyValue, trace::TraceContextExt};
        let cx = Context::current();
        let span = cx.span();
        span.set_attribute(KeyValue::new($key, $value));
    }};
}

/// Record an event in the current span
#[macro_export]
macro_rules! span_event {
    ($name:expr) => {{
        use opentelemetry::{trace::TraceContextExt, Context};
        let cx = Context::current();
        let span = cx.span();
        span.add_event($name, vec![]);
    }};
    ($name:expr, $($key:expr => $value:expr),*) => {{
        use opentelemetry::{trace::TraceContextExt, Context, KeyValue};
        let cx = Context::current();
        let span = cx.span();
        let attributes = vec![
            $(KeyValue::new($key, $value)),*
        ];
        span.add_event($name, attributes);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::propagation::{Extractor, Injector};

    #[test]
    fn test_telemetry_middleware_creation() {
        let middleware = TelemetryMiddleware::new("test-service");
        assert_eq!(middleware.service_name, "test-service");
        assert!(middleware.metrics.is_none());
    }

    #[test]
    fn test_header_extractor() {
        let mut headers = armature_core::headers::HeaderMap::new();
        headers.insert("traceparent".to_string(), "00-123-456-01".to_string());

        let extractor = HeaderExtractor(&headers);
        assert_eq!(extractor.get("traceparent"), Some("00-123-456-01"));
        assert_eq!(extractor.get("non-existent"), None);

        let keys = extractor.keys();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "traceparent");
    }

    #[test]
    fn test_header_injector() {
        let mut headers = std::collections::HashMap::new();
        let mut injector = HeaderInjector(&mut headers);

        injector.set("traceparent", "00-789-012-01".to_string());

        assert_eq!(
            headers.get("traceparent"),
            Some(&"00-789-012-01".to_string())
        );
    }

    #[test]
    fn test_header_extractor_multiple_keys() {
        let mut headers = armature_core::headers::HeaderMap::new();
        headers.insert("key1".to_string(), "value1".to_string());
        headers.insert("key2".to_string(), "value2".to_string());

        let extractor = HeaderExtractor(&headers);
        assert_eq!(extractor.keys().len(), 2);
    }

    #[test]
    fn test_header_injector_multiple_sets() {
        let mut headers = std::collections::HashMap::new();
        let mut injector = HeaderInjector(&mut headers);

        injector.set("key1", "value1".to_string());
        injector.set("key2", "value2".to_string());

        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("key1"), Some(&"value1".to_string()));
        assert_eq!(headers.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_header_injector_overwrite() {
        let mut headers = std::collections::HashMap::new();
        let mut injector = HeaderInjector(&mut headers);

        injector.set("key", "value1".to_string());
        injector.set("key", "value2".to_string());

        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("key"), Some(&"value2".to_string()));
    }
}
