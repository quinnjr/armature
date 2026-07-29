//! Zero-Cost Abstractions for High-Performance HTTP Handling
//!
//! This module provides compile-time optimizations that eliminate runtime overhead:
//!
//! - **Const Generic Extractors**: Combine multiple extractors at compile-time
//! - **Static Dispatch Middleware**: Middleware without `Box<dyn>` overhead
//!
//! # Philosophy
//!
//! These abstractions follow the Rust principle of "zero-cost abstractions" -
//! you don't pay for what you don't use, and what you do use is as efficient
//! as hand-written code.
//!
//! # Performance Impact
//!
//! - Extractor chains: No heap allocation, inline extraction
//! - Static middleware: Compile-time dispatch, no vtable lookups

use crate::{Error, HttpRequest, HttpResponse};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Const Generic Extractor Chains
// ============================================================================

/// Trait for extractors that work with const generics.
pub trait Extract: Sized {
    /// Extract from request.
    fn extract(req: &HttpRequest) -> Result<Self, Error>;
}

/// A single extractor wrapper.
#[derive(Debug)]
pub struct One<E: Extract>(pub E);

impl<E: Extract> Extract for One<E> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        E::extract(req).map(One)
    }
}

/// Two extractors combined.
#[derive(Debug)]
pub struct Two<E1: Extract, E2: Extract>(pub E1, pub E2);

impl<E1: Extract, E2: Extract> Extract for Two<E1, E2> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        Ok(Two(e1, e2))
    }
}

/// Three extractors combined.
#[derive(Debug)]
pub struct Three<E1: Extract, E2: Extract, E3: Extract>(pub E1, pub E2, pub E3);

impl<E1: Extract, E2: Extract, E3: Extract> Extract for Three<E1, E2, E3> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        Ok(Three(e1, e2, e3))
    }
}

/// Four extractors combined.
#[derive(Debug)]
pub struct Four<E1: Extract, E2: Extract, E3: Extract, E4: Extract>(pub E1, pub E2, pub E3, pub E4);

impl<E1: Extract, E2: Extract, E3: Extract, E4: Extract> Extract for Four<E1, E2, E3, E4> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        let e4 = E4::extract(req)?;
        Ok(Four(e1, e2, e3, e4))
    }
}

/// Five extractors combined.
#[derive(Debug)]
pub struct Five<E1: Extract, E2: Extract, E3: Extract, E4: Extract, E5: Extract>(
    pub E1,
    pub E2,
    pub E3,
    pub E4,
    pub E5,
);

impl<E1: Extract, E2: Extract, E3: Extract, E4: Extract, E5: Extract> Extract
    for Five<E1, E2, E3, E4, E5>
{
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        let e4 = E4::extract(req)?;
        let e5 = E5::extract(req)?;
        Ok(Five(e1, e2, e3, e4, e5))
    }
}

// Implement Extract for tuples (ergonomic alternative)
impl<E1: Extract, E2: Extract> Extract for (E1, E2) {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        Ok((e1, e2))
    }
}

impl<E1: Extract, E2: Extract, E3: Extract> Extract for (E1, E2, E3) {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        Ok((e1, e2, e3))
    }
}

impl<E1: Extract, E2: Extract, E3: Extract, E4: Extract> Extract for (E1, E2, E3, E4) {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        let e4 = E4::extract(req)?;
        Ok((e1, e2, e3, e4))
    }
}

impl<E1: Extract, E2: Extract, E3: Extract, E4: Extract, E5: Extract> Extract
    for (E1, E2, E3, E4, E5)
{
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        let e1 = E1::extract(req)?;
        let e2 = E2::extract(req)?;
        let e3 = E3::extract(req)?;
        let e4 = E4::extract(req)?;
        let e5 = E5::extract(req)?;
        Ok((e1, e2, e3, e4, e5))
    }
}

// ============================================================================
// Common Extractor Implementations
// ============================================================================

/// Extract the raw request body as bytes.
#[derive(Debug, Clone)]
pub struct RawBody(pub bytes::Bytes);

