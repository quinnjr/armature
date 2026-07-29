//! Lambda response conversion.

use bytes::Bytes;
use lambda_http::{Body, Response};
use std::collections::HashMap;

/// Lambda HTTP response.
pub struct LambdaResponse {
    /// Status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: Bytes,
    /// Whether body is base64 encoded.
    pub is_base64: bool,
}

impl LambdaResponse {
    /// Create a new response.
    pub fn new(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: body.into(),
            is_base64: false,
        }
    }

    /// Create an OK response.
    pub fn ok(body: impl Into<Bytes>) -> Self {
        Self::new(200, body)
    }

    /// Create a JSON response.
    pub fn json<T: serde::Serialize>(data: &T) -> Result<Self, serde_json::Error> {
        let body = serde_json::to_vec(data)?;
        Ok(Self::new(200, body).header("content-type", "application/json"))
    }

    /// Create an error response.
    pub fn error(status: u16, message: impl Into<String>) -> Self {
        let body = serde_json::json!({
            "error": message.into()
        });
        Self::new(status, serde_json::to_vec(&body).unwrap_or_default())
            .header("content-type", "application/json")
    }

    /// Create a not found response.
    pub fn not_found() -> Self {
        Self::error(404, "Not Found")
    }

    /// Create an internal server error response.
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::error(500, message)
    }

    /// Add a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Set content type.
    pub fn content_type(self, content_type: impl Into<String>) -> Self {
        self.header("content-type", content_type)
    }

    /// Mark body as base64 encoded.
    pub fn base64(mut self) -> Self {
        self.is_base64 = true;
        self
    }

    /// Convert to lambda_http::Response.
    pub fn into_lambda_response(self) -> Response<Body> {
        let mut builder = Response::builder().status(self.status);

        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }

        let body = if self.is_base64 {
            Body::Binary(self.body.to_vec())
        } else if let Ok(s) = String::from_utf8(self.body.to_vec()) {
            Body::Text(s)
        } else {
            Body::Binary(self.body.to_vec())
        };

        builder.body(body).unwrap_or_else(|_| {
            Response::builder()
                .status(500)
                .body(Body::Text("Internal Server Error".to_string()))
                .unwrap()
        })
    }
}

impl Default for LambdaResponse {
    fn default() -> Self {
        Self::new(200, Bytes::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_body_maps_to_text() {
        let resp = LambdaResponse::ok("hello world");
        let lambda = resp.into_lambda_response();
        assert_eq!(lambda.status(), 200);
        match lambda.body() {
            Body::Text(s) => assert_eq!(s, "hello world"),
            other => panic!("expected text body, got {other:?}"),
        }
    }

    #[test]
    fn base64_flag_forces_binary_body() {
        let resp = LambdaResponse::ok("hello").base64();
        let lambda = resp.into_lambda_response();
        match lambda.body() {
            Body::Binary(b) => assert_eq!(b, b"hello"),
            other => panic!("expected binary body, got {other:?}"),
        }
    }

    #[test]
    fn non_utf8_body_maps_to_binary() {
        let resp = LambdaResponse::new(200, Bytes::from_static(&[0xff, 0xfe, 0x00]));
        let lambda = resp.into_lambda_response();
        match lambda.body() {
            Body::Binary(b) => assert_eq!(b, &[0xff, 0xfe, 0x00]),
            other => panic!("expected binary body, got {other:?}"),
        }
    }

    #[test]
    fn headers_are_forwarded() {
        let resp = LambdaResponse::json(&serde_json::json!({ "ok": true })).unwrap();
        let lambda = resp.into_lambda_response();
        assert_eq!(
            lambda
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn error_response_sets_status_and_json() {
        let resp = LambdaResponse::not_found();
        assert_eq!(resp.status, 404);
        let lambda = resp.into_lambda_response();
        assert_eq!(lambda.status(), 404);
        match lambda.body() {
            Body::Text(s) => assert!(s.contains("Not Found")),
            other => panic!("expected text body, got {other:?}"),
        }
    }
}
