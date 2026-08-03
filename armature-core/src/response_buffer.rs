//! Pre-Allocated Response Buffer
//!
//! This module provides pre-allocated buffers for response bodies to avoid
//! reallocations during response building.
//!
//! ## Performance
//!
//! Most HTTP responses are small (< 1KB for JSON APIs). By pre-allocating
//! a default buffer size, we avoid the overhead of:
//! - Initial allocation when body is first written
//! - Reallocations as body grows
//! - Memory fragmentation from small allocations
//!
//! ## Default Sizes
//!
//! - `DEFAULT_RESPONSE_CAPACITY`: 512 bytes (covers most small JSON responses)
//! - `MEDIUM_RESPONSE_CAPACITY`: 4096 bytes (4KB, typical API response)
//! - `LARGE_RESPONSE_CAPACITY`: 65536 bytes (64KB, large responses)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::response_buffer::{ResponseBuffer, DEFAULT_RESPONSE_CAPACITY};
//!
//! // Pre-allocated buffer
//! let buf = ResponseBuffer::new();
//! buf.write(b"Hello, World!");
//!
//! // With custom capacity
//! let buf = ResponseBuffer::with_capacity(4096);
//! ```

use bytes::{BufMut, Bytes, BytesMut};
use std::ops::Deref;

/// Default pre-allocated response buffer size (512 bytes).
///
/// This covers most small JSON API responses like:
/// - `{"status": "ok"}`
/// - `{"id": 123, "name": "user"}`
/// - Simple error responses
pub const DEFAULT_RESPONSE_CAPACITY: usize = 512;

/// Medium response buffer size (4KB).
///
/// Good for typical API responses with moderate data.
pub const MEDIUM_RESPONSE_CAPACITY: usize = 4096;

/// Large response buffer size (64KB).
///
/// For large responses like paginated lists or bulk data.
pub const LARGE_RESPONSE_CAPACITY: usize = 65536;

/// A pre-allocated response buffer using `BytesMut`.
///
/// This provides a growable buffer that's pre-allocated to avoid
/// reallocations for typical response sizes.
#[derive(Debug)]
pub struct ResponseBuffer {
    inner: BytesMut,
}

impl ResponseBuffer {
    /// Create a new buffer with default capacity (512 bytes).
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: BytesMut::with_capacity(DEFAULT_RESPONSE_CAPACITY),
        }
    }

    /// Create a buffer with specified capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: BytesMut::with_capacity(capacity),
        }
    }

    /// Create a buffer for medium-sized responses (4KB).
    #[inline]
    pub fn medium() -> Self {
        Self::with_capacity(MEDIUM_RESPONSE_CAPACITY)
    }

    /// Create a buffer for large responses (64KB).
    #[inline]
    pub fn large() -> Self {
        Self::with_capacity(LARGE_RESPONSE_CAPACITY)
    }

    /// Get current length.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get remaining capacity before reallocation.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.inner.capacity() - self.inner.len()
    }

    /// Get total capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Write bytes to buffer.
    #[inline]
    pub fn write(&mut self, data: &[u8]) {
        self.inner.extend_from_slice(data);
    }

    /// Write a string to buffer.
    #[inline]
    pub fn write_str(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
    }

    /// Write using BufMut interface.
    #[inline]
    pub fn put(&mut self, data: &[u8]) {
        self.inner.put_slice(data);
    }

    /// Reserve additional capacity.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.inner.reserve(additional);
    }

    /// Clear the buffer (keeps capacity).
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Convert to `Bytes` (zero-copy).
    #[inline]
    pub fn freeze(self) -> Bytes {
        self.inner.freeze()
    }

    /// Get as byte slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }

    /// Get mutable reference to inner BytesMut.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut BytesMut {
        &mut self.inner
    }

    /// Consume and return inner BytesMut.
    #[inline]
    pub fn into_inner(self) -> BytesMut {
        self.inner
    }

    /// Split off the filled portion as Bytes.
    #[inline]
    pub fn split(&mut self) -> Bytes {
        self.inner.split().freeze()
    }
}

impl Default for ResponseBuffer {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for ResponseBuffer {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<[u8]> for ResponseBuffer {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

impl AsMut<BytesMut> for ResponseBuffer {
    #[inline]
    fn as_mut(&mut self) -> &mut BytesMut {
        &mut self.inner
    }
}

impl From<ResponseBuffer> for Bytes {
    #[inline]
    fn from(buf: ResponseBuffer) -> Self {
        buf.freeze()
    }
}

impl From<ResponseBuffer> for BytesMut {
    #[inline]
    fn from(buf: ResponseBuffer) -> Self {
        buf.inner
    }
}

impl std::io::Write for ResponseBuffer {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ============================================================================
// Response Builder with Pre-allocated Buffer
// ============================================================================

/// A response builder with pre-allocated buffer for efficient response creation.
///
/// This builder allocates a buffer upfront and allows building the response
/// body incrementally without reallocations.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::response_buffer::ResponseBuilder;
///
/// let response = ResponseBuilder::new()
///     .status(200)
///     .header("Content-Type", "application/json")
///     .body_json(&data)?
///     .build();
/// ```
#[derive(Debug)]
pub struct ResponseBuilder {
    status: u16,
    headers: Vec<(String, String)>,
    body: ResponseBuffer,
}

impl ResponseBuilder {
    /// Create a new builder with default buffer capacity.
    #[inline]
    pub fn new() -> Self {
        Self {
            status: 200,
            headers: Vec::with_capacity(8), // Most responses have <8 headers
            body: ResponseBuffer::new(),
        }
    }

