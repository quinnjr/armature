//! Request and response interceptors.

#![allow(dead_code)]

use crate::{HttpClientError, Response, Result};
use async_trait::async_trait;
use reqwest::Request;

/// Interceptor trait for modifying requests and responses.
#[async_trait]
pub trait Interceptor: Send + Sync {
    /// Intercept and optionally modify the request before sending.
    async fn intercept_request(&self, request: Request) -> Result<Request> {
        Ok(request)
    }

    /// Intercept and optionally modify the response after receiving.
    async fn intercept_response(&self, response: Response) -> Result<Response> {
        Ok(response)
    }
}

/// Request-only interceptor.
#[async_trait]
pub trait RequestInterceptor: Send + Sync {
    /// Intercept and optionally modify the request.
    async fn intercept(&self, request: Request) -> Result<Request>;
}

/// Response-only interceptor.
#[async_trait]
pub trait ResponseInterceptor: Send + Sync {
    /// Intercept and optionally modify the response.
    async fn intercept(&self, response: Response) -> Result<Response>;
}

/// Logging interceptor that logs requests and responses.
pub struct LoggingInterceptor {
    log_headers: bool,
    log_body: bool,
}

impl LoggingInterceptor {
    /// Create a new logging interceptor.
    pub fn new() -> Self {
        Self {
            log_headers: false,
            log_body: false,
        }
    }

    /// Enable logging of headers.
    pub fn with_headers(mut self) -> Self {
        self.log_headers = true;
        self
    }

    /// Enable logging of body.
    pub fn with_body(mut self) -> Self {
        self.log_body = true;
        self
    }
}

impl Default for LoggingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Interceptor for LoggingInterceptor {
    async fn intercept_request(&self, request: Request) -> Result<Request> {
        tracing::debug!(
            method = %request.method(),
            url = %request.url(),
            "Sending HTTP request"
        );

        if self.log_headers {
            for (name, value) in request.headers() {
                tracing::trace!(
                    header = %name,
                    value = ?value,
                    "Request header"
                );
            }
        }

        Ok(request)
    }

    async fn intercept_response(&self, response: Response) -> Result<Response> {
        tracing::debug!(
            status = %response.status(),
            "Received HTTP response"
        );

        if self.log_headers {
            for (name, value) in response.headers() {
                tracing::trace!(
                    header = %name,
                    value = ?value,
                    "Response header"
                );
            }
        }

        Ok(response)
    }
}

/// Authentication interceptor that adds auth headers.
pub struct AuthInterceptor {
    auth_type: AuthType,
}

enum AuthType {
    Bearer(String),
    Basic { username: String, password: String },
    ApiKey { header: String, key: String },
}

impl AuthInterceptor {
    /// Create a bearer token interceptor.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            auth_type: AuthType::Bearer(token.into()),
        }
    }

    /// Create a basic auth interceptor.
    pub fn basic(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            auth_type: AuthType::Basic {
                username: username.into(),
                password: password.into(),
            },
        }
    }

    /// Create an API key interceptor.
    pub fn api_key(header: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            auth_type: AuthType::ApiKey {
                header: header.into(),
                key: key.into(),
            },
        }
    }
}

