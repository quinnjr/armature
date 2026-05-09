//! Vectored I/O for HTTP Responses
//!
//! This module provides vectored write support using `writev()` to send
//! multiple buffers (headers + body) in a single syscall, reducing overhead.
//!
//! ## Performance Benefits
//!
//! Traditional approach:
//! ```text
//! write(headers)  → syscall + context switch
//! write(body)     → syscall + context switch
//! ```
//!
//! Vectored I/O:
//! ```text
//! writev([headers, body]) → single syscall
//! ```
//!
//! This can provide +2-3% throughput improvement by:
//! - Reducing syscall overhead
//! - Better TCP segment utilization
//! - Avoiding Nagle's algorithm delays
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::vectored_io::{VectoredResponse, ResponseChunks};
//!
//! // Build response with separate header and body buffers
//! let response = VectoredResponse::new(200)
//!     .header("Content-Type", "application/json")
//!     .body(json_bytes);
//!
//! // Get IoSlices for writev
//! let chunks = response.into_io_slices();
//! ```

use bytes::{BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use std::io::IoSlice;

/// Maximum number of IoSlice buffers for vectored writes.
/// Most responses have: status line + headers + body = 3 parts.
/// We allow up to 16 for chunked responses with multiple parts.
pub const MAX_IO_SLICES: usize = 16;

/// Pre-computed common HTTP status lines (avoids formatting on hot path)
static STATUS_200: &[u8] = b"HTTP/1.1 200 OK\r\n";
static STATUS_201: &[u8] = b"HTTP/1.1 201 Created\r\n";
static STATUS_204: &[u8] = b"HTTP/1.1 204 No Content\r\n";
static STATUS_301: &[u8] = b"HTTP/1.1 301 Moved Permanently\r\n";
static STATUS_302: &[u8] = b"HTTP/1.1 302 Found\r\n";
static STATUS_304: &[u8] = b"HTTP/1.1 304 Not Modified\r\n";
static STATUS_400: &[u8] = b"HTTP/1.1 400 Bad Request\r\n";
static STATUS_401: &[u8] = b"HTTP/1.1 401 Unauthorized\r\n";
static STATUS_403: &[u8] = b"HTTP/1.1 403 Forbidden\r\n";
static STATUS_404: &[u8] = b"HTTP/1.1 404 Not Found\r\n";
static STATUS_405: &[u8] = b"HTTP/1.1 405 Method Not Allowed\r\n";
static STATUS_500: &[u8] = b"HTTP/1.1 500 Internal Server Error\r\n";
static STATUS_502: &[u8] = b"HTTP/1.1 502 Bad Gateway\r\n";
static STATUS_503: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\n";

/// Header separator
static HEADER_SEP: &[u8] = b": ";
/// Line ending
static CRLF: &[u8] = b"\r\n";

/// Get pre-computed status line for common status codes.
#[inline]
pub fn status_line(status: u16) -> &'static [u8] {
    match status {
        200 => STATUS_200,
        201 => STATUS_201,
        204 => STATUS_204,
        301 => STATUS_301,
        302 => STATUS_302,
        304 => STATUS_304,
        400 => STATUS_400,
        401 => STATUS_401,
        403 => STATUS_403,
        404 => STATUS_404,
        405 => STATUS_405,
        500 => STATUS_500,
        502 => STATUS_502,
        503 => STATUS_503,
        _ => STATUS_200, // Fallback, caller should use format_status_line
    }
}

/// Check if status code has a pre-computed status line.
#[inline]
pub fn has_precomputed_status(status: u16) -> bool {
    matches!(
        status,
        200 | 201 | 204 | 301 | 302 | 304 | 400 | 401 | 403 | 404 | 405 | 500 | 502 | 503
    )
}

/// Format a status line for non-common status codes.
#[inline]
pub fn format_status_line(status: u16, buf: &mut BytesMut) {
    buf.extend_from_slice(b"HTTP/1.1 ");
    // Write status code directly
    let mut n = status;
    let d2 = (n % 10) as u8 + b'0';
    n /= 10;
    let d1 = (n % 10) as u8 + b'0';
    n /= 10;
    let d0 = n as u8 + b'0';
    buf.put_u8(d0);
    buf.put_u8(d1);
    buf.put_u8(d2);
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(status_reason(status).as_bytes());
    buf.extend_from_slice(CRLF);
}