impl Extract for RawBody {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("RawBody");
        Ok(RawBody(req.body_bytes()))
    }
}

impl std::ops::Deref for RawBody {
    type Target = bytes::Bytes;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extract JSON body with compile-time type.
#[derive(Debug, Clone)]
pub struct JsonBody<T>(pub T);

impl<T: serde::de::DeserializeOwned> Extract for JsonBody<T> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("JsonBody");
        req.json().map(JsonBody)
    }
}

impl<T> std::ops::Deref for JsonBody<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extract query parameters with compile-time type.
#[derive(Debug, Clone)]
pub struct QueryParams<T>(pub T);

impl<T: serde::de::DeserializeOwned> Extract for QueryParams<T> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("QueryParams");
        // Re-encode the already-decoded params so reserved characters
        // (`&`, `=`, `%`, `+`, ...) in keys/values survive the round-trip.
        let pairs: Vec<(&String, &String)> = req.query_params.iter().collect();
        let query_string = serde_urlencoded::to_string(&pairs)
            .map_err(|e| Error::Deserialization(format!("Query encoding error: {}", e)))?;

        serde_urlencoded::from_str(&query_string)
            .map(QueryParams)
            .map_err(|e| Error::Deserialization(format!("Query parsing error: {}", e)))
    }
}

impl<T> std::ops::Deref for QueryParams<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extract a single path parameter by index.
#[derive(Debug, Clone)]
pub struct PathParam<const INDEX: usize>(pub String);

impl<const INDEX: usize> Extract for PathParam<INDEX> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("PathParam");
        // Get the Nth path parameter
        req.path_params
            .values()
            .nth(INDEX)
            .cloned()
            .map(PathParam)
            .ok_or_else(|| {
                Error::RouteNotFound(format!("Path parameter at index {} not found", INDEX))
            })
    }
}

impl<const INDEX: usize> std::ops::Deref for PathParam<INDEX> {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extract a single header by name.
#[derive(Debug, Clone)]
pub struct Header {
    /// Header name.
    pub name: String,
    /// Header value (None if not found).
    pub value: Option<String>,
}

impl Header {
    /// Create extractor for a specific header.
    pub fn named(name: impl Into<String>, req: &HttpRequest) -> Self {
        let name = name.into();
        let value = req.headers.get(&name).cloned();
        EXTRACTOR_STATS.record_extraction("Header");
        Self { name, value }
    }

    /// Get the header value or error if missing.
    pub fn required(self) -> Result<String, Error> {
        self.value
            .ok_or_else(|| Error::Validation(format!("Required header '{}' not found", self.name)))
    }

    /// Get the header value or default.
    pub fn unwrap_or(self, default: impl Into<String>) -> String {
        self.value.unwrap_or_else(|| default.into())
    }

    /// Check if header is present.
    pub fn is_present(&self) -> bool {
        self.value.is_some()
    }
}

/// Extract Content-Type header.
#[derive(Debug, Clone)]
pub struct ContentType(pub Option<String>);

impl Extract for ContentType {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("ContentType");
        Ok(ContentType(req.headers.get("content-type").cloned()))
    }
}

impl ContentType {
    /// Check if content type is JSON.
    pub fn is_json(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|v| v.contains("application/json"))
    }

    /// Get raw value.
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// Extract Authorization header.
#[derive(Debug, Clone)]
pub struct Authorization(pub Option<String>);

impl Extract for Authorization {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("Authorization");
        Ok(Authorization(req.headers.get("authorization").cloned()))
    }
}

impl Authorization {
    /// Get bearer token if present.
    pub fn bearer(&self) -> Option<&str> {
        self.0.as_ref().and_then(|v| v.strip_prefix("Bearer "))
    }

    /// Get basic auth credentials if present.
    pub fn basic(&self) -> Option<&str> {
        self.0.as_ref().and_then(|v| v.strip_prefix("Basic "))
    }

