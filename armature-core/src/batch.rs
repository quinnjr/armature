//! HTTP Request Batching
//!
//! This module provides efficient batch reading and parsing of HTTP requests
//! from socket buffers. By reading multiple requests in a single syscall and
//! parsing them together, we reduce overhead and improve throughput.
//!
//! ## How Batching Works
//!
//! 1. Read a large chunk from the socket into a buffer
//! 2. Parse multiple complete HTTP requests from the buffer
//! 3. Process all requests (concurrently if configured)
//! 4. Batch responses for efficient writing
//!
//! ## Performance Impact
//!
//! - Reduces syscall overhead (fewer read() calls)
//! - Improves cache efficiency (process related data together)
//! - Enables SIMD parsing across multiple requests
//! - Better utilization of network buffers
//!
//! ## Configuration
//!
//! ```rust,ignore
//! use armature_core::batch::{BatchConfig, BatchReader};
//!
//! let config = BatchConfig::builder()
//!     .buffer_size(65536)      // 64KB read buffer
//!     .max_requests(32)        // Max requests per batch
//!     .parse_timeout_ms(100)   // Max time to accumulate batch
//!     .build();
//! ```

use bytes::{Bytes, BytesMut};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Maximum number of headers the parser can handle per request.
///
/// This is the size of the fixed `httparse` header array; configured
/// `max_headers` values above this are clamped to it.
pub const MAX_SUPPORTED_HEADERS: usize = 100;

// ============================================================================
// Batch Configuration
// ============================================================================

/// Configuration for request batching
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Size of the read buffer in bytes
    pub buffer_size: usize,

    /// Maximum number of requests per batch
    pub max_requests: usize,

    /// Maximum time to wait for batch to fill (milliseconds)
    pub parse_timeout_ms: u64,

    /// Minimum number of requests before processing a batch
    pub min_batch_size: usize,

    /// Maximum request size (for DoS prevention)
    pub max_request_size: usize,

    /// Maximum header count per request
    ///
    /// The parser uses a fixed array of [`MAX_SUPPORTED_HEADERS`] header
    /// slots, so values above that are effectively clamped to it.
    pub max_headers: usize,

    /// Enable adaptive batch sizing based on load
    pub adaptive_batching: bool,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            buffer_size: 65536, // 64KB
            max_requests: 32,
            parse_timeout_ms: 10,      // 10ms max wait
            min_batch_size: 1,         // Process at least 1
            max_request_size: 1048576, // 1MB
            max_headers: 100,
            adaptive_batching: true,
        }
    }
}

impl BatchConfig {
    /// Create a new builder
    pub fn builder() -> BatchConfigBuilder {
        BatchConfigBuilder::default()
    }

    /// High-throughput configuration for batch processing
    pub fn high_throughput() -> Self {
        Self {
            buffer_size: 131072, // 128KB
            max_requests: 64,
            parse_timeout_ms: 20,
            min_batch_size: 4,
            max_request_size: 2097152, // 2MB
            max_headers: 100,
            adaptive_batching: true,
        }
    }

    /// Low-latency configuration (minimal batching)
    pub fn low_latency() -> Self {
        Self {
            buffer_size: 16384, // 16KB
            max_requests: 8,
            parse_timeout_ms: 1,
            min_batch_size: 1,
            max_request_size: 524288, // 512KB
            max_headers: 64,
            adaptive_batching: false,
        }
    }

    /// Memory-efficient configuration
    pub fn memory_efficient() -> Self {
        Self {
            buffer_size: 32768, // 32KB
            max_requests: 16,
            parse_timeout_ms: 5,
            min_batch_size: 2,
            max_request_size: 524288, // 512KB
            max_headers: 50,
            adaptive_batching: true,
        }
    }
}

/// Builder for BatchConfig
#[derive(Debug, Clone, Default)]
pub struct BatchConfigBuilder {
    config: BatchConfig,
}

impl BatchConfigBuilder {
    /// Set the read buffer size
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    /// Set maximum requests per batch
    pub fn max_requests(mut self, max: usize) -> Self {
        self.config.max_requests = max;
        self
    }

