//! Write Buffer Coalescing Module
//!
//! This module provides write buffer coalescing to combine multiple small
//! writes into a single larger write, reducing syscall overhead and improving
//! TCP efficiency.
//!
//! ## The Problem
//!
//! Without coalescing, each small write triggers:
//! 1. A syscall (expensive context switch)
//! 2. Potential TCP small packet issues (Nagle's algorithm)
//! 3. Multiple kernel copies
//!
//! ```text
//! write("HTTP/1.1 200 OK\r\n")     → syscall
//! write("Content-Type: text/plain\r\n") → syscall
//! write("Content-Length: 5\r\n")   → syscall
//! write("\r\n")                     → syscall
//! write("Hello")                    → syscall
//! Total: 5 syscalls
//! ```
//!
//! ## The Solution
//!
//! With coalescing:
//! ```text
//! coalesce("HTTP/1.1 200 OK\r\n")
//! coalesce("Content-Type: text/plain\r\n")
//! coalesce("Content-Length: 5\r\n")
//! coalesce("\r\n")
//! coalesce("Hello")
//! flush() → single syscall with entire response
//! Total: 1 syscall
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::write_coalesce::{WriteCoalescer, CoalesceConfig};
//!
//! let mut coalescer = WriteCoalescer::new(CoalesceConfig::default());
//!
//! // Small writes are buffered
//! coalescer.write(b"HTTP/1.1 200 OK\r\n");
//! coalescer.write(b"Content-Type: application/json\r\n");
//! coalescer.write(b"\r\n");
//! coalescer.write(b"{\"status\":\"ok\"}");
//!
//! // Flush when ready (or auto-flush on threshold)
//! let data = coalescer.take();
//! socket.write_all(&data).await?;
//! ```

use bytes::{Bytes, BytesMut};
use std::io::IoSlice;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

// ============================================================================
// Constants
// ============================================================================

/// Default initial buffer capacity (4KB)
pub const DEFAULT_COALESCE_CAPACITY: usize = 4096;

/// Default flush threshold (16KB)
pub const DEFAULT_FLUSH_THRESHOLD: usize = 16384;

/// Minimum write size before coalescing (below this, always coalesce)
pub const MIN_COALESCE_SIZE: usize = 512;

/// Maximum coalesce buffer size (1MB)
pub const MAX_COALESCE_BUFFER: usize = 1024 * 1024;

/// Default flush timeout (100 microseconds)
pub const DEFAULT_FLUSH_TIMEOUT_US: u64 = 100;

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for write buffer coalescing.
#[derive(Debug, Clone)]
pub struct CoalesceConfig {
    /// Initial buffer capacity
    pub initial_capacity: usize,
    /// Flush when buffer exceeds this size
    pub flush_threshold: usize,
    /// Maximum buffer size before forcing flush
    pub max_buffer_size: usize,
    /// Minimum write size to bypass coalescing (write directly)
    pub bypass_threshold: usize,
    /// Auto-flush timeout (microseconds, 0 = disabled)
    pub flush_timeout_us: u64,
    /// Enable TCP_CORK during coalescing (Linux only)
    pub use_tcp_cork: bool,
    /// Collect statistics
    pub collect_stats: bool,
}

impl Default for CoalesceConfig {
    fn default() -> Self {
        Self {
            initial_capacity: DEFAULT_COALESCE_CAPACITY,
            flush_threshold: DEFAULT_FLUSH_THRESHOLD,
            max_buffer_size: MAX_COALESCE_BUFFER,
            bypass_threshold: 65536, // 64KB writes go direct
            flush_timeout_us: DEFAULT_FLUSH_TIMEOUT_US,
            use_tcp_cork: true,
            collect_stats: true,
        }
    }
}

