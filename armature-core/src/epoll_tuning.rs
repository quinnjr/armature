//! Epoll Tuning and Optimization
//!
//! This module provides configuration and utilities for optimizing Linux epoll
//! performance for high-throughput HTTP servers.
//!
//! # Key Optimizations
//!
//! - **Batch Size**: Optimal number of events to process per epoll_wait
//! - **Edge vs Level Triggered**: EPOLLET for reduced syscalls
//! - **EPOLLONESHOT**: Avoid thundering herd
//! - **Busy Polling**: Reduce latency for ultra-low-latency scenarios
//!
//! # Platform Support
//!
//! This module is Linux-specific. On other platforms, it provides no-op
//! implementations that return sensible defaults.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

// ============================================================================
// Configuration
// ============================================================================

/// Epoll tuning configuration.
#[derive(Debug, Clone)]
pub struct EpollConfig {
    /// Maximum events per epoll_wait call
    pub max_events: usize,
    /// Minimum events to process before yielding
    pub min_batch: usize,
    /// Timeout for epoll_wait in milliseconds (-1 = infinite)
    pub timeout_ms: i32,
    /// Use edge-triggered mode (EPOLLET)
    pub edge_triggered: bool,
    /// Use EPOLLONESHOT to prevent thundering herd
    pub oneshot: bool,
    /// Enable busy polling (requires CAP_NET_ADMIN)
    pub busy_poll_us: Option<u32>,
    /// Enable EPOLLEXCLUSIVE for load balancing
    pub exclusive: bool,
    /// Socket receive buffer size hint
    pub recv_buffer_size: Option<usize>,
    /// Socket send buffer size hint
    pub send_buffer_size: Option<usize>,
    /// TCP_NODELAY (disable Nagle's algorithm)
    pub tcp_nodelay: bool,
    /// TCP_QUICKACK (send ACKs immediately)
    pub tcp_quickack: bool,
    /// SO_REUSEPORT for multi-worker load balancing
    pub reuse_port: bool,
    /// SO_REUSEADDR for quick restart
    pub reuse_addr: bool,
    /// TCP keepalive settings
    pub keepalive: Option<KeepaliveConfig>,
}

impl Default for EpollConfig {
    fn default() -> Self {
        Self {
            max_events: 1024,
            min_batch: 1,
            timeout_ms: 100,
            edge_triggered: true,
            oneshot: false,
            busy_poll_us: None,
            exclusive: false,
            recv_buffer_size: Some(256 * 1024), // 256KB
            send_buffer_size: Some(256 * 1024), // 256KB
            tcp_nodelay: true,
            tcp_quickack: true,
            reuse_port: true,
            reuse_addr: true,
            keepalive: Some(KeepaliveConfig::default()),
        }
    }
}

impl EpollConfig {
    /// Create new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configuration optimized for throughput.
    ///
    /// Larger batches, edge-triggered, busy polling disabled.
    pub fn throughput() -> Self {
        Self {
            max_events: 2048,
            min_batch: 64,
            timeout_ms: 10,
            edge_triggered: true,
            oneshot: false,
            busy_poll_us: None,
            exclusive: true,
            recv_buffer_size: Some(512 * 1024),
            send_buffer_size: Some(512 * 1024),
            tcp_nodelay: true,
            tcp_quickack: false, // Batch ACKs for throughput
            reuse_port: true,
            reuse_addr: true,
            keepalive: Some(KeepaliveConfig::default()),
        }
    }

    /// Configuration optimized for low latency.
    ///
    /// Smaller batches, busy polling if available.
    pub fn low_latency() -> Self {
        Self {
            max_events: 256,
            min_batch: 1,
            timeout_ms: 0, // Return immediately
            edge_triggered: true,
            oneshot: false,
            busy_poll_us: Some(50), // 50us busy poll
            exclusive: false,
            recv_buffer_size: Some(64 * 1024),
            send_buffer_size: Some(64 * 1024),
            tcp_nodelay: true,
            tcp_quickack: true, // Immediate ACKs
            reuse_port: true,
            reuse_addr: true,
            keepalive: None, // Disable for latency
        }
    }