    /// Set parse timeout in milliseconds
    pub fn parse_timeout_ms(mut self, ms: u64) -> Self {
        self.config.parse_timeout_ms = ms;
        self
    }

    /// Set minimum batch size before processing
    pub fn min_batch_size(mut self, min: usize) -> Self {
        self.config.min_batch_size = min;
        self
    }

    /// Set maximum request size
    pub fn max_request_size(mut self, size: usize) -> Self {
        self.config.max_request_size = size;
        self
    }

    /// Set maximum headers per request
    ///
    /// Values above [`MAX_SUPPORTED_HEADERS`] are clamped to it, since the
    /// parser uses a fixed-size header array.
    pub fn max_headers(mut self, max: usize) -> Self {
        self.config.max_headers = max.min(MAX_SUPPORTED_HEADERS);
        self
    }

    /// Enable or disable adaptive batching
    pub fn adaptive_batching(mut self, enable: bool) -> Self {
        self.config.adaptive_batching = enable;
        self
    }

    /// Build the configuration
    pub fn build(self) -> BatchConfig {
        self.config
    }
}

// ============================================================================
// Parsed Request
// ============================================================================

/// A parsed HTTP request from a batch (borrowed version)
#[derive(Debug)]
pub struct ParsedRequest<'a> {
    /// HTTP method (GET, POST, etc.)
    pub method: &'a str,
    /// Request path
    pub path: &'a str,
    /// HTTP version (e.g., "1.1")
    pub version: &'a str,
    /// Request headers as (name, value) pairs
    pub headers: Vec<(&'a str, &'a [u8])>,
    /// Request body (may be empty)
    pub body: &'a [u8],
    /// Total bytes consumed for this request
    pub bytes_consumed: usize,
}

/// An owned parsed HTTP request (no lifetime dependencies)
#[derive(Debug, Clone)]
pub struct OwnedParsedRequest {
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Request path
    pub path: String,
    /// HTTP version (e.g., "1.1")
    pub version: String,
    /// Request headers as (name, value) pairs
    pub headers: Vec<(String, Vec<u8>)>,
    /// Request body
    pub body: Bytes,
    /// Total bytes consumed for this request
    pub bytes_consumed: usize,
}

impl OwnedParsedRequest {
    /// Get a header value by name (case-insensitive)
    #[inline]
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_slice())
    }

    /// Get Content-Length header value
    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|s| s.trim().parse().ok())
    }

    /// Check if this is a keep-alive connection
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        if self.version == "1.1" {
            !self
                .header("connection")
                .map(|v| v.eq_ignore_ascii_case(b"close"))
                .unwrap_or(false)
        } else {
            self.header("connection")
                .map(|v| v.eq_ignore_ascii_case(b"keep-alive"))
                .unwrap_or(false)
        }
    }

    /// Convert to HttpRequest
    pub fn to_http_request(&self) -> crate::HttpRequest {
        let mut req = crate::HttpRequest::new(self.method.clone(), self.path.clone());

        for (name, value) in &self.headers {
            if let Ok(v) = std::str::from_utf8(value) {
                req.headers.insert(name.clone(), v.to_string());
            }
        }

        if !self.body.is_empty() {
            req.set_body_bytes(self.body.clone());
        }

        req
    }
}

/// Result of parsing a batch of requests (owned version)
#[derive(Debug)]
pub struct ParsedBatch {
    /// Successfully parsed requests
    pub requests: Vec<OwnedParsedRequest>,
    /// Total bytes consumed from the buffer
    pub bytes_consumed: usize,
    /// Whether there's partial data remaining
    pub partial: bool,
    /// Parse error if any
    pub error: Option<BatchParseError>,
}