impl CoalesceConfig {
    /// Create configuration for high throughput.
    pub fn high_throughput() -> Self {
        Self {
            initial_capacity: 8192,
            flush_threshold: 32768,
            max_buffer_size: MAX_COALESCE_BUFFER,
            bypass_threshold: 131072, // 128KB
            flush_timeout_us: 500,    // Longer timeout for more batching
            use_tcp_cork: true,
            collect_stats: false,
        }
    }

    /// Create configuration for low latency.
    pub fn low_latency() -> Self {
        Self {
            initial_capacity: 2048,
            flush_threshold: 4096,
            max_buffer_size: 65536,
            bypass_threshold: 16384, // 16KB
            flush_timeout_us: 10,    // Very short timeout
            use_tcp_cork: false,
            collect_stats: false,
        }
    }

    /// Create configuration for memory efficiency.
    pub fn memory_efficient() -> Self {
        Self {
            initial_capacity: 1024,
            flush_threshold: 8192,
            max_buffer_size: 65536,
            bypass_threshold: 32768,
            flush_timeout_us: 200,
            use_tcp_cork: true,
            collect_stats: true,
        }
    }
}

// ============================================================================
// Write Coalescer
// ============================================================================

/// A write buffer that coalesces small writes into larger batches.
#[derive(Debug)]
pub struct WriteCoalescer {
    /// Internal buffer
    buffer: BytesMut,
    /// Configuration
    config: CoalesceConfig,
    /// Number of writes coalesced
    writes_coalesced: usize,
    /// First write timestamp (for timeout)
    first_write_time: Option<Instant>,
    /// Total bytes written to this coalescer
    total_bytes: usize,
}

impl WriteCoalescer {
    /// Create a new write coalescer with default configuration.
    pub fn new(config: CoalesceConfig) -> Self {
        Self {
            buffer: BytesMut::with_capacity(config.initial_capacity),
            config,
            writes_coalesced: 0,
            first_write_time: None,
            total_bytes: 0,
        }
    }

    /// Create with default configuration.
    pub fn default_config() -> Self {
        Self::new(CoalesceConfig::default())
    }

    /// Write data to the coalesce buffer.
    ///
    /// Returns `WriteResult` indicating what action should be taken.
    #[inline]
    pub fn write(&mut self, data: &[u8]) -> WriteResult {
        if data.is_empty() {
            return WriteResult::Buffered;
        }

        // Large writes bypass coalescing
        if data.len() >= self.config.bypass_threshold {
            COALESCE_STATS.record_bypass(data.len());
            return WriteResult::Bypass(Bytes::copy_from_slice(data));
        }

        // Track first write time for timeout
        if self.first_write_time.is_none() {
            self.first_write_time = Some(Instant::now());
        }

        // Append to buffer
        self.buffer.extend_from_slice(data);
        self.writes_coalesced += 1;
        self.total_bytes += data.len();

        if self.config.collect_stats {
            COALESCE_STATS.record_coalesce(data.len());
        }

        // Check if we should flush
        if self.should_flush() {
            WriteResult::ShouldFlush
        } else {
            WriteResult::Buffered
        }
    }

    /// Write bytes directly (owned).
    #[inline]
    pub fn write_bytes(&mut self, data: Bytes) -> WriteResult {
        if data.is_empty() {
            return WriteResult::Buffered;
        }

        // Large writes bypass coalescing
        if data.len() >= self.config.bypass_threshold {
            COALESCE_STATS.record_bypass(data.len());
            return WriteResult::Bypass(data);
        }

        // Track first write time for timeout
        if self.first_write_time.is_none() {
            self.first_write_time = Some(Instant::now());
        }

        // Append to buffer
        self.buffer.extend_from_slice(&data);
        self.writes_coalesced += 1;
        self.total_bytes += data.len();

        if self.config.collect_stats {
            COALESCE_STATS.record_coalesce(data.len());
        }

        // Check if we should flush
        if self.should_flush() {
            WriteResult::ShouldFlush
        } else {
            WriteResult::Buffered
        }
    }

