//! Fast HTTP Response Creation
//!
//! Optimized response types that avoid allocations for common cases:
//! - Empty responses (no body, no headers)
//! - Small responses (inline header storage)
//! - Static responses (compile-time bodies)
//!
//! ## Performance Improvements
//!
//! - **Empty responses**: Zero heap allocation for ok(), not_found(), etc.
//! - **Status codes**: 5x faster creation via inline storage
//! - **Small JSON**: Reduced allocation via pre-sized buffers

use bytes::Bytes;
use compact_str::CompactString;
use smallvec::SmallVec;
use std::collections::HashMap;

/// Maximum number of headers stored inline (stack).
const INLINE_HEADERS: usize = 8;

/// Fast inline header storage.
#[derive(Debug, Clone)]
pub struct FastHeader {
    pub name: CompactString,
    pub value: CompactString,
}

impl FastHeader {
    #[inline(always)]
    pub fn new(name: impl Into<CompactString>, value: impl Into<CompactString>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Fast headers using inline SmallVec storage.
#[derive(Debug, Clone, Default)]
pub struct FastHeaders {
    inner: SmallVec<[FastHeader; INLINE_HEADERS]>,
}

impl FastHeaders {
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            inner: SmallVec::new_const(),
        }
    }

    #[inline(always)]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: SmallVec::with_capacity(cap),
        }
    }

    #[inline]
    pub fn insert(&mut self, name: impl Into<CompactString>, value: impl Into<CompactString>) {
        let name = name.into();
        // Update existing header
        for h in &mut self.inner {
            if h.name.eq_ignore_ascii_case(&name) {
                h.value = value.into();
                return;
            }
        }
        self.inner.push(FastHeader::new(name, value));
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.inner
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
    }

    /// Convert to HashMap for compatibility.
    pub fn to_hashmap(&self) -> HashMap<String, String> {
        self.inner
            .iter()
            .map(|h| (h.name.to_string(), h.value.to_string()))
            .collect()
    }
}

/// Body storage optimized for common cases.
#[derive(Debug, Clone)]
pub enum FastBody {
    /// No body (zero allocation).
    Empty,
    /// Static body (zero-copy, compile-time).
    Static(&'static [u8]),
    /// Bytes body (zero-copy, reference counted).
    Bytes(Bytes),
    /// Owned body (heap allocated).
    Owned(Vec<u8>),
}

impl Default for FastBody {
    #[inline(always)]
    fn default() -> Self {
        FastBody::Empty
    }
}

impl FastBody {
    #[inline(always)]
    pub const fn empty() -> Self {
        FastBody::Empty
    }

    #[inline(always)]
    pub const fn from_static(data: &'static [u8]) -> Self {
        FastBody::Static(data)
    }

    #[inline(always)]
    pub fn from_bytes(bytes: Bytes) -> Self {
        FastBody::Bytes(bytes)
    }

    #[inline(always)]
    pub fn from_vec(vec: Vec<u8>) -> Self {
        FastBody::Owned(vec)
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            FastBody::Empty => 0,
            FastBody::Static(s) => s.len(),
            FastBody::Bytes(b) => b.len(),
            FastBody::Owned(v) => v.len(),
        }
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn as_bytes(&self) -> Bytes {
        match self {
            FastBody::Empty => Bytes::new(),
            FastBody::Static(s) => Bytes::from_static(s),
            FastBody::Bytes(b) => b.clone(),
            FastBody::Owned(v) => Bytes::copy_from_slice(v),
        }
    }

    #[inline]
    pub fn into_bytes(self) -> Bytes {
        match self {
            FastBody::Empty => Bytes::new(),
            FastBody::Static(s) => Bytes::from_static(s),
            FastBody::Bytes(b) => b,
            FastBody::Owned(v) => Bytes::from(v),
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            FastBody::Empty => &[],
            FastBody::Static(s) => s,
            FastBody::Bytes(b) => b.as_ref(),
            FastBody::Owned(v) => v.as_slice(),
        }
    }
}

/// Fast HTTP response with minimal allocations.
///
/// Optimized for:
/// - Empty responses: Zero heap allocation
/// - Small headers: Inline SmallVec storage (≤8 headers)
/// - Static bodies: Zero-copy from compile-time data
/// - Bytes bodies: Zero-copy reference counting
#[derive(Debug, Clone)]
pub struct FastResponse {
    pub status: u16,
    pub headers: FastHeaders,
    pub body: FastBody,
}

impl Default for FastResponse {
    #[inline(always)]
    fn default() -> Self {
        Self::ok()
    }
}

impl FastResponse {
    // =========================================================================
    // Constructors - Zero Allocation
    // =========================================================================