    /// Configuration for balanced workloads.
    pub fn balanced() -> Self {
        Self::default()
    }

    /// Builder pattern: set max events.
    pub fn max_events(mut self, n: usize) -> Self {
        self.max_events = n;
        self
    }

    /// Builder pattern: set minimum batch size.
    pub fn min_batch(mut self, n: usize) -> Self {
        self.min_batch = n;
        self
    }

    /// Builder pattern: set timeout.
    pub fn timeout(mut self, duration: Duration) -> Self {
        self.timeout_ms = duration.as_millis() as i32;
        self
    }

    /// Builder pattern: set infinite timeout.
    pub fn timeout_infinite(mut self) -> Self {
        self.timeout_ms = -1;
        self
    }

    /// Builder pattern: enable edge-triggered mode.
    pub fn edge_triggered(mut self, enabled: bool) -> Self {
        self.edge_triggered = enabled;
        self
    }

    /// Builder pattern: enable oneshot mode.
    pub fn oneshot(mut self, enabled: bool) -> Self {
        self.oneshot = enabled;
        self
    }

    /// Builder pattern: enable busy polling.
    pub fn busy_poll(mut self, microseconds: u32) -> Self {
        self.busy_poll_us = Some(microseconds);
        self
    }

    /// Builder pattern: enable exclusive mode.
    pub fn exclusive(mut self, enabled: bool) -> Self {
        self.exclusive = enabled;
        self
    }

    /// Builder pattern: set receive buffer size.
    pub fn recv_buffer(mut self, size: usize) -> Self {
        self.recv_buffer_size = Some(size);
        self
    }

    /// Builder pattern: set send buffer size.
    pub fn send_buffer(mut self, size: usize) -> Self {
        self.send_buffer_size = Some(size);
        self
    }

    /// Builder pattern: enable TCP_NODELAY.
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.tcp_nodelay = enabled;
        self
    }

    /// Builder pattern: enable TCP_QUICKACK.
    pub fn tcp_quickack(mut self, enabled: bool) -> Self {
        self.tcp_quickack = enabled;
        self
    }

    /// Builder pattern: enable SO_REUSEPORT.
    pub fn reuse_port(mut self, enabled: bool) -> Self {
        self.reuse_port = enabled;
        self
    }

    /// Builder pattern: set keepalive config.
    pub fn keepalive(mut self, config: KeepaliveConfig) -> Self {
        self.keepalive = Some(config);
        self
    }

    /// Builder pattern: disable keepalive.
    pub fn no_keepalive(mut self) -> Self {
        self.keepalive = None;
        self
    }

    /// Calculate epoll interest flags based on configuration.
    ///
    /// The returned mask (`EPOLLIN | EPOLLOUT | EPOLLRDHUP`, plus `EPOLLET`,
    /// `EPOLLONESHOT`, and `EPOLLEXCLUSIVE` as configured) is an **advisory
    /// constant for custom reactors only**. The framework's built-in server
    /// runs on tokio, which owns its epoll instance and provides no way to
    /// inject registration flags, so these flags are **not applied** by
    /// `Application::listen` or any other built-in server path. Use them
    /// only when registering file descriptors with your own epoll-based
    /// event loop. (The socket-level options in this config, by contrast,
    /// *are* applied by the server when enabled via
    /// `Application::with_socket_tuning`.)
    #[cfg(target_os = "linux")]
    pub fn epoll_flags(&self) -> u32 {
        use libc::{EPOLLET, EPOLLEXCLUSIVE, EPOLLIN, EPOLLONESHOT, EPOLLOUT, EPOLLRDHUP};

        let mut flags: u32 = (EPOLLIN | EPOLLOUT | EPOLLRDHUP) as u32;

        if self.edge_triggered {
            flags |= EPOLLET as u32;
        }

        if self.oneshot {
            flags |= EPOLLONESHOT as u32;
        }

        if self.exclusive {
            flags |= EPOLLEXCLUSIVE as u32;
        }

        flags
    }

    /// Non-Linux stub. See the Linux implementation: these flags are
    /// advisory constants for custom reactors and are not applied by the
    /// framework's built-in server.
    #[cfg(not(target_os = "linux"))]
    pub fn epoll_flags(&self) -> u32 {
        0
    }
}

