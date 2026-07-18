//! `io_uring` Utilities (Linux 5.1+)
//!
//! This module provides an optional, **standalone** `io_uring` layer for
//! Linux systems. What is actually implemented:
//!
//! - Configuration types ([`IoUringConfig`], [`TcpOptions`]) and statistics
//!   counters ([`IoUringStats`]) — available on all platforms.
//! - A [`BufferPool`] of reusable byte buffers — available on all platforms.
//! - [`IoUringRuntime`]: a real `io_uring` ring (built on the
//!   [`io-uring`](https://docs.rs/io-uring) crate) with synchronous
//!   `read_at`/`write_at` submission helpers for file/socket file
//!   descriptors. Only available on Linux with the `io-uring` Cargo feature
//!   enabled.
//! - Runtime availability and feature probing ([`is_available`],
//!   [`IoUringFeatures::detect`]). With the `io-uring` feature enabled these
//!   probe the kernel by creating a ring and registering an opcode probe;
//!   without it they fall back to a `/proc/version` kernel-version heuristic.
//!
//! **Not implemented:** this module is *not* wired into the HTTP server
//! runtime. The armature HTTP server continues to use tokio's epoll-based
//! I/O. `IoUringRuntime` is a self-contained utility for performing
//! positional reads/writes on raw file descriptors via `io_uring`.
//!
//! ## Requirements
//!
//! - Linux kernel 5.1 or later
//! - `io-uring` feature enabled in Cargo.toml
//!
//! ## Usage
//!
//! ```rust
//! # #[cfg(all(target_os = "linux", feature = "io-uring"))]
//! # fn main() -> std::io::Result<()> {
//! use armature_core::io_uring::{IoUringConfig, IoUringRuntime};
//!
//! // Check if io_uring is available (probes the kernel)
//! if IoUringRuntime::is_available() {
//!     let config = IoUringConfig::builder()
//!         .ring_size(256)
//!         .build();
//!
//!     let runtime = IoUringRuntime::new(config)?;
//!     let _ = runtime.stats().submissions();
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(all(target_os = "linux", feature = "io-uring")))]
//! # fn main() {}
//! ```
//!
//! ## Security Considerations
//!
//! io_uring has had security vulnerabilities. Consider:
//! - Keep kernel updated
//! - Use seccomp to restrict io_uring opcodes if needed
//! - Monitor kernel security advisories

use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the io_uring backend
#[derive(Debug, Clone)]
pub struct IoUringConfig {
    /// Size of the submission/completion rings (must be power of 2)
    pub ring_size: u32,

    /// Enable kernel-side SQ polling (SQPOLL)
    /// Reduces syscalls but uses CPU
    pub sqpoll: bool,

    /// SQPOLL idle timeout in milliseconds
    pub sqpoll_idle_ms: u32,

    /// Enable IO polling mode (busy-waiting for completions)
    pub iopoll: bool,

    /// Enable single issuer mode for better performance
    pub single_issuer: bool,

    /// Enable deferred task running
    pub defer_taskrun: bool,

    /// Maximum number of fixed buffers for zero-copy I/O
    pub fixed_buffers: usize,

    /// Size of each fixed buffer
    pub buffer_size: usize,

    /// Enable buffer ring for automatic buffer selection
    pub buffer_ring: bool,

    /// Number of buffer ring entries
    pub buffer_ring_entries: u32,
}

impl Default for IoUringConfig {
    fn default() -> Self {
        Self {
            ring_size: 4096,
            sqpoll: false,
            sqpoll_idle_ms: 1000,
            iopoll: false,
            single_issuer: true,
            defer_taskrun: true,
            fixed_buffers: 1024,
            buffer_size: 16384, // 16KB
            buffer_ring: true,
            buffer_ring_entries: 4096,
        }
    }
}

impl IoUringConfig {
    /// Create a new builder
    pub fn builder() -> IoUringConfigBuilder {
        IoUringConfigBuilder::default()
    }

    /// High-performance configuration with SQPOLL
    pub fn high_performance() -> Self {
        Self {
            ring_size: 8192,
            sqpoll: true,
            sqpoll_idle_ms: 2000,
            iopoll: false,
            single_issuer: true,
            defer_taskrun: true,
            fixed_buffers: 2048,
            buffer_size: 32768, // 32KB
            buffer_ring: true,
            buffer_ring_entries: 8192,
        }
    }

    /// Balanced configuration (good performance, moderate resources)
    pub fn balanced() -> Self {
        Self::default()
    }

    /// Low-resource configuration
    pub fn low_resource() -> Self {
        Self {
            ring_size: 1024,
            sqpoll: false,
            sqpoll_idle_ms: 500,
            iopoll: false,
            single_issuer: true,
            defer_taskrun: true,
            fixed_buffers: 256,
            buffer_size: 8192, // 8KB
            buffer_ring: false,
            buffer_ring_entries: 1024,
        }
    }
}

