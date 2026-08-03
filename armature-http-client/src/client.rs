//! HTTP client implementation.

use http::{HeaderMap, Method};
use reqwest::Request;
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

use crate::{
    CircuitBreaker, HttpClientConfig, HttpClientError, Interceptor, RequestBuilder,
    RequestInterceptor, Response, ResponseInterceptor, Result, RetryStrategy,
};

/// A fully captured representation of a request to be sent.
///
/// Unlike a single `reqwest::Request` (which is consumed on send and whose
/// body may not be reusable), a `RequestSpec` retains everything needed to
/// build a fresh, complete `reqwest::Request` - including the body bytes and
/// per-request timeout - for every attempt of a retried request.
pub(crate) struct RequestSpec {
    pub method: Method,
    pub url: url::Url,
    pub headers: HeaderMap,
    pub body: Option<Vec<u8>>,
    pub timeout: Option<Duration>,
}

impl RequestSpec {
    /// Build a brand new `reqwest::Request` from this spec.
    ///
    /// Called once per attempt so that retries always carry the original
    /// body and timeout instead of a lossy, bodyless clone.
    fn to_request(&self) -> Request {
        let mut request = Request::new(self.method.clone(), self.url.clone());
        *request.headers_mut() = self.headers.clone();
        if let Some(body) = &self.body {
            *request.body_mut() = Some(reqwest::Body::from(body.clone()));
        }
        if let Some(timeout) = self.timeout {
            *request.timeout_mut() = Some(timeout);
        }
        request
    }
}

/// HTTP client with retry, circuit breaker, and timeout support.
#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    config: Arc<HttpClientConfig>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    interceptors: Vec<Arc<dyn Interceptor>>,
    request_interceptors: Vec<Arc<dyn RequestInterceptor>>,
    response_interceptors: Vec<Arc<dyn ResponseInterceptor>>,
}

impl HttpClient {
    /// Create a new HTTP client with the given configuration.
    pub fn new(config: HttpClientConfig) -> Self {
        let mut builder = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .pool_idle_timeout(config.pool_idle_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .user_agent(&config.user_agent);

        if config.gzip {
            builder = builder.gzip(true);
        }
        if config.brotli {
            builder = builder.brotli(true);
        }
        if config.follow_redirects {
            builder = builder.redirect(reqwest::redirect::Policy::limited(config.max_redirects));
        } else {
            builder = builder.redirect(reqwest::redirect::Policy::none());
        }

        let inner = builder.build().expect("Failed to build HTTP client");

        let circuit_breaker = config
            .circuit_breaker
            .as_ref()
            .map(|cb_config| Arc::new(CircuitBreaker::new(cb_config.clone())));

        Self {
            inner,
            config: Arc::new(config),
            circuit_breaker,
            interceptors: Vec::new(),
            request_interceptors: Vec::new(),
            response_interceptors: Vec::new(),
        }
    }

    /// Create a new HTTP client with default configuration.
    pub fn default_client() -> Self {
        Self::new(HttpClientConfig::default())
    }

    /// Register a combined request/response interceptor.
    ///
    /// Its request phase runs once, before the first send attempt (and
    /// before any retries); its response phase runs once, on the final
    /// response returned from the retry/circuit-breaker machinery.
    pub fn with_interceptor<I: Interceptor + 'static>(mut self, interceptor: I) -> Self {
        self.interceptors.push(Arc::new(interceptor));
        self
    }

    /// Register a shared combined interceptor (e.g. one also held by the
    /// caller for inspection in tests).
    pub fn with_shared_interceptor(mut self, interceptor: Arc<dyn Interceptor>) -> Self {
        self.interceptors.push(interceptor);
        self
    }