/// TCP keepalive configuration.
#[derive(Debug, Clone, Copy)]
pub struct KeepaliveConfig {
    /// Time before first keepalive probe (seconds)
    pub idle_secs: u32,
    /// Interval between probes (seconds)
    pub interval_secs: u32,
    /// Number of probes before giving up
    pub count: u32,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            idle_secs: 60,
            interval_secs: 10,
            count: 3,
        }
    }
}

impl KeepaliveConfig {
    /// Create new keepalive config.
    pub fn new(idle_secs: u32, interval_secs: u32, count: u32) -> Self {
        Self {
            idle_secs,
            interval_secs,
            count,
        }
    }

    /// Aggressive keepalive for detecting dead connections quickly.
    pub fn aggressive() -> Self {
        Self {
            idle_secs: 10,
            interval_secs: 5,
            count: 2,
        }
    }

    /// Relaxed keepalive for long-lived connections.
    pub fn relaxed() -> Self {
        Self {
            idle_secs: 300,
            interval_secs: 60,
            count: 5,
        }
    }
}

// ============================================================================
// Socket Configuration
// ============================================================================

/// Apply socket options based on configuration.
#[cfg(target_os = "linux")]
pub fn configure_socket(fd: std::os::unix::io::RawFd, config: &EpollConfig) -> std::io::Result<()> {
    use std::io::Error;

    // Helper for setsockopt
    fn setsockopt<T>(fd: i32, level: i32, name: i32, value: &T) -> std::io::Result<()> {
        let res = unsafe {
            libc::setsockopt(
                fd,
                level,
                name,
                value as *const T as *const libc::c_void,
                std::mem::size_of::<T>() as libc::socklen_t,
            )
        };
        if res < 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    // SO_REUSEADDR
    if config.reuse_addr {
        let val: i32 = 1;
        setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &val)?;
    }

    // SO_REUSEPORT
    if config.reuse_port {
        let val: i32 = 1;
        setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &val)?;
    }

    // TCP_NODELAY
    if config.tcp_nodelay {
        let val: i32 = 1;
        setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY, &val)?;
    }

    // TCP_QUICKACK
    if config.tcp_quickack {
        let val: i32 = 1;
        // TCP_QUICKACK = 12 on Linux
        setsockopt(fd, libc::IPPROTO_TCP, 12, &val)?;
    }

    // Receive buffer
    if let Some(size) = config.recv_buffer_size {
        let val: i32 = size as i32;
        setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, &val)?;
    }

    // Send buffer
    if let Some(size) = config.send_buffer_size {
        let val: i32 = size as i32;
        setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF, &val)?;
    }

    // Busy polling
    if let Some(us) = config.busy_poll_us {
        let val: i32 = us as i32;
        // SO_BUSY_POLL = 46 on Linux
        let _ = setsockopt(fd, libc::SOL_SOCKET, 46, &val);
        // Ignore error - requires CAP_NET_ADMIN
    }

    // Keepalive
    if let Some(ref ka) = config.keepalive {
        let val: i32 = 1;
        setsockopt(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE, &val)?;

        // TCP_KEEPIDLE
        let val: i32 = ka.idle_secs as i32;
        setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, &val)?;

        // TCP_KEEPINTVL
        let val: i32 = ka.interval_secs as i32;
        setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, &val)?;

        // TCP_KEEPCNT
        let val: i32 = ka.count as i32;
        setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT, &val)?;
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn configure_socket(_fd: i32, _config: &EpollConfig) -> std::io::Result<()> {
    Ok(())
}

// ============================================================================
// Adaptive Batch Sizing
// ============================================================================

/// Adaptively adjusts batch sizes based on workload.
#[derive(Debug)]
pub struct AdaptiveBatcher {
    /// Current batch size
    current_batch: AtomicUsize,
    /// Minimum batch size
    min_batch: usize,
    /// Maximum batch size
    max_batch: usize,
    /// Events processed in last interval
    events_processed: AtomicU64,
    /// Intervals with high load
    high_load_intervals: AtomicU64,
    /// Intervals with low load
    low_load_intervals: AtomicU64,
    /// High load threshold (events per interval)
    high_threshold: usize,
    /// Low load threshold
    low_threshold: usize,
}

