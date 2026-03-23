//! Tower Service and HTTP Crate Compatibility
//!
//! This module provides interoperability between Armature's types and:
//! - `http` crate's `Request`/`Response` types
//! - `tower::Service` trait for middleware composition
//! - Hyper 1.0's service model
//!
//! # Features
//!
//! - Zero-cost conversions between Armature and http crate types
//! - Tower Service implementation for Armature handlers
//! - Layer composition for middleware stacks
//! - Hyper service adapter for direct integration
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_core::tower_compat::{ArmatureService, IntoHttpResponse};
//! use tower::{ServiceBuilder, ServiceExt};
//!
//! // Wrap Armature handler as Tower service
//! let service = ArmatureService::new(my_handler);
//!
//! // Compose with Tower middleware
//! let service = ServiceBuilder::new()
//!     .timeout(Duration::from_secs(30))
//!     .service(service);
//! ```

use crate::headers::HeaderMap as ArmatureHeaderMap;
use crate::http::{HttpRequest, HttpResponse};
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode};
use http_body_util::Full;
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use tower_service::Service;

// ============================================================================
// HTTP Crate Conversions
// ============================================================================

/// Extension trait for converting HttpRequest to http::Request.
pub trait IntoHttpRequest {
    /// Convert to http crate Request.
    fn into_http_request(self) -> Request<Bytes>;
}

impl IntoHttpRequest for HttpRequest {
    fn into_http_request(self) -> Request<Bytes> {
        let mut builder = Request::builder()
            .method(self.method.as_str())
            .uri(&self.path);

        // Add headers
        if let Some(headers) = builder.headers_mut() {
            for (name, value) in &self.headers {
                if let (Ok(name), Ok(value)) = (
                    HeaderName::try_from(name.as_str()),
                    HeaderValue::try_from(value.as_str()),
                ) {
                    headers.insert(name, value);
                }
            }
        }

        builder
            .body(self.body_bytes())
            .unwrap_or_else(|_| Request::new(Bytes::new()))
    }
}

/// Extension trait for creating HttpRequest from http::Request.
pub trait FromHttpRequest {
    /// Create from http crate Request.
    fn from_http_request(req: Request<Bytes>) -> Self;
}

impl FromHttpRequest for HttpRequest {
    fn from_http_request(req: Request<Bytes>) -> Self {
        let method = req.method().as_str().to_string();
        let path = req.uri().path().to_string();

        // Parse query params
        let query_params: HashMap<String, String> = req
            .uri()
            .query()
            .map(|q| serde_urlencoded::from_str(q).unwrap_or_default())
            .unwrap_or_default();

        // Convert headers
        let mut headers = HashMap::new();
        for (name, value) in req.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.as_str().to_string(), v.to_string());
            }
        }

        let body = req.into_body();

        HttpRequest::with_bytes_body(method, path, body)
            .with_headers_map(headers)
            .with_query_params(query_params)
    }
}

/// Extension methods for HttpRequest to add headers/query.
trait HttpRequestBuilderExt {
    fn with_headers_map(self, headers: HashMap<String, String>) -> Self;
    fn with_query_params(self, params: HashMap<String, String>) -> Self;
}

impl HttpRequestBuilderExt for HttpRequest {
    fn with_headers_map(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }

    fn with_query_params(mut self, params: HashMap<String, String>) -> Self {
        self.query_params = params;
        self
    }
}

/// Trait for converting to http::Response.
pub trait IntoHttpResponse {
    /// Convert to http crate Response.
    fn into_http_response(self) -> Response<Full<Bytes>>;
}

impl IntoHttpResponse for HttpResponse {
    fn into_http_response(self) -> Response<Full<Bytes>> {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

        let mut builder = Response::builder().status(status);

        // Add headers
        if let Some(headers) = builder.headers_mut() {
            for (name, value) in &self.headers {
                if let (Ok(name), Ok(value)) = (
                    HeaderName::try_from(name.as_str()),
                    HeaderValue::try_from(value.as_str()),
                ) {
                    headers.insert(name, value);
                }
            }
            for cookie_value in &self.cookies {
                if let Ok(value) = HeaderValue::try_from(cookie_value.as_str()) {
                    headers.append(HeaderName::from_static("set-cookie"), value);
                }
            }
        }

        builder
            .body(Full::new(self.into_body_bytes()))
            .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
    }
}

/// Trait for converting HttpResponse from http::Response.
pub trait HttpResponseFromHttp {
    /// Create from http crate Response.
    fn from_http_response(resp: Response<Bytes>) -> Self;
}