    /// Check if buffer should be flushed.
    #[inline]
    pub fn should_flush(&self) -> bool {
        // Size threshold
        if self.buffer.len() >= self.config.flush_threshold {
            return true;
        }

        // Max buffer size
        if self.buffer.len() >= self.config.max_buffer_size {
            return true;
        }

        // Timeout (if enabled)
        if self.config.flush_timeout_us > 0
            && let Some(first_time) = self.first_write_time
        {
            let elapsed_us = first_time.elapsed().as_micros() as u64;
            if elapsed_us >= self.config.flush_timeout_us {
                return true;
            }
        }

        false
    }

    /// Check if buffer must be flushed (hard limits reached).
    #[inline]
    pub fn must_flush(&self) -> bool {
        self.buffer.len() >= self.config.max_buffer_size
    }

    /// Get current buffer length.
    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Get number of writes coalesced.
    #[inline]
    pub fn writes_coalesced(&self) -> usize {
        self.writes_coalesced
    }

    /// Get total bytes written.
    #[inline]
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Get remaining capacity before flush threshold.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.config
            .flush_threshold
            .saturating_sub(self.buffer.len())
    }

    /// Take the buffered data, resetting the coalescer.
    #[inline]
    pub fn take(&mut self) -> Bytes {
        let writes = self.writes_coalesced;
        let bytes = self.buffer.len();

        let data = self.buffer.split().freeze();

        // Reset state
        self.writes_coalesced = 0;
        self.first_write_time = None;

        if self.config.collect_stats && bytes > 0 {
            COALESCE_STATS.record_flush(writes, bytes);
        }

        data
    }

    /// Take the buffered data as BytesMut (for further modification).
    #[inline]
    pub fn take_mut(&mut self) -> BytesMut {
        let writes = self.writes_coalesced;
        let bytes = self.buffer.len();

        let data = self.buffer.split();

        // Reset state
        self.writes_coalesced = 0;
        self.first_write_time = None;

        if self.config.collect_stats && bytes > 0 {
            COALESCE_STATS.record_flush(writes, bytes);
        }

        data
    }

    /// Peek at the buffered data without taking it.
    #[inline]
    pub fn peek(&self) -> &[u8] {
        &self.buffer
    }

    /// Clear the buffer without returning data.
    #[inline]
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.writes_coalesced = 0;
        self.first_write_time = None;
    }

    /// Reset the coalescer completely.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.writes_coalesced = 0;
        self.first_write_time = None;
        self.total_bytes = 0;
    }

    /// Get configuration.
    #[inline]
    pub fn config(&self) -> &CoalesceConfig {
        &self.config
    }

    /// Reserve additional capacity.
    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.buffer.reserve(additional);
    }

    /// Get time since first write (for timeout checking).
    pub fn time_since_first_write(&self) -> Option<Duration> {
        self.first_write_time.map(|t| t.elapsed())
    }
}

/// Result of a write operation.
#[derive(Debug)]
pub enum WriteResult {
    /// Data was buffered, no action needed
    Buffered,
    /// Buffer should be flushed (threshold reached)
    ShouldFlush,
    /// Data bypassed coalescing (too large), write directly
    Bypass(Bytes),
}

impl WriteResult {
    /// Check if write was buffered.
    #[inline]
    pub fn is_buffered(&self) -> bool {
        matches!(self, Self::Buffered)
    }

    /// Check if flush is needed.
    #[inline]
    pub fn should_flush(&self) -> bool {
        matches!(self, Self::ShouldFlush)
    }

    /// Check if write was bypassed.
    #[inline]
    pub fn is_bypass(&self) -> bool {
        matches!(self, Self::Bypass(_))
    }

    /// Take bypass data if present.
    #[inline]
    pub fn take_bypass(self) -> Option<Bytes> {
        match self {
            Self::Bypass(data) => Some(data),
            _ => None,
        }
    }
}