impl<'a> ParsedRequest<'a> {
    /// Get a header value by name (case-insensitive)
    #[inline]
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        let name_lower = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name_lower))
            .map(|(_, v)| *v)
    }

    /// Get Content-Length header value
    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|s| s.trim().parse().ok())
    }

    /// Check if this is a keep-alive connection
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        if self.version == "1.1" {
            // HTTP/1.1 defaults to keep-alive
            !self
                .header("connection")
                .map(|v| v.eq_ignore_ascii_case(b"close"))
                .unwrap_or(false)
        } else {
            // HTTP/1.0 requires explicit keep-alive
            self.header("connection")
                .map(|v| v.eq_ignore_ascii_case(b"keep-alive"))
                .unwrap_or(false)
        }
    }

    /// Convert to owned HttpRequest
    pub fn to_http_request(&self) -> crate::HttpRequest {
        let mut req = crate::HttpRequest::new(self.method.to_string(), self.path.to_string());

        for (name, value) in &self.headers {
            if let Ok(v) = std::str::from_utf8(value) {
                req.headers.insert(name.to_string(), v.to_string());
            }
        }

        if !self.body.is_empty() {
            req.set_body_bytes(Bytes::copy_from_slice(self.body));
        }

        req
    }
}

// ============================================================================
// Batch Parser
// ============================================================================

/// Parse result for a batch of requests
#[derive(Debug)]
pub struct BatchParseResult<'a> {
    /// Successfully parsed requests
    pub requests: Vec<ParsedRequest<'a>>,
    /// Total bytes consumed from the buffer
    pub bytes_consumed: usize,
    /// Whether there's partial data remaining
    pub partial: bool,
    /// Parse error if any
    pub error: Option<BatchParseError>,
}

/// Error during batch parsing
#[derive(Debug, Clone)]
pub enum BatchParseError {
    /// Request too large
    RequestTooLarge(usize),
    /// Too many headers
    TooManyHeaders(usize),
    /// Invalid HTTP syntax
    InvalidSyntax(String),
    /// Buffer overflow
    BufferOverflow,
}

impl std::fmt::Display for BatchParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestTooLarge(size) => write!(f, "Request too large: {} bytes", size),
            Self::TooManyHeaders(count) => write!(f, "Too many headers: {}", count),
            Self::InvalidSyntax(msg) => write!(f, "Invalid HTTP syntax: {}", msg),
            Self::BufferOverflow => write!(f, "Buffer overflow"),
        }
    }
}

impl std::error::Error for BatchParseError {}

/// Batch parser for HTTP/1.1 requests
pub struct BatchParser {
    config: BatchConfig,
}

impl BatchParser {
    /// Create a new batch parser with configuration
    pub fn new(config: BatchConfig) -> Self {
        Self { config }
    }

