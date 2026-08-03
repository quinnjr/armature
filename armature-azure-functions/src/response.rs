//! Azure Functions response conversion.

use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Azure Functions HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResponse {
    /// HTTP status code.
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    /// Response headers, in emission order.
    ///
    /// A list rather than a map because HTTP allows the same field name to
    /// appear more than once and a handler must be able to use that — most
    /// importantly to emit several `Set-Cookie` lines, which cannot legally be
    /// folded into one. A map would silently keep only the last one.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
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
            headers: Vec::new(),
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
            headers: Vec::new(),
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

    /// Append a header.
    ///
    /// Appends rather than replaces, so calling this twice with the same name
    /// emits two header lines (e.g. two `Set-Cookie`s). Use [`set_header`] to
    /// replace instead.
    ///
    /// [`set_header`]: FunctionResponse::set_header
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
        self.set_header("content-type", content_type)
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

    /// Serialize this crate's internal response shape to JSON.
    ///
    /// Note: this is the JSON form of [`FunctionResponse`] itself — `headers`
    /// is an array of `[name, value]` pairs so repeated names survive — not the
    /// Azure Functions custom-handler output envelope
    /// (`{"Outputs":{"res":{..}},"Logs":[..]}`). The runtime never calls this;
    /// it writes an HTTP response directly. Use it for logging or for
    /// round-tripping the internal shape, not as a wire format Azure will read.
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
        assert_eq!(r.header_value("content-type"), Some("application/json"));
        assert_eq!(r.status_code, 200);
    }

    #[test]
    fn error_response_carries_status_and_json_body() {
        let r = FunctionResponse::error(503, "down");
        assert_eq!(r.status_code, 503);
        assert!(r.body.contains("down"));
        assert_eq!(r.header_value("content-type"), Some("application/json"));
    }

    #[test]
    fn duplicate_headers_are_kept_separate() {
        // Session/auth flows need more than one Set-Cookie line, and these
        // cannot be folded into a single comma-separated value.
        let r = FunctionResponse::ok()
            .header("set-cookie", "a=1")
            .header("set-cookie", "b=2");
        assert_eq!(
            r.header_values("Set-Cookie").collect::<Vec<_>>(),
            ["a=1", "b=2"]
        );
    }

    #[test]
    fn set_header_replaces_existing_lines() {
        let r = FunctionResponse::ok()
            .header("content-type", "text/plain")
            .content_type("application/json");
        assert_eq!(r.header_values("content-type").count(), 1);
        assert_eq!(r.header_value("content-type"), Some("application/json"));
    }
}
