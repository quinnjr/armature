//! OpenTelemetry middleware for automatic instrumentation

use armature_core::{Error, HttpRequest, HttpResponse, Middleware, Next};
use async_trait::async_trait;
use opentelemetry::{
    Context as OtelContext, KeyValue, global,
    trace::{FutureExt, SpanKind, TraceContextExt, Tracer},
};
use std::sync::Arc;
use std::time::Instant;

/// OpenTelemetry middleware for automatic tracing and metrics
pub struct TelemetryMiddleware {
    service_name: Arc<str>,
    /// Tracer resolved once at construction and reused for every request.
    ///
    /// Previously each request cloned the service name and called
    /// `global::tracer(...)` — a provider lookup plus a boxed-tracer allocation
    /// on the hot path. Resolving it once here removes both from per-request
    /// handling.
    tracer: global::BoxedTracer,
    metrics: Option<Arc<crate::metrics::HttpMetrics>>,
}

impl TelemetryMiddleware {
    /// Create a new telemetry middleware
    pub fn new(service_name: impl Into<String>) -> Self {
        let service_name: Arc<str> = service_name.into().into();
        let tracer = global::tracer(service_name.to_string());
        Self {
            service_name,
            tracer,
            metrics: None,
        }
    }

    /// The service name this middleware was constructed with.
    pub fn service_name(&self) -> &str {
        &self.service_name
    }

    /// Create with metrics collection
    pub fn with_metrics(mut self, metrics: Arc<crate::metrics::HttpMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Extract span attributes from request.
    ///
    /// Names follow OTel semantic conventions 1.0 and later. The pre-1.0
    /// spellings (`http.method`, `http.target`, `http.host`,
    /// `http.user_agent`, `http.scheme`) were retired when the HTTP
    /// conventions stabilized, and collectors/backends key their HTTP
    /// dashboards off the current names.
    fn extract_attributes(&self, req: &HttpRequest) -> Vec<KeyValue> {
        let mut attributes = vec![
            KeyValue::new("http.request.method", req.method_str().to_owned()),
            // `url.path` is the path component only; the query belongs in
            // `url.query`, which is omitted here because it is unbounded and
            // routinely carries credentials.
            KeyValue::new("url.path", req.path_only().to_owned()),
            KeyValue::new("url.scheme", "http"),
        ];

        // Add host header if present
        if let Some(host) = req.headers.get("host") {
            attributes.push(KeyValue::new("server.address", host.to_owned()));
        }

        // Add user agent if present
        if let Some(ua) = req.headers.get("user-agent") {
            attributes.push(KeyValue::new("user_agent.original", ua.to_owned()));
        }

        attributes
    }

    /// Extract response attributes
    fn extract_response_attributes(&self, res: &HttpResponse) -> Vec<KeyValue> {
        vec![
            KeyValue::new("http.response.status_code", res.status as i64),
            KeyValue::new("http.response.body.size", res.body.len() as i64),
        ]
    }

    /// Derive the `(method, route, status)` triple recorded for a completed
    /// request.
    ///
    /// Both dimensions come from the real request (previously the middleware
    /// recorded the literal strings `"method"`/`"path"`), and a value is
    /// produced for the error path too (previously errors were never counted)
    /// — a failed request is recorded with HTTP status 500.
    fn recording(
        method: &str,
        route: &str,
        result: &Result<HttpResponse, Error>,
    ) -> (String, String, u16) {
        let status = match result {
            Ok(res) => res.status,
            Err(_) => 500,
        };
        (method.to_string(), route.to_string(), status)
    }
}

#[async_trait]
impl Middleware for TelemetryMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let start_time = Instant::now();

        // Capture the real request dimensions before `req` is consumed by `next`.
        let method = req.method_str().to_owned();
        // `http.route` is defined by OTel as the low-cardinality route
        // template. The matched template is not available to a middleware here,
        // so use the query-less path: still higher cardinality than a template,
        // but bounded by the URL space the app actually serves rather than by
        // every distinct query string a client cares to send.
        let route = req.path_only().to_owned();

        // Increment active requests
        if let Some(ref metrics) = self.metrics {
            metrics.increment_active();
        }

        // Extract span context from headers (for distributed tracing)
        let parent_context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(&req.headers))
        });

        // Create a span for this request using the cached tracer.
        let request_attrs = self.extract_attributes(&req);
        let span = self
            .tracer
            // Span names are a grouping dimension in every tracing backend, so
            // the query string stays out of them for the same reason it stays
            // out of `http.route`.
            .span_builder(format!("{} {}", method, route))
            .with_kind(SpanKind::Server)
            .with_attributes(request_attrs)
            .start_with_context(&self.tracer, &parent_context);

        let cx = OtelContext::current_with_span(span);

        // Execute request with the request's tracing context attached as the
        // ACTIVE context. Without this, `next(req)` runs under whatever
        // context (if any) happened to be active on the polling task, so any
        // span application code starts during request handling — or an
        // outbound HTTP client injecting a `traceparent` for downstream
        // propagation — would not nest under this request's span; it would
        // start a new, disconnected trace instead.
        let response_result = next(req).with_context(cx.clone()).await;

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

            // Record every request — success *and* error — with the real
            // request method and route as dimensions.
            let (method, route, status) = Self::recording(&method, &route, &result);
            metrics.record_request(&method, &route, status, duration);
        }

        result
    }
}

