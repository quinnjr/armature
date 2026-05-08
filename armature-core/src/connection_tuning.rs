//! Connection Tuning and Optimization
//!
//! This module provides comprehensive connection handling optimizations:
//!
//! - **HTTP/2 Priority**: Stream prioritization for efficient resource delivery
//! - **TCP Tuning**: Socket options for low latency and high throughput
//! - **Keep-Alive**: Optimized connection reuse and timeout handling
//!
//! # Performance Impact
//!
//! - HTTP/2 priority: Better page load times through smart resource ordering
//! - TCP_NODELAY: -40-80ms latency for small messages
//! - TCP_QUICKACK: -20-40ms latency on request start
//! - Keep-alive tuning: Reduced connection overhead, better resource utilization

use std::collections::HashMap;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

// ============================================================================
// HTTP/2 Priority Handling
// ============================================================================

/// HTTP/2 stream priority weight (1-256, default 16).
pub type StreamWeight = u8;

/// Stream dependency for HTTP/2 priority tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamDependency {
    /// Parent stream ID (0 = root)
    pub stream_id: u32,
    /// Whether this dependency is exclusive
    pub exclusive: bool,
}

/// HTTP/2 stream priority configuration.
#[derive(Debug, Clone)]
pub struct StreamPriority {
    /// Stream weight (1-256)
    pub weight: StreamWeight,
    /// Parent stream dependency
    pub dependency: StreamDependency,
}

impl Default for StreamPriority {
    fn default() -> Self {
        Self {
            weight: 16, // Default weight per HTTP/2 spec
            dependency: StreamDependency::default(),
        }
    }
}

impl StreamPriority {
    /// Create new priority with weight.
    pub fn with_weight(weight: StreamWeight) -> Self {
        Self {
            weight: weight.max(1), // Weight must be at least 1
            ..Default::default()
        }
    }

    /// Create priority dependent on another stream.
    pub fn dependent_on(stream_id: u32, exclusive: bool) -> Self {
        Self {
            weight: 16,
            dependency: StreamDependency {
                stream_id,
                exclusive,
            },
        }
    }

    /// Set weight.
    pub fn weight(mut self, weight: StreamWeight) -> Self {
        self.weight = weight.max(1);
        self
    }
}

/// Resource type for automatic priority assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    /// HTML document (highest priority)
    Html,
    /// CSS stylesheets (very high priority)
    Css,
    /// JavaScript files (high priority, blocks render)
    JavaScript,
    /// Fonts (medium-high priority)
    Font,
    /// Images (medium priority)
    Image,
    /// XHR/Fetch API requests (medium priority)
    Xhr,
    /// Prefetch resources (low priority)
    Prefetch,
    /// Other resources (default priority)
    Other,
}

impl ResourceType {
    /// Detect resource type from content-type header.
    pub fn from_content_type(content_type: &str) -> Self {
        let ct = content_type.to_lowercase();
        if ct.contains("text/html") {
            Self::Html
        } else if ct.contains("text/css") {
            Self::Css
        } else if ct.contains("javascript") || ct.contains("application/json") {
            Self::JavaScript
        } else if ct.contains("font") || ct.contains("woff") || ct.contains("ttf") {
            Self::Font
        } else if ct.contains("image/") {
            Self::Image
        } else {
            Self::Other
        }
    }

    /// Detect resource type from path extension.
    pub fn from_path(path: &str) -> Self {
        let path_lower = path.to_lowercase();
        if path_lower.ends_with(".html") || path_lower.ends_with(".htm") {
            Self::Html
        } else if path_lower.ends_with(".css") {
            Self::Css
        } else if path_lower.ends_with(".js") || path_lower.ends_with(".mjs") {
            Self::JavaScript
        } else if path_lower.ends_with(".woff")
            || path_lower.ends_with(".woff2")
            || path_lower.ends_with(".ttf")
            || path_lower.ends_with(".otf")
        {
            Self::Font
        } else if path_lower.ends_with(".png")
            || path_lower.ends_with(".jpg")
            || path_lower.ends_with(".jpeg")
            || path_lower.ends_with(".gif")
            || path_lower.ends_with(".webp")
            || path_lower.ends_with(".svg")
            || path_lower.ends_with(".ico")
        {
            Self::Image
        } else {
            Self::Other
        }
    }