impl HttpResponseFromHttp for HttpResponse {
    fn from_http_response(resp: Response<Bytes>) -> Self {
        let status = resp.status().as_u16();

        let mut headers = HashMap::new();
        for (name, value) in resp.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.as_str().to_string(), v.to_string());
            }
        }

        let body_bytes = resp.into_body();

        HttpResponse::new(status)
            .with_headers(headers)
            .with_bytes_body(body_bytes)
    }
}

/// Extension trait for http::HeaderMap conversions.
pub trait HeaderMapExt {
    /// Convert to Armature HeaderMap.
    fn to_armature_headers(&self) -> ArmatureHeaderMap;
}

impl HeaderMapExt for HeaderMap {
    fn to_armature_headers(&self) -> ArmatureHeaderMap {
        let mut result = ArmatureHeaderMap::new();
        for (name, value) in self {
            if let Ok(v) = value.to_str() {
                result.insert(name.as_str(), v);
            }
        }
        result
    }
}

/// Extension trait for Armature HeaderMap.
pub trait ArmatureHeaderMapExt {
    /// Convert to http::HeaderMap.
    fn to_http_headers(&self) -> HeaderMap;
}

impl ArmatureHeaderMapExt for ArmatureHeaderMap {
    fn to_http_headers(&self) -> HeaderMap {
        let mut result = HeaderMap::new();
        for (name, value) in self.iter() {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(name.as_str()),
                HeaderValue::try_from(value.as_str()),
            ) {
                result.insert(name, value);
            }
        }
        result
    }
}

// ============================================================================
// Tower Service Implementation
// ============================================================================

/// Handler function type for Tower service.
pub type BoxedHandler =
    Box<dyn Fn(HttpRequest) -> Pin<Box<dyn Future<Output = HttpResponse> + Send>> + Send + Sync>;

/// Tower-compatible service wrapping an Armature handler.
pub struct ArmatureService<H> {
    handler: Arc<H>,
}

impl<H> ArmatureService<H> {
    /// Create new service from handler.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl<H> Clone for ArmatureService<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
        }
    }
}

/// Service implementation for async function handlers.
impl<H, Fut> Service<Request<Bytes>> for ArmatureService<H>
where
    H: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HttpResponse> + Send + 'static,
{
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        TOWER_STATS.record_poll_ready();
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Bytes>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        TOWER_STATS.record_call();

        Box::pin(async move {
            let armature_req = <HttpRequest as FromHttpRequest>::from_http_request(req);
            let armature_resp = handler(armature_req).await;
            Ok(armature_resp.into_http_response())
        })
    }
}

// ============================================================================
// Hyper Service Adapter
// ============================================================================

/// Adapter for using Armature handlers with Hyper directly.
pub struct HyperServiceAdapter<H> {
    handler: Arc<H>,
}

impl<H> HyperServiceAdapter<H> {
    /// Create new adapter.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl<H> Clone for HyperServiceAdapter<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<H, Fut> Service<hyper::Request<hyper::body::Incoming>> for HyperServiceAdapter<H>
where
    H: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = HttpResponse> + Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: hyper::Request<hyper::body::Incoming>) -> Self::Future {
        let handler = Arc::clone(&self.handler);
        TOWER_STATS.record_hyper_call();

        Box::pin(async move {
            // Collect incoming body
            let (parts, body) = req.into_parts();
            let body_bytes = match http_body_util::BodyExt::collect(body).await {
                Ok(collected) => collected.to_bytes(),
                Err(_) => Bytes::new(),
            };

            let http_req = Request::from_parts(parts, body_bytes);
            let armature_req = <HttpRequest as FromHttpRequest>::from_http_request(http_req);
            let armature_resp = handler(armature_req).await;

            Ok(armature_resp.into_http_response())
        })
    }
}

// ============================================================================
// Service Factory
// ============================================================================

/// Factory for creating services per connection (Hyper pattern).
pub struct ServiceFactory<H> {
    handler: Arc<H>,
}

impl<H> ServiceFactory<H> {
    /// Create new factory.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl<H: Clone> Clone for ServiceFactory<H> {
    fn clone(&self) -> Self {
        Self {
            handler: Arc::clone(&self.handler),
        }
    }
}

impl<H, Fut> Service<()> for ServiceFactory<H>
where
    H: Fn(HttpRequest) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = HttpResponse> + Send + 'static,
{
    type Response = HyperServiceAdapter<H>;
    type Error = Infallible;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: ()) -> Self::Future {
        let handler = (*self.handler).clone();
        std::future::ready(Ok(HyperServiceAdapter::new(handler)))
    }
}