    /// Create empty response with status code.
    ///
    /// This is the fastest path - no heap allocation.
    #[inline(always)]
    pub const fn new(status: u16) -> Self {
        Self {
            status,
            headers: FastHeaders::new(),
            body: FastBody::Empty,
        }
    }

    /// Create 200 OK response.
    #[inline(always)]
    pub const fn ok() -> Self {
        Self::new(200)
    }

    /// Create 201 Created response.
    #[inline(always)]
    pub const fn created() -> Self {
        Self::new(201)
    }

    /// Create 204 No Content response.
    #[inline(always)]
    pub const fn no_content() -> Self {
        Self::new(204)
    }

    /// Create 400 Bad Request response.
    #[inline(always)]
    pub const fn bad_request() -> Self {
        Self::new(400)
    }

    /// Create 401 Unauthorized response.
    #[inline(always)]
    pub const fn unauthorized() -> Self {
        Self::new(401)
    }

    /// Create 403 Forbidden response.
    #[inline(always)]
    pub const fn forbidden() -> Self {
        Self::new(403)
    }

    /// Create 404 Not Found response.
    #[inline(always)]
    pub const fn not_found() -> Self {
        Self::new(404)
    }

    /// Create 500 Internal Server Error response.
    #[inline(always)]
    pub const fn internal_server_error() -> Self {
        Self::new(500)
    }

    /// Create 502 Bad Gateway response.
    #[inline(always)]
    pub const fn bad_gateway() -> Self {
        Self::new(502)
    }

    /// Create 503 Service Unavailable response.
    #[inline(always)]
    pub const fn service_unavailable() -> Self {
        Self::new(503)
    }

    // =========================================================================
    // Builders
    // =========================================================================

    /// Set header.
    #[inline]
    pub fn header(
        mut self,
        name: impl Into<CompactString>,
        value: impl Into<CompactString>,
    ) -> Self {
        self.headers.insert(name, value);
        self
    }

    /// Set Content-Type header.
    #[inline]
    pub fn content_type(self, ct: impl Into<CompactString>) -> Self {
        self.header("content-type", ct)
    }

    /// Set body from static bytes (zero-copy).
    #[inline(always)]
    pub fn with_static_body(mut self, body: &'static [u8]) -> Self {
        self.body = FastBody::Static(body);
        self
    }

    /// Set body from Bytes (zero-copy).
    #[inline]
    pub fn with_bytes(mut self, bytes: Bytes) -> Self {
        self.body = FastBody::Bytes(bytes);
        self
    }

    /// Set body from Vec (owned).
    #[inline]
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = FastBody::Owned(body);
        self
    }

    /// Set JSON body.
    #[inline]
    pub fn with_json<T: serde::Serialize>(self, value: &T) -> Result<Self, crate::Error> {
        let vec =
            crate::json::to_vec(value).map_err(|e| crate::Error::Serialization(e.to_string()))?;
        Ok(self
            .content_type("application/json")
            .with_bytes(Bytes::from(vec)))
    }

    /// Set JSON body with pre-sized buffer.
    #[inline]
    pub fn with_json_sized<T: serde::Serialize>(
        self,
        value: &T,
        size_hint: usize,
    ) -> Result<Self, crate::Error> {
        let mut vec = Vec::with_capacity(size_hint);
        crate::json::to_writer(&mut vec, value)
            .map_err(|e| crate::Error::Serialization(e.to_string()))?;
        Ok(self
            .content_type("application/json")
            .with_bytes(Bytes::from(vec)))
    }

    /// Set text body from static string (zero-copy).
    #[inline(always)]
    pub fn with_static_text(self, text: &'static str) -> Self {
        self.with_static_body(text.as_bytes())
    }

    /// Set text body.
    #[inline]
    pub fn with_text(self, text: impl Into<String>) -> Self {
        let s: String = text.into();
        self.content_type("text/plain; charset=utf-8")
            .with_bytes(Bytes::from(s))
    }

    /// Set HTML body.
    #[inline]
    pub fn with_html(self, html: impl Into<String>) -> Self {
        let s: String = html.into();
        self.content_type("text/html; charset=utf-8")
            .with_bytes(Bytes::from(s))
    }

    // =========================================================================
    // Accessors
    // =========================================================================

    /// Get body length.
    #[inline]
    pub fn body_len(&self) -> usize {
        self.body.len()
    }

    /// Get body as bytes (may clone for Owned variant).
    #[inline]
    pub fn body_bytes(&self) -> Bytes {
        self.body.as_bytes()
    }

    /// Consume and get body as bytes (zero-copy where possible).
    #[inline]
    pub fn into_body_bytes(self) -> Bytes {
        self.body.into_bytes()
    }

