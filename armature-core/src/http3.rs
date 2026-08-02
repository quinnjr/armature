//! HTTP/3 (QUIC) Support
//!
//! This module provides HTTP/3 server capabilities for the Armature framework.
//! HTTP/3 is the latest HTTP protocol, using QUIC instead of TCP for transport.
//!
//! ## Key Benefits
//!
//! - **0-RTT Connection Establishment**: Faster initial connections
//! - **Multiplexing without Head-of-Line Blocking**: Streams are independent
//! - **Connection Migration**: Seamless network changes (WiFi → cellular)
//! - **Built-in Encryption**: TLS 1.3 integrated into QUIC
//! - **Improved Loss Recovery**: Better congestion control
//!
//! ## Requirements
//!
//! - Enable the `http3` feature in Cargo.toml
//! - Provide TLS certificates (QUIC always requires encryption)
//! - Open UDP port (not TCP!) on your firewall
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::{Application, Http3Config, TlsConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let app = Application::new(container, router);
//! let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
//!
//! // Start HTTP/3 server on UDP port 443
//! app.listen_h3(443, tls).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Alt-Svc Header
//!
//! To advertise HTTP/3 support from your HTTP/1.1 or HTTP/2 server,
//! add the `Alt-Svc` header to responses:
//!
//! ```text
//! Alt-Svc: h3=":443"; ma=86400
//! ```
//!
//! This tells clients that HTTP/3 is available on UDP port 443.
//!
//! ## Browser Support
//!
//! As of 2024, HTTP/3 is supported by:
//! - Chrome 87+
//! - Firefox 88+
//! - Safari 14+
//! - Edge 87+
//!
//! ## Note on Port Numbers
//!
//! HTTP/3 typically runs on the same port number as HTTPS (443),
//! but uses UDP instead of TCP. This means you can run both
//! HTTP/2 (TCP) and HTTP/3 (UDP) on port 443 simultaneously.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(feature = "http3")]
use std::net::SocketAddr;

/// HTTP/3 server configuration
///
/// Marked `#[non_exhaustive]`: construct it with [`Http3Config::default`],
/// one of the preset constructors, or [`Http3Config::builder`] rather than an
/// exhaustive struct literal, so that adding fields (like
/// `max_request_body_size`) stays backwards compatible for downstream crates.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Http3Config {
    /// Maximum concurrent bidirectional streams per connection
    /// Default: 100 - same as HTTP/2
    pub max_concurrent_bidi_streams: u32,

    /// Maximum concurrent unidirectional streams per connection
    /// Default: 3 - for QPACK encoder/decoder and control stream
    pub max_concurrent_uni_streams: u32,

    /// Initial stream receive window size (bytes)
    /// Default: 1MB
    pub initial_stream_receive_window: u32,

    /// Initial connection receive window size (bytes)
    /// Default: 10MB
    pub initial_connection_receive_window: u32,

    /// Maximum idle timeout before closing connection
    /// Default: 30 seconds
    pub max_idle_timeout: Duration,

    /// Keep-alive interval (QUIC PING frames)
    /// Default: 15 seconds
    pub keep_alive_interval: Option<Duration>,

    /// Enable 0-RTT (early data) for faster connection establishment
    /// **Security note**: 0-RTT data is replayable. Only enable for idempotent requests.
    /// Default: false
    pub enable_0rtt: bool,

    /// Maximum UDP payload size
    /// Default: 1350 bytes (safe for most networks)
    pub max_udp_payload_size: u16,

    /// Enable DATAGRAM extension (RFC 9221)
    /// Used for real-time/low-latency data
    /// Default: false
    pub enable_datagram: bool,

    /// QPACK max table capacity (bytes)
    /// Default: 4096
    pub qpack_max_table_capacity: u32,

    /// QPACK blocked streams
    /// Default: 16
    pub qpack_blocked_streams: u16,

    /// Maximum request body size (bytes)
    /// Requests with larger bodies are rejected with 413 Payload Too Large
    /// Default: 10MB
    pub max_request_body_size: usize,
}

impl Default for Http3Config {
    fn default() -> Self {
        Self {
            max_concurrent_bidi_streams: 100,
            max_concurrent_uni_streams: 3,
            initial_stream_receive_window: 1024 * 1024, // 1MB
            initial_connection_receive_window: 10 * 1024 * 1024, // 10MB
            max_idle_timeout: Duration::from_secs(30),
            keep_alive_interval: Some(Duration::from_secs(15)),
            enable_0rtt: false,
            max_udp_payload_size: 1350,
            enable_datagram: false,
            qpack_max_table_capacity: 4096,
            qpack_blocked_streams: 16,
            max_request_body_size: 10 * 1024 * 1024, // 10MB
        }
    }
}