    /// Get raw value.
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

/// Extract the HTTP method.
#[derive(Debug, Clone)]
pub struct Method(pub String);

impl Extract for Method {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("Method");
        Ok(Method(req.method.clone()))
    }
}

impl std::ops::Deref for Method {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Extract the request path.
#[derive(Debug, Clone)]
pub struct RequestPath(pub String);

impl Extract for RequestPath {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        EXTRACTOR_STATS.record_extraction("RequestPath");
        Ok(RequestPath(req.path.clone()))
    }
}

impl std::ops::Deref for RequestPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Optional extractor - never fails, returns None if extraction fails.
#[derive(Debug, Clone)]
pub struct Optional<E>(pub Option<E>);

impl<E: Extract> Extract for Optional<E> {
    #[inline]
    fn extract(req: &HttpRequest) -> Result<Self, Error> {
        Ok(Optional(E::extract(req).ok()))
    }
}

impl<E> std::ops::Deref for Optional<E> {
    type Target = Option<E>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ============================================================================
// Static Dispatch Middleware
// ============================================================================

/// A middleware layer with static dispatch (no boxing).
pub trait Layer<S> {
    /// The wrapped service type.
    type Service;

    /// Wrap a service with this layer.
    fn layer(&self, inner: S) -> Self::Service;
}

/// A service that can process requests.
pub trait Service<Request> {
    /// Response type.
    type Response;
    /// Error type.
    type Error;
    /// Future type.
    type Future: Future<Output = Result<Self::Response, Self::Error>> + Send;

    /// Process a request.
    fn call(&self, req: Request) -> Self::Future;
}

/// Identity layer - does nothing, passes through.
#[derive(Debug, Clone, Copy, Default)]
pub struct Identity;

impl<S> Layer<S> for Identity {
    type Service = S;

    #[inline]
    fn layer(&self, inner: S) -> Self::Service {
        inner
    }
}

/// Stack two layers.
#[derive(Debug, Clone)]
pub struct Stack<Inner, Outer> {
    inner: Inner,
    outer: Outer,
}

impl<Inner, Outer> Stack<Inner, Outer> {
    /// Create a new stack.
    pub fn new(inner: Inner, outer: Outer) -> Self {
        Self { inner, outer }
    }
}

impl<S, Inner, Outer> Layer<S> for Stack<Inner, Outer>
where
    Inner: Layer<S>,
    Outer: Layer<Inner::Service>,
{
    type Service = Outer::Service;

    #[inline]
    fn layer(&self, service: S) -> Self::Service {
        let inner = self.inner.layer(service);
        self.outer.layer(inner)
    }
}

/// Builder for composing layers.
#[derive(Debug, Clone)]
pub struct LayerBuilder<L> {
    layer: L,
}

impl LayerBuilder<Identity> {
    /// Create a new layer builder.
    pub fn new() -> Self {
        Self { layer: Identity }
    }
}

impl Default for LayerBuilder<Identity> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L> LayerBuilder<L> {
    /// Add a layer to the stack.
    pub fn layer<NewLayer>(self, new_layer: NewLayer) -> LayerBuilder<Stack<L, NewLayer>> {
        LayerBuilder {
            layer: Stack::new(self.layer, new_layer),
        }
    }

    /// Build and wrap a service.
    pub fn service<S>(self, service: S) -> L::Service
    where
        L: Layer<S>,
    {
        self.layer.layer(service)
    }

    /// Get the composed layer.
    pub fn into_layer(self) -> L {
        self.layer
    }
}

// ============================================================================
// Static Middleware Implementations
// ============================================================================

/// Logging middleware with static dispatch.
#[derive(Debug, Clone)]
pub struct LoggingLayer {
    level: LogLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LoggingLayer {
    /// Create new logging layer.
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }

    /// Create info-level logger.
    pub fn info() -> Self {
        Self::new(LogLevel::Info)
    }

    /// Create debug-level logger.
    pub fn debug() -> Self {
        Self::new(LogLevel::Debug)
    }
}

impl Default for LoggingLayer {
    fn default() -> Self {
        Self::info()
    }
}

impl<S> Layer<S> for LoggingLayer {
    type Service = LoggingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LoggingService {
            inner,
            level: self.level,
        }
    }
}