/// Get reason phrase for status code.
#[inline]
fn status_reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// Response chunks for vectored I/O.
///
/// This holds all the buffers that make up an HTTP response,
/// ready to be written with a single `writev()` call.
#[derive(Debug)]
pub struct ResponseChunks {
    /// Status line buffer (either static or formatted)
    status_line: StatusLine,
    /// Headers buffer (formatted as "Name: Value\r\n...")
    headers: BytesMut,
    /// Header terminator (\r\n)
    header_end: &'static [u8],
    /// Body buffer
    body: Bytes,
}

#[derive(Debug)]
enum StatusLine {
    Static(&'static [u8]),
    Dynamic(BytesMut),
}

impl StatusLine {
    fn as_slice(&self) -> &[u8] {
        match self {
            StatusLine::Static(s) => s,
            StatusLine::Dynamic(b) => b,
        }
    }
}

impl ResponseChunks {
    /// Create new response chunks from status, headers, and body.
    pub fn new(status: u16, headers: &HashMap<String, String>, body: Bytes) -> Self {
        Self::with_cookies(status, headers, &[], body)
    }

    pub fn with_cookies(
        status: u16,
        headers: &HashMap<String, String>,
        cookies: &[String],
        body: Bytes,
    ) -> Self {
        // Status line
        let status_line = if has_precomputed_status(status) {
            StatusLine::Static(status_line(status))
        } else {
            let mut buf = BytesMut::with_capacity(32);
            format_status_line(status, &mut buf);
            StatusLine::Dynamic(buf)
        };

        // Headers - estimate size: avg 30 bytes per header
        let mut headers_buf = BytesMut::with_capacity((headers.len() + cookies.len()) * 30 + 32);
        for (name, value) in headers {
            headers_buf.extend_from_slice(name.as_bytes());
            headers_buf.extend_from_slice(HEADER_SEP);
            headers_buf.extend_from_slice(value.as_bytes());
            headers_buf.extend_from_slice(CRLF);
        }
        for cookie_value in cookies {
            headers_buf.extend_from_slice(b"Set-Cookie");
            headers_buf.extend_from_slice(HEADER_SEP);
            headers_buf.extend_from_slice(cookie_value.as_bytes());
            headers_buf.extend_from_slice(CRLF);
        }

        // Add Content-Length if body is not empty
        if !body.is_empty() {
            headers_buf.extend_from_slice(b"Content-Length: ");
            // Format number without allocation
            let len = body.len();
            let mut num_buf = [0u8; 20];
            let num_str = format_usize(len, &mut num_buf);
            headers_buf.extend_from_slice(num_str);
            headers_buf.extend_from_slice(CRLF);
        }

        Self {
            status_line,
            headers: headers_buf,
            header_end: CRLF,
            body,
        }
    }

    /// Get the total size of the response.
    #[inline]
    pub fn total_len(&self) -> usize {
        self.status_line.as_slice().len()
            + self.headers.len()
            + self.header_end.len()
            + self.body.len()
    }

    /// Get IoSlices for vectored write.
    ///
    /// Returns slices that can be passed to `writev()`.
    #[inline]
    pub fn as_io_slices(&self) -> [IoSlice<'_>; 4] {
        [
            IoSlice::new(self.status_line.as_slice()),
            IoSlice::new(&self.headers),
            IoSlice::new(self.header_end),
            IoSlice::new(&self.body),
        ]
    }

    /// Get number of chunks.
    #[inline]
    pub fn chunk_count(&self) -> usize {
        if self.body.is_empty() { 3 } else { 4 }
    }

