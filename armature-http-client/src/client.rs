//! HTTP client implementation.

use http::Method;
use reqwest::Request;
use std::sync::Arc;
use tracing::debug;

use crate::{
    CircuitBreaker, HttpClientConfig, HttpClientError, RequestBuilder, Response, Result,
    RetryStrategy,
};

/// HTTP client with retry, circuit breaker, and timeout support.
#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    config: Arc<HttpClientConfig>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
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
        }
    }

    /// Create a new HTTP client with default configuration.
    pub fn default_client() -> Self {
        Self::new(HttpClientConfig::default())
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
    pub(crate) async fn execute(&self, request: Request) -> Result<Response> {
        // Check circuit breaker
        if let Some(cb) = &self.circuit_breaker
            && !cb.is_allowed()
        {
            return Err(HttpClientError::CircuitOpen);
        }

        // Execute with retry if configured
        if let Some(retry_config) = &self.config.retry {
            self.execute_with_retry(request, retry_config).await
        } else {
            self.execute_once(request).await
        }
    }

    /// Execute request with retry logic.
    async fn execute_with_retry(
        &self,
        request: Request,
        retry_config: &crate::RetryConfig,
    ) -> Result<Response> {
        let mut attempt = 0;
        let mut last_error: Option<HttpClientError> = None;
        let start = std::time::Instant::now();

        loop {
            // Check max retry time
            if let Some(max_time) = retry_config.max_retry_time
                && start.elapsed() > max_time
            {
                break;
            }

            // Clone request for retry (reqwest requests can't be reused)
            let request_clone = clone_request(&request);

            match self.execute_once(request_clone).await {
                Ok(response) => {
                    // Record success with circuit breaker
                    if let Some(cb) = &self.circuit_breaker {
                        cb.record_success();
                    }

                    // Check if response status should trigger retry
                    if retry_config.should_retry_status(response.status().as_u16())
                        && attempt < retry_config.max_attempts - 1
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
                    // Record failure with circuit breaker
                    if let Some(cb) = &self.circuit_breaker {
                        cb.record_failure();
                    }

                    // Check if error is retryable
                    if retry_config.should_retry(attempt, &e)
                        && attempt < retry_config.max_attempts - 1
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
    async fn execute_once(&self, request: Request) -> Result<Response> {
        let response = self.inner.execute(request).await?;
        Ok(Response::from_reqwest(response).await)
    }
}

/// Clone a request (best effort - body may be empty).
fn clone_request(request: &Request) -> Request {
    let mut builder = reqwest::Request::new(request.method().clone(), request.url().clone());
    *builder.headers_mut() = request.headers().clone();
    builder
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::default_client()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
}