impl Http3Config {
    /// Create a new builder
    pub fn builder() -> Http3ConfigBuilder {
        Http3ConfigBuilder::default()
    }

    /// High-throughput configuration for large file transfers
    pub fn high_throughput() -> Self {
        Self {
            max_concurrent_bidi_streams: 250,
            max_concurrent_uni_streams: 10,
            initial_stream_receive_window: 4 * 1024 * 1024, // 4MB
            initial_connection_receive_window: 50 * 1024 * 1024, // 50MB
            max_idle_timeout: Duration::from_secs(60),
            keep_alive_interval: Some(Duration::from_secs(20)),
            enable_0rtt: true,
            max_udp_payload_size: 1452, // Larger for better throughput
            enable_datagram: false,
            qpack_max_table_capacity: 16384,
            qpack_blocked_streams: 32,
            max_request_body_size: 50 * 1024 * 1024, // 50MB for large transfers
        }
    }

    /// Low-latency configuration for real-time applications
    pub fn low_latency() -> Self {
        Self {
            max_concurrent_bidi_streams: 50,
            max_concurrent_uni_streams: 3,
            initial_stream_receive_window: 256 * 1024, // 256KB
            initial_connection_receive_window: 2 * 1024 * 1024, // 2MB
            max_idle_timeout: Duration::from_secs(15),
            keep_alive_interval: Some(Duration::from_secs(5)),
            enable_0rtt: true,          // Faster connection establishment
            max_udp_payload_size: 1200, // Smaller for faster delivery
            enable_datagram: true,      // For real-time data
            qpack_max_table_capacity: 2048,
            qpack_blocked_streams: 8,
            max_request_body_size: 10 * 1024 * 1024, // 10MB
        }
    }

    /// Mobile-optimized configuration
    /// Handles network changes and high latency gracefully
    pub fn mobile_optimized() -> Self {
        Self {
            max_concurrent_bidi_streams: 64,
            max_concurrent_uni_streams: 3,
            initial_stream_receive_window: 512 * 1024, // 512KB
            initial_connection_receive_window: 4 * 1024 * 1024, // 4MB
            max_idle_timeout: Duration::from_secs(120), // Longer for mobile networks
            keep_alive_interval: Some(Duration::from_secs(30)),
            enable_0rtt: true,
            max_udp_payload_size: 1200, // Conservative for mobile networks
            enable_datagram: false,
            qpack_max_table_capacity: 4096,
            qpack_blocked_streams: 16,
            max_request_body_size: 10 * 1024 * 1024, // 10MB
        }
    }
}

/// Builder for Http3Config
#[derive(Debug, Clone, Default)]
pub struct Http3ConfigBuilder {
    config: Http3Config,
}

impl Http3ConfigBuilder {
    /// Set maximum concurrent bidirectional streams
    pub fn max_concurrent_bidi_streams(mut self, max: u32) -> Self {
        self.config.max_concurrent_bidi_streams = max;
        self
    }

    /// Set maximum concurrent unidirectional streams
    pub fn max_concurrent_uni_streams(mut self, max: u32) -> Self {
        self.config.max_concurrent_uni_streams = max;
        self
    }

    /// Set initial stream receive window
    pub fn initial_stream_receive_window(mut self, size: u32) -> Self {
        self.config.initial_stream_receive_window = size;
        self
    }

    /// Set initial connection receive window
    pub fn initial_connection_receive_window(mut self, size: u32) -> Self {
        self.config.initial_connection_receive_window = size;
        self
    }

    /// Set maximum idle timeout
    pub fn max_idle_timeout(mut self, timeout: Duration) -> Self {
        self.config.max_idle_timeout = timeout;
        self
    }

    /// Set keep-alive interval
    pub fn keep_alive_interval(mut self, interval: Option<Duration>) -> Self {
        self.config.keep_alive_interval = interval;
        self
    }

    /// Enable or disable 0-RTT early data
    ///
    /// **Security warning**: 0-RTT data is replayable.
    /// Only enable for idempotent requests.
    pub fn enable_0rtt(mut self, enable: bool) -> Self {
        self.config.enable_0rtt = enable;
        self
    }