    /// Parse multiple HTTP requests from a buffer
    ///
    /// Returns all complete requests found in the buffer.
    /// Partial requests are indicated by `partial: true` in the result.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let parser = BatchParser::new(BatchConfig::default());
    /// let buffer = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\nGET /api HTTP/1.1\r\nHost: example.com\r\n\r\n";
    /// let result = parser.parse_batch(buffer);
    /// assert_eq!(result.requests.len(), 2);
    /// ```
    #[inline]
    pub fn parse_batch<'a>(&self, buffer: &'a [u8]) -> BatchParseResult<'a> {
        let mut requests = Vec::with_capacity(self.config.max_requests);
        let mut offset = 0;
        let mut error = None;

        while offset < buffer.len() && requests.len() < self.config.max_requests {
            match self.parse_single(&buffer[offset..]) {
                Ok(Some((req, consumed))) => {
                    // Check size limits
                    if consumed > self.config.max_request_size {
                        error = Some(BatchParseError::RequestTooLarge(consumed));
                        break;
                    }
                    if req.headers.len() > self.config.max_headers {
                        error = Some(BatchParseError::TooManyHeaders(req.headers.len()));
                        break;
                    }

                    requests.push(req);
                    offset += consumed;
                }
                Ok(None) => {
                    // Partial request, stop parsing
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }

        BatchParseResult {
            requests,
            bytes_consumed: offset,
            partial: offset < buffer.len() && error.is_none(),
            error,
        }
    }

    /// Parse a single HTTP request from the buffer
    ///
    /// Returns the parsed request and bytes consumed, or None if incomplete.
    #[inline]
    fn parse_single<'a>(
        &self,
        buffer: &'a [u8],
    ) -> Result<Option<(ParsedRequest<'a>, usize)>, BatchParseError> {
        // Need at least minimal request line
        if buffer.len() < 16 {
            return Ok(None);
        }

        // Use httparse for efficient parsing
        let mut headers = [httparse::EMPTY_HEADER; MAX_SUPPORTED_HEADERS];
        let mut req = httparse::Request::new(&mut headers);

        match req.parse(buffer) {
            Ok(httparse::Status::Complete(header_len)) => {
                let method = req
                    .method
                    .ok_or_else(|| BatchParseError::InvalidSyntax("Missing method".to_string()))?;
                let path = req
                    .path
                    .ok_or_else(|| BatchParseError::InvalidSyntax("Missing path".to_string()))?;
                let version = match req.version {
                    Some(0) => "1.0",
                    Some(1) => "1.1",
                    _ => "1.1",
                };

                // Collect headers
                let parsed_headers: Vec<(&str, &[u8])> = req
                    .headers
                    .iter()
                    .take_while(|h| !h.name.is_empty())
                    .map(|h| (h.name, h.value))
                    .collect();

                // The batch parser only understands Content-Length framing.
                // Treating a chunked request as body-less would re-parse its
                // chunk bytes as the next pipelined request (request desync),
                // so reject Transfer-Encoding explicitly.
                if parsed_headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("transfer-encoding"))
                {
                    return Err(BatchParseError::InvalidSyntax(
                        "Transfer-Encoding is not supported by the batch parser".to_string(),
                    ));
                }

                // Determine body length
                let content_length = parsed_headers
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| std::str::from_utf8(v).ok())
                    .and_then(|s| s.trim().parse::<usize>().ok())
                    .unwrap_or(0);

                // Check if we have complete body
                let total_len = header_len + content_length;
                if buffer.len() < total_len {
                    // A request larger than the read buffer can never
                    // complete: returning Ok(None) would stall the
                    // connection forever waiting for bytes that cannot fit.
                    if total_len > self.config.buffer_size {
                        return Err(BatchParseError::BufferOverflow);
                    }
                    return Ok(None); // Incomplete body
                }

                let body = &buffer[header_len..total_len];

                Ok(Some((
                    ParsedRequest {
                        method,
                        path,
                        version,
                        headers: parsed_headers,
                        body,
                        bytes_consumed: total_len,
                    },
                    total_len,
                )))
            }
            Ok(httparse::Status::Partial) => Ok(None),
            Err(e) => Err(BatchParseError::InvalidSyntax(e.to_string())),
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &BatchConfig {
        &self.config
    }
}

// ============================================================================
// Batch Reader Statistics
// ============================================================================

/// Statistics for batch reading operations
#[derive(Debug, Default)]
pub struct BatchStats {
    /// Total batches processed
    batches_processed: AtomicU64,
    /// Total requests in batches
    total_requests: AtomicU64,
    /// Total bytes read
    total_bytes_read: AtomicU64,
    /// Average batch size (requests * 100 for precision)
    avg_batch_size: AtomicU64,
    /// Maximum batch size seen
    max_batch_size: AtomicUsize,
    /// Parse errors encountered
    parse_errors: AtomicU64,
}

impl BatchStats {
    /// Create new batch statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a processed batch
    #[inline]
    pub fn record_batch(&self, request_count: usize, bytes_read: usize) {
        self.batches_processed.fetch_add(1, Ordering::Relaxed);
        self.total_requests
            .fetch_add(request_count as u64, Ordering::Relaxed);
        self.total_bytes_read
            .fetch_add(bytes_read as u64, Ordering::Relaxed);

        // Update average (exponential moving average)
        let current = self.avg_batch_size.load(Ordering::Relaxed);
        let new_avg = (current * 95 + (request_count as u64 * 100) * 5) / 100;
        self.avg_batch_size.store(new_avg, Ordering::Relaxed);

        // Update max
        self.max_batch_size
            .fetch_max(request_count, Ordering::Relaxed);
    }

