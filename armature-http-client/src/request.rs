//! Request builder.

use crate::{HttpClient, HttpClientError, Response, Result};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use serde::Serialize;
use std::time::Duration;

/// HTTP request builder.
pub struct RequestBuilder<'a> {
    client: &'a HttpClient,
    method: Method,
    url: String,
    headers: HeaderMap,
    query: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
}

impl<'a> RequestBuilder<'a> {
    /// Create a new request builder.
    pub(crate) fn new(client: &'a HttpClient, method: Method, url: String) -> Self {
        Self {
            client,
            method,
            url,
            headers: HeaderMap::new(),
            query: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    /// Add a header to the request.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) {
            self.headers.insert(name, value);
        }
        self
    }

    /// Add multiple headers to the request.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers.extend(headers);
        self
    }

    /// Add a query parameter.
    pub fn query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.push((key.into(), value.into()));
        self
    }

    /// Add multiple query parameters.
    pub fn queries<I, K, V>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in params {
            self.query.push((k.into(), v.into()));
        }
        self
    }

    /// Set the request body as raw bytes.
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set the request body as text.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.headers.insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        self.body = Some(text.into_bytes());
        self
    }

    /// Set the request body as JSON.
    ///
    /// Returns an error rather than silently producing a bodyless request if
    /// serialization fails.
    pub fn json<T: Serialize>(mut self, json: &T) -> Result<Self> {
        match serde_json::to_vec(json) {
            Ok(bytes) => {
                self.headers.insert(
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                self.body = Some(bytes);
                Ok(self)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialize JSON body");
                Err(HttpClientError::Json(e.to_string()))
            }
        }
    }

    /// Set the request body as form data.
    ///
    /// Returns an error rather than silently producing a bodyless request if
    /// encoding fails.
    pub fn form<T: Serialize>(mut self, form: &T) -> Result<Self> {
        match serde_urlencoded::to_string(form) {
            Ok(encoded) => {
                self.headers.insert(
                    http::header::CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
                self.body = Some(encoded.into_bytes());
                Ok(self)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to encode form data");
                Err(HttpClientError::RequestBuild(e.to_string()))
            }
        }
    }

    /// Set a custom timeout for this request.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set bearer authentication.
    pub fn bearer_auth(self, token: impl Into<String>) -> Self {
        self.header("Authorization", format!("Bearer {}", token.into()))
    }

    /// Set basic authentication.
    pub fn basic_auth(
        self,
        username: impl Into<String>,
        password: Option<impl Into<String>>,
    ) -> Self {
        use base64::Engine;
        let credentials = match password {
            Some(p) => format!("{}:{}", username.into(), p.into()),
            None => format!("{}:", username.into()),
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        self.header("Authorization", format!("Basic {}", encoded))
    }

    /// Build the URL with query parameters.
    fn build_url(&self) -> Result<url::Url> {
        let mut url = if let Some(base) = &self.client.config().base_url {
            let base =
                url::Url::parse(base).map_err(|e| HttpClientError::InvalidUrl(e.to_string()))?;
            base.join(&self.url)
                .map_err(|e| HttpClientError::InvalidUrl(e.to_string()))?
        } else {
            url::Url::parse(&self.url).map_err(|e| HttpClientError::InvalidUrl(e.to_string()))?
        };

        // Add query parameters
        if !self.query.is_empty() {
            let mut query_pairs = url.query_pairs_mut();
            for (key, value) in &self.query {
                query_pairs.append_pair(key, value);
            }
        }

        Ok(url)
    }

    /// Send the request.
    ///
    /// The request is captured as a [`crate::client::RequestSpec`] (method,
    /// URL, headers, body bytes, and per-request timeout) rather than a
    /// single `reqwest::Request`, so that retries can rebuild a complete,
    /// non-empty request for every attempt.
    pub async fn send(self) -> Result<Response> {
        let url = self.build_url()?;

        let mut headers = HeaderMap::new();

        // Add default headers from config
        for (name, value) in &self.client.config().default_headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(name.as_str()),
                HeaderValue::try_from(value.as_str()),
            ) {
                headers.insert(name, value);
            }
        }

        // Add request-specific headers (these take precedence)
        headers.extend(self.headers);

        let spec = crate::client::RequestSpec {
            method: self.method,
            url,
            headers,
            body: self.body,
            timeout: self.timeout,
        };

        self.client.execute(spec).await
    }
}