    /// Create with custom buffer capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            status: 200,
            headers: Vec::with_capacity(8),
            body: ResponseBuffer::with_capacity(capacity),
        }
    }

    /// Set status code.
    #[inline]
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Add a header.
    #[inline]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set Content-Type header.
    #[inline]
    pub fn content_type(self, value: impl Into<String>) -> Self {
        self.header("Content-Type", value)
    }

    /// Set body from bytes.
    #[inline]
    pub fn body(mut self, data: &[u8]) -> Self {
        self.body.write(data);
        self
    }

    /// Set body from string.
    #[inline]
    pub fn body_str(mut self, s: &str) -> Self {
        self.body.write_str(s);
        self
    }

    /// Set body from JSON.
    #[inline]
    pub fn body_json<T: serde::Serialize>(mut self, value: &T) -> Result<Self, crate::Error> {
        let json =
            crate::json::to_vec(value).map_err(|e| crate::Error::Serialization(e.to_string()))?;
        self.body.write(&json);
        self.headers
            .push(("Content-Type".to_string(), "application/json".to_string()));
        Ok(self)
    }

    /// Get mutable reference to body buffer for direct writes.
    #[inline]
    pub fn body_mut(&mut self) -> &mut ResponseBuffer {
        &mut self.body
    }

    /// Build the final HttpResponse.
    #[inline]
    pub fn build(self) -> crate::HttpResponse {
        let mut response = crate::HttpResponse::new(self.status);
        for (name, value) in self.headers {
            response.headers.insert(name, value);
        }
        if !self.body.is_empty() {
            response = response.with_bytes_body(self.body.freeze());
        }
        response
    }
}

impl Default for ResponseBuilder {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Extension to HttpResponse
// ============================================================================

// Extension methods for HttpResponse are defined in http.rs
// to access private fields. Use the builder pattern here.

/// Create a response builder for more control over response creation.
#[inline]
pub fn response_builder() -> ResponseBuilder {
    ResponseBuilder::new()
}

/// Create a response builder with custom buffer capacity.
#[inline]
pub fn response_builder_with_capacity(capacity: usize) -> ResponseBuilder {
    ResponseBuilder::with_capacity(capacity)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_buffer_new() {
        let buf = ResponseBuffer::new();
        assert_eq!(buf.capacity(), DEFAULT_RESPONSE_CAPACITY);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_response_buffer_write() {
        let mut buf = ResponseBuffer::new();
        buf.write(b"Hello, ");
        buf.write(b"World!");
        assert_eq!(buf.as_slice(), b"Hello, World!");
    }

    #[test]
    fn test_response_buffer_no_realloc_small() {
        let mut buf = ResponseBuffer::new();
        let initial_cap = buf.capacity();

        // Write less than default capacity
        buf.write(b"Small response");

        // Capacity should not have grown
        assert_eq!(buf.capacity(), initial_cap);
    }

    #[test]
    fn test_response_buffer_freeze() {
        let mut buf = ResponseBuffer::new();
        buf.write(b"test data");
        let bytes = buf.freeze();
        assert_eq!(&*bytes, b"test data");
    }

    #[test]
    fn test_response_buffer_medium() {
        let buf = ResponseBuffer::medium();
        assert_eq!(buf.capacity(), MEDIUM_RESPONSE_CAPACITY);
    }

    #[test]
    fn test_response_buffer_large() {
        let buf = ResponseBuffer::large();
        assert_eq!(buf.capacity(), LARGE_RESPONSE_CAPACITY);
    }

    #[test]
    fn test_response_builder() {
        let response = ResponseBuilder::new()
            .status(201)
            .header("X-Custom", "value")
            .body_str("Created")
            .build();

        assert_eq!(response.status, 201);
        assert_eq!(response.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_response_builder_json() {
        #[derive(serde::Serialize)]
        struct Data {
            message: &'static str,
        }

        let response = ResponseBuilder::new()
            .status(200)
            .body_json(&Data { message: "ok" })
            .unwrap()
            .build();

        assert_eq!(response.status, 200);
        assert!(
            response
                .headers
                .get("Content-Type")
                .unwrap()
                .contains("json")
        );
    }

    /// The `capacity` argument no longer reserves body space: `Bytes` is handed
    /// a finished buffer rather than grown in place, so there is no spare
    /// capacity to reserve. The constructors are kept for source compatibility
    /// and must still produce an empty body with the right status.
    #[test]
    fn test_http_response_with_capacity() {
        let response = crate::HttpResponse::with_capacity(200, 1024);
        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
    }

    #[test]
    fn test_http_response_preallocated() {
        let response = crate::HttpResponse::ok_preallocated();
        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
    }

    #[test]
    fn test_response_builder_standalone() {
        let response = response_builder()
            .status(404)
            .content_type("text/plain")
            .body_str("Not Found")
            .build();

        assert_eq!(response.status, 404);
    }

    #[test]
    fn test_response_buffer_io_write() {
        use std::io::Write;

        let mut buf = ResponseBuffer::new();
        write!(buf, "Hello, World!").unwrap();
        assert_eq!(buf.as_slice(), b"Hello, World!");
    }
}