    /// Serialize to a single contiguous buffer.
    ///
    /// Use this when vectored I/O is not available.
    pub fn to_bytes(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(self.total_len());
        buf.extend_from_slice(self.status_line.as_slice());
        buf.extend_from_slice(&self.headers);
        buf.extend_from_slice(self.header_end);
        buf.extend_from_slice(&self.body);
        buf.freeze()
    }
}

/// Format usize to bytes without allocation.
#[inline]
fn format_usize(n: usize, buf: &mut [u8; 20]) -> &[u8] {
    if n == 0 {
        buf[19] = b'0';
        return &buf[19..];
    }

    let mut n = n;
    let mut pos = 20;
    while n > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    &buf[pos..]
}

/// A response builder optimized for vectored I/O.
///
/// Builds responses with separate buffers for efficient `writev()`.
#[derive(Debug)]
pub struct VectoredResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Option<Bytes>,
}

impl VectoredResponse {
    /// Create a new response with status code.
    #[inline]
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::with_capacity(8),
            body: None,
        }
    }

    /// Create a 200 OK response.
    #[inline]
    pub fn ok() -> Self {
        Self::new(200)
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

    /// Set body from Bytes.
    #[inline]
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Set body from JSON.
    #[inline]
    pub fn body_json<T: serde::Serialize>(self, value: &T) -> Result<Self, crate::Error> {
        let json =
            crate::json::to_vec(value).map_err(|e| crate::Error::Serialization(e.to_string()))?;
        Ok(self
            .content_type("application/json")
            .body(Bytes::from(json)))
    }

    /// Build into ResponseChunks for vectored I/O.
    #[inline]
    pub fn build(self) -> ResponseChunks {
        let headers: HashMap<String, String> = self.headers.into_iter().collect();
        ResponseChunks::new(self.status, &headers, self.body.unwrap_or_default())
    }

    /// Build and serialize to contiguous bytes.
    #[inline]
    pub fn build_bytes(self) -> Bytes {
        self.build().to_bytes()
    }
}

impl Default for VectoredResponse {
    fn default() -> Self {
        Self::ok()
    }
}

// ============================================================================
// Conversion from HttpResponse
// ============================================================================

impl From<crate::HttpResponse> for ResponseChunks {
    fn from(response: crate::HttpResponse) -> Self {
        let status = response.status;
        let headers = response.headers.to_hashmap();
        let cookies = response.cookies.clone();
        let body = response.into_body_bytes();
        Self::with_cookies(status, &headers, &cookies, body)
    }
}

impl crate::HttpResponse {
    /// Convert to vectored response chunks.
    ///
    /// Use this for efficient vectored writes when you have
    /// direct socket access.
    #[inline]
    pub fn into_chunks(self) -> ResponseChunks {
        ResponseChunks::from(self)
    }

    /// Get IoSlices for vectored write.
    ///
    /// This is a convenience method that creates ResponseChunks
    /// and returns the slices. For repeated use, prefer `into_chunks()`.
    #[inline]
    pub fn to_vectored(&self) -> ResponseChunks {
        let headers = self.headers.to_hashmap();
        ResponseChunks::with_cookies(self.status, &headers, &self.cookies, self.body_bytes())
    }
}

// ============================================================================
// Statistics
// ============================================================================

use std::sync::atomic::{AtomicU64, Ordering};

/// Statistics for vectored I/O operations.
#[derive(Debug, Default)]
pub struct VectoredIoStats {
    /// Total vectored writes
    writes: AtomicU64,
    /// Total bytes written
    bytes_written: AtomicU64,
    /// Responses with precomputed status lines
    precomputed_status: AtomicU64,
    /// Responses with dynamic status lines
    dynamic_status: AtomicU64,
}

impl VectoredIoStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a write.
    #[inline]
    pub fn record_write(&self, bytes: usize, precomputed: bool) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes_written
            .fetch_add(bytes as u64, Ordering::Relaxed);
        if precomputed {
            self.precomputed_status.fetch_add(1, Ordering::Relaxed);
        } else {
            self.dynamic_status.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get total writes.
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    /// Get total bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Get precomputed status line percentage.
    pub fn precomputed_percentage(&self) -> f64 {
        let total = self.writes();
        if total == 0 {
            return 0.0;
        }
        (self.precomputed_status.load(Ordering::Relaxed) as f64 / total as f64) * 100.0
    }
}