    /// Set maximum UDP payload size
    pub fn max_udp_payload_size(mut self, size: u16) -> Self {
        self.config.max_udp_payload_size = size;
        self
    }

    /// Enable DATAGRAM extension
    pub fn enable_datagram(mut self, enable: bool) -> Self {
        self.config.enable_datagram = enable;
        self
    }

    /// Set maximum request body size in bytes
    pub fn max_request_body_size(mut self, size: usize) -> Self {
        self.config.max_request_body_size = size;
        self
    }

    /// Build the configuration
    pub fn build(self) -> Http3Config {
        self.config
    }
}

// ============================================================================
// HTTP/3 Statistics
// ============================================================================

/// Statistics for HTTP/3 connections
#[derive(Debug, Default)]
pub struct Http3Stats {
    /// Active QUIC connections
    active_connections: AtomicUsize,
    /// Total QUIC connections
    total_connections: AtomicU64,
    /// Active HTTP/3 streams
    active_streams: AtomicUsize,
    /// Total HTTP/3 streams
    total_streams: AtomicU64,
    /// Total requests processed
    total_requests: AtomicU64,
    /// 0-RTT connections accepted
    zero_rtt_accepted: AtomicU64,
    /// Connection migrations (IP/port changes)
    connection_migrations: AtomicU64,
    /// Total bytes sent
    bytes_sent: AtomicU64,
    /// Total bytes received
    bytes_received: AtomicU64,
}

impl Http3Stats {
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

    /// Record 0-RTT connection accepted
    #[inline]
    pub fn zero_rtt_accepted(&self) {
        self.zero_rtt_accepted.fetch_add(1, Ordering::Relaxed);
    }

    /// Record connection migration
    #[inline]
    pub fn connection_migrated(&self) {
        self.connection_migrations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bytes transferred
    #[inline]
    pub fn record_transfer(&self, sent: u64, received: u64) {
        self.bytes_sent.fetch_add(sent, Ordering::Relaxed);
        self.bytes_received.fetch_add(received, Ordering::Relaxed);
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

    /// Get 0-RTT connections accepted
    #[inline]
    pub fn total_zero_rtt_accepted(&self) -> u64 {
        self.zero_rtt_accepted.load(Ordering::Relaxed)
    }

    /// Get total connection migrations
    #[inline]
    pub fn total_connection_migrations(&self) -> u64 {
        self.connection_migrations.load(Ordering::Relaxed)
    }

    /// Get total bytes sent
    #[inline]
    pub fn total_bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get total bytes received
    #[inline]
    pub fn total_bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::Relaxed)
    }
}

// ============================================================================
// HTTP/3 Server Implementation (feature-gated)
// ============================================================================

#[cfg(feature = "http3")]
mod server {
    use super::*;
    use crate::{Error, HttpRequest, route_cache::OptimizedRouter};
    use bytes::{Buf, Bytes};
    use h3_quinn::quinn;
    use http::Response;
    use std::sync::Arc;
    use tracing::{debug, error, info};

    /// HTTP/3 Server
    pub struct Http3Server {
        config: Http3Config,
        stats: Arc<Http3Stats>,
        router: Arc<OptimizedRouter>,
    }

    impl Http3Server {
        /// Create a new HTTP/3 server
        pub fn new(config: Http3Config, router: Arc<OptimizedRouter>) -> Self {
            Self {
                config,
                stats: Arc::new(Http3Stats::new()),
                router,
            }
        }

        /// Get statistics
        pub fn stats(&self) -> Arc<Http3Stats> {
            Arc::clone(&self.stats)
        }

        /// Configure QUIC server from TLS config
        pub fn configure_quinn(
            &self,
            tls_config: Arc<rustls::ServerConfig>,
        ) -> Result<quinn::ServerConfig, Error> {
            // Create QUIC crypto config from TLS config.
            //
            // 0-RTT (early data) is enabled by advertising a non-zero
            // `max_early_data_size` on the rustls server config. The caller's
            // `tls_config` is shared behind an `Arc`, so clone it before
            // mutating to avoid affecting other consumers (e.g. the HTTP/2
            // listener sharing the same certificates).
            let crypto = if self.config.enable_0rtt {
                let mut tls = (*tls_config).clone();
                tls.max_early_data_size = u32::MAX;
                quinn::crypto::rustls::QuicServerConfig::try_from(Arc::new(tls))
            } else {
                quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            }
            .map_err(|e| Error::Internal(format!("Failed to create QUIC crypto: {}", e)))?;

            let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
            server_config.transport_config(Arc::new(build_transport(&self.config)));

            Ok(server_config)
        }