// ============================================================================
// Multi-Buffer Coalescer
// ============================================================================

/// Coalescer that manages multiple buffers for different purposes.
#[derive(Debug)]
pub struct MultiBufferCoalescer {
    /// Headers buffer
    headers: WriteCoalescer,
    /// Body buffer
    body: WriteCoalescer,
    /// Trailer buffer (for chunked encoding)
    trailers: WriteCoalescer,
}

impl MultiBufferCoalescer {
    /// Create a new multi-buffer coalescer.
    pub fn new(config: CoalesceConfig) -> Self {
        Self {
            headers: WriteCoalescer::new(CoalesceConfig {
                initial_capacity: 1024,
                flush_threshold: 4096,
                max_buffer_size: 16384,
                bypass_threshold: 8192,
                ..config.clone()
            }),
            body: WriteCoalescer::new(config.clone()),
            trailers: WriteCoalescer::new(CoalesceConfig {
                initial_capacity: 256,
                flush_threshold: 1024,
                max_buffer_size: 4096,
                bypass_threshold: 2048,
                ..config
            }),
        }
    }

    /// Write to headers buffer.
    #[inline]
    pub fn write_header(&mut self, data: &[u8]) -> WriteResult {
        self.headers.write(data)
    }

    /// Write a header line (name: value\r\n).
    #[inline]
    pub fn write_header_line(&mut self, name: &str, value: &str) {
        self.headers.buffer.extend_from_slice(name.as_bytes());
        self.headers.buffer.extend_from_slice(b": ");
        self.headers.buffer.extend_from_slice(value.as_bytes());
        self.headers.buffer.extend_from_slice(b"\r\n");
        self.headers.writes_coalesced += 1;
    }

    /// Write to body buffer.
    #[inline]
    pub fn write_body(&mut self, data: &[u8]) -> WriteResult {
        self.body.write(data)
    }

    /// Write to trailers buffer.
    #[inline]
    pub fn write_trailer(&mut self, data: &[u8]) -> WriteResult {
        self.trailers.write(data)
    }

    /// Check if any buffer should be flushed.
    #[inline]
    pub fn should_flush(&self) -> bool {
        self.headers.should_flush() || self.body.should_flush() || self.trailers.should_flush()
    }

    /// Get total buffered size.
    #[inline]
    pub fn total_len(&self) -> usize {
        self.headers.len() + self.body.len() + self.trailers.len()
    }

    /// Take all buffers and combine into a single Bytes.
    pub fn take_combined(&mut self) -> Bytes {
        let total = self.total_len();
        if total == 0 {
            return Bytes::new();
        }

        let mut combined = BytesMut::with_capacity(total);
        combined.extend_from_slice(self.headers.peek());
        combined.extend_from_slice(self.body.peek());
        combined.extend_from_slice(self.trailers.peek());

        self.headers.clear();
        self.body.clear();
        self.trailers.clear();

        combined.freeze()
    }

    /// Get IoSlices for vectored I/O.
    pub fn as_io_slices(&self) -> Vec<IoSlice<'_>> {
        let mut slices = Vec::with_capacity(3);
        if !self.headers.is_empty() {
            slices.push(IoSlice::new(self.headers.peek()));
        }
        if !self.body.is_empty() {
            slices.push(IoSlice::new(self.body.peek()));
        }
        if !self.trailers.is_empty() {
            slices.push(IoSlice::new(self.trailers.peek()));
        }
        slices
    }

    /// Reset all buffers.
    pub fn reset(&mut self) {
        self.headers.reset();
        self.body.reset();
        self.trailers.reset();
    }
}

// ============================================================================
// Connection Write Buffer
// ============================================================================

/// Per-connection write buffer with coalescing and flush management.
#[derive(Debug)]
pub struct ConnectionWriteBuffer {
    /// Main coalescer
    coalescer: WriteCoalescer,
    /// Pending large writes (bypassed coalescing)
    pending_large: Vec<Bytes>,
    /// Connection ID (for logging)
    #[allow(dead_code)]
    connection_id: u64,
    /// Total flushes
    flushes: usize,
}

