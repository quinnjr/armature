//! Middleware chain for HTTP client.

#![allow(dead_code)]

use crate::{Response, Result};
use async_trait::async_trait;
use reqwest::Request;
use std::sync::Arc;

/// Type alias for HTTP metrics callback function.
pub type HttpMetricsCallbackFn = Arc<dyn Fn(&str, &str, u16, std::time::Duration) + Send + Sync>;

/// Middleware trait for processing requests and responses.
#[async_trait]
pub trait Middleware: Send + Sync {
    /// Process the request and call the next middleware.
    async fn handle(&self, request: Request, next: &MiddlewareChain) -> Result<Response>;
}

/// Chain of middleware handlers.
pub struct MiddlewareChain {
    middlewares: Vec<Arc<dyn Middleware>>,
    client: reqwest::Client,
    index: usize,
}

impl MiddlewareChain {
    /// Create a new middleware chain.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            middlewares: Vec::new(),
            client,
            index: 0,
        }
    }

    /// Add a middleware to the chain.
    pub fn with_middleware<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    /// Execute the request through the middleware chain.
    pub async fn execute(&self, request: Request) -> Result<Response> {
        self.execute_at(0, request).await
    }

    /// Execute starting at a specific index.
    async fn execute_at(&self, index: usize, request: Request) -> Result<Response> {
        if index >= self.middlewares.len() {
            // End of chain, execute the actual request
            let response = self.client.execute(request).await?;
            Response::from_reqwest(response).await
        } else {
            let next = MiddlewareChain {
                middlewares: self.middlewares.clone(),
                client: self.client.clone(),
                index: index + 1,
            };
            self.middlewares[index].handle(request, &next).await
        }
    }

    /// Continue to the next middleware.
    pub async fn next(&self, request: Request) -> Result<Response> {
        self.execute_at(self.index, request).await
    }
}

/// Timeout middleware.
pub struct TimeoutMiddleware {
    timeout: std::time::Duration,
}

impl TimeoutMiddleware {
    /// Create a new timeout middleware.
    pub fn new(timeout: std::time::Duration) -> Self {
        Self { timeout }
    }
}

#[async_trait]
impl Middleware for TimeoutMiddleware {
    async fn handle(&self, request: Request, next: &MiddlewareChain) -> Result<Response> {
        match tokio::time::timeout(self.timeout, next.next(request)).await {
            Ok(result) => result,
            Err(_) => Err(crate::HttpClientError::Timeout(self.timeout)),
        }
    }
}

/// Request ID middleware that adds a unique ID to each request.
pub struct RequestIdMiddleware {
    header_name: String,
}

impl RequestIdMiddleware {
    /// Create a new request ID middleware.
    pub fn new() -> Self {
        Self {
            header_name: "X-Request-ID".to_string(),
        }
    }

    /// Create with a custom header name.
    pub fn with_header(header: impl Into<String>) -> Self {
        Self {
            header_name: header.into(),
        }
    }
}

impl Default for RequestIdMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for RequestIdMiddleware {
    async fn handle(&self, mut request: Request, next: &MiddlewareChain) -> Result<Response> {
        // Generate a simple request ID
        let request_id = format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        request.headers_mut().insert(
            http::header::HeaderName::from_bytes(self.header_name.as_bytes()).unwrap(),
            request_id.parse().unwrap(),
        );

        next.next(request).await
    }
}

/// Metrics middleware that records request timing.
pub struct MetricsMiddleware {
    on_complete: HttpMetricsCallbackFn,
}

impl MetricsMiddleware {
    /// Create a new metrics middleware with a callback.
    pub fn new<F>(on_complete: F) -> Self
    where
        F: Fn(&str, &str, u16, std::time::Duration) + Send + Sync + 'static,
    {
        Self {
            on_complete: Arc::new(on_complete),
        }
    }
}

#[async_trait]
impl Middleware for MetricsMiddleware {
    async fn handle(&self, request: Request, next: &MiddlewareChain) -> Result<Response> {
        let method = request.method().to_string();
        let url = request.url().to_string();
        let start = std::time::Instant::now();

        let result = next.next(request).await;
        let duration = start.elapsed();

        let status = match &result {
            Ok(resp) => resp.status().as_u16(),
            Err(_) => 0,
        };

        (self.on_complete)(&method, &url, status, duration);

        result
    }
}