    /// Get recommended weight for this resource type.
    pub fn recommended_weight(&self) -> StreamWeight {
        match self {
            Self::Html => 255,       // Highest priority
            Self::Css => 220,        // Very high - blocks render
            Self::JavaScript => 180, // High - often blocks render
            Self::Font => 140,       // Medium-high - needed for FOUT prevention
            Self::Xhr => 120,        // Medium - application data
            Self::Image => 80,       // Medium-low - can load progressively
            Self::Prefetch => 20,    // Low - speculative
            Self::Other => 100,      // Default
        }
    }

    /// Get recommended stream group for this resource type.
    ///
    /// Resources in the same group compete for bandwidth.
    pub fn stream_group(&self) -> u32 {
        match self {
            Self::Html => 1,
            Self::Css => 2,
            Self::JavaScript => 3,
            Self::Font => 4,
            Self::Xhr => 5,
            Self::Image => 6,
            Self::Prefetch => 7,
            Self::Other => 0,
        }
    }
}

/// HTTP/2 priority manager for automatic priority assignment.
#[derive(Debug)]
pub struct PriorityManager {
    /// Custom priority overrides by path
    overrides: HashMap<String, StreamPriority>,
    /// Group root streams
    group_roots: HashMap<u32, u32>,
    /// Statistics
    stats: PriorityStats,
}

impl Default for PriorityManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityManager {
    /// Create new priority manager.
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            group_roots: HashMap::new(),
            stats: PriorityStats::default(),
        }
    }

    /// Add priority override for a path pattern.
    pub fn override_path(&mut self, pattern: impl Into<String>, priority: StreamPriority) {
        self.overrides.insert(pattern.into(), priority);
    }

    /// Get priority for a request.
    pub fn get_priority(&self, path: &str, content_type: Option<&str>) -> StreamPriority {
        self.stats.lookups.fetch_add(1, Ordering::Relaxed);

        // Check overrides first
        if let Some(priority) = self.overrides.get(path) {
            self.stats.overrides_used.fetch_add(1, Ordering::Relaxed);
            return priority.clone();
        }

        // Determine resource type
        let resource_type = content_type
            .map(ResourceType::from_content_type)
            .unwrap_or_else(|| ResourceType::from_path(path));

        let weight = resource_type.recommended_weight();
        let group = resource_type.stream_group();

        // Create priority with group dependency
        let dependency = self
            .group_roots
            .get(&group)
            .map(|&root| StreamDependency {
                stream_id: root,
                exclusive: false,
            })
            .unwrap_or_default();

        StreamPriority { weight, dependency }
    }

    /// Register a stream as a group root.
    pub fn register_group_root(&mut self, group: u32, stream_id: u32) {
        self.group_roots.insert(group, stream_id);
    }

    /// Get statistics.
    pub fn stats(&self) -> &PriorityStats {
        &self.stats
    }
}

/// Priority statistics.
#[derive(Debug, Default)]
pub struct PriorityStats {
    lookups: AtomicU64,
    overrides_used: AtomicU64,
}

impl PriorityStats {
    /// Get total lookups.
    pub fn lookups(&self) -> u64 {
        self.lookups.load(Ordering::Relaxed)
    }

    /// Get override usage count.
    pub fn overrides_used(&self) -> u64 {
        self.overrides_used.load(Ordering::Relaxed)
    }
}

// ============================================================================
// TCP Tuning
// ============================================================================

/// TCP socket tuning configuration.
#[derive(Debug, Clone)]
pub struct TcpConfig {
    /// Enable TCP_NODELAY (disable Nagle's algorithm)
    ///
    /// Recommended for low-latency applications. Sends data immediately
    /// instead of waiting for more data to batch.
    pub nodelay: bool,

    /// Enable TCP_QUICKACK (Linux only)
    ///
    /// Immediately ACK incoming data instead of waiting for delayed ACK.
    /// Reduces latency at cost of slightly more ACK packets.
    pub quickack: bool,

    /// Send buffer size (SO_SNDBUF)
    ///
    /// Larger buffers improve throughput for high-bandwidth connections.
    /// None = use system default.
    pub send_buffer: Option<usize>,