/// Builder for IoUringConfig
#[derive(Debug, Clone, Default)]
pub struct IoUringConfigBuilder {
    config: IoUringConfig,
}

impl IoUringConfigBuilder {
    /// Set ring size (must be power of 2)
    pub fn ring_size(mut self, size: u32) -> Self {
        self.config.ring_size = size.next_power_of_two();
        self
    }

    /// Enable SQPOLL mode
    pub fn sqpoll(mut self, enable: bool) -> Self {
        self.config.sqpoll = enable;
        self
    }

    /// Set SQPOLL idle timeout
    pub fn sqpoll_idle_ms(mut self, ms: u32) -> Self {
        self.config.sqpoll_idle_ms = ms;
        self
    }

    /// Enable IO polling mode
    pub fn iopoll(mut self, enable: bool) -> Self {
        self.config.iopoll = enable;
        self
    }

    /// Enable single issuer mode
    pub fn single_issuer(mut self, enable: bool) -> Self {
        self.config.single_issuer = enable;
        self
    }

    /// Enable deferred task running
    pub fn defer_taskrun(mut self, enable: bool) -> Self {
        self.config.defer_taskrun = enable;
        self
    }

    /// Set number of fixed buffers
    pub fn fixed_buffers(mut self, count: usize) -> Self {
        self.config.fixed_buffers = count;
        self
    }

    /// Set fixed buffer size
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    /// Enable buffer ring
    pub fn buffer_ring(mut self, enable: bool) -> Self {
        self.config.buffer_ring = enable;
        self
    }