    /// Record a parse error
    #[inline]
    pub fn record_error(&self) {
        self.parse_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total batches processed
    #[inline]
    pub fn batches_processed(&self) -> u64 {
        self.batches_processed.load(Ordering::Relaxed)
    }

    /// Get total requests processed
    #[inline]
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Get total bytes read
    #[inline]
    pub fn total_bytes_read(&self) -> u64 {
        self.total_bytes_read.load(Ordering::Relaxed)
    }

    /// Get average batch size
    #[inline]
    pub fn avg_batch_size(&self) -> f64 {
        self.avg_batch_size.load(Ordering::Relaxed) as f64 / 100.0
    }

    /// Get maximum batch size
    #[inline]
    pub fn max_batch_size(&self) -> usize {
        self.max_batch_size.load(Ordering::Relaxed)
    }

    /// Get parse error count
    #[inline]
    pub fn parse_errors(&self) -> u64 {
        self.parse_errors.load(Ordering::Relaxed)
    }
}

// ============================================================================
// Batch Read Buffer
// ============================================================================

/// A reusable buffer for batch reading from sockets
#[derive(Debug)]
pub struct BatchBuffer {
    /// The underlying buffer
    buffer: BytesMut,
    /// Configuration
    config: BatchConfig,
    /// Read position (start of unprocessed data)
    read_pos: usize,
    /// Write position (end of valid data)
    write_pos: usize,
}

impl BatchBuffer {
    /// Create a new batch buffer
    pub fn new(config: BatchConfig) -> Self {
        Self {
            buffer: BytesMut::with_capacity(config.buffer_size),
            config,
            read_pos: 0,
            write_pos: 0,
        }
    }

    /// Get the remaining capacity for writing
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.config.buffer_size - self.write_pos
    }

    /// Get the unprocessed data as a slice
    #[inline]
    pub fn unprocessed(&self) -> &[u8] {
        &self.buffer[self.read_pos..self.write_pos]
    }

    /// Get a mutable slice for writing new data
    #[inline]
    pub fn write_slice(&mut self) -> &mut [u8] {
        // Ensure buffer is large enough
        if self.buffer.len() < self.config.buffer_size {
            self.buffer.resize(self.config.buffer_size, 0);
        }
        &mut self.buffer[self.write_pos..self.config.buffer_size]
    }

    /// Mark bytes as written
    #[inline]
    pub fn advance_write(&mut self, count: usize) {
        self.write_pos += count;
    }

    /// Mark bytes as processed
    #[inline]
    pub fn advance_read(&mut self, count: usize) {
        self.read_pos += count;
    }

    /// Compact the buffer by moving unprocessed data to the front
    #[inline]
    pub fn compact(&mut self) {
        if self.read_pos > 0 {
            let remaining = self.write_pos - self.read_pos;
            if remaining > 0 {
                self.buffer.copy_within(self.read_pos..self.write_pos, 0);
            }
            self.read_pos = 0;
            self.write_pos = remaining;
        }
    }

    /// Reset the buffer
    #[inline]
    pub fn reset(&mut self) {
        self.read_pos = 0;
        self.write_pos = 0;
    }

    /// Check if buffer is empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.read_pos >= self.write_pos
    }

    /// Get unprocessed data length
    #[inline]
    pub fn len(&self) -> usize {
        self.write_pos - self.read_pos
    }
}

// ============================================================================
// High-Level Batch Reader
// ============================================================================

/// High-level batch reader that combines buffer management and parsing
pub struct BatchReader {
    /// Parser for HTTP requests
    parser: BatchParser,
    /// Buffer for accumulating data
    buffer: BatchBuffer,
    /// Statistics
    stats: BatchStats,
}