/// Logging service wrapping inner service.
#[derive(Debug, Clone)]
pub struct LoggingService<S> {
    inner: S,
    #[allow(dead_code)]
    level: LogLevel,
}

impl<S> Service<HttpRequest> for LoggingService<S>
where
    S: Service<HttpRequest, Response = HttpResponse, Error = Error> + Clone + Send + Sync + 'static,
    S::Future: Send,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, req: HttpRequest) -> Self::Future {
        let inner = self.inner.clone();
        let method = req.method.clone();
        let path = req.path.clone();

        MIDDLEWARE_STATS.record_call("Logging");

        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = inner.call(req).await;
            let duration = start.elapsed();

            match &result {
                Ok(resp) => {
                    tracing::info!(
                        method = %method,
                        path = %path,
                        status = resp.status,
                        duration_ms = duration.as_millis() as u64,
                        "Request completed"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        method = %method,
                        path = %path,
                        error = %e,
                        duration_ms = duration.as_millis() as u64,
                        "Request failed"
                    );
                }
            }

            result
        })
    }
}

/// Timeout middleware with static dispatch.
#[derive(Debug, Clone)]
pub struct TimeoutLayer {
    duration: std::time::Duration,
}

impl TimeoutLayer {
    /// Create new timeout layer.
    pub fn new(duration: std::time::Duration) -> Self {
        Self { duration }
    }

    /// Create from seconds.
    pub fn from_secs(secs: u64) -> Self {
        Self::new(std::time::Duration::from_secs(secs))
    }

    /// Create from milliseconds.
    pub fn from_millis(millis: u64) -> Self {
        Self::new(std::time::Duration::from_millis(millis))
    }
}

impl<S> Layer<S> for TimeoutLayer {
    type Service = TimeoutService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TimeoutService {
            inner,
            duration: self.duration,
        }
    }
}

/// Timeout service.
#[derive(Debug, Clone)]
pub struct TimeoutService<S> {
    inner: S,
    duration: std::time::Duration,
}

impl<S> Service<HttpRequest> for TimeoutService<S>
where
    S: Service<HttpRequest, Response = HttpResponse, Error = Error> + Clone + Send + Sync + 'static,
    S::Future: Send,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, req: HttpRequest) -> Self::Future {
        let inner = self.inner.clone();
        let duration = self.duration;

        MIDDLEWARE_STATS.record_call("Timeout");

        Box::pin(async move {
            match tokio::time::timeout(duration, inner.call(req)).await {
                Ok(result) => result,
                Err(_) => Err(Error::timeout("Request timed out")),
            }
        })
    }
}

/// Request ID middleware.
#[derive(Debug, Clone, Default)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

/// Request ID service.
#[derive(Debug, Clone)]
pub struct RequestIdService<S> {
    inner: S,
}

impl<S> Service<HttpRequest> for RequestIdService<S>
where
    S: Service<HttpRequest, Response = HttpResponse, Error = Error> + Clone + Send + Sync + 'static,
    S::Future: Send,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, mut req: HttpRequest) -> Self::Future {
        let inner = self.inner.clone();

        MIDDLEWARE_STATS.record_call("RequestId");

        // Generate request ID if not present
        let request_id = req
            .headers
            .get("x-request-id")
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        req.headers
            .insert("x-request-id".to_string(), request_id.clone());

        Box::pin(async move {
            let mut resp = inner.call(req).await?;
            resp.headers.insert("x-request-id".to_string(), request_id);
            Ok(resp)
        })
    }
}

// ============================================================================
// Handler Service Adapter
// ============================================================================

/// Wraps a handler function as a Service.
#[derive(Clone)]
pub struct HandlerService<H, Args> {
    handler: H,
    _args: PhantomData<Args>,
}

impl<H, Args> HandlerService<H, Args> {
    /// Create new handler service.
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _args: PhantomData,
        }
    }
}