    /// Build the configuration
    pub fn build(self) -> IoUringConfig {
        self.config
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Statistics for io_uring operations
#[derive(Debug, Default)]
pub struct IoUringStats {
    /// Total submissions
    submissions: AtomicU64,
    /// Total completions
    completions: AtomicU64,
    /// Submission queue full events
    sq_full: AtomicU64,
    /// Completion queue overflow events
    cq_overflow: AtomicU64,
    /// Total bytes read
    bytes_read: AtomicU64,
    /// Total bytes written
    bytes_written: AtomicU64,
    /// Successful accepts
    accepts: AtomicU64,
    /// Operations that completed with an error result
    errors: AtomicU64,
    /// Current ring utilization (0-100)
    ring_utilization: AtomicU64,
}

impl IoUringStats {
    /// Create new statistics
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a submission
    #[inline]
    pub fn record_submission(&self) {
        self.submissions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a completion
    #[inline]
    pub fn record_completion(&self) {
        self.completions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record SQ full event
    #[inline]
    pub fn record_sq_full(&self) {
        self.sq_full.fetch_add(1, Ordering::Relaxed);
    }

    /// Record CQ overflow
    #[inline]
    pub fn record_cq_overflow(&self) {
        self.cq_overflow.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bytes read
    #[inline]
    pub fn record_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record bytes written
    #[inline]
    pub fn record_write(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record an accept
    #[inline]
    pub fn record_accept(&self) {
        self.accepts.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an operation that completed with an error result
    #[inline]
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Update ring utilization
    #[inline]
    pub fn update_utilization(&self, percent: u64) {
        self.ring_utilization.store(percent, Ordering::Relaxed);
    }

    /// Get total submissions
    pub fn submissions(&self) -> u64 {
        self.submissions.load(Ordering::Relaxed)
    }

    /// Get total completions
    pub fn completions(&self) -> u64 {
        self.completions.load(Ordering::Relaxed)
    }

    /// Get SQ full count
    pub fn sq_full(&self) -> u64 {
        self.sq_full.load(Ordering::Relaxed)
    }

    /// Get CQ overflow count
    pub fn cq_overflow(&self) -> u64 {
        self.cq_overflow.load(Ordering::Relaxed)
    }

    /// Get bytes read
    pub fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    /// Get bytes written
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Get accepts
    pub fn accepts(&self) -> u64 {
        self.accepts.load(Ordering::Relaxed)
    }

    /// Get the number of operations that completed with an error result
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Get ring utilization
    pub fn ring_utilization(&self) -> u64 {
        self.ring_utilization.load(Ordering::Relaxed)
    }

    /// Get pending operations
    pub fn pending(&self) -> u64 {
        self.submissions().saturating_sub(self.completions())
    }
}

// ============================================================================
// Runtime Detection
// ============================================================================

/// Check if io_uring is available on this system.
///
/// With the `io-uring` feature enabled this actually probes the kernel by
/// creating a small ring, so it also catches rings disabled by seccomp,
/// containers, or `kernel.io_uring_disabled`.
#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub fn is_available() -> bool {
    ::io_uring::IoUring::new(2).is_ok()
}

/// Check if io_uring is available on this system.
///
/// Without the `io-uring` feature this is a `/proc/version` kernel-version
/// heuristic (>= 5.1); it cannot detect rings disabled by seccomp or sysctl.
#[cfg(all(target_os = "linux", not(feature = "io-uring")))]
pub fn is_available() -> bool {
    // Check kernel version >= 5.1
    if let Ok(version) = std::fs::read_to_string("/proc/version")
        && let Some(ver) = parse_kernel_version(&version)
    {
        return ver >= (5, 1);
    }
    false
}

/// Check if io_uring is available (non-Linux always returns false)
#[cfg(not(target_os = "linux"))]
pub fn is_available() -> bool {
    false
}

/// Parse kernel version from /proc/version
#[cfg(target_os = "linux")]
#[cfg_attr(feature = "io-uring", allow(dead_code))]
fn parse_kernel_version(version_str: &str) -> Option<(u32, u32)> {
    // Format: "Linux version X.Y.Z ..."
    let parts: Vec<&str> = version_str.split_whitespace().collect();
    if parts.len() >= 3 && parts[0] == "Linux" && parts[1] == "version" {
        let ver_parts: Vec<&str> = parts[2].split('.').collect();
        if ver_parts.len() >= 2 {
            let major = ver_parts[0].parse().ok()?;
            let minor_str = ver_parts[1].split('-').next()?;
            let minor = minor_str.parse().ok()?;
            return Some((major, minor));
        }
    }
    None
}

/// Check if specific io_uring features are supported
#[derive(Debug, Clone)]
pub struct IoUringFeatures {
    /// Basic io_uring support
    pub basic: bool,
    /// SQPOLL support
    pub sqpoll: bool,
    /// Buffer ring support (5.19+)
    pub buffer_ring: bool,
    /// Multi-shot accept (5.19+)
    pub multishot_accept: bool,
    /// Send/recv zero-copy (6.0+)
    pub zerocopy: bool,
    /// Fixed files
    pub fixed_files: bool,
}

impl IoUringFeatures {
    /// Detect available features by probing the kernel.
    ///
    /// Creates a small ring and registers an [`io_uring::Probe`] to test
    /// opcode support. If ring creation fails (old kernel, seccomp,
    /// `kernel.io_uring_disabled`, ...), all features report `false`.
    ///
    /// Notes on mapping:
    /// - `sqpoll` is tested by actually building an SQPOLL ring, so it
    ///   reflects both kernel support and process privileges.
    /// - `buffer_ring` and `multishot_accept` are flag-based kernel features
    ///   (not opcodes), so they cannot be probed directly; both landed in
    ///   Linux 5.19 and are approximated by probing the `IORING_OP_SOCKET`
    ///   opcode, which was also added in 5.19.
    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    pub fn detect() -> Self {
        use ::io_uring::{IoUring, Probe, opcode};

        let Ok(ring) = IoUring::new(2) else {
            return Self::unsupported();
        };

        let mut probe = Probe::new();
        let probed = ring.submitter().register_probe(&mut probe).is_ok();
        let supported = |code: u8| probed && probe.is_supported(code);

        let sqpoll_ring: std::io::Result<IoUring> = IoUring::builder().setup_sqpoll(100).build(2);

        Self {
            basic: true,
            sqpoll: sqpoll_ring.is_ok(),
            buffer_ring: supported(opcode::Socket::CODE),
            multishot_accept: supported(opcode::Socket::CODE),
            zerocopy: supported(opcode::SendZc::CODE),
            fixed_files: supported(opcode::ReadFixed::CODE),
        }
    }

    /// Detect available features.
    ///
    /// Without the `io-uring` feature this is a `/proc/version`
    /// kernel-version heuristic; it cannot detect rings disabled by seccomp
    /// or sysctl. Enable the `io-uring` feature for real probing.
    #[cfg(all(target_os = "linux", not(feature = "io-uring")))]
    pub fn detect() -> Self {
        let version = std::fs::read_to_string("/proc/version")
            .ok()
            .and_then(|v| parse_kernel_version(&v))
            .unwrap_or((0, 0));

        Self {
            basic: version >= (5, 1),
            sqpoll: version >= (5, 11),
            buffer_ring: version >= (5, 19),
            multishot_accept: version >= (5, 19),
            zerocopy: version >= (6, 0),
            fixed_files: version >= (5, 1),
        }
    }

    /// Detect available features (non-Linux: always unsupported)
    #[cfg(not(target_os = "linux"))]
    pub fn detect() -> Self {
        Self::unsupported()
    }

    /// All features reported as unsupported
    #[cfg(any(not(target_os = "linux"), feature = "io-uring"))]
    fn unsupported() -> Self {
        Self {
            basic: false,
            sqpoll: false,
            buffer_ring: false,
            multishot_accept: false,
            zerocopy: false,
            fixed_files: false,
        }
    }
}

// ============================================================================
// I/O Backend Abstraction
// ============================================================================

/// Backend selection for I/O operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IoBackend {
    /// Traditional epoll/kqueue (default)
    #[default]
    Epoll,
    /// Linux io_uring (requires kernel 5.1+)
    IoUring,
    /// Automatic selection based on availability
    Auto,
}

impl IoBackend {
    /// Resolve Auto to a concrete backend
    pub fn resolve(self) -> Self {
        match self {
            Self::Auto => {
                if is_available() {
                    Self::IoUring
                } else {
                    Self::Epoll
                }
            }
            other => other,
        }
    }

    /// Check if this backend is io_uring
    pub fn is_io_uring(self) -> bool {
        matches!(self.resolve(), Self::IoUring)
    }
}

// ============================================================================
// IoUringRuntime (Linux + `io-uring` feature only)
// ============================================================================

/// A standalone `io_uring` ring with synchronous submission helpers.
///
/// This wraps an [`io_uring::IoUring`] instance and offers blocking,
/// positional [`read_at`](Self::read_at) / [`write_at`](Self::write_at)
/// helpers on raw file descriptors. Each call builds an SQE, submits it,
/// and waits for its completion before returning, so buffer lifetimes are
/// enforced by ordinary Rust borrows.
///
/// The ring is internally serialized with a mutex, so the runtime is
/// `Send + Sync` and any thread may submit.
///
/// ## Configuration mapping
///
/// From [`IoUringConfig`], the following fields are applied:
/// - `ring_size`: submission queue depth (normalized to a power of two,
///   capped at 32768).
/// - `sqpoll` / `sqpoll_idle_ms`: kernel-side submission polling. If the
///   kernel or environment refuses SQPOLL, construction falls back to a
///   plain ring instead of failing.
/// - `iopoll`: busy-wait completion polling (only meaningful for `O_DIRECT`
///   file I/O; regular buffered fds will fail with `EOPNOTSUPP`).
///
/// The remaining knobs are **not** applied by this minimal layer:
/// `single_issuer` and `defer_taskrun` would forbid the any-thread
/// submission model used here, and `fixed_buffers` / `buffer_size` /
/// `buffer_ring` / `buffer_ring_entries` require registered-buffer plumbing
/// this utility does not implement.
///
/// ## Not an HTTP server backend
///
/// This type is not wired into the armature HTTP server runtime; it is a
/// self-contained utility for fd-based I/O.
#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub struct IoUringRuntime {
    ring: std::sync::Mutex<::io_uring::IoUring>,
    stats: IoUringStats,
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
impl std::fmt::Debug for IoUringRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoUringRuntime")
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

#[cfg(all(target_os = "linux", feature = "io-uring"))]
impl IoUringRuntime {
    /// Check if io_uring is available on this system (probes the kernel).
    pub fn is_available() -> bool {
        is_available()
    }

    /// Create a new runtime from the given configuration.
    ///
    /// See the type-level docs for which configuration fields are applied.
    ///
    /// # Errors
    ///
    /// Returns the underlying `io_uring_setup(2)` error if the kernel cannot
    /// create a ring (e.g. `ENOSYS` on kernels < 5.1, `EPERM` in restricted
    /// containers or with `kernel.io_uring_disabled` set).
    pub fn new(config: IoUringConfig) -> std::io::Result<Self> {
        let entries = config.ring_size.max(1).next_power_of_two().min(32768);

        let build = |sqpoll: bool| {
            let mut builder = ::io_uring::IoUring::builder();
            if config.iopoll {
                builder.setup_iopoll();
            }
            if sqpoll {
                builder.setup_sqpoll(config.sqpoll_idle_ms);
            }
            builder.build(entries)
        };

        let ring = match build(config.sqpoll) {
            Ok(ring) => ring,
            // SQPOLL may be refused by older kernels or restricted
            // environments; fall back to a plain ring rather than failing.
            Err(_) if config.sqpoll => build(false)?,
            Err(e) => return Err(e),
        };

        Ok(Self {
            ring: std::sync::Mutex::new(ring),
            stats: IoUringStats::new(),
        })
    }

    /// Get the statistics recorded by this runtime's submission paths.
    pub fn stats(&self) -> &IoUringStats {
        &self.stats
    }

    /// Read from `fd` at `offset` into `buf` via the ring.
    ///
    /// Builds an `IORING_OP_READ` SQE, submits it, and blocks until its
    /// completion is reaped, then returns the number of bytes read
    /// (0 = end of file). Passing `offset = u64::MAX` (`-1`) reads at the
    /// fd's current file position (useful for sockets/pipes).
    ///
    /// The kernel is guaranteed to be done with `buf` when this returns,
    /// so the `&mut` borrow fully covers the I/O lifetime.
    ///
    /// Reads longer than `u32::MAX` bytes are truncated to `u32::MAX`
    /// (like a short `pread(2)`).
    pub fn read_at(
        &self,
        fd: std::os::fd::BorrowedFd<'_>,
        buf: &mut [u8],
        offset: u64,
    ) -> std::io::Result<usize> {
        use std::os::fd::AsRawFd;

        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let sqe = ::io_uring::opcode::Read::new(
            ::io_uring::types::Fd(fd.as_raw_fd()),
            buf.as_mut_ptr(),
            len,
        )
        .offset(offset)
        .build();

        // SAFETY: `submit_one` blocks until the completion for this SQE is
        // reaped, so the kernel never touches `buf` after this call returns,
        // and the `&mut` borrow guarantees exclusivity in the meantime.
        let n = unsafe { self.submit_one(sqe) }?;
        self.stats.record_read(n as u64);
        Ok(n)
    }

    /// Write `buf` to `fd` at `offset` via the ring.
    ///
    /// Builds an `IORING_OP_WRITE` SQE, submits it, and blocks until its
    /// completion is reaped, then returns the number of bytes written.
    /// Passing `offset = u64::MAX` (`-1`) writes at the fd's current file
    /// position (useful for sockets/pipes).
    ///
    /// The kernel is guaranteed to be done with `buf` when this returns,
    /// so the borrow fully covers the I/O lifetime.
    ///
    /// Writes longer than `u32::MAX` bytes are truncated to `u32::MAX`
    /// (like a short `pwrite(2)`).
    pub fn write_at(
        &self,
        fd: std::os::fd::BorrowedFd<'_>,
        buf: &[u8],
        offset: u64,
    ) -> std::io::Result<usize> {
        use std::os::fd::AsRawFd;

        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let sqe = ::io_uring::opcode::Write::new(
            ::io_uring::types::Fd(fd.as_raw_fd()),
            buf.as_ptr(),
            len,
        )
        .offset(offset)
        .build();

        // SAFETY: `submit_one` blocks until the completion for this SQE is
        // reaped, so the kernel never reads `buf` after this call returns,
        // and the shared borrow keeps the memory alive in the meantime.
        let n = unsafe { self.submit_one(sqe) }?;
        self.stats.record_write(n as u64);
        Ok(n)
    }

    /// Submit a single SQE and block until its completion is reaped.
    ///
    /// Records submission/completion/error statistics.
    ///
    /// # Safety
    ///
    /// Any buffers referenced by `sqe` must remain valid (and, for reads,
    /// exclusively borrowed) until this function returns: the function only
    /// returns after the CQE for the operation has been reaped (or before
    /// the SQE was ever handed to the kernel).
    unsafe fn submit_one(&self, sqe: ::io_uring::squeue::Entry) -> std::io::Result<usize> {
        let mut ring = self.ring.lock().unwrap();

        // SAFETY (push): per this function's contract, the buffers the SQE
        // points at stay valid until the completion is reaped below.
        unsafe {
            let mut sq = ring.submission();
            if sq.push(&sqe).is_err() {
                // SQ full: flush pending entries and retry once.
                self.stats.record_sq_full();
                drop(sq);
                ring.submit()?;
                let mut sq = ring.submission();
                sq.push(&sqe)
                    .map_err(|_| std::io::Error::other("io_uring submission queue full"))?;
            }
        }
        self.stats.record_submission();

        // Wait for the completion, retrying on EINTR: we must not return
        // while the kernel may still be using the caller's buffer. Because
        // submissions are serialized (one in flight per lock hold) the CQ
        // can never overflow here, so other wait errors are not expected.
        loop {
            match ring.submit_and_wait(1) {
                Ok(_) => break,
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => continue,
                Err(e) => {
                    self.stats.record_error();
                    return Err(e);
                }
            }
        }

        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| std::io::Error::other("io_uring completion queue empty after wait"))?;
        self.stats.record_completion();

        let res = cqe.result();
        if res < 0 {
            self.stats.record_error();
            Err(std::io::Error::from_raw_os_error(-res))
        } else {
            Ok(res as usize)
        }
    }
}

// ============================================================================
// Buffer Pool for io_uring
// ============================================================================

/// A pool of pre-allocated buffers for io_uring operations
#[derive(Debug)]
pub struct BufferPool {
    /// Buffer data (each slot behind its own lock so exclusive access is
    /// enforced by the type system rather than by caller discipline)
    buffers: Vec<std::sync::Mutex<Vec<u8>>>,
    /// Free buffer indices
    free_list: std::sync::Mutex<Vec<usize>>,
    /// Buffer size
    buffer_size: usize,
    /// Statistics
    stats: BufferPoolStats,
}

#[derive(Debug, Default)]
struct BufferPoolStats {
    allocations: AtomicU64,
    deallocations: AtomicU64,
    pool_misses: AtomicU64,
}

impl BufferPool {
    /// Create a new buffer pool
    pub fn new(count: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(count);
        let mut free_list = Vec::with_capacity(count);

        for i in 0..count {
            buffers.push(std::sync::Mutex::new(vec![0u8; buffer_size]));
            free_list.push(i);
        }

        Self {
            buffers,
            free_list: std::sync::Mutex::new(free_list),
            buffer_size,
            stats: BufferPoolStats::default(),
        }
    }

    /// Acquire a buffer from the pool.
    ///
    /// Returns an RAII guard that dereferences to the buffer and returns
    /// it to the pool when dropped. Returns `None` if the pool is empty.
    pub fn acquire(&self) -> Option<BufferGuard<'_>> {
        let idx = self.free_list.lock().unwrap().pop();
        if let Some(idx) = idx {
            self.stats.allocations.fetch_add(1, Ordering::Relaxed);
            // The index was removed from the free list, so this slot lock is
            // uncontended; it enforces exclusive access for the guard's lifetime.
            let buf = self.buffers[idx].lock().unwrap();
            Some(BufferGuard {
                pool: self,
                index: idx,
                buf,
            })
        } else {
            self.stats.pool_misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Get buffer size
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Get pool capacity
    pub fn capacity(&self) -> usize {
        self.buffers.len()
    }

    /// Get available buffers count
    pub fn available(&self) -> usize {
        self.free_list.lock().unwrap().len()
    }
}

/// RAII guard for a buffer acquired from a [`BufferPool`].
///
/// Dereferences to the buffer contents and releases the buffer back to
/// the pool when dropped.
#[derive(Debug)]
pub struct BufferGuard<'a> {
    pool: &'a BufferPool,
    index: usize,
    buf: std::sync::MutexGuard<'a, Vec<u8>>,
}

impl BufferGuard<'_> {
    /// Get the pool index of this buffer (e.g. for io_uring registration).
    pub fn index(&self) -> usize {
        self.index
    }
}

impl std::ops::Deref for BufferGuard<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.buf
    }
}

impl std::ops::DerefMut for BufferGuard<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}