impl AdaptiveBatcher {
    /// Create new adaptive batcher.
    pub fn new(min_batch: usize, max_batch: usize) -> Self {
        Self {
            current_batch: AtomicUsize::new(min_batch),
            min_batch,
            max_batch,
            events_processed: AtomicU64::new(0),
            high_load_intervals: AtomicU64::new(0),
            low_load_intervals: AtomicU64::new(0),
            high_threshold: max_batch / 2,
            low_threshold: min_batch * 2,
        }
    }

    /// Get current batch size.
    #[inline]
    pub fn batch_size(&self) -> usize {
        self.current_batch.load(Ordering::Relaxed)
    }

    /// Record events processed.
    #[inline]
    pub fn record_events(&self, count: usize) {
        self.events_processed
            .fetch_add(count as u64, Ordering::Relaxed);
        EPOLL_STATS.record_events(count);
    }

    /// Adjust batch size based on load.
    ///
    /// Call this periodically (e.g., every 100ms).
    pub fn adjust(&self) {
        let events = self.events_processed.swap(0, Ordering::Relaxed) as usize;
        let current = self.current_batch.load(Ordering::Relaxed);

        if events > self.high_threshold {
            // High load - increase batch size
            self.high_load_intervals.fetch_add(1, Ordering::Relaxed);
            let new_batch = (current * 3 / 2).min(self.max_batch);
            self.current_batch.store(new_batch, Ordering::Relaxed);
            EPOLL_STATS.record_batch_increase();
        } else if events < self.low_threshold {
            // Low load - decrease batch size
            self.low_load_intervals.fetch_add(1, Ordering::Relaxed);
            let new_batch = (current * 2 / 3).max(self.min_batch);
            self.current_batch.store(new_batch, Ordering::Relaxed);
            EPOLL_STATS.record_batch_decrease();
        }
    }

    /// Get statistics.
    pub fn stats(&self) -> AdaptiveBatcherStats {
        AdaptiveBatcherStats {
            current_batch: self.batch_size(),
            high_load_intervals: self.high_load_intervals.load(Ordering::Relaxed),
            low_load_intervals: self.low_load_intervals.load(Ordering::Relaxed),
        }
    }
}

/// Statistics for adaptive batcher.
#[derive(Debug, Clone, Copy)]
pub struct AdaptiveBatcherStats {
    /// Current batch size
    pub current_batch: usize,
    /// Number of high load intervals
    pub high_load_intervals: u64,
    /// Number of low load intervals
    pub low_load_intervals: u64,
}

// ============================================================================
// Event Coalescing
// ============================================================================

/// Coalesces multiple events for the same FD.
///
/// Reduces syscalls when multiple operations complete on same connection.
#[derive(Debug)]
pub struct EventCoalescer {
    /// Pending events by FD
    pending: Vec<CoalescedEvent>,
    /// Capacity
    capacity: usize,
}

/// A coalesced event combining read/write readiness.
#[derive(Debug, Clone, Copy, Default)]
pub struct CoalescedEvent {
    /// File descriptor
    pub fd: i32,
    /// Has read readiness
    pub readable: bool,
    /// Has write readiness
    pub writable: bool,
    /// Has error
    pub error: bool,
    /// Connection closed
    pub closed: bool,
}

impl CoalescedEvent {
    /// Create new coalesced event.
    pub fn new(fd: i32) -> Self {
        Self {
            fd,
            readable: false,
            writable: false,
            error: false,
            closed: false,
        }
    }

    /// Check if any event is pending.
    #[inline]
    pub fn has_events(&self) -> bool {
        self.readable || self.writable || self.error || self.closed
    }

    /// Merge another event into this one.
    #[inline]
    pub fn merge(&mut self, other: &CoalescedEvent) {
        self.readable |= other.readable;
        self.writable |= other.writable;
        self.error |= other.error;
        self.closed |= other.closed;
    }
}

