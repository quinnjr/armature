//! Lambda response conversion.

use bytes::Bytes;
use lambda_http::{Body, Response};

/// Lambda HTTP response.
pub struct LambdaResponse {
    /// Status code.
    pub status: u16,
    /// Response headers, in emission order.
    ///
    /// A list rather than a map because HTTP allows the same field name to
    /// appear more than once and a handler must be able to use that — most
    /// importantly to emit several `Set-Cookie` lines, which cannot legally be
    /// folded into one. A map would silently keep only the last one.
    pub headers: Vec<(String, String)>,
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
            headers: Vec::new(),
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

    /// Append a header.
    ///
    /// Appends rather than replaces, so calling this twice with the same name
    /// emits two header lines (e.g. two `Set-Cookie`s). Use [`set_header`] to
    /// replace instead.
    ///
    /// [`set_header`]: LambdaResponse::set_header
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set a header, removing any existing lines with the same name.
    pub fn set_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.headers.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
        self
    }

    /// The first value for `name`, matched case-insensitively.
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.header_values(name).next()
    }

    /// Every value for `name`, in emission order, matched case-insensitively.
    pub fn header_values<'a, 'n>(
        &'a self,
        name: &'n str,
    ) -> impl Iterator<Item = &'a str> + use<'a, 'n> {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Set content type.
    pub fn content_type(self, content_type: impl Into<String>) -> Self {
        self.set_header("content-type", content_type)
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
    fn duplicate_headers_are_all_emitted() {
        // Session/auth flows need more than one Set-Cookie line, and these
        // cannot be folded into a single comma-separated value.
        let resp = LambdaResponse::ok("x")
            .header("set-cookie", "a=1")
            .header("set-cookie", "b=2");
        let lambda = resp.into_lambda_response();
        let cookies: Vec<_> = lambda
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(cookies, vec!["a=1", "b=2"]);
    }

    #[test]
    fn set_header_replaces_existing_lines() {
        let resp = LambdaResponse::ok("x")
            .header("content-type", "text/plain")
            .set_header("content-type", "application/json");
        assert_eq!(resp.header_values("content-type").count(), 1);
        assert_eq!(resp.header_value("content-type"), Some("application/json"));
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