// ============================================================================
// Tower Layer Support
// ============================================================================

/// Layer for wrapping Armature handlers.
pub struct ArmatureLayer;

impl ArmatureLayer {
    /// Create new layer.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ArmatureLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> tower::Layer<S> for ArmatureLayer {
    type Service = ArmatureLayerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ArmatureLayerService { inner }
    }
}

/// Service created by ArmatureLayer.
pub struct ArmatureLayerService<S> {
    inner: S,
}

impl<S: Clone> Clone for ArmatureLayerService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S, ReqBody> Service<Request<ReqBody>> for ArmatureLayerService<S>
where
    S: Service<Request<ReqBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        self.inner.call(req)
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Global Tower compatibility statistics.
#[derive(Debug, Default)]
pub struct TowerStats {
    /// poll_ready calls
    poll_ready_calls: AtomicU64,
    /// Service calls
    service_calls: AtomicU64,
    /// Hyper adapter calls
    hyper_calls: AtomicU64,
    /// Conversions from http::Request
    from_http_request: AtomicU64,
    /// Conversions to http::Response
    to_http_response: AtomicU64,
}

impl TowerStats {
    fn record_poll_ready(&self) {
        self.poll_ready_calls.fetch_add(1, Ordering::Relaxed);
    }

    fn record_call(&self) {
        self.service_calls.fetch_add(1, Ordering::Relaxed);
    }

    fn record_hyper_call(&self) {
        self.hyper_calls.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn record_from_http_request(&self) {
        self.from_http_request.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn record_to_http_response(&self) {
        self.to_http_response.fetch_add(1, Ordering::Relaxed);
    }

    /// Get poll_ready calls.
    pub fn poll_ready_calls(&self) -> u64 {
        self.poll_ready_calls.load(Ordering::Relaxed)
    }

    /// Get service calls.
    pub fn service_calls(&self) -> u64 {
        self.service_calls.load(Ordering::Relaxed)
    }

    /// Get hyper calls.
    pub fn hyper_calls(&self) -> u64 {
        self.hyper_calls.load(Ordering::Relaxed)
    }
}

/// Global statistics.
static TOWER_STATS: TowerStats = TowerStats {
    poll_ready_calls: AtomicU64::new(0),
    service_calls: AtomicU64::new(0),
    hyper_calls: AtomicU64::new(0),
    from_http_request: AtomicU64::new(0),
    to_http_response: AtomicU64::new(0),
};

/// Get global Tower statistics.
pub fn tower_stats() -> &'static TowerStats {
    &TOWER_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_request_conversion() {
        let http_req = Request::builder()
            .method("POST")
            .uri("/api/users?page=1")
            .header("Content-Type", "application/json")
            .body(Bytes::from(r#"{"name":"test"}"#))
            .unwrap();

        let armature_req = <HttpRequest as FromHttpRequest>::from_http_request(http_req);
        assert_eq!(armature_req.method, "POST");
        assert_eq!(armature_req.path, "/api/users");
    }

    #[test]
    fn test_http_response_conversion() {
        let armature_resp = HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/plain".to_string())
            .with_body("Hello, World!".as_bytes().to_vec());

        let http_resp = armature_resp.into_http_response();
        assert_eq!(http_resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_round_trip_request() {
        let original = HttpRequest::new("GET".to_string(), "/test".to_string());

        let http_req = original.clone().into_http_request();
        let back = <HttpRequest as FromHttpRequest>::from_http_request(http_req);

        assert_eq!(original.method, back.method);
        assert_eq!(original.path, back.path);
    }

    #[test]
    fn test_header_map_conversion() {
        let mut http_headers = HeaderMap::new();
        http_headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        let armature_headers = http_headers.to_armature_headers();
        assert!(armature_headers.get("content-type").is_some());

        let back = armature_headers.to_http_headers();
        assert!(back.get("content-type").is_some());
    }

    #[tokio::test]
    async fn test_armature_service() {
        async fn handler(_req: HttpRequest) -> HttpResponse {
            HttpResponse::ok().with_body(b"OK".to_vec())
        }

        let mut service = ArmatureService::new(handler);

        let req = Request::builder()
            .method("GET")
            .uri("/")
            .body(Bytes::new())
            .unwrap();

        let resp = service.call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn test_armature_layer() {
        let _layer = ArmatureLayer::new();
    }

    #[test]
    fn test_tower_stats() {
        let stats = tower_stats();
        let _ = stats.poll_ready_calls();
        let _ = stats.service_calls();
        let _ = stats.hyper_calls();
    }
}