    /// Receive buffer size (SO_RCVBUF)
    ///
    /// Larger buffers improve throughput for high-bandwidth connections.
    /// None = use system default.
    pub recv_buffer: Option<usize>,

    /// Keep-alive configuration
    pub keepalive: Option<TcpKeepalive>,

    /// SO_REUSEADDR - allow rapid rebinding
    pub reuse_addr: bool,

    /// SO_REUSEPORT - allow multiple listeners on same port
    pub reuse_port: bool,

    /// TCP_CORK (Linux) / TCP_NOPUSH (BSD)
    ///
    /// Buffer writes until explicitly uncorked or buffer is full.
    /// Useful for combining headers + body into fewer packets.
    pub cork: bool,

    /// IP_TOS - Type of Service / DSCP value
    ///
    /// Set traffic class for QoS. Common values:
    /// - 0x00: Best effort (default)
    /// - 0x10: Low delay
    /// - 0x08: High throughput
    /// - 0x04: High reliability
    pub tos: Option<u8>,

    /// TCP_DEFER_ACCEPT (Linux) / SO_ACCEPTFILTER (BSD)
    ///
    /// Don't complete accept() until data arrives. Reduces overhead
    /// from connections that never send data.
    pub defer_accept: Option<Duration>,
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            nodelay: true,
            quickack: false, // Disabled by default (Linux-only)
            send_buffer: None,
            recv_buffer: None,
            keepalive: Some(TcpKeepalive::default()),
            reuse_addr: true,
            reuse_port: false,
            cork: false,
            tos: None,
            defer_accept: None,
        }
    }
}

impl TcpConfig {
    /// Create configuration optimized for low latency.
    pub fn low_latency() -> Self {
        Self {
            nodelay: true,
            quickack: true,
            send_buffer: Some(32 * 1024), // 32KB
            recv_buffer: Some(32 * 1024),
            keepalive: Some(TcpKeepalive::aggressive()),
            reuse_addr: true,
            reuse_port: false,
            cork: false,
            tos: Some(0x10), // Low delay
            defer_accept: None,
        }
    }

    /// Create configuration optimized for high throughput.
    pub fn high_throughput() -> Self {
        Self {
            nodelay: false, // Allow Nagle's algorithm for batching
            quickack: false,
            send_buffer: Some(256 * 1024), // 256KB
            recv_buffer: Some(256 * 1024),
            keepalive: Some(TcpKeepalive::default()),
            reuse_addr: true,
            reuse_port: true, // Multi-accept for load distribution
            cork: true,       // Batch writes
            tos: Some(0x08),  // High throughput
            defer_accept: Some(Duration::from_secs(1)),
        }
    }