impl ConnectionWriteBuffer {
    /// Create a new connection write buffer.
    pub fn new(connection_id: u64, config: CoalesceConfig) -> Self {
        Self {
            coalescer: WriteCoalescer::new(config),
            pending_large: Vec::new(),
            connection_id,
            flushes: 0,
        }
    }

    /// Write data to the buffer.
    #[inline]
    pub fn write(&mut self, data: &[u8]) {
        if let WriteResult::Bypass(bytes) = self.coalescer.write(data) {
            self.pending_large.push(bytes);
        }
    }

    /// Write owned bytes.
    #[inline]
    pub fn write_bytes(&mut self, data: Bytes) {
        if let WriteResult::Bypass(bytes) = self.coalescer.write_bytes(data) {
            self.pending_large.push(bytes);
        }
    }

    /// Check if ready to flush.
    #[inline]
    pub fn should_flush(&self) -> bool {
        !self.pending_large.is_empty() || self.coalescer.should_flush()
    }

    /// Get all data ready for writing.
    pub fn take_all(&mut self) -> Vec<Bytes> {
        let mut result = Vec::with_capacity(1 + self.pending_large.len());

        // Coalesced data first
        if !self.coalescer.is_empty() {
            result.push(self.coalescer.take());
        }

        // Then large writes
        result.append(&mut self.pending_large);

        self.flushes += 1;
        result
    }

    /// Get IoSlices for all pending data.
    pub fn as_io_slices(&self) -> Vec<IoSlice<'_>> {
        let mut slices = Vec::with_capacity(1 + self.pending_large.len());

        if !self.coalescer.is_empty() {
            slices.push(IoSlice::new(self.coalescer.peek()));
        }

        for large in &self.pending_large {
            slices.push(IoSlice::new(large));
        }