        /// Start listening for HTTP/3 connections
        pub async fn listen(
            self,
            addr: SocketAddr,
            tls_config: Arc<rustls::ServerConfig>,
        ) -> Result<(), Error> {
            let server_config = self.configure_quinn(tls_config)?;

            let endpoint = quinn::Endpoint::server(server_config, addr)
                .map_err(|e| Error::Internal(format!("Failed to bind QUIC endpoint: {}", e)))?;

            info!(address = %addr, "HTTP/3 server listening (QUIC/UDP)");

            let max_request_body_size = self.config.max_request_body_size;

            while let Some(incoming) = endpoint.accept().await {
                let stats = Arc::clone(&self.stats);
                let router = Arc::clone(&self.router);

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(incoming, router, stats, max_request_body_size).await
                    {
                        error!(error = %e, "HTTP/3 connection error");
                    }
                });
            }

            Ok(())
        }
    }

    /// Build the quinn transport config from an [`Http3Config`], applying every
    /// advertised knob (stream/connection flow-control windows, concurrency
    /// limits, idle timeout, keep-alive, and DATAGRAM buffers).
    ///
    /// Split out from [`Http3Server::configure_quinn`] so the mapping from
    /// config to transport parameters is unit-testable without a TLS
    /// certificate.
    pub(super) fn build_transport(config: &Http3Config) -> quinn::TransportConfig {
        let mut transport = quinn::TransportConfig::default();

        transport
            .max_concurrent_bidi_streams(config.max_concurrent_bidi_streams.into())
            .max_concurrent_uni_streams(config.max_concurrent_uni_streams.into())
            // Apply the advertised flow-control windows so large transfers are
            // not throttled by quinn's conservative defaults.
            .stream_receive_window(config.initial_stream_receive_window.into())
            .receive_window(config.initial_connection_receive_window.into())
            .initial_mtu(config.max_udp_payload_size)
            .max_idle_timeout(Some(
                config
                    .max_idle_timeout
                    .try_into()
                    .unwrap_or(quinn::IdleTimeout::from(quinn::VarInt::from_u32(30_000))),
            ));

        if let Some(interval) = config.keep_alive_interval {
            transport.keep_alive_interval(Some(interval));
        }

        if config.enable_datagram {
            transport.datagram_receive_buffer_size(Some(65536));
            transport.datagram_send_buffer_size(65536);
        }

        transport
    }

    /// Handle a single QUIC connection
    async fn handle_connection(
        incoming: quinn::Incoming,
        router: Arc<OptimizedRouter>,
        stats: Arc<Http3Stats>,
        max_request_body_size: usize,
    ) -> Result<(), Error> {
        // Accept the connection. When the server config advertises early data
        // (0-RTT enabled), `into_0rtt` succeeds and lets the peer send early
        // data before the handshake completes; the returned future resolves to
        // whether that early data was ultimately accepted (not replay-rejected),
        // which is what we count. When 0-RTT is disabled, `into_0rtt` returns
        // the `Connecting` back and we complete the handshake normally.
        let connecting = incoming
            .accept()
            .map_err(|e| Error::Internal(format!("Accept failed: {}", e)))?;
        let conn = match connecting.into_0rtt() {
            Ok((conn, zero_rtt_accepted)) => {
                let stats_0rtt = Arc::clone(&stats);
                tokio::spawn(async move {
                    if zero_rtt_accepted.await {
                        stats_0rtt.zero_rtt_accepted();
                    }
                });
                conn
            }
            Err(connecting) => connecting
                .await
                .map_err(|e| Error::Internal(format!("Connection failed: {}", e)))?,
        };

        stats.connection_opened();
        let remote_addr = conn.remote_address();
        debug!(client = %remote_addr, "HTTP/3 connection established");

        // Create HTTP/3 connection
        let mut h3_conn = h3::server::Connection::new(h3_quinn::Connection::new(conn))
            .await
            .map_err(|e| Error::Internal(format!("H3 connection failed: {}", e)))?;

        // Handle requests using the new RequestResolver API
        loop {
            match h3_conn.accept().await {
                Ok(Some(resolver)) => {
                    stats.stream_created();
                    let router = Arc::clone(&router);
                    let stats = Arc::clone(&stats);

                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_request_resolver(resolver, router, stats, max_request_body_size)
                                .await
                        {
                            error!(error = %e, "HTTP/3 request error");
                        }
                    });
                }
                Ok(None) => {
                    // Connection closed gracefully
                    break;
                }
                Err(e) => {
                    error!(error = %e, "HTTP/3 accept error");
                    break;
                }
            }
        }

        stats.connection_closed();
        debug!(client = %remote_addr, "HTTP/3 connection closed");

        Ok(())
    }

    /// Convert a resolved HTTP/3 request into an Armature [`HttpRequest`],
    /// copying method, headers, and the full request target — query string
    /// included, so [`HttpRequest::query`] can parse it on demand (mirrors
    /// `application.rs::handle_request`'s hyper conversion).
    ///
    /// Split out from [`handle_request_resolver`] so the conversion is
    /// unit-testable without a live QUIC connection.
    pub(super) fn build_armature_request(request: &http::Request<()>) -> HttpRequest {
        let method = request.method().to_string();
        // The full target, query included. Without the query, `?a=b` on a QUIC
        // request was silently dropped; carrying it in the target means the
        // router still matches on the path alone and a handler that asks for
        // the query gets it.
        let target = match request.uri().query() {
            Some(q) => format!("{}?{}", request.uri().path(), q),
            None => request.uri().path().to_string(),
        };
        let mut armature_req = HttpRequest::new(method, target);

        // Copy headers
        for (name, value) in request.headers() {
            if let Ok(v) = value.to_str() {
                armature_req.headers.insert(name.to_string(), v.to_string());
            }
        }

        armature_req
    }

    /// Handle a request using the RequestResolver API (h3 0.0.8+)
    async fn handle_request_resolver(
        resolver: h3::server::RequestResolver<h3_quinn::Connection, Bytes>,
        router: Arc<OptimizedRouter>,
        stats: Arc<Http3Stats>,
        max_request_body_size: usize,
    ) -> Result<(), Error> {
        // Resolve the request to get the request and stream
        let (request, mut stream) = resolver
            .resolve_request()
            .await
            .map_err(|e| Error::Internal(format!("Failed to resolve request: {}", e)))?;

        stats.request_processed();

        // Convert to Armature request, including percent-decoded query
        // parameters (see `build_armature_request`).
        let mut armature_req = build_armature_request(&request);

        // Read body if present (using Buf trait), enforcing the configured size limit
        let mut body: Vec<u8> = Vec::new();
        let mut payload_too_large = false;
        while let Some(chunk) = stream
            .recv_data()
            .await
            .map_err(|e| Error::Internal(format!("Failed to read body: {}", e)))?
        {
            let data = chunk.chunk();
            if body.len() + data.len() > max_request_body_size {
                payload_too_large = true;
                break;
            }
            body.extend_from_slice(data);
        }

        // Reject oversized requests with 413 Payload Too Large
        if payload_too_large {
            let http_response: http::Response<()> = Response::builder()
                .status(413)
                .body(())
                .map_err(|e| Error::Internal(format!("Failed to build response: {}", e)))?;
            stream
                .send_response(http_response)
                .await
                .map_err(|e| Error::Internal(format!("Failed to send response: {}", e)))?;
            stream
                .finish()
                .await
                .map_err(|e| Error::Internal(format!("Failed to finish stream: {}", e)))?;
            stats.stream_closed();
            return Ok(());
        }
        armature_req.body = body;

        // Route the request using the async router. A routing error is mapped
        // through the canonical client-safe response so HTTP/3 returns proper
        // 404/400/403 (etc.) with a JSON body, matching HTTP/1 and HTTP/2 —
        // rather than collapsing every error to a bare 500. 5xx messages are
        // still redacted by `to_client_response`.
        let response = match router.route(armature_req).await {
            Ok(resp) => resp,
            Err(err) => err.to_client_response(),
        };

        // Build HTTP/3 response, copying headers and cookies
        let mut builder = Response::builder().status(response.status);
        for (name, value) in &response.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        for cookie in &response.cookies {
            builder = builder.header("set-cookie", cookie.as_str());
        }
        let http_response: http::Response<()> = builder
            .body(())
            .map_err(|e| Error::Internal(format!("Failed to build response: {}", e)))?;

        // Send response
        stream
            .send_response(http_response)
            .await
            .map_err(|e| Error::Internal(format!("Failed to send response: {}", e)))?;

        // Send body (zero-copy when the handler stored it as Bytes)
        let body = response.into_body_bytes();
        if !body.is_empty() {
            stream
                .send_data(body)
                .await
                .map_err(|e| Error::Internal(format!("Failed to send body: {}", e)))?;
        }

        // Finish stream
        stream
            .finish()
            .await
            .map_err(|e| Error::Internal(format!("Failed to finish stream: {}", e)))?;

        stats.stream_closed();

        Ok(())
    }
}

