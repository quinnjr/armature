//! Azure Functions response conversion.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Azure Functions HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    /// HTTP status code.
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: String,
    /// Whether the body is base64 encoded.
    #[serde(rename = "isBase64Encoded", default)]
    pub is_base64_encoded: bool,
}

impl FunctionResponse {
    /// Create a new response.
    pub fn new(status_code: u16) -> Self {
        Self {
            status_code,
            headers: HashMap::new(),
            body: String::new(),
            is_base64_encoded: false,
        }
    }

    /// Create an OK response.
    pub fn ok() -> Self {
        Self::new(200)
    }

    /// Create a response with a body.
    pub fn with_body(status_code: u16, body: impl Into<String>) -> Self {
        Self {
            status_code,
            headers: HashMap::new(),
            body: body.into(),
            is_base64_encoded: false,
        }
    }

    /// Create a JSON response.
    pub fn json<T: Serialize>(data: &T) -> Result<Self, serde_json::Error> {
        let body = serde_json::to_string(data)?;
        Ok(Self::with_body(200, body).header("content-type", "application/json"))
    }

    /// Create an error response.
    pub fn error(status_code: u16, message: impl Into<String>) -> Self {
        let body = serde_json::json!({
            "error": message.into()
        });
        Self::with_body(status_code, body.to_string()).header("content-type", "application/json")
    }

    /// Create a not found response.
    pub fn not_found() -> Self {
        Self::error(404, "Not Found")
    }

    /// Create a bad request response.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::error(400, message)
    }

    /// Create an internal server error response.
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::error(500, message)
    }

    /// Set a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Set the body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Set binary body (base64 encoded).
    pub fn binary_body(mut self, data: &[u8]) -> Self {
        use base64::Engine;
        self.body = base64::engine::general_purpose::STANDARD.encode(data);
        self.is_base64_encoded = true;
        self
    }

    /// Set the content type.
    pub fn content_type(self, content_type: impl Into<String>) -> Self {
        self.header("content-type", content_type)
    }

    /// Add CORS headers.
    pub fn cors(self, origin: impl Into<String>) -> Self {
        self.header("access-control-allow-origin", origin)
            .header(
                "access-control-allow-methods",
                "GET, POST, PUT, DELETE, OPTIONS",
            )
            .header(
                "access-control-allow-headers",
                "Content-Type, Authorization",
            )
    }

    /// Convert to JSON string for Azure Functions output.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl Default for FunctionResponse {
    fn default() -> Self {
        Self::ok()
    }
}

impl From<&str> for FunctionResponse {
    fn from(body: &str) -> Self {
        Self::with_body(200, body)
    }
}

impl From<String> for FunctionResponse {
    fn from(body: String) -> Self {
        Self::with_body(200, body)
    }
}

impl From<Bytes> for FunctionResponse {
    fn from(body: Bytes) -> Self {
        if let Ok(s) = String::from_utf8(body.to_vec()) {
            Self::with_body(200, s)
        } else {
            Self::ok().binary_body(&body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn decode(body: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(body)
            .unwrap()
    }

    #[test]
    fn from_str_is_plain_200() {
        let r: FunctionResponse = "hello".into();
        assert_eq!(r.status_code, 200);
        assert_eq!(r.body, "hello");
        assert!(!r.is_base64_encoded);
    }

    #[test]
    fn from_string_is_plain_200() {
        let r: FunctionResponse = String::from("world").into();
        assert_eq!(r.status_code, 200);
        assert_eq!(r.body, "world");
        assert!(!r.is_base64_encoded);
    }

    #[test]
    fn from_utf8_bytes_stays_plain_text() {
        let r: FunctionResponse = Bytes::from_static(b"hi").into();
        assert_eq!(r.body, "hi");
        assert!(!r.is_base64_encoded);
    }

    #[test]
    fn from_non_utf8_bytes_is_base64_encoded() {
        let raw = vec![0u8, 159, 146, 150];
        let r: FunctionResponse = Bytes::from(raw.clone()).into();
        assert!(r.is_base64_encoded);
        assert_eq!(decode(&r.body), raw);
    }

    #[test]
    fn binary_body_sets_flag_and_encodes() {
        let r = FunctionResponse::ok().binary_body(&[0u8, 1, 2, 3]);
        assert!(r.is_base64_encoded);
        assert_eq!(decode(&r.body), vec![0u8, 1, 2, 3]);
    }

    #[test]
    fn json_sets_content_type_header() {
        let r = FunctionResponse::json(&serde_json::json!({ "a": 1 })).unwrap();
        assert_eq!(
            r.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(r.status_code, 200);
    }

    #[test]
    fn error_response_carries_status_and_json_body() {
        let r = FunctionResponse::error(503, "down");
        assert_eq!(r.status_code, 503);
        assert!(r.body.contains("down"));
        assert_eq!(
            r.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }
}
