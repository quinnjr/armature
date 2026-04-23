//! HTTP/2 Support
//!
//! This module provides HTTP/2 server capabilities for the Armature framework.
//! HTTP/2 offers significant performance improvements over HTTP/1.1:
//!
//! - **Multiplexing**: Multiple requests over a single connection
//! - **Header Compression**: HPACK compression reduces overhead
//! - **Server Push**: Proactively send resources (optional)
//! - **Binary Protocol**: More efficient parsing than text-based HTTP/1.1
//! - **Flow Control**: Per-stream and connection-level flow control
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::{Application, Http2Config};
//!
//! // HTTP/2 over TLS (recommended)
//! let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?
//!     .with_alpn_protocols(vec!["h2", "http/1.1"]);
//! app.listen_https_h2(443, tls).await?;
//!
//! // HTTP/2 cleartext (h2c) - for development/internal use
//! app.listen_h2c(8080).await?;
//! ```
//!
//! ## ALPN Negotiation
//!
//! When using HTTPS, the server automatically negotiates the best protocol:
//! 1. If client supports HTTP/2 and advertises "h2", use HTTP/2
//! 2. Otherwise, fall back to HTTP/1.1
//!
//! ## Performance Considerations
//!
//! - HTTP/2 excels with many concurrent requests
//! - Single large file transfers may not see improvement
//! - Header compression benefits repeated similar requests
//! - Flow control prevents fast senders from overwhelming slow receivers

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// HTTP/2 server configuration
#[derive(Debug, Clone)]
pub struct Http2Config {
    /// Initial connection-level flow control window size (bytes)
    /// Default: 65535 (64KB) - HTTP/2 spec default
    /// Increase for high-bandwidth connections
    pub initial_connection_window_size: u32,

    /// Initial stream-level flow control window size (bytes)
    /// Default: 65535 (64KB) - HTTP/2 spec default
    pub initial_stream_window_size: u32,

    /// Maximum concurrent streams per connection
    /// Default: 100 - balance between parallelism and resource usage
    pub max_concurrent_streams: u32,

    /// Maximum frame size (bytes)
    /// Default: 16384 (16KB) - HTTP/2 spec default
    /// Max: 16777215 (16MB)
    pub max_frame_size: u32,

    /// Maximum header list size (bytes)
    /// Default: 16384 (16KB)
    pub max_header_list_size: u32,

    /// Enable HTTP/2 PING frames for keep-alive
    /// Default: true
    pub enable_connect_protocol: bool,

    /// Keep-alive interval for PING frames
    /// Default: 20 seconds
    pub keep_alive_interval: Option<Duration>,

    /// Keep-alive timeout (how long to wait for PING response)
    /// Default: 10 seconds
    pub keep_alive_timeout: Duration,

    /// Maximum send buffer size per stream (bytes)
    /// Default: 1MB
    pub max_send_buffer_size: usize,

    /// Enable adaptive flow control window sizing
    /// Default: true
    pub adaptive_window: bool,
}

impl Default for Http2Config {
    fn default() -> Self {
        Self {
            initial_connection_window_size: 65535,
            initial_stream_window_size: 65535,
            max_concurrent_streams: 100,
            max_frame_size: 16384,
            max_header_list_size: 16384,
            enable_connect_protocol: false,
            keep_alive_interval: Some(Duration::from_secs(20)),
            keep_alive_timeout: Duration::from_secs(10),
            max_send_buffer_size: 1024 * 1024, // 1MB
            adaptive_window: true,
        }
    }
}

impl Http2Config {
    /// Create a new builder
    pub fn builder() -> Http2ConfigBuilder {
        Http2ConfigBuilder::default()
    }

    /// High-throughput configuration for bandwidth-intensive workloads
    pub fn high_throughput() -> Self {
        Self {
            initial_connection_window_size: 1024 * 1024, // 1MB
            initial_stream_window_size: 512 * 1024,      // 512KB
            max_concurrent_streams: 250,
            max_frame_size: 65536, // 64KB
            max_header_list_size: 32768,
            enable_connect_protocol: false,
            keep_alive_interval: Some(Duration::from_secs(30)),
            keep_alive_timeout: Duration::from_secs(15),
            max_send_buffer_size: 4 * 1024 * 1024, // 4MB
            adaptive_window: true,
        }
    }

    /// Low-latency configuration for request/response workloads
    pub fn low_latency() -> Self {
        Self {
            initial_connection_window_size: 65535,
            initial_stream_window_size: 32768,
            max_concurrent_streams: 50,
            max_frame_size: 16384,
            max_header_list_size: 8192,
            enable_connect_protocol: false,
            keep_alive_interval: Some(Duration::from_secs(10)),
            keep_alive_timeout: Duration::from_secs(5),
            max_send_buffer_size: 256 * 1024, // 256KB
            adaptive_window: false,
        }
    }