    /// Register a request-only interceptor. Runs once, before the first
    /// send attempt, after any combined interceptors' request phase.
    pub fn with_request_interceptor<I: RequestInterceptor + 'static>(
        mut self,
        interceptor: I,
    ) -> Self {
        self.request_interceptors.push(Arc::new(interceptor));
        self
    }

    /// Register a response-only interceptor. Runs once, on the final
    /// response, after any combined interceptors' response phase.
    pub fn with_response_interceptor<I: ResponseInterceptor + 'static>(
        mut self,
        interceptor: I,
    ) -> Self {
        self.response_interceptors.push(Arc::new(interceptor));
        self
    }

    /// Get the underlying reqwest client.
    pub fn inner(&self) -> &reqwest::Client {
        &self.inner
    }

    /// Get the client configuration.
    pub fn config(&self) -> &HttpClientConfig {
        &self.config
    }

    /// Create a GET request builder.
    pub fn get(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::GET, url.into())
    }

    /// Create a POST request builder.
    pub fn post(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::POST, url.into())
    }

    /// Create a PUT request builder.
    pub fn put(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::PUT, url.into())
    }

    /// Create a PATCH request builder.
    pub fn patch(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::PATCH, url.into())
    }

    /// Create a DELETE request builder.
    pub fn delete(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::DELETE, url.into())
    }

    /// Create a HEAD request builder.
    pub fn head(&self, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, Method::HEAD, url.into())
    }

    /// Create a request builder with a custom method.
    pub fn request(&self, method: Method, url: impl Into<String>) -> RequestBuilder<'_> {
        RequestBuilder::new(self, method, url.into())
    }

    /// Execute a request with retry and circuit breaker logic.
    pub(crate) async fn execute(&self, mut spec: RequestSpec) -> Result<Response> {
        // Check circuit breaker
        if let Some(cb) = &self.circuit_breaker
            && !cb.is_allowed()
        {
            return Err(HttpClientError::CircuitOpen);
        }

        // Run request interceptors once, before the first attempt. Any
        // method/url/header/body mutation they make is folded back into the
        // spec so it is preserved across every retry attempt. The request
        // built here for the interceptors to run against is also reused as
        // the first send attempt below (instead of rebuilding an identical
        // request from `spec`), so the body is only cloned once even though
        // interceptors are configured.
        let mut first_request = None;
        if !self.interceptors.is_empty() || !self.request_interceptors.is_empty() {
            let mut request = spec.to_request();
            for interceptor in &self.interceptors {
                request = interceptor.intercept_request(request).await?;
            }
            for interceptor in &self.request_interceptors {
                request = interceptor.intercept(request).await?;
            }
            spec.method = request.method().clone();
            spec.url = request.url().clone();
            spec.headers = request.headers().clone();
            // Copy any body mutation back into the spec too, otherwise a
            // body-rewriting interceptor's change would be silently dropped
            // once the send path rebuilds a fresh request from `spec`.
            let buffered_body = request
                .body()
                .and_then(|b| b.as_bytes())
                .map(|b| b.to_vec());

            // Guard against the streaming-interceptor retry hazard: if an
            // interceptor installed a body that cannot be buffered (e.g. a
            // `reqwest::Body::wrap_stream`), `as_bytes()` returns `None`. The
            // first attempt would consume that stream, so every retry attempt
            // rebuilt from `spec` would send an empty body. Rather than
            // silently truncating the request, reject it up front. A streaming
            // body is only unsafe when more than one attempt is possible; with
            // retries disabled (or a single attempt) it is sent once and is
            // fine, so the common buffered-body path is entirely unaffected.
            let retries_possible = self
                .config
                .retry
                .as_ref()
                .is_some_and(|r| r.max_attempts > 1);
            if retries_possible && buffered_body.is_none() && request.body().is_some() {
                return Err(HttpClientError::StreamingBodyNotRetryable);
            }

            spec.body = buffered_body;
            first_request = Some(request);
        }

        // Execute with retry if configured
        let result = if let Some(retry_config) = &self.config.retry {
            self.execute_with_retry(&spec, retry_config, first_request)
                .await
        } else {
            let request = first_request.unwrap_or_else(|| spec.to_request());
            self.execute_once(request).await
        };

        // Run response interceptors once, on the final outcome.
        match result {
            Ok(mut response) => {
                for interceptor in &self.interceptors {
                    response = interceptor.intercept_response(response).await?;
                }
                for interceptor in &self.response_interceptors {
                    response = interceptor.intercept(response).await?;
                }
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }

    /// Execute request with retry logic.
    ///
    /// A fresh `reqwest::Request` (with the original body and timeout) is
    /// built from `spec` for every attempt, except the first, which reuses
    /// `first_request` when the caller already built one (e.g. because
    /// interceptors ran) to avoid cloning the body out of `spec` twice.
    async fn execute_with_retry(
        &self,
        spec: &RequestSpec,
        retry_config: &crate::RetryConfig,
        first_request: Option<Request>,
    ) -> Result<Response> {
        let mut attempt = 0;
        let mut last_error: Option<HttpClientError> = None;
        let start = std::time::Instant::now();
        let mut first_request = first_request;

        loop {
            // Check max retry time
            if let Some(max_time) = retry_config.max_retry_time
                && start.elapsed() > max_time
            {
                break;
            }

            // Reuse the already-built (and possibly interceptor-mutated)
            // request for the first attempt instead of cloning the body out
            // of `spec` a second time; every subsequent retry attempt still
            // needs a fresh request built from `spec`.
            let request = first_request.take().unwrap_or_else(|| spec.to_request());

            match self.execute_once(request).await {
                Ok(response) => {
                    // Check if response status should trigger retry
                    // Written as `attempt + 1 < max_attempts` rather than
                    // `attempt < max_attempts - 1`: the latter underflows when
                    // `max_attempts == 0`, panicking in debug and wrapping to
                    // `u32::MAX` (effectively infinite retries) in release.
                    if retry_config.should_retry_status(response.status().as_u16())
                        && attempt + 1 < retry_config.max_attempts
                    {
                        debug!(
                            attempt = attempt + 1,
                            status = %response.status(),
                            "Retrying request due to status code"
                        );
                        last_error = Some(HttpClientError::Response {
                            status: response.status().as_u16(),
                            message: "Retriable status code".to_string(),
                        });
                        attempt += 1;
                        let delay = retry_config.delay_for_attempt(attempt);
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    return Ok(response);
                }
                Err(e) => {
                    // Check if error is retryable
                    if retry_config.should_retry(attempt, &e)
                        && attempt + 1 < retry_config.max_attempts
                    {
                        debug!(
                            attempt = attempt + 1,
                            error = %e,
                            "Retrying request due to error"
                        );
                        last_error = Some(e);
                        attempt += 1;
                        let delay = retry_config.delay_for_attempt(attempt);
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    return Err(e);
                }
            }
        }

        Err(HttpClientError::RetryExhausted {
            attempts: attempt + 1,
            message: last_error
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string()),
        })
    }

    /// Execute request once without retry.
    ///
    /// Records the outcome (success/failure) with the circuit breaker, if
    /// configured, based on the *actual* result: a transport-level error, or
    /// an HTTP response whose status indicates a server failure (5xx / 429),
    /// both count as failures. This runs for every attempt regardless of
    /// whether a retry loop is driving it, so the breaker can open purely on
    /// a run of failing status codes, not just transport errors.
    async fn execute_once(&self, request: Request) -> Result<Response> {
        match self.inner.execute(request).await {
            Ok(response) => match Response::from_reqwest(response).await {
                Ok(response) => {
                    if let Some(cb) = &self.circuit_breaker {
                        if is_failure_status(response.status()) {
                            cb.record_failure();
                        } else {
                            cb.record_success();
                        }
                    }
                    Ok(response)
                }
                Err(e) => {
                    if let Some(cb) = &self.circuit_breaker {
                        cb.record_failure();
                    }
                    Err(e)
                }
            },
            Err(e) => {
                if let Some(cb) = &self.circuit_breaker {
                    cb.record_failure();
                }
                Err(e.into())
            }
        }
    }
}

/// Whether a response status should count as a circuit-breaker failure.
///
/// Mirrors the set of statuses `RetryConfig` retries by default (server
/// errors and rate limiting), but is independent of retry configuration so
/// the breaker still works when no retry policy is configured.
fn is_failure_status(status: http::StatusCode) -> bool {
    status.is_server_error() || status.as_u16() == 429
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::default_client()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RetryConfig;
    use async_trait::async_trait;
    use std::time::Duration;

    /// Request interceptor that swaps the body for a non-buffered streaming
    /// body (as `reqwest::Body::wrap_stream` produces), for which
    /// `Body::as_bytes()` returns `None`.
    struct StreamingBodyInterceptor;

    #[async_trait]
    impl RequestInterceptor for StreamingBodyInterceptor {
        async fn intercept(&self, mut request: Request) -> Result<Request> {
            let stream = futures::stream::once(async {
                Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"streamed chunk"))
            });
            *request.body_mut() = Some(reqwest::Body::wrap_stream(stream));
            Ok(request)
        }
    }

    #[test]
    fn test_client_creation() {
        let client = HttpClient::default();
        assert!(client.config().gzip);
        assert!(client.config().brotli);
    }

    #[test]
    fn test_client_with_config() {
        let config = HttpClientConfig::builder()
            .timeout(Duration::from_secs(60))
            .base_url("https://api.example.com")
            .build();

        let client = HttpClient::new(config);
        assert_eq!(client.config().timeout, Duration::from_secs(60));
        assert_eq!(
            client.config().base_url.as_deref(),
            Some("https://api.example.com")
        );
    }

    /// A streaming body installed by an interceptor cannot be replayed, so
    /// when retries are configured the request must be rejected loudly rather
    /// than silently sending an empty body on retry attempts. The guard runs
    /// inside `execute` before any network I/O, so no server is needed.
    #[tokio::test]
    async fn streaming_interceptor_body_with_retries_is_rejected() {
        let config = HttpClientConfig::builder()
            .retry(RetryConfig::constant(3, Duration::from_millis(1)))
            .build();
        let client = HttpClient::new(config).with_request_interceptor(StreamingBodyInterceptor);

        let result = client
            .post("http://127.0.0.1:0/")
            .body(b"original".to_vec())
            .send()
            .await;

        assert!(
            matches!(result, Err(HttpClientError::StreamingBodyNotRetryable)),
            "expected StreamingBodyNotRetryable, got {result:?}"
        );
    }

    /// A streaming body installed by an interceptor is fine when retries are
    /// NOT configured: it is sent exactly once, so the guard must not trip.
    /// (The request fails at the transport layer against the bogus address,
    /// but the error must NOT be `StreamingBodyNotRetryable`.)
    #[tokio::test]
    async fn streaming_interceptor_body_without_retries_is_allowed() {
        // No `.retry(..)` -> single attempt, streaming body is replay-safe.
        let client =
            HttpClient::default_client().with_request_interceptor(StreamingBodyInterceptor);

        let result = client
            .post("http://127.0.0.1:0/")
            .body(b"original".to_vec())
            .send()
            .await;

        assert!(
            !matches!(result, Err(HttpClientError::StreamingBodyNotRetryable)),
            "streaming body must be allowed without retries, got {result:?}"
        );
    }
}