impl Drop for BufferGuard<'_> {
    fn drop(&mut self) {
        self.pool.free_list.lock().unwrap().push(self.index);
        self.pool
            .stats
            .deallocations
            .fetch_add(1, Ordering::Relaxed);
    }
}

// ============================================================================
// TCP Operations for io_uring
// ============================================================================

/// TCP socket options optimized for io_uring
#[derive(Debug, Clone)]
pub struct TcpOptions {
    /// Enable TCP_NODELAY
    pub nodelay: bool,
    /// Enable SO_REUSEADDR
    pub reuseaddr: bool,
    /// Enable SO_REUSEPORT
    pub reuseport: bool,
    /// TCP keep-alive interval
    pub keepalive_secs: Option<u32>,
    /// Send buffer size
    pub send_buffer: Option<usize>,
    /// Receive buffer size
    pub recv_buffer: Option<usize>,
    /// TCP backlog size
    pub backlog: u32,
}

impl Default for TcpOptions {
    fn default() -> Self {
        Self {
            nodelay: true,
            reuseaddr: true,
            reuseport: true,
            keepalive_secs: Some(60),
            send_buffer: Some(65536),
            recv_buffer: Some(65536),
            backlog: 1024,
        }
    }
}

impl TcpOptions {
    /// High-performance options
    pub fn high_performance() -> Self {
        Self {
            nodelay: true,
            reuseaddr: true,
            reuseport: true,
            keepalive_secs: Some(120),
            send_buffer: Some(262144), // 256KB
            recv_buffer: Some(262144),
            backlog: 4096,
        }
    }