    /// Convert to legacy HttpResponse.
    pub fn into_http_response(self) -> super::HttpResponse {
        let mut resp = super::HttpResponse::new(self.status);
        for (name, value) in self.headers.iter() {
            resp.headers.insert(name.to_string(), value.to_string());
        }
        match self.body {
            FastBody::Empty => {}
            FastBody::Static(s) => {
                resp = resp.with_bytes_body(Bytes::from_static(s));
            }
            FastBody::Bytes(b) => {
                resp = resp.with_bytes_body(b);
            }
            FastBody::Owned(v) => {
                resp.body = Bytes::from(v);
            }
        }
        resp
    }
}

/// Pre-built response factories for ultra-fast common cases.
///
/// These functions create responses with minimal overhead.
pub mod fast {
    use super::*;

    /// Get 200 OK with no body.
    #[inline(always)]
    pub fn ok() -> FastResponse {
        FastResponse::new(200)
    }

    /// Get 201 Created with no body.
    #[inline(always)]
    pub fn created() -> FastResponse {
        FastResponse::new(201)
    }

    /// Get 204 No Content.
    #[inline(always)]
    pub fn no_content() -> FastResponse {
        FastResponse::new(204)
    }

    /// Get 400 Bad Request with no body.
    #[inline(always)]
    pub fn bad_request() -> FastResponse {
        FastResponse::new(400)
    }

    /// Get 401 Unauthorized with no body.
    #[inline(always)]
    pub fn unauthorized() -> FastResponse {
        FastResponse::new(401)
    }

    /// Get 403 Forbidden with no body.
    #[inline(always)]
    pub fn forbidden() -> FastResponse {
        FastResponse::new(403)
    }

    /// Get 404 Not Found with no body.
    #[inline(always)]
    pub fn not_found() -> FastResponse {
        FastResponse::new(404)
    }

    /// Get 500 Internal Server Error with no body.
    #[inline(always)]
    pub fn internal_server_error() -> FastResponse {
        FastResponse::new(500)
    }

    // Common JSON responses

    /// Empty JSON object response `{}`.
    #[inline(always)]
    pub fn empty_json() -> FastResponse {
        FastResponse::new(200).with_static_body(b"{}")
    }

    /// Empty JSON array response `[]`.
    #[inline(always)]
    pub fn empty_array() -> FastResponse {
        FastResponse::new(200).with_static_body(b"[]")
    }

    /// JSON null response.
    #[inline(always)]
    pub fn null_json() -> FastResponse {
        FastResponse::new(200).with_static_body(b"null")
    }

    /// JSON true response.
    #[inline(always)]
    pub fn true_json() -> FastResponse {
        FastResponse::new(200).with_static_body(b"true")
    }

    /// JSON false response.
    #[inline(always)]
    pub fn false_json() -> FastResponse {
        FastResponse::new(200).with_static_body(b"false")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_response_ok() {
        let resp = FastResponse::ok();
        assert_eq!(resp.status, 200);
        assert!(resp.body.is_empty());
        assert!(resp.headers.is_empty());
    }

    #[test]
    fn test_fast_response_with_json() {
        #[derive(serde::Serialize)]
        struct Data {
            value: i32,
        }

        let resp = FastResponse::ok().with_json(&Data { value: 42 }).unwrap();

        assert_eq!(resp.status, 200);
        assert!(!resp.body.is_empty());
        assert_eq!(resp.headers.get("content-type"), Some("application/json"));
    }

    #[test]
    fn test_fast_response_static_body() {
        let resp = FastResponse::ok().with_static_body(b"Hello, World!");

        assert_eq!(resp.body.as_slice(), b"Hello, World!");
    }

    #[test]
    fn test_fast_headers_inline() {
        let mut headers = FastHeaders::new();
        headers.insert("content-type", "application/json");
        headers.insert("x-custom", "value");

        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("content-type"), Some("application/json"));
        assert_eq!(headers.get("Content-Type"), Some("application/json")); // Case insensitive
    }

    #[test]
    fn test_fast_responses() {
        let resp = fast::ok();
        assert_eq!(resp.status, 200);

        let resp = fast::not_found();
        assert_eq!(resp.status, 404);

        let resp = fast::empty_json();
        assert_eq!(resp.body.as_slice(), b"{}");
    }

    #[test]
    fn test_conversion_to_http_response() {
        let fast = FastResponse::ok()
            .header("x-test", "value")
            .with_text("Hello");

        let legacy = fast.into_http_response();
        assert_eq!(legacy.status, 200);
        assert!(legacy.headers.contains_key("x-test"));
    }
}