    /// Memory-efficient configuration for resource-constrained environments
    pub fn memory_efficient() -> Self {
        Self {
            initial_connection_window_size: 32768,
            initial_stream_window_size: 16384,
            max_concurrent_streams: 25,
            max_frame_size: 16384,
            max_header_list_size: 8192,
            enable_connect_protocol: false,
            keep_alive_interval: Some(Duration::from_secs(60)),
            keep_alive_timeout: Duration::from_secs(20),
            max_send_buffer_size: 128 * 1024, // 128KB
            adaptive_window: false,
        }
    }
}

/// Builder for Http2Config
#[derive(Debug, Clone, Default)]
pub struct Http2ConfigBuilder {
    config: Http2Config,
}

impl Http2ConfigBuilder {
    /// Set initial connection window size
    pub fn initial_connection_window_size(mut self, size: u32) -> Self {
        self.config.initial_connection_window_size = size;
        self
    }

    /// Set initial stream window size
    pub fn initial_stream_window_size(mut self, size: u32) -> Self {
        self.config.initial_stream_window_size = size;
        self
    }

    /// Set maximum concurrent streams
    pub fn max_concurrent_streams(mut self, max: u32) -> Self {
        self.config.max_concurrent_streams = max;
        self
    }

    /// Set maximum frame size
    pub fn max_frame_size(mut self, size: u32) -> Self {
        self.config.max_frame_size = size.min(16777215); // HTTP/2 max
        self
    }

    /// Set maximum header list size
    pub fn max_header_list_size(mut self, size: u32) -> Self {
        self.config.max_header_list_size = size;
        self
    }

    /// Set keep-alive interval
    pub fn keep_alive_interval(mut self, interval: Option<Duration>) -> Self {
        self.config.keep_alive_interval = interval;
        self
    }

    /// Set keep-alive timeout
    pub fn keep_alive_timeout(mut self, timeout: Duration) -> Self {
        self.config.keep_alive_timeout = timeout;
        self
    }

    /// Set maximum send buffer size
    pub fn max_send_buffer_size(mut self, size: usize) -> Self {
        self.config.max_send_buffer_size = size;
        self
    }

    /// Enable or disable adaptive window sizing
    pub fn adaptive_window(mut self, enable: bool) -> Self {
        self.config.adaptive_window = enable;
        self
    }

    /// Build the configuration
    pub fn build(self) -> Http2Config {
        self.config
    }
}

// ============================================================================
// HTTP/2 Statistics
// ============================================================================

/// Statistics for HTTP/2 connections
#[derive(Debug, Default)]
pub struct Http2Stats {
    /// Active HTTP/2 connections
    active_connections: AtomicUsize,
    /// Total HTTP/2 connections
    total_connections: AtomicU64,
    /// Total streams created
    total_streams: AtomicU64,
    /// Active streams
    active_streams: AtomicUsize,
    /// Total requests processed
    total_requests: AtomicU64,
    /// GOAWAY frames sent (graceful shutdown)
    goaway_sent: AtomicU64,
    /// RST_STREAM frames sent (stream reset)
    rst_stream_sent: AtomicU64,
}