#[async_trait]
impl RequestInterceptor for AuthInterceptor {
    async fn intercept(&self, mut request: Request) -> Result<Request> {
        let headers = request.headers_mut();

        match &self.auth_type {
            AuthType::Bearer(token) => {
                let value = format!("Bearer {}", token).parse().map_err(|_| {
                    HttpClientError::Interceptor(
                        "bearer token contains characters that are not valid in an \
                         HTTP header value"
                            .to_string(),
                    )
                })?;
                headers.insert(http::header::AUTHORIZATION, value);
            }
            AuthType::Basic { username, password } => {
                use base64::Engine;
                let credentials = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", username, password));
                // base64 output is always header-safe, but stay fallible for
                // consistency rather than panicking.
                let value = format!("Basic {}", credentials).parse().map_err(|_| {
                    HttpClientError::Interceptor(
                        "basic auth credentials could not be encoded as a valid \
                         HTTP header value"
                            .to_string(),
                    )
                })?;
                headers.insert(http::header::AUTHORIZATION, value);
            }
            AuthType::ApiKey { header, key } => {
                let name =
                    http::header::HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
                        HttpClientError::Interceptor(format!(
                            "API key header name {:?} is not a valid HTTP header name",
                            header
                        ))
                    })?;
                let value = key.parse().map_err(|_| {
                    HttpClientError::Interceptor(
                        "API key contains characters that are not valid in an HTTP \
                         header value"
                            .to_string(),
                    )
                })?;
                headers.insert(name, value);
            }
        }

        Ok(request)
    }
}

/// Retry-After header interceptor that respects rate limiting.
///
/// When a response is a `429 Too Many Requests` carrying a numeric
/// `Retry-After` header (seconds), this interceptor transparently sleeps for
/// that duration before returning the response, so that a caller simply
/// retrying immediately after `intercept` returns will not hammer the
/// upstream service. The duration actually waited is capped by
/// `max_wait`, to avoid a hostile/misconfigured server stalling the caller
/// indefinitely.
pub struct RateLimitInterceptor {
    max_wait: std::time::Duration,
}

impl RateLimitInterceptor {
    /// Create a new rate-limit interceptor with a default max wait of 60s.
    pub fn new() -> Self {
        Self {
            max_wait: std::time::Duration::from_secs(60),
        }
    }

    /// Create a rate-limit interceptor with a custom cap on how long it will
    /// ever sleep for, regardless of what `Retry-After` requests.
    pub fn with_max_wait(max_wait: std::time::Duration) -> Self {
        Self { max_wait }
    }
}

impl Default for RateLimitInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResponseInterceptor for RateLimitInterceptor {
    async fn intercept(&self, response: Response) -> Result<Response> {
        if response.status() == http::StatusCode::TOO_MANY_REQUESTS
            && let Some(retry_after) = response.headers().get(http::header::RETRY_AFTER)
            && let Ok(seconds) = retry_after.to_str().unwrap_or("0").parse::<u64>()
        {
            let wait = std::time::Duration::from_secs(seconds).min(self.max_wait);
            tracing::warn!(
                retry_after_seconds = seconds,
                waited_seconds = wait.as_secs(),
                "Rate limited, waiting before returning response"
            );
            tokio::time::sleep(wait).await;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Method;

    fn dummy_request() -> Request {
        Request::new(Method::GET, "https://example.com/".parse().unwrap())
    }

    /// A bearer token containing a byte that is invalid in an HTTP header
    /// value (an embedded newline) must produce an `Err`, not a panic.
    #[tokio::test]
    async fn bearer_token_with_newline_returns_err_not_panic() {
        let interceptor = AuthInterceptor::bearer("good\ntoken");
        let result = interceptor.intercept(dummy_request()).await;
        assert!(
            matches!(result, Err(HttpClientError::Interceptor(_))),
            "malformed bearer token should return Interceptor error, got: {result:?}"
        );
    }

    /// An API key header name / value with an embedded newline must also
    /// return an `Err` rather than panicking on `.unwrap()`.
    #[tokio::test]
    async fn api_key_with_newline_returns_err_not_panic() {
        let bad_name = AuthInterceptor::api_key("X-Api\nKey", "value");
        assert!(
            matches!(
                bad_name.intercept(dummy_request()).await,
                Err(HttpClientError::Interceptor(_))
            ),
            "malformed API key header name should return an error"
        );

        let bad_value = AuthInterceptor::api_key("X-Api-Key", "bad\nvalue");
        assert!(
            matches!(
                bad_value.intercept(dummy_request()).await,
                Err(HttpClientError::Interceptor(_))
            ),
            "malformed API key value should return an error"
        );
    }
}