    /// Low-latency options
    pub fn low_latency() -> Self {
        Self {
            nodelay: true,
            reuseaddr: true,
            reuseport: false,
            keepalive_secs: Some(30),
            send_buffer: Some(32768),
            recv_buffer: Some(32768),
            backlog: 512,
        }
    }
}

// ============================================================================
// io_uring Operation Types
// ============================================================================

/// Types of io_uring operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoOp {
    /// Accept new connection
    Accept,
    /// Read from socket
    Read,
    /// Write to socket
    Write,
    /// Close socket
    Close,
    /// Connect to remote
    Connect,
    /// Send with MSG_ZEROCOPY
    SendZC,
    /// Receive with provided buffers
    RecvBuf,
    /// Poll for events
    Poll,
    /// Timeout
    Timeout,
    /// Cancel operation
    Cancel,
    /// Link operations
    Link,
    /// No-op (for benchmarking)
    Nop,
}

impl IoOp {
    /// Get the opcode for this operation.
    ///
    /// **Reference data only:** these constants mirror the kernel's
    /// `IORING_OP_*` values for documentation/diagnostic purposes. To check
    /// what the running kernel actually supports, use
    /// [`IoUringFeatures::detect`] (or `io_uring::Probe` directly when the
    /// `io-uring` feature is enabled) instead of comparing against this
    /// table.
    pub fn opcode(self) -> u8 {
        match self {
            Self::Accept => 13,
            Self::Read => 22,
            Self::Write => 23,
            Self::Close => 19,
            Self::Connect => 16,
            Self::SendZC => 52,
            Self::RecvBuf => 58,
            Self::Poll => 6,
            Self::Timeout => 11,
            Self::Cancel => 14,
            Self::Link => 255, // Not a real opcode
            Self::Nop => 0,
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
    fn test_config_builder() {
        let config = IoUringConfig::builder()
            .ring_size(2048)
            .sqpoll(true)
            .buffer_size(32768)
            .build();

        assert_eq!(config.ring_size, 2048);
        assert!(config.sqpoll);
        assert_eq!(config.buffer_size, 32768);
    }

    #[test]
    fn test_high_performance_config() {
        let config = IoUringConfig::high_performance();
        assert_eq!(config.ring_size, 8192);
        assert!(config.sqpoll);
    }

    #[test]
    fn test_stats() {
        let stats = IoUringStats::new();

        stats.record_submission();
        stats.record_submission();
        stats.record_completion();

        assert_eq!(stats.submissions(), 2);
        assert_eq!(stats.completions(), 1);
        assert_eq!(stats.pending(), 1);

        stats.record_read(1024);
        stats.record_write(2048);
        assert_eq!(stats.bytes_read(), 1024);
        assert_eq!(stats.bytes_written(), 2048);
    }

    #[test]
    fn test_io_backend() {
        let backend = IoBackend::Auto;
        let resolved = backend.resolve();
        // Should resolve to something
        assert!(matches!(resolved, IoBackend::Epoll | IoBackend::IoUring));
    }

    #[test]
    fn test_buffer_pool() {
        let pool = BufferPool::new(10, 1024);
        assert_eq!(pool.capacity(), 10);
        assert_eq!(pool.available(), 10);
        assert_eq!(pool.buffer_size(), 1024);

        // Acquire a buffer
        let mut buf = pool.acquire().unwrap();
        assert_eq!(buf.len(), 1024);
        assert_eq!(pool.available(), 9);
        buf[0] = 42;

        // Released on drop
        drop(buf);
        assert_eq!(pool.available(), 10);
    }

    #[test]
    fn test_buffer_pool_exclusive_buffers() {
        let pool = BufferPool::new(2, 64);

        // Two concurrent acquisitions must hand out distinct buffers
        let a = pool.acquire().unwrap();
        let b = pool.acquire().unwrap();
        assert_ne!(a.index(), b.index());
        assert_eq!(pool.available(), 0);

        // Pool exhausted
        assert!(pool.acquire().is_none());

        drop(a);
        drop(b);
        assert_eq!(pool.available(), 2);

        // Re-acquire after release works
        let c = pool.acquire().unwrap();
        assert_eq!(c.len(), 64);
    }

    #[test]
    fn test_tcp_options() {
        let opts = TcpOptions::high_performance();
        assert!(opts.nodelay);
        assert!(opts.reuseport);
        assert_eq!(opts.backlog, 4096);
    }

    #[test]
    fn test_features_detect() {
        let features = IoUringFeatures::detect();
        // Just verify it doesn't panic
        let _ = features.basic;
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_parse_kernel_version() {
        let version = "Linux version 5.15.0-generic (buildd@lcy02-amd64-086)";
        let parsed = parse_kernel_version(version);
        assert_eq!(parsed, Some((5, 15)));

        let version2 = "Linux version 6.1.0-18-amd64 (debian-kernel@lists.debian.org)";
        let parsed2 = parse_kernel_version(version2);
        assert_eq!(parsed2, Some((6, 1)));
    }

    #[test]
    fn test_io_op_opcodes() {
        assert_eq!(IoOp::Nop.opcode(), 0);
        assert_eq!(IoOp::Accept.opcode(), 13);
        assert_eq!(IoOp::Read.opcode(), 22);
    }

    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    mod runtime_tests {
        use super::super::*;
        use std::io::Read as _;
        use std::os::fd::AsFd;

        /// Create a runtime, or None (with a message) if the kernel refuses
        /// io_uring (e.g. EPERM in containers, ENOSYS on old kernels).
        fn runtime_or_skip(config: IoUringConfig) -> Option<IoUringRuntime> {
            match IoUringRuntime::new(config) {
                Ok(rt) => Some(rt),
                Err(e) => {
                    eprintln!("skipping io_uring test: kernel refused ring creation: {e}");
                    None
                }
            }
        }

        #[test]
        fn test_write_read_round_trip() {
            let config = IoUringConfig::builder().ring_size(64).build();
            let Some(runtime) = runtime_or_skip(config) else {
                return;
            };

            let path = std::env::temp_dir().join(format!(
                "armature-io-uring-roundtrip-{}",
                std::process::id()
            ));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .expect("create temp file");

            let data = b"hello from io_uring: the quick brown fox";

            // Write via the ring
            let written = runtime
                .write_at(file.as_fd(), data, 0)
                .expect("write_at failed");
            assert_eq!(written, data.len());

            // Read back via the ring
            let mut buf = vec![0u8; data.len()];
            let read = runtime
                .read_at(file.as_fd(), &mut buf, 0)
                .expect("read_at failed");
            assert_eq!(read, data.len());
            assert_eq!(&buf[..], &data[..]);

            // Cross-check against ordinary file I/O
            let mut verify = Vec::new();
            let mut reopened = std::fs::File::open(&path).expect("reopen temp file");
            reopened.read_to_end(&mut verify).expect("std read");
            assert_eq!(&verify[..], &data[..]);

            // Stats flowed through the real submission path
            assert_eq!(runtime.stats().submissions(), 2);
            assert_eq!(runtime.stats().completions(), 2);
            assert_eq!(runtime.stats().pending(), 0);
            assert_eq!(runtime.stats().errors(), 0);
            assert_eq!(runtime.stats().bytes_written(), data.len() as u64);
            assert_eq!(runtime.stats().bytes_read(), data.len() as u64);

            drop(file);
            drop(reopened);
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_read_at_eof_returns_zero() {
            let Some(runtime) = runtime_or_skip(IoUringConfig::low_resource()) else {
                return;
            };

            let path =
                std::env::temp_dir().join(format!("armature-io-uring-eof-{}", std::process::id()));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .expect("create temp file");

            let mut buf = [0u8; 16];
            let read = runtime
                .read_at(file.as_fd(), &mut buf, 0)
                .expect("read_at failed");
            assert_eq!(read, 0, "empty file should read 0 bytes (EOF)");

            drop(file);
            let _ = std::fs::remove_file(&path);
        }

        #[test]
        fn test_error_result_recorded_in_stats() {
            let Some(runtime) = runtime_or_skip(IoUringConfig::default()) else {
                return;
            };

            // /dev/null opened read-only: writing must fail with EBADF.
            let file = std::fs::File::open("/dev/null").expect("open /dev/null");
            let err = runtime
                .write_at(file.as_fd(), b"nope", 0)
                .expect_err("write to read-only fd should fail");
            assert_eq!(err.raw_os_error(), Some(libc::EBADF));
            assert_eq!(runtime.stats().errors(), 1);
            assert_eq!(runtime.stats().submissions(), 1);
            assert_eq!(runtime.stats().completions(), 1);
            assert_eq!(runtime.stats().bytes_written(), 0);
        }

        #[test]
        fn test_is_available_probes() {
            // On this machine (Linux with io_uring compiled in) availability
            // should agree with whether we can actually build a ring.
            let can_build = IoUringRuntime::new(IoUringConfig::low_resource()).is_ok();
            assert_eq!(IoUringRuntime::is_available(), can_build);
        }

        #[test]
        fn test_features_detect_probed() {
            let features = IoUringFeatures::detect();
            if !features.basic {
                eprintln!("skipping: io_uring not available for feature probing");
                return;
            }
            // Fixed files (IORING_OP_READ_FIXED) exist since 5.1, so any
            // kernel that can create a ring should support them.
            assert!(features.fixed_files);
        }

        #[test]
        fn test_sqpoll_config_falls_back() {
            // SQPOLL may or may not be permitted; either way construction
            // must not fail because of it.
            let config = IoUringConfig::builder().ring_size(32).sqpoll(true).build();
            let Some(runtime) = runtime_or_skip(config) else {
                return;
            };
            let _ = runtime.stats();
        }
    }
}