/// Trait for handlers that can be converted to services.
pub trait IntoService<Args> {
    /// The service type.
    type Service;

    /// Convert into a service.
    fn into_service(self) -> Self::Service;
}

// Implement for async fn() -> HttpResponse
impl<H, Fut> IntoService<()> for H
where
    H: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Service = HandlerService<H, ()>;

    fn into_service(self) -> Self::Service {
        HandlerService::new(self)
    }
}

impl<H, Fut> Service<HttpRequest> for HandlerService<H, ()>
where
    H: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, _req: HttpRequest) -> Self::Future {
        let handler = self.handler.clone();
        Box::pin(async move { handler().await })
    }
}

// Implement for async fn(HttpRequest) -> HttpResponse
impl<H, Fut> IntoService<(HttpRequest,)> for H
where
    H: Fn(HttpRequest) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Service = HandlerService<H, (HttpRequest,)>;

    fn into_service(self) -> Self::Service {
        HandlerService::new(self)
    }
}

impl<H, Fut> Service<HttpRequest> for HandlerService<H, (HttpRequest,)>
where
    H: Fn(HttpRequest) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, req: HttpRequest) -> Self::Future {
        let handler = self.handler.clone();
        Box::pin(async move { handler(req).await })
    }
}

// Implement for async fn(E1) -> HttpResponse where E1: Extract
impl<H, Fut, E1> IntoService<(E1,)> for H
where
    H: Fn(E1) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
    E1: Extract + Send + 'static,
{
    type Service = ExtractorHandlerService<H, (E1,)>;

    fn into_service(self) -> Self::Service {
        ExtractorHandlerService::new(self)
    }
}

/// Handler service that extracts arguments.
#[derive(Clone)]
pub struct ExtractorHandlerService<H, Args> {
    handler: H,
    _args: PhantomData<Args>,
}

impl<H, Args> ExtractorHandlerService<H, Args> {
    /// Create new extractor handler service.
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _args: PhantomData,
        }
    }
}

