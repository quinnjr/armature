//! Request and response interceptors.

#![allow(dead_code)]

use crate::{Response, Result};
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
                headers.insert(
                    http::header::AUTHORIZATION,
                    format!("Bearer {}", token).parse().unwrap(),
                );
            }
            AuthType::Basic { username, password } => {
                use base64::Engine;
                let credentials = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", username, password));
                headers.insert(
                    http::header::AUTHORIZATION,
                    format!("Basic {}", credentials).parse().unwrap(),
                );
            }
            AuthType::ApiKey { header, key } => {
                headers.insert(
                    http::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                    key.parse().unwrap(),
                );
            }
        }

        Ok(request)
    }
}

/// Retry-After header interceptor that respects rate limiting.
pub struct RateLimitInterceptor;

#[async_trait]
impl ResponseInterceptor for RateLimitInterceptor {
    async fn intercept(&self, response: Response) -> Result<Response> {
        if response.status() == http::StatusCode::TOO_MANY_REQUESTS
            && let Some(retry_after) = response.headers().get(http::header::RETRY_AFTER)
            && let Ok(seconds) = retry_after.to_str().unwrap_or("0").parse::<u64>()
        {
            tracing::warn!(
                retry_after_seconds = seconds,
                "Rate limited, should retry after {} seconds",
                seconds
            );
        }
        Ok(response)
    }
}