#[cfg(feature = "http3")]
pub use server::Http3Server;

// ============================================================================
// Alt-Svc Header Helper
// ============================================================================

/// Generate Alt-Svc header value for HTTP/3 advertisement
///
/// # Example
///
/// ```rust
/// use armature_core::http3::alt_svc_header;
///
/// // Advertise HTTP/3 on port 443
/// let header = alt_svc_header(443, 86400);
/// assert_eq!(header, "h3=\":443\"; ma=86400");
/// ```
pub fn alt_svc_header(port: u16, max_age_seconds: u32) -> String {
    format!("h3=\":{}\"; ma={}", port, max_age_seconds)
}

/// Generate Alt-Svc header for multiple protocols
///
/// # Example
///
/// ```rust
/// use armature_core::http3::alt_svc_header_full;
///
/// // Advertise h3 and h3-29 (draft) on port 443
/// let header = alt_svc_header_full(443, 86400, true);
/// ```
pub fn alt_svc_header_full(port: u16, max_age_seconds: u32, include_draft: bool) -> String {
    if include_draft {
        format!(
            "h3=\":{}\"; ma={}, h3-29=\":{}\"; ma={}",
            port, max_age_seconds, port, max_age_seconds
        )
    } else {
        alt_svc_header(port, max_age_seconds)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Http3Config::default();
        assert_eq!(config.max_concurrent_bidi_streams, 100);
        assert_eq!(config.max_concurrent_uni_streams, 3);
        assert!(!config.enable_0rtt);
        assert!(!config.enable_datagram);
    }

    #[test]
    fn test_high_throughput_config() {
        let config = Http3Config::high_throughput();
        assert_eq!(config.max_concurrent_bidi_streams, 250);
        assert!(config.enable_0rtt);
        assert_eq!(config.initial_connection_receive_window, 50 * 1024 * 1024);
    }

    #[test]
    fn test_low_latency_config() {
        let config = Http3Config::low_latency();
        assert!(config.enable_0rtt);
        assert!(config.enable_datagram);
        assert_eq!(config.max_udp_payload_size, 1200);
    }

    #[test]
    fn test_mobile_optimized_config() {
        let config = Http3Config::mobile_optimized();
        assert_eq!(config.max_idle_timeout, Duration::from_secs(120));
        assert!(config.enable_0rtt);
    }

    #[test]
    fn test_config_builder() {
        let config = Http3Config::builder()
            .max_concurrent_bidi_streams(200)
            .enable_0rtt(true)
            .enable_datagram(true)
            .max_idle_timeout(Duration::from_secs(60))
            .build();

        assert_eq!(config.max_concurrent_bidi_streams, 200);
        assert!(config.enable_0rtt);
        assert!(config.enable_datagram);
        assert_eq!(config.max_idle_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_stats() {
        let stats = Http3Stats::new();

        stats.connection_opened();
        stats.connection_opened();
        assert_eq!(stats.active_connections(), 2);
        assert_eq!(stats.total_connections(), 2);

        stats.stream_created();
        stats.stream_created();
        stats.stream_created();
        assert_eq!(stats.active_streams(), 3);

        stats.request_processed();
        stats.request_processed();
        assert_eq!(stats.total_requests(), 2);

        stats.zero_rtt_accepted();
        assert_eq!(stats.total_zero_rtt_accepted(), 1);

        stats.connection_migrated();
        stats.connection_migrated();
        assert_eq!(stats.total_connection_migrations(), 2);

        stats.stream_closed();
        assert_eq!(stats.active_streams(), 2);

        stats.connection_closed();
        assert_eq!(stats.active_connections(), 1);
    }

    #[test]
    fn test_alt_svc_header() {
        let header = alt_svc_header(443, 86400);
        assert_eq!(header, "h3=\":443\"; ma=86400");

        let header = alt_svc_header(8443, 3600);
        assert_eq!(header, "h3=\":8443\"; ma=3600");
    }

    #[test]
    fn test_alt_svc_header_full() {
        let header = alt_svc_header_full(443, 86400, true);
        assert!(header.contains("h3=\":443\""));
        assert!(header.contains("h3-29=\":443\""));

        let header = alt_svc_header_full(443, 86400, false);
        assert!(!header.contains("h3-29"));
    }

    /// Regression: the advertised flow-control windows must actually be applied
    /// to the quinn transport config. Two configs that differ only in their
    /// stream/connection receive windows must produce different transport
    /// parameters; before the fix the knobs were dead and both were identical.
    #[cfg(feature = "http3")]
    #[test]
    fn test_configure_quinn_applies_flow_control_windows() {
        let base = Http3Config::default();
        // Same as `base` except for the two flow-control windows.
        let widened = Http3Config::builder()
            .initial_stream_receive_window(base.initial_stream_receive_window * 3 + 1)
            .initial_connection_receive_window(base.initial_connection_receive_window * 3 + 1)
            .build();

        let base_transport = format!("{:?}", super::server::build_transport(&base));
        let widened_transport = format!("{:?}", super::server::build_transport(&widened));

        assert_ne!(
            base_transport, widened_transport,
            "flow-control windows must be reflected in the quinn transport config"
        );
        assert!(
            widened_transport.contains(&widened.initial_stream_receive_window.to_string()),
            "stream receive window must appear in the applied transport config"
        );
        assert!(
            widened_transport.contains(&widened.initial_connection_receive_window.to_string()),
            "connection receive window must appear in the applied transport config"
        );
    }

    /// Regression: query strings must be parsed and percent-decoded on the
    /// HTTP/3 path, just like the hyper-based `application.rs::handle_request`.
    /// Before the fix, `handle_request_resolver` built the Armature request
    /// from `request.uri().path()` only and never touched `.query()`, so
    /// `?a=b` was silently dropped on every `listen_h3` / `listen_dual_stack`
    /// QUIC request.
    #[cfg(feature = "http3")]
    #[test]
    fn test_build_armature_request_decodes_query_params() {
        let request = http::Request::builder()
            .method("GET")
            .uri("https://example.com/search?a=b&name=hello%20world")
            .body(())
            .unwrap();

        let armature_req = super::server::build_armature_request(&request);

        assert_eq!(armature_req.method, "GET");
        assert_eq!(armature_req.path_only(), "/search");
        assert_eq!(armature_req.query_param("a"), Some("b"));
        assert_eq!(armature_req.query_param("name"), Some("hello world"));
    }

    /// Requests without a query string must yield an empty query view.
    #[cfg(feature = "http3")]
    #[test]
    fn test_build_armature_request_without_query_string() {
        let request = http::Request::builder()
            .method("GET")
            .uri("https://example.com/no-query")
            .body(())
            .unwrap();

        let armature_req = super::server::build_armature_request(&request);

        assert!(armature_req.query().is_empty());
        assert_eq!(armature_req.query_string(), None);
    }

    /// A URI ending in a bare `?` has a query string that is present but
    /// empty (`request.uri().query()` returns `Some("")`), distinct from no
    /// query string at all. `build_armature_request` must not panic and
    /// must produce an empty query view in this case.
    #[cfg(feature = "http3")]
    #[test]
    fn test_build_armature_request_with_empty_query_string() {
        let request = http::Request::builder()
            .method("GET")
            .uri("https://example.com/search?")
            .body(())
            .unwrap();

        assert_eq!(request.uri().query(), Some(""));

        let armature_req = super::server::build_armature_request(&request);

        assert_eq!(armature_req.path_only(), "/search");
        assert!(armature_req.query().is_empty());
    }

    /// Duplicate query keys are all preserved, in the order the client sent
    /// them; `get` answers with the first. The old `HashMap`-backed parse kept
    /// only the last.
    #[cfg(feature = "http3")]
    #[test]
    fn test_build_armature_request_with_duplicate_query_keys() {
        let request = http::Request::builder()
            .method("GET")
            .uri("https://example.com/search?a=1&a=2")
            .body(())
            .unwrap();

        let armature_req = super::server::build_armature_request(&request);

        assert_eq!(armature_req.query().len(), 2);
        assert_eq!(armature_req.query_param("a"), Some("1"));
        assert_eq!(
            armature_req.query().get_all("a").collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }
}