/// Global vectored I/O statistics.
static VECTORED_STATS: VectoredIoStats = VectoredIoStats {
    writes: AtomicU64::new(0),
    bytes_written: AtomicU64::new(0),
    precomputed_status: AtomicU64::new(0),
    dynamic_status: AtomicU64::new(0),
};

/// Get global vectored I/O statistics.
pub fn vectored_stats() -> &'static VectoredIoStats {
    &VECTORED_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_line_precomputed() {
        assert_eq!(status_line(200), b"HTTP/1.1 200 OK\r\n");
        assert_eq!(status_line(404), b"HTTP/1.1 404 Not Found\r\n");
        assert_eq!(status_line(500), b"HTTP/1.1 500 Internal Server Error\r\n");
    }

    #[test]
    fn test_format_status_line() {
        let mut buf = BytesMut::with_capacity(64);
        format_status_line(418, &mut buf);
        assert!(buf.starts_with(b"HTTP/1.1 418"));
    }

    #[test]
    fn test_format_usize() {
        let mut buf = [0u8; 20];
        assert_eq!(format_usize(0, &mut buf), b"0");
        assert_eq!(format_usize(123, &mut buf), b"123");
        assert_eq!(format_usize(1000000, &mut buf), b"1000000");
    }

    #[test]
    fn test_response_chunks_basic() {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "text/plain".to_string());

        let chunks = ResponseChunks::new(200, &headers, Bytes::from_static(b"Hello"));

        assert!(chunks.total_len() > 0);
        assert_eq!(chunks.chunk_count(), 4);
    }

    #[test]
    fn test_response_chunks_io_slices() {
        let mut headers = HashMap::new();
        headers.insert("X-Test".to_string(), "value".to_string());

        let chunks = ResponseChunks::new(200, &headers, Bytes::from_static(b"body"));
        let slices = chunks.as_io_slices();

        assert_eq!(slices.len(), 4);
        assert!(!slices[0].is_empty()); // Status line
        assert!(!slices[1].is_empty()); // Headers
    }

    #[test]
    fn test_response_chunks_to_bytes() {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "text/plain".to_string());

        let chunks = ResponseChunks::new(200, &headers, Bytes::from_static(b"Hello"));
        let bytes = chunks.to_bytes();

        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("HTTP/1.1 200 OK"));
        assert!(s.contains("Content-Type: text/plain"));
        assert!(s.contains("Hello"));
    }

    #[test]
    fn test_vectored_response_builder() {
        let chunks = VectoredResponse::ok()
            .header("X-Custom", "test")
            .body(Bytes::from_static(b"body data"))
            .build();

        let bytes = chunks.to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("X-Custom: test"));
        assert!(s.contains("body data"));
    }

    #[test]
    fn test_vectored_response_json() {
        #[derive(serde::Serialize)]
        struct Data {
            status: &'static str,
        }

        let chunks = VectoredResponse::ok()
            .body_json(&Data { status: "ok" })
            .unwrap()
            .build();

        let bytes = chunks.to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("application/json"));
        assert!(s.contains(r#""status":"ok""#));
    }

    #[test]
    fn test_http_response_to_chunks() {
        let response = crate::HttpResponse::ok()
            .with_header("X-Test".to_string(), "value".to_string())
            .with_body(b"test body".to_vec());

        let chunks = response.into_chunks();
        let bytes = chunks.to_bytes();
        let s = String::from_utf8_lossy(&bytes);

        assert!(s.contains("HTTP/1.1 200 OK"));
        assert!(s.contains("X-Test: value"));
        assert!(s.contains("test body"));
    }

    #[test]
    fn test_empty_body() {
        let chunks = ResponseChunks::new(204, &HashMap::new(), Bytes::new());
        assert_eq!(chunks.chunk_count(), 3); // No body chunk
    }

    #[test]
    fn test_content_length_header() {
        let chunks = ResponseChunks::new(200, &HashMap::new(), Bytes::from_static(b"12345"));
        let bytes = chunks.to_bytes();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Content-Length: 5"));
    }
}