    /// Apply configuration to a TCP stream.
    #[cfg(unix)]
    pub fn apply(&self, stream: &TcpStream) -> io::Result<()> {
        use libc::{
            IPPROTO_TCP, SO_KEEPALIVE, SO_RCVBUF, SO_REUSEADDR, SO_SNDBUF, SOL_SOCKET, TCP_NODELAY,
            setsockopt,
        };

        let fd = stream.as_raw_fd();

        // TCP_NODELAY
        if self.nodelay {
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    IPPROTO_TCP,
                    TCP_NODELAY,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // TCP_QUICKACK (Linux only)
        #[cfg(target_os = "linux")]
        if self.quickack {
            const TCP_QUICKACK: libc::c_int = 12;
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    IPPROTO_TCP,
                    TCP_QUICKACK,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // SO_SNDBUF
        if let Some(size) = self.send_buffer {
            unsafe {
                let val: libc::c_int = size as libc::c_int;
                setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_SNDBUF,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // SO_RCVBUF
        if let Some(size) = self.recv_buffer {
            unsafe {
                let val: libc::c_int = size as libc::c_int;
                setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_RCVBUF,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // SO_REUSEADDR
        if self.reuse_addr {
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_REUSEADDR,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // SO_REUSEPORT (Linux)
        #[cfg(target_os = "linux")]
        if self.reuse_port {
            const SO_REUSEPORT: libc::c_int = 15;
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_REUSEPORT,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // TCP_CORK (Linux)
        #[cfg(target_os = "linux")]
        if self.cork {
            const TCP_CORK: libc::c_int = 3;
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    IPPROTO_TCP,
                    TCP_CORK,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }
        }

        // SO_KEEPALIVE
        if let Some(ref keepalive) = self.keepalive {
            unsafe {
                let val: libc::c_int = 1;
                setsockopt(
                    fd,
                    SOL_SOCKET,
                    SO_KEEPALIVE,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                );
            }

            // Apply keepalive settings
            keepalive.apply_to_fd(fd);
        }

        TCP_STATS
            .connections_configured
            .fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Apply configuration (non-Unix stub).
    #[cfg(not(unix))]
    pub fn apply(&self, stream: &TcpStream) -> io::Result<()> {
        stream.set_nodelay(self.nodelay)?;
        TCP_STATS
            .connections_configured
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// TCP keep-alive configuration.
#[derive(Debug, Clone)]
pub struct TcpKeepalive {
    /// Time before first probe (idle time)
    pub time: Duration,
    /// Interval between probes
    pub interval: Duration,
    /// Number of probes before giving up
    pub retries: u32,
}

impl Default for TcpKeepalive {
    fn default() -> Self {
        Self {
            time: Duration::from_secs(60),
            interval: Duration::from_secs(10),
            retries: 6,
        }
    }
}

impl TcpKeepalive {
    /// Create aggressive keepalive (detect dead connections quickly).
    pub fn aggressive() -> Self {
        Self {
            time: Duration::from_secs(10),
            interval: Duration::from_secs(3),
            retries: 3,
        }
    }

    /// Create relaxed keepalive (fewer probes, longer timeout).
    pub fn relaxed() -> Self {
        Self {
            time: Duration::from_secs(300), // 5 minutes
            interval: Duration::from_secs(30),
            retries: 10,
        }
    }

    /// Apply keepalive settings to file descriptor.
    #[cfg(target_os = "linux")]
    fn apply_to_fd(&self, fd: std::os::unix::io::RawFd) {
        use libc::{IPPROTO_TCP, setsockopt};

        const TCP_KEEPIDLE: libc::c_int = 4;
        const TCP_KEEPINTVL: libc::c_int = 5;
        const TCP_KEEPCNT: libc::c_int = 6;

        unsafe {
            let idle = self.time.as_secs() as libc::c_int;
            setsockopt(
                fd,
                IPPROTO_TCP,
                TCP_KEEPIDLE,
                &idle as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            let interval = self.interval.as_secs() as libc::c_int;
            setsockopt(
                fd,
                IPPROTO_TCP,
                TCP_KEEPINTVL,
                &interval as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            let retries = self.retries as libc::c_int;
            setsockopt(
                fd,
                IPPROTO_TCP,
                TCP_KEEPCNT,
                &retries as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
        }
    }

    /// Apply keepalive settings (non-Linux Unix stub).
    #[cfg(all(unix, not(target_os = "linux")))]
    fn apply_to_fd(&self, _fd: std::os::unix::io::RawFd) {
        // Platform-specific implementation would go here for macOS/BSD
    }
}

// ============================================================================
// Connection Keep-Alive Management
// ============================================================================

/// Keep-alive policy for connection management.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeepAlivePolicy {
    /// Always keep connections alive until timeout
    Always,
    /// Keep alive only for same-origin requests
    SameOrigin,
    /// Close after each request (HTTP/1.0 behavior)
    Never,
    /// Adaptive based on server load
    #[default]
    Adaptive,
}

/// Connection keep-alive configuration.
#[derive(Debug, Clone)]
pub struct KeepAliveConfig {
    /// Keep-alive policy
    pub policy: KeepAlivePolicy,

    /// Idle timeout before closing
    pub idle_timeout: Duration,

    /// Maximum requests per connection
    pub max_requests: Option<u64>,

    /// Maximum connection age
    pub max_age: Option<Duration>,

    /// Timeout for initial request (after accept)
    pub request_timeout: Duration,

    /// Timeout for reading request headers
    pub header_timeout: Duration,

    /// Timeout for reading request body
    pub body_timeout: Duration,

    /// Adaptive load threshold (connections per worker)
    /// Above this, new connections may be rejected
    pub adaptive_threshold: usize,
}

impl Default for KeepAliveConfig {
    fn default() -> Self {
        Self {
            policy: KeepAlivePolicy::Adaptive,
            idle_timeout: Duration::from_secs(60),
            max_requests: Some(10_000),
            max_age: Some(Duration::from_secs(3600)), // 1 hour
            request_timeout: Duration::from_secs(30),
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(60),
            adaptive_threshold: 1000,
        }
    }
}

impl KeepAliveConfig {
    /// Create config for high-concurrency servers.
    pub fn high_concurrency() -> Self {
        Self {
            policy: KeepAlivePolicy::Adaptive,
            idle_timeout: Duration::from_secs(30),
            max_requests: Some(1_000),
            max_age: Some(Duration::from_secs(300)), // 5 minutes
            request_timeout: Duration::from_secs(15),
            header_timeout: Duration::from_secs(5),
            body_timeout: Duration::from_secs(30),
            adaptive_threshold: 500,
        }
    }

    /// Create config for long-lived connections (websockets, SSE).
    pub fn long_lived() -> Self {
        Self {
            policy: KeepAlivePolicy::Always,
            idle_timeout: Duration::from_secs(300), // 5 minutes
            max_requests: None,
            max_age: None,
            request_timeout: Duration::from_secs(120),
            header_timeout: Duration::from_secs(30),
            body_timeout: Duration::from_secs(300),
            adaptive_threshold: 10_000,
        }
    }
}

/// Connection state tracker.
#[derive(Debug)]
pub struct ConnectionTracker {
    /// Connection creation time
    created_at: Instant,
    /// Last activity time
    last_activity: Instant,
    /// Requests processed
    requests: u64,
    /// Bytes received
    bytes_in: u64,
    /// Bytes sent
    bytes_out: u64,
}

impl ConnectionTracker {
    /// Create new tracker.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            created_at: now,
            last_activity: now,
            requests: 0,
            bytes_in: 0,
            bytes_out: 0,
        }
    }

    /// Record activity.
    #[inline]
    pub fn record_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Record request.
    #[inline]
    pub fn record_request(&mut self, bytes_in: usize, bytes_out: usize) {
        self.requests += 1;
        self.bytes_in += bytes_in as u64;
        self.bytes_out += bytes_out as u64;
        self.record_activity();
    }

    /// Get connection age.
    #[inline]
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Get idle time.
    #[inline]
    pub fn idle_time(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Get request count.
    #[inline]
    pub fn requests(&self) -> u64 {
        self.requests
    }

    /// Check if connection should be kept alive.
    pub fn should_keep_alive(&self, config: &KeepAliveConfig) -> bool {
        // Check idle timeout
        if self.idle_time() > config.idle_timeout {
            return false;
        }

        // Check max requests
        if let Some(max) = config.max_requests
            && self.requests >= max
        {
            return false;
        }

        // Check max age
        if let Some(max_age) = config.max_age
            && self.age() > max_age
        {
            return false;
        }

        true
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Adaptive keep-alive manager.
#[derive(Debug)]
pub struct AdaptiveKeepAlive {
    config: KeepAliveConfig,
    active_connections: AtomicUsize,
    stats: KeepAliveStats,
}

impl AdaptiveKeepAlive {
    /// Create new adaptive manager.
    pub fn new(config: KeepAliveConfig) -> Self {
        Self {
            config,
            active_connections: AtomicUsize::new(0),
            stats: KeepAliveStats::default(),
        }
    }

    /// Register new connection.
    pub fn connection_opened(&self) -> bool {
        let count = self.active_connections.fetch_add(1, Ordering::Relaxed);
        self.stats
            .connections_opened
            .fetch_add(1, Ordering::Relaxed);

        if self.config.policy == KeepAlivePolicy::Adaptive {
            // Reject if over threshold
            if count >= self.config.adaptive_threshold {
                self.stats
                    .connections_rejected
                    .fetch_add(1, Ordering::Relaxed);
                self.active_connections.fetch_sub(1, Ordering::Relaxed);
                return false;
            }
        }

        true
    }

    /// Unregister connection.
    pub fn connection_closed(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
        self.stats
            .connections_closed
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Check if keep-alive is allowed for current load.
    pub fn allow_keep_alive(&self) -> bool {
        match self.config.policy {
            KeepAlivePolicy::Always => true,
            KeepAlivePolicy::Never => false,
            KeepAlivePolicy::SameOrigin => true, // Caller must verify origin
            KeepAlivePolicy::Adaptive => {
                let count = self.active_connections.load(Ordering::Relaxed);
                count < self.config.adaptive_threshold
            }
        }
    }

    /// Get current load factor (0.0 - 1.0+).
    pub fn load_factor(&self) -> f64 {
        let count = self.active_connections.load(Ordering::Relaxed) as f64;
        count / self.config.adaptive_threshold as f64
    }

    /// Get statistics.
    pub fn stats(&self) -> &KeepAliveStats {
        &self.stats
    }

    /// Get active connection count.
    pub fn active_connections(&self) -> usize {
        self.active_connections.load(Ordering::Relaxed)
    }

    /// Get config.
    pub fn config(&self) -> &KeepAliveConfig {
        &self.config
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// TCP configuration statistics.
#[derive(Debug, Default)]
pub struct TcpStats {
    connections_configured: AtomicU64,
}

impl TcpStats {
    /// Get configured connection count.
    pub fn connections_configured(&self) -> u64 {
        self.connections_configured.load(Ordering::Relaxed)
    }
}

/// Keep-alive statistics.
#[derive(Debug, Default)]
pub struct KeepAliveStats {
    connections_opened: AtomicU64,
    connections_closed: AtomicU64,
    connections_rejected: AtomicU64,
}

impl KeepAliveStats {
    /// Get opened connection count.
    pub fn connections_opened(&self) -> u64 {
        self.connections_opened.load(Ordering::Relaxed)
    }

    /// Get closed connection count.
    pub fn connections_closed(&self) -> u64 {
        self.connections_closed.load(Ordering::Relaxed)
    }

    /// Get rejected connection count.
    pub fn connections_rejected(&self) -> u64 {
        self.connections_rejected.load(Ordering::Relaxed)
    }
}

/// Global TCP statistics.
static TCP_STATS: TcpStats = TcpStats {
    connections_configured: AtomicU64::new(0),
};

/// Get global TCP stats.
pub fn tcp_stats() -> &'static TcpStats {
    &TCP_STATS
}

// ============================================================================
// HTTP/2 Settings
// ============================================================================

/// HTTP/2 connection settings.
#[derive(Debug, Clone)]
pub struct Http2Settings {
    /// Maximum concurrent streams
    pub max_concurrent_streams: u32,

    /// Initial window size (flow control)
    pub initial_window_size: u32,

    /// Maximum frame size
    pub max_frame_size: u32,

    /// Maximum header list size
    pub max_header_list_size: u32,

    /// Enable server push
    pub enable_push: bool,

    /// Connection-level flow control window
    pub connection_window_size: u32,

    /// Enable HPACK dynamic table
    pub header_table_size: u32,
}

impl Default for Http2Settings {
    fn default() -> Self {
        Self {
            max_concurrent_streams: 128,
            initial_window_size: 65535, // 64KB - 1
            max_frame_size: 16384,      // 16KB (minimum required)
            max_header_list_size: 16384,
            enable_push: false, // Server push is generally not recommended
            connection_window_size: 1024 * 1024, // 1MB
            header_table_size: 4096,
        }
    }
}

impl Http2Settings {
    /// Create high-performance settings.
    pub fn high_performance() -> Self {
        Self {
            max_concurrent_streams: 256,
            initial_window_size: 1024 * 1024, // 1MB
            max_frame_size: 65535,            // Max allowed
            max_header_list_size: 65535,
            enable_push: false,
            connection_window_size: 16 * 1024 * 1024, // 16MB
            header_table_size: 65535,
        }
    }

    /// Create low-memory settings.
    pub fn low_memory() -> Self {
        Self {
            max_concurrent_streams: 32,
            initial_window_size: 16384,
            max_frame_size: 16384,
            max_header_list_size: 8192,
            enable_push: false,
            connection_window_size: 65535,
            header_table_size: 4096,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_priority_default() {
        let priority = StreamPriority::default();
        assert_eq!(priority.weight, 16);
        assert_eq!(priority.dependency.stream_id, 0);
    }

    #[test]
    fn test_stream_priority_with_weight() {
        let priority = StreamPriority::with_weight(200);
        assert_eq!(priority.weight, 200);
    }

    #[test]
    fn test_resource_type_detection() {
        assert_eq!(ResourceType::from_path("/index.html"), ResourceType::Html);
        assert_eq!(ResourceType::from_path("/style.css"), ResourceType::Css);
        assert_eq!(ResourceType::from_path("/app.js"), ResourceType::JavaScript);
        assert_eq!(ResourceType::from_path("/logo.png"), ResourceType::Image);
        assert_eq!(ResourceType::from_path("/font.woff2"), ResourceType::Font);
    }

    #[test]
    fn test_resource_type_weights() {
        assert!(ResourceType::Html.recommended_weight() > ResourceType::Css.recommended_weight());
        assert!(
            ResourceType::Css.recommended_weight() > ResourceType::JavaScript.recommended_weight()
        );
        assert!(
            ResourceType::JavaScript.recommended_weight()
                > ResourceType::Image.recommended_weight()
        );
    }

    #[test]
    fn test_priority_manager() {
        let mut manager = PriorityManager::new();
        manager.override_path("/api/critical", StreamPriority::with_weight(255));

        let priority = manager.get_priority("/api/critical", None);
        assert_eq!(priority.weight, 255);

        let priority = manager.get_priority("/style.css", None);
        assert_eq!(priority.weight, ResourceType::Css.recommended_weight());
    }

    #[test]
    fn test_tcp_config_default() {
        let config = TcpConfig::default();
        assert!(config.nodelay);
        assert!(config.reuse_addr);
    }

    #[test]
    fn test_tcp_config_low_latency() {
        let config = TcpConfig::low_latency();
        assert!(config.nodelay);
        assert!(config.quickack);
        assert_eq!(config.tos, Some(0x10));
    }

    #[test]
    fn test_tcp_config_high_throughput() {
        let config = TcpConfig::high_throughput();
        assert!(!config.nodelay); // Nagle's allowed for batching
        assert!(config.cork);
        assert!(config.reuse_port);
    }

    #[test]
    fn test_keepalive_default() {
        let keepalive = TcpKeepalive::default();
        assert_eq!(keepalive.time, Duration::from_secs(60));
        assert_eq!(keepalive.retries, 6);
    }

    #[test]
    fn test_connection_tracker() {
        let mut tracker = ConnectionTracker::new();
        assert_eq!(tracker.requests(), 0);

        tracker.record_request(100, 200);
        assert_eq!(tracker.requests(), 1);
        assert_eq!(tracker.bytes_in, 100);
        assert_eq!(tracker.bytes_out, 200);
    }

    #[test]
    fn test_connection_tracker_keep_alive() {
        let tracker = ConnectionTracker::new();
        let config = KeepAliveConfig::default();

        assert!(tracker.should_keep_alive(&config));
    }

    #[test]
    fn test_adaptive_keep_alive() {
        let config = KeepAliveConfig {
            adaptive_threshold: 10,
            ..Default::default()
        };
        let manager = AdaptiveKeepAlive::new(config);

        // Open 10 connections
        for _ in 0..10 {
            assert!(manager.connection_opened());
        }

        // 11th should be rejected in adaptive mode
        assert!(!manager.connection_opened());

        // Close one
        manager.connection_closed();

        // Now we can open another
        assert!(manager.connection_opened());
    }

    #[test]
    fn test_adaptive_load_factor() {
        let config = KeepAliveConfig {
            adaptive_threshold: 100,
            ..Default::default()
        };
        let manager = AdaptiveKeepAlive::new(config);

        assert_eq!(manager.load_factor(), 0.0);

        for _ in 0..50 {
            manager.connection_opened();
        }

        assert_eq!(manager.load_factor(), 0.5);
    }

    #[test]
    fn test_http2_settings_default() {
        let settings = Http2Settings::default();
        assert_eq!(settings.max_concurrent_streams, 128);
        assert!(!settings.enable_push);
    }

    #[test]
    fn test_http2_settings_high_performance() {
        let settings = Http2Settings::high_performance();
        assert!(settings.max_concurrent_streams > Http2Settings::default().max_concurrent_streams);
        assert!(settings.initial_window_size > Http2Settings::default().initial_window_size);
    }
}
