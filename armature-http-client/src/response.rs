//! HTTP response wrapper.

use crate::{HttpClientError, Result};
use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;

/// HTTP response wrapper.
#[derive(Debug)]
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
    url: url::Url,
}

impl Response {
    /// Create a response from a reqwest response.
    ///
    /// Returns an error if the response body cannot be fully read (e.g. the
    /// connection is reset mid-body) instead of silently substituting an
    /// empty body on an otherwise-successful status.
    pub(crate) async fn from_reqwest(response: reqwest::Response) -> Result<Self> {
        let status = response.status();
        let headers = response.headers().clone();
        let url = response.url().clone();
        let body = response.bytes().await.map_err(HttpClientError::Http)?;

        Ok(Self {
            status,
            headers,
            body,
            url,
        })
    }

    /// Get the status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Check if the response was successful (2xx).
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Check if the response was a client error (4xx).
    pub fn is_client_error(&self) -> bool {
        self.status.is_client_error()
    }

    /// Check if the response was a server error (5xx).
    pub fn is_server_error(&self) -> bool {
        self.status.is_server_error()
    }

    /// Get the response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Get a specific header value.
    pub fn header(&self, name: impl AsRef<str>) -> Option<&str> {
        self.headers
            .get(name.as_ref())
            .and_then(|v| v.to_str().ok())
    }

    /// Get the response URL.
    pub fn url(&self) -> &url::Url {
        &self.url
    }

    /// Get the response body as bytes.
    pub fn bytes(&self) -> &Bytes {
        &self.body
    }

    /// Consume the response and return the body as bytes.
    pub fn into_bytes(self) -> Bytes {
        self.body
    }

    /// Get the response body as text.
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.to_vec()).map_err(|e| HttpClientError::Json(e.to_string()))
    }

    /// Consume the response and return the body as text.
    pub fn into_text(self) -> Result<String> {
        self.text()
    }

    /// Parse the response body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| HttpClientError::Json(e.to_string()))
    }

    /// Consume the response and parse as JSON.
    pub fn into_json<T: DeserializeOwned>(self) -> Result<T> {
        self.json()
    }

    /// Get the content length if available.
    pub fn content_length(&self) -> Option<u64> {
        self.headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    }

    /// Get the content type if available.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Check for an error response and return it.
    pub fn error_for_status(self) -> Result<Self> {
        if self.status.is_client_error() || self.status.is_server_error() {
            let message = self.text().unwrap_or_else(|_| "Unknown error".to_string());
            Err(HttpClientError::Response {
                status: self.status.as_u16(),
                message,
            })
        } else {
            Ok(self)
        }
    }
}