impl Http2Stats {
    /// Create new statistics tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Record connection opened
    #[inline]
    pub fn connection_opened(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
        self.total_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Record connection closed
    #[inline]
    pub fn connection_closed(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record stream created
    #[inline]
    pub fn stream_created(&self) {
        self.active_streams.fetch_add(1, Ordering::Relaxed);
        self.total_streams.fetch_add(1, Ordering::Relaxed);
    }

    /// Record stream closed
    #[inline]
    pub fn stream_closed(&self) {
        self.active_streams.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record request processed
    #[inline]
    pub fn request_processed(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Get active connections
    #[inline]
    pub fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Get total connections
    #[inline]
    pub fn total_connections(&self) -> u64 {
        self.total_connections.load(Ordering::Relaxed)
    }

    /// Get active streams
    #[inline]
    pub fn active_streams(&self) -> usize {
        self.active_streams.load(Ordering::Relaxed)
    }

    /// Get total streams
    #[inline]
    pub fn total_streams(&self) -> u64 {
        self.total_streams.load(Ordering::Relaxed)
    }

    /// Get total requests
    #[inline]
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Record a GOAWAY frame sent (graceful shutdown)
    #[inline]
    pub fn goaway_sent(&self) {
        self.goaway_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total GOAWAY frames sent
    #[inline]
    pub fn total_goaway_sent(&self) -> u64 {
        self.goaway_sent.load(Ordering::Relaxed)
    }

    /// Record an RST_STREAM frame sent
    #[inline]
    pub fn rst_stream_sent(&self) {
        self.rst_stream_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total RST_STREAM frames sent
    #[inline]
    pub fn total_rst_stream_sent(&self) -> u64 {
        self.rst_stream_sent.load(Ordering::Relaxed)
    }
}

// ============================================================================
// HTTP/2 Connection Builder
// ============================================================================

/// Builder for configuring HTTP/2 connections
pub struct Http2Builder {
    config: Http2Config,
    stats: Arc<Http2Stats>,
}

impl Http2Builder {
    /// Create a new HTTP/2 builder with default config
    pub fn new() -> Self {
        Self {
            config: Http2Config::default(),
            stats: Arc::new(Http2Stats::new()),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: Http2Config) -> Self {
        Self {
            config,
            stats: Arc::new(Http2Stats::new()),
        }
    }

    /// Create with shared statistics
    pub fn with_stats(config: Http2Config, stats: Arc<Http2Stats>) -> Self {
        Self { config, stats }
    }

    /// Get the configuration
    pub fn config(&self) -> &Http2Config {
        &self.config
    }

    /// Get shared statistics
    pub fn stats(&self) -> Arc<Http2Stats> {
        Arc::clone(&self.stats)
    }

    /// Configure a Hyper http2::Builder
    #[inline]
    pub fn configure_hyper_builder(
        &self,
    ) -> hyper::server::conn::http2::Builder<hyper_util::rt::TokioExecutor> {
        let mut builder =
            hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new());

        builder
            .initial_connection_window_size(self.config.initial_connection_window_size)
            .initial_stream_window_size(self.config.initial_stream_window_size)
            .max_concurrent_streams(self.config.max_concurrent_streams)
            .max_frame_size(self.config.max_frame_size)
            .max_header_list_size(self.config.max_header_list_size)
            .max_send_buf_size(self.config.max_send_buffer_size)
            .adaptive_window(self.config.adaptive_window);

        if let Some(interval) = self.config.keep_alive_interval {
            builder.keep_alive_interval(interval);
            builder.keep_alive_timeout(self.config.keep_alive_timeout);
        }

        builder
    }
}

impl Default for Http2Builder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Protocol Detection
// ============================================================================

/// HTTP protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpProtocol {
    /// HTTP/1.0
    Http10,
    /// HTTP/1.1
    Http11,
    /// HTTP/2
    H2,
    /// HTTP/2 over cleartext (h2c)
    H2c,
}

impl HttpProtocol {
    /// Check if this is HTTP/2
    #[inline]
    pub fn is_h2(&self) -> bool {
        matches!(self, Self::H2 | Self::H2c)
    }

    /// Get ALPN protocol identifier
    pub fn alpn_id(&self) -> &'static [u8] {
        match self {
            Self::Http10 | Self::Http11 => b"http/1.1",
            Self::H2 | Self::H2c => b"h2",
        }
    }
}

/// ALPN protocols for HTTP/2 with HTTP/1.1 fallback
pub const ALPN_H2_HTTP11: &[&[u8]] = &[b"h2", b"http/1.1"];

/// ALPN protocols for HTTP/2 only
pub const ALPN_H2_ONLY: &[&[u8]] = &[b"h2"];

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Http2Config::default();
        assert_eq!(config.initial_connection_window_size, 65535);
        assert_eq!(config.max_concurrent_streams, 100);
        assert_eq!(config.max_frame_size, 16384);
    }

    #[test]
    fn test_high_throughput_config() {
        let config = Http2Config::high_throughput();
        assert_eq!(config.initial_connection_window_size, 1024 * 1024);
        assert_eq!(config.max_concurrent_streams, 250);
    }

    #[test]
    fn test_config_builder() {
        let config = Http2Config::builder()
            .max_concurrent_streams(200)
            .max_frame_size(32768)
            .adaptive_window(false)
            .build();

        assert_eq!(config.max_concurrent_streams, 200);
        assert_eq!(config.max_frame_size, 32768);
        assert!(!config.adaptive_window);
    }

    #[test]
    fn test_stats() {
        let stats = Http2Stats::new();

        stats.connection_opened();
        stats.connection_opened();
        assert_eq!(stats.active_connections(), 2);
        assert_eq!(stats.total_connections(), 2);

        stats.stream_created();
        stats.stream_created();
        stats.stream_created();
        assert_eq!(stats.active_streams(), 3);

        stats.stream_closed();
        assert_eq!(stats.active_streams(), 2);

        stats.connection_closed();
        assert_eq!(stats.active_connections(), 1);
    }

    #[test]
    fn test_protocol_detection() {
        assert!(HttpProtocol::H2.is_h2());
        assert!(HttpProtocol::H2c.is_h2());
        assert!(!HttpProtocol::Http11.is_h2());

        assert_eq!(HttpProtocol::H2.alpn_id(), b"h2");
        assert_eq!(HttpProtocol::Http11.alpn_id(), b"http/1.1");
    }

    #[test]
    fn test_builder() {
        let config = Http2Config::high_throughput();
        let builder = Http2Builder::with_config(config.clone());

        assert_eq!(
            builder.config().max_concurrent_streams,
            config.max_concurrent_streams
        );
        assert_eq!(builder.stats().active_connections(), 0);
    }
}