        slices
    }

    /// Get total pending bytes.
    #[inline]
    pub fn pending_bytes(&self) -> usize {
        let large_bytes: usize = self.pending_large.iter().map(|b| b.len()).sum();
        self.coalescer.len() + large_bytes
    }

    /// Get number of flushes.
    #[inline]
    pub fn flushes(&self) -> usize {
        self.flushes
    }

    /// Reset the buffer.
    pub fn reset(&mut self) {
        self.coalescer.reset();
        self.pending_large.clear();
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Statistics for write coalescing.
#[derive(Debug, Default)]
pub struct CoalesceStats {
    /// Total writes coalesced
    coalesced: AtomicU64,
    /// Total bytes coalesced
    bytes_coalesced: AtomicU64,
    /// Total writes bypassed
    bypassed: AtomicU64,
    /// Total bytes bypassed
    bytes_bypassed: AtomicU64,
    /// Total flushes
    flushes: AtomicU64,
    /// Total writes per flush (sum)
    writes_per_flush_sum: AtomicU64,
    /// Maximum writes per flush
    max_writes_per_flush: AtomicUsize,
}

impl CoalesceStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a coalesced write.
    #[inline]
    pub fn record_coalesce(&self, bytes: usize) {
        self.coalesced.fetch_add(1, Ordering::Relaxed);
        self.bytes_coalesced
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record a bypassed write.
    #[inline]
    pub fn record_bypass(&self, bytes: usize) {
        self.bypassed.fetch_add(1, Ordering::Relaxed);
        self.bytes_bypassed
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record a flush.
    #[inline]
    pub fn record_flush(&self, writes: usize, _bytes: usize) {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        self.writes_per_flush_sum
            .fetch_add(writes as u64, Ordering::Relaxed);
        self.max_writes_per_flush
            .fetch_max(writes, Ordering::Relaxed);
    }

    /// Get total coalesced writes.
    pub fn coalesced(&self) -> u64 {
        self.coalesced.load(Ordering::Relaxed)
    }

    /// Get total bytes coalesced.
    pub fn bytes_coalesced(&self) -> u64 {
        self.bytes_coalesced.load(Ordering::Relaxed)
    }

    /// Get total bypassed writes.
    pub fn bypassed(&self) -> u64 {
        self.bypassed.load(Ordering::Relaxed)
    }

    /// Get total bytes bypassed.
    pub fn bytes_bypassed(&self) -> u64 {
        self.bytes_bypassed.load(Ordering::Relaxed)
    }

    /// Get total flushes.
    pub fn flushes(&self) -> u64 {
        self.flushes.load(Ordering::Relaxed)
    }

    /// Get average writes per flush.
    pub fn avg_writes_per_flush(&self) -> f64 {
        let flushes = self.flushes();
        let sum = self.writes_per_flush_sum.load(Ordering::Relaxed);
        if flushes > 0 {
            sum as f64 / flushes as f64
        } else {
            0.0
        }
    }

    /// Get max writes per flush.
    pub fn max_writes_per_flush(&self) -> usize {
        self.max_writes_per_flush.load(Ordering::Relaxed)
    }

    /// Get coalesce ratio (higher is better).
    pub fn coalesce_ratio(&self) -> f64 {
        let coalesced = self.coalesced();
        let bypassed = self.bypassed();
        let total = coalesced + bypassed;
        if total > 0 {
            (coalesced as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Get syscall reduction ratio.
    ///
    /// This estimates how many syscalls were saved by coalescing.
    /// Formula: (writes_coalesced - flushes) / writes_coalesced * 100
    pub fn syscall_reduction_ratio(&self) -> f64 {
        let writes = self.coalesced();
        let flushes = self.flushes();
        if writes > 0 {
            let saved = writes.saturating_sub(flushes);
            (saved as f64 / writes as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// Global coalesce statistics.
static COALESCE_STATS: CoalesceStats = CoalesceStats {
    coalesced: AtomicU64::new(0),
    bytes_coalesced: AtomicU64::new(0),
    bypassed: AtomicU64::new(0),
    bytes_bypassed: AtomicU64::new(0),
    flushes: AtomicU64::new(0),
    writes_per_flush_sum: AtomicU64::new(0),
    max_writes_per_flush: AtomicUsize::new(0),
};

/// Get global coalesce statistics.
pub fn coalesce_stats() -> &'static CoalesceStats {
    &COALESCE_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coalesce_config_default() {
        let config = CoalesceConfig::default();
        assert_eq!(config.initial_capacity, DEFAULT_COALESCE_CAPACITY);
        assert_eq!(config.flush_threshold, DEFAULT_FLUSH_THRESHOLD);
    }

    #[test]
    fn test_coalesce_config_presets() {
        let high = CoalesceConfig::high_throughput();
        assert!(high.flush_threshold > DEFAULT_FLUSH_THRESHOLD);

        let low = CoalesceConfig::low_latency();
        assert!(low.flush_threshold < DEFAULT_FLUSH_THRESHOLD);
    }

    #[test]
    fn test_write_coalescer_basic() {
        let mut coalescer = WriteCoalescer::new(CoalesceConfig::default());

        // Small write should be buffered
        let result = coalescer.write(b"Hello");
        // Result can be Buffered or ShouldFlush depending on timing
        assert!(!result.is_bypass());
        assert_eq!(coalescer.len(), 5);

        // Another small write
        coalescer.write(b", World!");
        assert_eq!(coalescer.len(), 13);
        assert_eq!(coalescer.writes_coalesced(), 2);

        // Take data
        let data = coalescer.take();
        assert_eq!(&data[..], b"Hello, World!");
        assert!(coalescer.is_empty());
    }

    #[test]
    fn test_write_coalescer_bypass() {
        let config = CoalesceConfig {
            bypass_threshold: 10,
            ..Default::default()
        };
        let mut coalescer = WriteCoalescer::new(config);

        // Large write should bypass
        let result = coalescer.write(b"This is a large write that exceeds threshold");
        assert!(result.is_bypass());
        assert!(coalescer.is_empty()); // Not buffered
    }

    #[test]
    fn test_write_coalescer_flush_threshold() {
        let config = CoalesceConfig {
            flush_threshold: 20,
            ..Default::default()
        };
        let mut coalescer = WriteCoalescer::new(config);

        coalescer.write(b"12345");
        assert!(!coalescer.should_flush());

        coalescer.write(b"1234567890");
        assert!(!coalescer.should_flush()); // 15 bytes

        coalescer.write(b"12345");
        assert!(coalescer.should_flush()); // 20 bytes = threshold
    }

    #[test]
    fn test_write_result_methods() {
        let buffered = WriteResult::Buffered;
        assert!(buffered.is_buffered());
        assert!(!buffered.should_flush());
        assert!(!buffered.is_bypass());

        let should_flush = WriteResult::ShouldFlush;
        assert!(!should_flush.is_buffered());
        assert!(should_flush.should_flush());

        let bypass = WriteResult::Bypass(Bytes::from_static(b"test"));
        assert!(bypass.is_bypass());
        if let Some(data) = bypass.take_bypass() {
            assert_eq!(&data[..], b"test");
        }
    }

    #[test]
    fn test_multi_buffer_coalescer() {
        let mut coalescer = MultiBufferCoalescer::new(CoalesceConfig::default());

        coalescer.write_header(b"HTTP/1.1 200 OK\r\n");
        coalescer.write_header_line("Content-Type", "text/plain");
        coalescer.write_header(b"\r\n");
        coalescer.write_body(b"Hello, World!");

        assert!(coalescer.total_len() > 0);

        let slices = coalescer.as_io_slices();
        assert_eq!(slices.len(), 2); // headers + body

        let combined = coalescer.take_combined();
        assert!(!combined.is_empty());
    }

    #[test]
    fn test_connection_write_buffer() {
        let mut buffer = ConnectionWriteBuffer::new(1, CoalesceConfig::default());

        buffer.write(b"Small write 1");
        buffer.write(b"Small write 2");
        buffer.write(b"Small write 3");

        assert!(buffer.pending_bytes() > 0);

        let data = buffer.take_all();
        assert!(!data.is_empty());
        assert_eq!(buffer.flushes(), 1);
    }

    #[test]
    fn test_connection_write_buffer_large_bypass() {
        let config = CoalesceConfig {
            bypass_threshold: 10,
            ..Default::default()
        };
        let mut buffer = ConnectionWriteBuffer::new(1, config);

        buffer.write(b"Small");
        buffer.write(b"This is a large write that will be bypassed");

        // Should have coalesced + pending large
        let slices = buffer.as_io_slices();
        assert_eq!(slices.len(), 2);
    }

    #[test]
    fn test_coalesce_stats() {
        let stats = coalesce_stats();
        // Just verify we can read stats
        let _ = stats.coalesced();
        let _ = stats.bypassed();
        let _ = stats.flushes();
        let _ = stats.coalesce_ratio();
        let _ = stats.syscall_reduction_ratio();
    }

    #[test]
    fn test_take_mut() {
        let mut coalescer = WriteCoalescer::new(CoalesceConfig::default());
        coalescer.write(b"Hello");

        let mut buf = coalescer.take_mut();
        buf.extend_from_slice(b", World!");

        assert_eq!(&buf[..], b"Hello, World!");
        assert!(coalescer.is_empty());
    }

    #[test]
    fn test_reserve() {
        let mut coalescer = WriteCoalescer::new(CoalesceConfig::default());
        coalescer.reserve(10000);
        coalescer.write(b"Now we have plenty of space");
        assert!(coalescer.len() < 10000);
    }
}