impl EventCoalescer {
    /// Create new coalescer.
    pub fn new(capacity: usize) -> Self {
        Self {
            pending: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Add an event.
    #[inline]
    pub fn add(&mut self, event: CoalescedEvent) {
        // Try to find existing event for same FD
        for existing in &mut self.pending {
            if existing.fd == event.fd {
                existing.merge(&event);
                EPOLL_STATS.record_coalesced();
                return;
            }
        }

        // Add new event
        if self.pending.len() < self.capacity {
            self.pending.push(event);
        }
    }

    /// Add read event.
    #[inline]
    pub fn add_read(&mut self, fd: i32) {
        self.add(CoalescedEvent {
            fd,
            readable: true,
            ..Default::default()
        });
    }

    /// Add write event.
    #[inline]
    pub fn add_write(&mut self, fd: i32) {
        self.add(CoalescedEvent {
            fd,
            writable: true,
            ..Default::default()
        });
    }

    /// Add error event.
    #[inline]
    pub fn add_error(&mut self, fd: i32) {
        self.add(CoalescedEvent {
            fd,
            error: true,
            ..Default::default()
        });
    }

    /// Add close event.
    #[inline]
    pub fn add_close(&mut self, fd: i32) {
        self.add(CoalescedEvent {
            fd,
            closed: true,
            ..Default::default()
        });
    }

    /// Drain all pending events.
    #[inline]
    pub fn drain(&mut self) -> impl Iterator<Item = CoalescedEvent> + '_ {
        self.pending.drain(..)
    }

    /// Get number of pending events.
    #[inline]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Clear all pending events.
    #[inline]
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

// ============================================================================
// Wakeup Optimization
// ============================================================================

/// Optimized wakeup mechanism using eventfd.
#[cfg(target_os = "linux")]
pub struct EventFdWaker {
    /// The eventfd file descriptor
    fd: i32,
}

#[cfg(target_os = "linux")]
impl EventFdWaker {
    /// Create new eventfd waker.
    pub fn new() -> std::io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { fd })
    }

    /// Get the file descriptor for epoll registration.
    pub fn fd(&self) -> i32 {
        self.fd
    }