impl BatchReader {
    /// Create a new batch reader
    pub fn new(config: BatchConfig) -> Self {
        Self {
            parser: BatchParser::new(config.clone()),
            buffer: BatchBuffer::new(config),
            stats: BatchStats::new(),
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &BatchConfig {
        self.parser.config()
    }

    /// Get statistics
    pub fn stats(&self) -> &BatchStats {
        &self.stats
    }

    /// Get a mutable slice for reading data into
    #[inline]
    pub fn read_buffer(&mut self) -> &mut [u8] {
        // Compact if needed
        if self.buffer.remaining_capacity() < 4096 {
            self.buffer.compact();
        }
        self.buffer.write_slice()
    }

    /// Notify that data has been read into the buffer
    #[inline]
    pub fn data_received(&mut self, count: usize) {
        self.buffer.advance_write(count);
    }

    /// Parse and return available requests
    ///
    /// Returns parsed requests and clears them from the buffer.
    /// The returned requests reference data in the internal buffer,
    /// so must be processed before the next read operation.
    #[inline]
    pub fn parse_available(&mut self) -> ParsedBatch {
        let data = self.buffer.unprocessed();
        let result = self.parser.parse_batch(data);

        // Collect bytes consumed and stats before borrowing ends
        let bytes_consumed = result.bytes_consumed;
        let request_count = result.requests.len();
        let has_error = result.error.is_some();
        let partial = result.partial;

        // Convert to owned requests
        let requests: Vec<OwnedParsedRequest> = result
            .requests
            .into_iter()
            .map(|r| OwnedParsedRequest {
                method: r.method.to_string(),
                path: r.path.to_string(),
                version: r.version.to_string(),
                headers: r
                    .headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_vec()))
                    .collect(),
                body: Bytes::copy_from_slice(r.body),
                bytes_consumed: r.bytes_consumed,
            })
            .collect();

        let error = result.error;

        // Now we can mutate the buffer
        self.buffer.advance_read(bytes_consumed);

        // Update statistics
        if request_count > 0 {
            self.stats.record_batch(request_count, bytes_consumed);
        }
        if has_error {
            self.stats.record_error();
        }

        ParsedBatch {
            requests,
            bytes_consumed,
            partial,
            error,
        }
    }