/// Helper to extract trace context from HTTP headers
struct HeaderExtractor<'a>(&'a armature_core::headers::HeaderMap);

impl<'a> opentelemetry::propagation::Extractor for HeaderExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().collect()
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
        assert_eq!(middleware.service_name(), "test-service");
        assert!(middleware.metrics.is_none());
    }

    #[test]
    fn recording_uses_real_request_dimensions_not_literals() {
        // Regression: the middleware used to record the literal strings
        // "method"/"path" regardless of the actual request.
        let res = Ok(HttpResponse::new(201));
        let (method, route, status) = TelemetryMiddleware::recording("GET", "/users/42", &res);

        assert_eq!(method, "GET");
        assert_eq!(route, "/users/42");
        assert_eq!(status, 201);
        assert_ne!(method, "method");
        assert_ne!(route, "path");
    }

    #[test]
    fn recording_counts_error_path_requests() {
        // Regression: recording used to live only inside the `Ok` branch, so
        // failed requests were never counted. Errors are recorded as HTTP 500.
        let res: Result<HttpResponse, Error> = Err(Error::internal("boom"));
        let (method, route, status) = TelemetryMiddleware::recording("POST", "/checkout", &res);

        assert_eq!(method, "POST");
        assert_eq!(route, "/checkout");
        assert_eq!(status, 500);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn handle_records_real_request_dimensions_end_to_end() {
        // Regression (end-to-end): the original bug lived at the
        // `record_request` call site inside `handle()`, which hardcoded the
        // literal strings "method"/"path" as the metric dimensions. The
        // `recording` helper tests only exercise a passthrough echo, so they
        // would stay green even if the call site were reverted to literals.
        //
        // This drives a REAL request through `handle()` with an in-memory
        // OpenTelemetry metrics pipeline (no global state, fully deterministic)
        // and asserts the exported data point carries the real
        // `http.method`/`http.route`, guarding the actual call site.
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::InMemoryMetricExporter;
        use opentelemetry_sdk::metrics::SdkMeterProvider;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
        use std::collections::HashMap;

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("test");
        let metrics = Arc::new(crate::metrics::HttpMetrics::new(&meter).unwrap());

        let middleware = TelemetryMiddleware::new("test-service").with_metrics(metrics);

        // A `Next` that returns HTTP 201, so we also confirm the real status is
        // propagated alongside the real method/route.
        let req = HttpRequest::new("GET", "/users/42".to_string());
        let next: Next = Box::new(|_req| Box::pin(async { Ok(HttpResponse::new(201)) }));
        let res = middleware.handle(req, next).await.unwrap();
        assert_eq!(res.status, 201);

        provider.force_flush().unwrap();

        // Locate the request-count counter and collect its single data point's
        // attributes into a key -> value map.
        let resource_metrics = exporter.get_finished_metrics().unwrap();
        let mut attrs: Option<HashMap<String, String>> = None;
        for rm in &resource_metrics {
            for sm in rm.scope_metrics() {
                for metric in sm.metrics() {
                    if metric.name() != "http.server.request.count" {
                        continue;
                    }
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data()
                        && let Some(dp) = sum.data_points().next()
                    {
                        attrs = Some(
                            dp.attributes()
                                .map(|kv| {
                                    (kv.key.as_str().to_string(), kv.value.as_str().to_string())
                                })
                                .collect(),
                        );
                    }
                }
            }
        }

        let attrs = attrs.expect("request count metric with a data point must be recorded");

        // The recorded dimensions must be the REAL request values, never the
        // old hardcoded literals "method"/"path". Attribute names are the
        // stable OTel HTTP semantic conventions, not the retired pre-1.0 ones.
        assert_eq!(
            attrs.get("http.request.method").map(String::as_str),
            Some("GET")
        );
        assert_eq!(
            attrs.get("http.route").map(String::as_str),
            Some("/users/42")
        );
        assert_eq!(
            attrs.get("http.response.status_code").map(String::as_str),
            Some("201")
        );
        assert!(
            !attrs.contains_key("http.method"),
            "the retired pre-1.0 attribute name must not be emitted"
        );
        assert!(
            !attrs.contains_key("http.status_code"),
            "the retired pre-1.0 attribute name must not be emitted"
        );
        assert_ne!(
            attrs.get("http.request.method").map(String::as_str),
            Some("method")
        );
        assert_ne!(attrs.get("http.route").map(String::as_str), Some("path"));
    }

    /// Regression: `http.route` is a low-cardinality dimension, so the query
    /// string must not reach it. Against the pre-fix code, which passed
    /// `req.path.clone()` (the raw target), every distinct query minted its own
    /// time series.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn handle_strips_the_query_string_from_http_route() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::InMemoryMetricExporter;
        use opentelemetry_sdk::metrics::SdkMeterProvider;
        use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};

        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("test");
        let metrics = Arc::new(crate::metrics::HttpMetrics::new(&meter).unwrap());
        let middleware = TelemetryMiddleware::new("test-service").with_metrics(metrics);

        // Two targets that differ only in their query string must land on the
        // same `http.route`.
        for target in ["/search?q=alpha&page=1", "/search?q=beta&page=9"] {
            let req = HttpRequest::new("GET", target.to_string());
            let next: Next = Box::new(|_req| Box::pin(async { Ok(HttpResponse::ok()) }));
            middleware.handle(req, next).await.unwrap();
        }

        provider.force_flush().unwrap();

        let resource_metrics = exporter.get_finished_metrics().unwrap();
        let mut routes: Vec<String> = Vec::new();
        for rm in &resource_metrics {
            for sm in rm.scope_metrics() {
                for metric in sm.metrics() {
                    if metric.name() != "http.server.request.count" {
                        continue;
                    }
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() {
                        for dp in sum.data_points() {
                            for kv in dp.attributes() {
                                if kv.key.as_str() == "http.route" {
                                    routes.push(kv.value.as_str().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(!routes.is_empty(), "http.route must be recorded");
        for route in &routes {
            assert_eq!(
                route, "/search",
                "the query string must not reach the label"
            );
        }
    }

    /// Regression: `next(req)` used to run without the request's OpenTelemetry
    /// context attached as the ACTIVE context. As a result, any span started
    /// by application code while handling the request (a DB call, an outbound
    /// HTTP client injecting a `traceparent`, etc.) would not nest under the
    /// request's root span — it would start a brand new, disconnected trace.
    ///
    /// This drives a real request through `handle()` with a real (in-memory)
    /// `SdkTracerProvider` installed as the global provider, has the `Next`
    /// closure simulate application code calling `global::tracer(...).start()`
    /// mid-request (exactly as real handler code would), and asserts the
    /// resulting child span's parent is the request's root span — both by
    /// `parent_span_id` and by shared `trace_id`.
    ///
    /// `#[serial]`: mutates the process-global tracer provider; see the
    /// matching note on `tracing_setup::tests::init_tracing_installs_w3c_propagator_that_parses_traceparent`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    #[serial_test::serial]
    async fn handle_attaches_context_so_application_spans_nest_under_request_span() {
        use opentelemetry::trace::Span as _;
        use opentelemetry::trace::Tracer as _;
        use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider.clone());

        // Constructed *after* the provider above is installed globally, so its
        // cached tracer (see the doc comment on the `tracer` field) resolves
        // to this test's in-memory provider.
        let middleware = TelemetryMiddleware::new("test-service");

        let req = HttpRequest::new("GET", "/orders".to_string());
        let next: Next = Box::new(|_req| {
            Box::pin(async {
                // Simulates application handler code — it has no access to
                // the middleware's internal tracer, so like real code it goes
                // through the global tracer, exactly like an outbound HTTP
                // client would when injecting a `traceparent` for downstream
                // propagation.
                let tracer = global::tracer("app-code");
                let mut child = tracer.start("db.query");
                child.end();
                Ok(HttpResponse::new(200))
            })
        });

        let res = middleware.handle(req, next).await.unwrap();
        assert_eq!(res.status, 200);

        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();

        let root = spans
            .iter()
            .find(|s| s.name == "GET /orders")
            .expect("request root span must be exported");
        let child = spans
            .iter()
            .find(|s| s.name == "db.query")
            .expect("application-code span created inside next() must be exported");

        assert!(
            root.span_context.is_valid(),
            "request root span must have a valid span context"
        );
        assert_eq!(
            child.parent_span_id,
            root.span_context.span_id(),
            "application span created inside next() must be a direct child of the \
             request's root span — it is not attached to the request context"
        );
        assert_eq!(
            child.span_context.trace_id(),
            root.span_context.trace_id(),
            "application span created inside next() must share the request's trace id, \
             not start a new, disconnected trace"
        );

        provider.shutdown().ok();
    }

    #[test]
    fn test_header_extractor() {
        let mut headers = armature_core::headers::HeaderMap::new();
        headers.insert("traceparent", "00-123-456-01".to_string());

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
        headers.insert("key1", "value1".to_string());
        headers.insert("key2", "value2".to_string());

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