    /// Wake up the epoll loop.
    pub fn wake(&self) -> std::io::Result<()> {
        let val: u64 = 1;
        let res = unsafe {
            libc::write(
                self.fd,
                &val as *const u64 as *const libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if res < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            EPOLL_STATS.record_wakeup();
            Ok(())
        }
    }

    /// Consume the wakeup notification.
    pub fn consume(&self) -> std::io::Result<u64> {
        let mut val: u64 = 0;
        let res = unsafe {
            libc::read(
                self.fd,
                &mut val as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if res < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                Ok(0)
            } else {
                Err(err)
            }
        } else {
            Ok(val)
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for EventFdWaker {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

// Non-Linux stub
#[cfg(not(target_os = "linux"))]
pub struct EventFdWaker;

#[cfg(not(target_os = "linux"))]
impl EventFdWaker {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }

    pub fn fd(&self) -> i32 {
        -1
    }

    pub fn wake(&self) -> std::io::Result<()> {
        Ok(())
    }

    pub fn consume(&self) -> std::io::Result<u64> {
        Ok(0)
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Global epoll statistics.
#[derive(Debug, Default)]
pub struct EpollStats {
    /// Total epoll_wait calls
    wait_calls: AtomicU64,
    /// Total events processed
    events_processed: AtomicU64,
    /// Events coalesced
    events_coalesced: AtomicU64,
    /// Batch size increases
    batch_increases: AtomicU64,
    /// Batch size decreases
    batch_decreases: AtomicU64,
    /// Wakeup calls
    wakeups: AtomicU64,
    /// Timeouts
    timeouts: AtomicU64,
}

impl EpollStats {
    /// Record epoll_wait call.
    pub fn record_wait(&self) {
        self.wait_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record events processed.
    pub fn record_events(&self, count: usize) {
        self.events_processed
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    /// Record coalesced event.
    pub fn record_coalesced(&self) {
        self.events_coalesced.fetch_add(1, Ordering::Relaxed);
    }

    /// Record batch increase.
    pub fn record_batch_increase(&self) {
        self.batch_increases.fetch_add(1, Ordering::Relaxed);
    }

    /// Record batch decrease.
    pub fn record_batch_decrease(&self) {
        self.batch_decreases.fetch_add(1, Ordering::Relaxed);
    }

    /// Record wakeup.
    pub fn record_wakeup(&self) {
        self.wakeups.fetch_add(1, Ordering::Relaxed);
    }

    /// Record timeout.
    pub fn record_timeout(&self) {
        self.timeouts.fetch_add(1, Ordering::Relaxed);
    }

    /// Get wait calls.
    pub fn wait_calls(&self) -> u64 {
        self.wait_calls.load(Ordering::Relaxed)
    }

    /// Get events processed.
    pub fn events_processed(&self) -> u64 {
        self.events_processed.load(Ordering::Relaxed)
    }

    /// Get coalesced events.
    pub fn events_coalesced(&self) -> u64 {
        self.events_coalesced.load(Ordering::Relaxed)
    }

    /// Get batch increases.
    pub fn batch_increases(&self) -> u64 {
        self.batch_increases.load(Ordering::Relaxed)
    }

    /// Get batch decreases.
    pub fn batch_decreases(&self) -> u64 {
        self.batch_decreases.load(Ordering::Relaxed)
    }

    /// Get wakeups.
    pub fn wakeups(&self) -> u64 {
        self.wakeups.load(Ordering::Relaxed)
    }

    /// Get timeouts.
    pub fn timeouts(&self) -> u64 {
        self.timeouts.load(Ordering::Relaxed)
    }

    /// Get average events per wait.
    pub fn avg_events_per_wait(&self) -> f64 {
        let waits = self.wait_calls() as f64;
        if waits > 0.0 {
            self.events_processed() as f64 / waits
        } else {
            0.0
        }
    }

    /// Get coalescing ratio.
    pub fn coalescing_ratio(&self) -> f64 {
        let total = self.events_processed() as f64;
        if total > 0.0 {
            self.events_coalesced() as f64 / total
        } else {
            0.0
        }
    }
}

/// Global statistics.
static EPOLL_STATS: EpollStats = EpollStats {
    wait_calls: AtomicU64::new(0),
    events_processed: AtomicU64::new(0),
    events_coalesced: AtomicU64::new(0),
    batch_increases: AtomicU64::new(0),
    batch_decreases: AtomicU64::new(0),
    wakeups: AtomicU64::new(0),
    timeouts: AtomicU64::new(0),
};

/// Get global epoll statistics.
pub fn epoll_stats() -> &'static EpollStats {
    &EPOLL_STATS
}

// ============================================================================
// Recommended Settings
// ============================================================================

/// Get recommended settings based on hardware.
pub fn recommended_config() -> EpollConfig {
    let cpus = num_cpus();

    // Base on CPU count
    let max_events = if cpus >= 32 {
        4096
    } else if cpus >= 16 {
        2048
    } else if cpus >= 8 {
        1024
    } else {
        512
    };

    // Buffer sizes scale with CPU
    let buffer_size = if cpus >= 16 {
        512 * 1024
    } else if cpus >= 8 {
        256 * 1024
    } else {
        128 * 1024
    };

    EpollConfig {
        max_events,
        min_batch: max_events / 16,
        recv_buffer_size: Some(buffer_size),
        send_buffer_size: Some(buffer_size),
        ..EpollConfig::default()
    }
}

/// Get number of CPUs.
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoll_config_default() {
        let config = EpollConfig::default();
        assert_eq!(config.max_events, 1024);
        assert!(config.edge_triggered);
        assert!(config.tcp_nodelay);
    }

    #[test]
    fn test_epoll_config_throughput() {
        let config = EpollConfig::throughput();
        assert_eq!(config.max_events, 2048);
        assert!(config.exclusive);
        assert!(!config.tcp_quickack); // Batched for throughput
    }

    #[test]
    fn test_epoll_config_low_latency() {
        let config = EpollConfig::low_latency();
        assert_eq!(config.max_events, 256);
        assert!(config.busy_poll_us.is_some());
        assert!(config.tcp_quickack);
    }

    #[test]
    fn test_epoll_config_builder() {
        let config = EpollConfig::new()
            .max_events(512)
            .timeout(Duration::from_millis(50))
            .edge_triggered(false)
            .tcp_nodelay(true);

        assert_eq!(config.max_events, 512);
        assert_eq!(config.timeout_ms, 50);
        assert!(!config.edge_triggered);
        assert!(config.tcp_nodelay);
    }

    #[test]
    fn test_keepalive_config() {
        let default = KeepaliveConfig::default();
        assert_eq!(default.idle_secs, 60);
        assert_eq!(default.interval_secs, 10);
        assert_eq!(default.count, 3);

        let aggressive = KeepaliveConfig::aggressive();
        assert_eq!(aggressive.idle_secs, 10);

        let relaxed = KeepaliveConfig::relaxed();
        assert_eq!(relaxed.idle_secs, 300);
    }

    #[test]
    fn test_adaptive_batcher() {
        let batcher = AdaptiveBatcher::new(64, 1024);
        assert_eq!(batcher.batch_size(), 64);

        // Simulate high load
        batcher.record_events(800);
        batcher.adjust();
        assert!(batcher.batch_size() > 64);

        // Simulate low load
        for _ in 0..5 {
            batcher.record_events(10);
            batcher.adjust();
        }
        // Should decrease back down
    }

    #[test]
    fn test_event_coalescer() {
        let mut coalescer = EventCoalescer::new(100);

        coalescer.add_read(5);
        coalescer.add_write(5);
        coalescer.add_read(10);

        assert_eq!(coalescer.len(), 2); // 5 and 10

        let events: Vec<_> = coalescer.drain().collect();
        assert_eq!(events.len(), 2);

        let fd5 = events.iter().find(|e| e.fd == 5).unwrap();
        assert!(fd5.readable);
        assert!(fd5.writable);

        let fd10 = events.iter().find(|e| e.fd == 10).unwrap();
        assert!(fd10.readable);
        assert!(!fd10.writable);
    }

    #[test]
    fn test_coalesced_event() {
        let mut event = CoalescedEvent::new(1);
        assert!(!event.has_events());

        event.readable = true;
        assert!(event.has_events());

        let mut other = CoalescedEvent::new(1);
        other.writable = true;
        other.error = true;

        event.merge(&other);
        assert!(event.readable);
        assert!(event.writable);
        assert!(event.error);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_eventfd_waker() {
        let waker = EventFdWaker::new().unwrap();
        assert!(waker.fd() >= 0);

        waker.wake().unwrap();
        let val = waker.consume().unwrap();
        assert_eq!(val, 1);

        // Second consume should return 0 (no wake pending)
        let val2 = waker.consume().unwrap();
        assert_eq!(val2, 0);
    }

    #[test]
    fn test_epoll_stats() {
        let stats = epoll_stats();
        stats.record_wait();
        stats.record_events(10);
        stats.record_coalesced();

        assert!(stats.wait_calls() >= 1);
        assert!(stats.events_processed() >= 10);
    }

    #[test]
    fn test_recommended_config() {
        let config = recommended_config();
        assert!(config.max_events >= 512);
        assert!(config.recv_buffer_size.is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_configure_socket_applies_tcp_nodelay() {
        use std::net::{TcpListener, TcpStream};
        use std::os::unix::io::AsRawFd;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let _client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();

        // Start from a known-disabled state so the readback proves that
        // configure_socket actually set the option.
        server.set_nodelay(false).unwrap();

        let config = EpollConfig::default().tcp_nodelay(true);
        configure_socket(server.as_raw_fd(), &config).unwrap();

        let mut val: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let res = unsafe {
            libc::getsockopt(
                server.as_raw_fd(),
                libc::IPPROTO_TCP,
                libc::TCP_NODELAY,
                &mut val as *mut libc::c_int as *mut libc::c_void,
                &mut len,
            )
        };
        assert_eq!(res, 0, "getsockopt(TCP_NODELAY) failed");
        assert_ne!(
            val, 0,
            "TCP_NODELAY should be enabled after configure_socket"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_epoll_flags() {
        let config = EpollConfig::default();
        let flags = config.epoll_flags();
        assert!(flags > 0);
        // EPOLLET should be set
        assert!(flags & (libc::EPOLLET as u32) != 0);
    }
}