    /// Reset the reader for a new connection
    #[inline]
    pub fn reset(&mut self) {
        self.buffer.reset();
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_config_builder() {
        let config = BatchConfig::builder()
            .buffer_size(32768)
            .max_requests(16)
            .parse_timeout_ms(5)
            .build();

        assert_eq!(config.buffer_size, 32768);
        assert_eq!(config.max_requests, 16);
        assert_eq!(config.parse_timeout_ms, 5);
    }

    #[test]
    fn test_parse_single_request() {
        let parser = BatchParser::new(BatchConfig::default());
        let request = b"GET /api/users HTTP/1.1\r\nHost: example.com\r\nContent-Length: 0\r\n\r\n";

        let result = parser.parse_batch(request);

        assert_eq!(result.requests.len(), 1);
        assert_eq!(result.requests[0].method, "GET");
        assert_eq!(result.requests[0].path, "/api/users");
        assert_eq!(result.requests[0].version, "1.1");
        assert!(!result.partial);
    }

    #[test]
    fn test_parse_multiple_requests() {
        let parser = BatchParser::new(BatchConfig::default());
        let requests = b"GET / HTTP/1.1\r\nHost: a.com\r\n\r\nPOST /api HTTP/1.1\r\nHost: b.com\r\nContent-Length: 4\r\n\r\ntest";

        let result = parser.parse_batch(requests);

        assert_eq!(result.requests.len(), 2);
        assert_eq!(result.requests[0].method, "GET");
        assert_eq!(result.requests[0].path, "/");
        assert_eq!(result.requests[1].method, "POST");
        assert_eq!(result.requests[1].path, "/api");
        assert_eq!(result.requests[1].body, b"test");
    }

    #[test]
    fn test_parse_partial_request() {
        let parser = BatchParser::new(BatchConfig::default());
        let partial = b"GET /api HTTP/1.1\r\nHost: exam";

        let result = parser.parse_batch(partial);

        assert_eq!(result.requests.len(), 0);
        assert!(result.partial || result.bytes_consumed == 0);
    }

    #[test]
    fn test_parsed_request_helpers() {
        let parser = BatchParser::new(BatchConfig::default());
        let request = b"GET /api HTTP/1.1\r\nHost: example.com\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhello";

        let result = parser.parse_batch(request);

        assert_eq!(result.requests.len(), 1);
        let req = &result.requests[0];

        assert_eq!(req.content_length(), Some(5));
        assert!(req.is_keep_alive());
        assert_eq!(req.header("host"), Some(b"example.com".as_slice()));
    }

    #[test]
    fn test_batch_buffer() {
        let config = BatchConfig::default();
        let mut buffer = BatchBuffer::new(config);

        // Write some data
        let data = b"GET / HTTP/1.1\r\n";
        buffer.write_slice()[..data.len()].copy_from_slice(data);
        buffer.advance_write(data.len());

        assert_eq!(buffer.len(), data.len());
        assert_eq!(buffer.unprocessed(), data);

        // Mark some as read
        buffer.advance_read(4);
        assert_eq!(buffer.unprocessed(), b"/ HTTP/1.1\r\n");

        // Compact
        buffer.compact();
        assert_eq!(buffer.len(), 12);
    }

    #[test]
    fn test_batch_reader() {
        let config = BatchConfig::default();
        let mut reader = BatchReader::new(config);

        // Write request data
        let request = b"GET /test HTTP/1.1\r\nHost: localhost\r\n\r\n";
        reader.read_buffer()[..request.len()].copy_from_slice(request);
        reader.data_received(request.len());

        // Parse
        let result = reader.parse_available();

        assert_eq!(result.requests.len(), 1);
        assert_eq!(result.requests[0].path, "/test");
        assert_eq!(reader.stats().total_requests(), 1);
    }

    #[test]
    fn test_transfer_encoding_rejected() {
        // Chunked framing is not understood by the batch parser; accepting it
        // would desync pipelined requests. It must be an explicit error.
        let parser = BatchParser::new(BatchConfig::default());
        let request = b"POST /api HTTP/1.1\r\nHost: a.com\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";

        let result = parser.parse_batch(request);

        assert_eq!(result.requests.len(), 0);
        assert!(matches!(
            result.error,
            Some(BatchParseError::InvalidSyntax(_))
        ));
    }

    #[test]
    fn test_request_exceeding_buffer_capacity_errors() {
        // A request whose header + body can never fit in the read buffer must
        // error out instead of returning "incomplete" forever.
        let config = BatchConfig::builder().buffer_size(128).build();
        let parser = BatchParser::new(config);
        let request =
            b"POST /api HTTP/1.1\r\nHost: a.com\r\nContent-Length: 100000\r\n\r\npartial body";

        let result = parser.parse_batch(request);

        assert_eq!(result.requests.len(), 0);
        assert!(matches!(
            result.error,
            Some(BatchParseError::BufferOverflow)
        ));
    }

    #[test]
    fn test_max_headers_clamped_to_supported() {
        let config = BatchConfig::builder().max_headers(500).build();
        assert_eq!(config.max_headers, MAX_SUPPORTED_HEADERS);

        let config = BatchConfig::builder().max_headers(10).build();
        assert_eq!(config.max_headers, 10);
    }

    #[test]
    fn test_batch_stats() {
        let stats = BatchStats::new();

        stats.record_batch(5, 1024);
        stats.record_batch(10, 2048);

        assert_eq!(stats.batches_processed(), 2);
        assert_eq!(stats.total_requests(), 15);
        assert_eq!(stats.total_bytes_read(), 3072);
        assert_eq!(stats.max_batch_size(), 10);
    }

    #[test]
    fn test_to_http_request() {
        let parser = BatchParser::new(BatchConfig::default());
        let request = b"POST /api/data HTTP/1.1\r\nHost: example.com\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n{\"key\":\"val\"}";

        let result = parser.parse_batch(request);
        assert_eq!(result.requests.len(), 1);

        let http_req = result.requests[0].to_http_request();
        assert_eq!(http_req.method, "POST");
        assert_eq!(http_req.path, "/api/data");
        assert_eq!(
            http_req.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
        assert_eq!(http_req.body_ref(), b"{\"key\":\"val\"}");
    }
}