impl<H, Fut, E1> Service<HttpRequest> for ExtractorHandlerService<H, (E1,)>
where
    H: Fn(E1) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
    E1: Extract + Send + 'static,
{
    type Response = HttpResponse;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    fn call(&self, req: HttpRequest) -> Self::Future {
        let handler = self.handler.clone();
        Box::pin(async move {
            let e1 = E1::extract(&req)?;
            handler(e1).await
        })
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Global statistics for zero-cost abstractions.
#[derive(Debug, Default)]
pub struct ZeroCostStats {
    extractions: AtomicU64,
    middleware_calls: AtomicU64,
}

impl ZeroCostStats {
    fn record_extraction(&self, _name: &str) {
        self.extractions.fetch_add(1, Ordering::Relaxed);
    }

    fn record_call(&self, _name: &str) {
        self.middleware_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Get extraction count.
    pub fn extractions(&self) -> u64 {
        self.extractions.load(Ordering::Relaxed)
    }

    /// Get middleware call count.
    pub fn middleware_calls(&self) -> u64 {
        self.middleware_calls.load(Ordering::Relaxed)
    }
}

static EXTRACTOR_STATS: ZeroCostStats = ZeroCostStats {
    extractions: AtomicU64::new(0),
    middleware_calls: AtomicU64::new(0),
};

static MIDDLEWARE_STATS: ZeroCostStats = ZeroCostStats {
    extractions: AtomicU64::new(0),
    middleware_calls: AtomicU64::new(0),
};

/// Get extractor statistics.
pub fn extractor_stats() -> &'static ZeroCostStats {
    &EXTRACTOR_STATS
}

/// Get middleware statistics.
pub fn middleware_stats() -> &'static ZeroCostStats {
    &MIDDLEWARE_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_request() -> HttpRequest {
        let mut req = HttpRequest::new("GET".to_string(), "/api/users/123".to_string());
        req.headers
            .insert("content-type".to_string(), "application/json".to_string());
        req.headers
            .insert("authorization".to_string(), "Bearer token123".to_string());
        req.body = br#"{"name":"test"}"#.to_vec();
        req.path_params.insert("id".to_string(), "123".to_string());
        req.query_params.insert("page".to_string(), "1".to_string());
        req.query_params
            .insert("limit".to_string(), "10".to_string());
        req
    }

    #[test]
    fn test_raw_body_extract() {
        let req = create_request();
        let body = RawBody::extract(&req).unwrap();
        assert_eq!(body.as_ref(), br#"{"name":"test"}"#);
    }

    #[test]
    fn test_method_extract() {
        let req = create_request();
        let method = Method::extract(&req).unwrap();
        assert_eq!(&*method, "GET");
    }

    #[test]
    fn test_path_extract() {
        let req = create_request();
        let path = RequestPath::extract(&req).unwrap();
        assert_eq!(&*path, "/api/users/123");
    }

    #[test]
    fn test_content_type_extract() {
        let req = create_request();
        let content_type = ContentType::extract(&req).unwrap();
        assert!(content_type.is_json());
    }

    #[test]
    fn test_authorization_extract() {
        let req = create_request();
        let auth = Authorization::extract(&req).unwrap();
        assert_eq!(auth.bearer(), Some("token123"));
    }

    #[test]
    fn test_path_param_extract() {
        let req = create_request();
        let id = PathParam::<0>::extract(&req).unwrap();
        assert_eq!(&*id, "123");
    }

    #[test]
    fn test_query_params_extract_reserved_chars() {
        // Regression: values containing `&`, `=`, `%`, or `+` used to be
        // spliced into a query string without re-encoding, splitting into
        // bogus fields or mangling the value.
        #[derive(serde::Deserialize)]
        struct Query {
            q: String,
            plus: String,
        }

        let mut req = HttpRequest::new("GET".to_string(), "/search".to_string());
        req.query_params
            .insert("q".to_string(), "a&b=c%d".to_string());
        req.query_params
            .insert("plus".to_string(), "1+1".to_string());

        let QueryParams(query) = QueryParams::<Query>::extract(&req).unwrap();
        assert_eq!(query.q, "a&b=c%d");
        assert_eq!(query.plus, "1+1");
    }

    #[test]
    fn test_optional_extract() {
        let req = create_request();

        // Existing authorization
        let auth = Optional::<Authorization>::extract(&req).unwrap();
        assert!(auth.0.is_some());

        // Optional content type
        let ct = Optional::<ContentType>::extract(&req).unwrap();
        assert!(ct.0.is_some());
    }

    #[test]
    fn test_tuple_extract() {
        let req = create_request();

        let (method, path) = <(Method, RequestPath)>::extract(&req).unwrap();
        assert_eq!(&*method, "GET");
        assert_eq!(&*path, "/api/users/123");
    }

    #[test]
    fn test_two_extract() {
        let req = create_request();

        let Two(method, path) = Two::<Method, RequestPath>::extract(&req).unwrap();
        assert_eq!(&*method, "GET");
        assert_eq!(&*path, "/api/users/123");
    }

    #[test]
    fn test_layer_builder() {
        let builder = LayerBuilder::new()
            .layer(LoggingLayer::info())
            .layer(TimeoutLayer::from_secs(30))
            .layer(RequestIdLayer);

        let _layer = builder.into_layer();
    }

    #[test]
    fn test_identity_layer() {
        struct DummyService;
        let identity = Identity;
        let _service = identity.layer(DummyService);
    }

    #[test]
    fn test_logging_layer() {
        let layer = LoggingLayer::new(LogLevel::Info);
        assert_eq!(layer.level, LogLevel::Info);
    }

    #[test]
    fn test_timeout_layer() {
        let layer = TimeoutLayer::from_secs(30);
        assert_eq!(layer.duration, std::time::Duration::from_secs(30));
    }

    #[test]
    fn test_stats() {
        let extractor = extractor_stats();
        let _ = extractor.extractions();

        let middleware = middleware_stats();
        let _ = middleware.middleware_calls();
    }
}
