//! Response Pipelining Module
//!
//! This module provides response queuing and batch-write capabilities
//! for HTTP/1.1 pipelined connections. It ensures responses are sent
//! in the correct order while maximizing I/O efficiency.
//!
//! ## How It Works
//!
//! 1. Responses are queued with their request sequence number
//! 2. The queue reorders responses if they complete out-of-order
//! 3. Batch writes combine multiple responses into a single syscall
//! 4. TCP_CORK/TCP_NODELAY are used strategically for efficiency
//!
//! ## Performance Benefits
//!
//! - **Reduced syscalls**: Multiple responses sent in one write
//! - **Better TCP efficiency**: Optimal packet sizing
//! - **Maintained ordering**: HTTP/1.1 compliance
//! - **Lower latency**: Pipelining reduces round-trip overhead
//!
//! ## Example
//!
//! ```rust,ignore
//! use armature_core::response_pipeline::{ResponseQueue, ResponseItem};
//!
//! let mut queue = ResponseQueue::new(16);
//!
//! // Responses can complete out of order
//! queue.push(ResponseItem::new(2, response2));
//! queue.push(ResponseItem::new(0, response0));
//! queue.push(ResponseItem::new(1, response1));
//!
//! // Drain returns them in order: 0, 1, 2
//! while let Some(item) = queue.pop_ready() {
//!     send_response(item.response).await?;
//! }
//! ```

use bytes::{Bytes, BytesMut};
use std::collections::VecDeque;
use std::io::IoSlice;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

// ============================================================================
// Response Item
// ============================================================================

/// A response item in the pipeline queue.
#[derive(Debug)]
pub struct ResponseItem {
    /// Request sequence number (for ordering)
    pub sequence: u64,
    /// Serialized response data
    pub data: Bytes,
    /// Response size in bytes
    pub size: usize,
}

impl ResponseItem {
    /// Create a new response item.
    #[inline]
    pub fn new(sequence: u64, data: Bytes) -> Self {
        let size = data.len();
        Self {
            sequence,
            data,
            size,
        }
    }

    /// Create from raw bytes.
    #[inline]
    pub fn from_vec(sequence: u64, data: Vec<u8>) -> Self {
        Self::new(sequence, Bytes::from(data))
    }

    /// Create from a static slice.
    #[inline]
    pub fn from_static(sequence: u64, data: &'static [u8]) -> Self {
        Self::new(sequence, Bytes::from_static(data))
    }
}

// ============================================================================
// Response Queue
// ============================================================================

/// A queue for pipelined responses that maintains ordering.
///
/// Responses can be pushed in any order but are only released
/// in sequence order (0, 1, 2, ...).
#[derive(Debug)]
pub struct ResponseQueue {
    /// Pending responses (may be out of order). Stored in a `VecDeque` so
    /// that promoting the front item to `ready` (see `promote_ready`) is
    /// O(1) amortized instead of the O(n) shift a `Vec::remove(0)` would
    /// incur on every promotion.
    pending: VecDeque<Option<ResponseItem>>,
    /// Next expected sequence number
    next_sequence: u64,
    /// Ready responses (in order, ready to send)
    ready: VecDeque<ResponseItem>,
    /// Running total of `ready` item sizes. Maintained incrementally because
    /// the hot path (`should_flush` / `must_flush`) asks for it on every
    /// poll, and re-summing the deque each time would make a cheap predicate
    /// walk the whole queue.
    ready_bytes: usize,
    /// Number of occupied slots in `pending`. Same rationale as
    /// `ready_bytes`: `has_pending` only needs a boolean and should not scan.
    pending_occupied: usize,
    /// Maximum pending responses
    capacity: usize,
    /// Statistics
    stats: ResponseQueueStats,
}

impl ResponseQueue {
    /// Create a new response queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut pending = VecDeque::with_capacity(capacity);
        pending.resize_with(capacity, || None);
        Self {
            pending,
            next_sequence: 0,
            ready: VecDeque::with_capacity(capacity),
            ready_bytes: 0,
            pending_occupied: 0,
            capacity,
            stats: ResponseQueueStats::new(),
        }
    }

    /// Push a response into the queue.
    ///
    /// Returns `true` if the response was accepted, `false` if the queue is full
    /// or the sequence number is out of range.
    #[inline]
    pub fn push(&mut self, item: ResponseItem) -> bool {
        // Check if sequence is in acceptable range
        if item.sequence < self.next_sequence {
            // Already sent this sequence (duplicate)
            self.stats.record_duplicate();
            return false;
        }

        let offset = (item.sequence - self.next_sequence) as usize;
        if offset >= self.capacity {
            // Too far ahead
            self.stats.record_overflow();
            return false;
        }

        // Ensure pending vec is large enough
        while self.pending.len() <= offset {
            self.pending.push_back(None);
        }

        self.stats.record_push(item.size);
        // A re-push of an already-occupied slot replaces it rather than
        // adding one, so only count genuinely new occupants.
        if self.pending[offset].replace(item).is_none() {
            self.pending_occupied += 1;
        }

        // Move any ready items to the ready queue
        self.promote_ready();

        true
    }

    /// Move items from pending to ready queue.
    ///
    /// Uses `VecDeque::pop_front` rather than `Vec::remove(0)` so each
    /// promoted item is removed in O(1) amortized time instead of O(n),
    /// keeping the overall promotion loop O(n) rather than O(n^2).
    fn promote_ready(&mut self) {
        while let Some(slot) = self.pending.front_mut() {
            if let Some(item) = slot.take() {
                self.pending.pop_front();
                self.pending_occupied -= 1;
                self.ready_bytes += item.size;
                self.ready.push_back(item);
                self.next_sequence += 1;
            } else {
                break;
            }
        }
    }

    /// Pop the next ready response.
    #[inline]
    pub fn pop_ready(&mut self) -> Option<ResponseItem> {
        let item = self.ready.pop_front();
        if let Some(item) = &item {
            self.ready_bytes -= item.size;
            self.stats.record_pop(item.size);
        }
        item
    }

    /// Peek at the next ready response without removing it.
    #[inline]
    pub fn peek_ready(&self) -> Option<&ResponseItem> {
        self.ready.front()
    }

    /// Check if there are ready responses.
    #[inline]
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }

    /// Get the number of ready responses.
    #[inline]
    pub fn ready_count(&self) -> usize {
        self.ready.len()
    }

    /// Get the number of pending (out-of-order) responses.
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.pending_occupied
    }

    /// Get the next expected sequence number.
    #[inline]
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Get total bytes ready to send.
    #[inline]
    pub fn ready_bytes(&self) -> usize {
        self.ready_bytes
    }

    /// Get queue statistics.
    #[inline]
    pub fn stats(&self) -> &ResponseQueueStats {
        &self.stats
    }

    /// Drain all ready responses into a batch.
    #[inline]
    pub fn drain_ready(&mut self) -> ResponseBatch {
        let items: Vec<_> = self.ready.drain(..).collect();
        // Everything left, so the running total is the batch total.
        let total_size = self.ready_bytes;
        self.ready_bytes = 0;
        self.stats.record_batch_drain(items.len(), total_size);
        ResponseBatch { items, total_size }
    }

    /// Drain up to `max` ready responses.
    #[inline]
    pub fn drain_ready_max(&mut self, max: usize) -> ResponseBatch {
        let count = max.min(self.ready.len());
        let drains_all = count == self.ready.len();
        let items: Vec<_> = self.ready.drain(..count).collect();
        let total_size = if drains_all {
            // Whole queue: reuse the running total instead of re-summing.
            std::mem::take(&mut self.ready_bytes)
        } else {
            let drained: usize = items.iter().map(|r| r.size).sum();
            self.ready_bytes -= drained;
            drained
        };
        self.stats.record_batch_drain(items.len(), total_size);
        ResponseBatch { items, total_size }
    }

    /// Clear all pending and ready responses.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.pending.resize_with(self.capacity, || None);
        self.pending_occupied = 0;
        self.ready.clear();
        self.ready_bytes = 0;
        // Note: don't reset next_sequence to maintain ordering
    }

    /// Reset the queue completely (including sequence counter).
    pub fn reset(&mut self) {
        self.clear();
        self.next_sequence = 0;
    }
}

// ============================================================================
// Response Batch
// ============================================================================

/// A batch of responses ready for sending.
#[derive(Debug)]
pub struct ResponseBatch {
    /// Response items in order
    pub items: Vec<ResponseItem>,
    /// Total size in bytes
    pub total_size: usize,
}

impl ResponseBatch {
    /// Check if batch is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Get number of responses.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Convert to IoSlices for vectored I/O.
    #[inline]
    pub fn as_io_slices(&self) -> Vec<IoSlice<'_>> {
        self.items
            .iter()
            .map(|item| IoSlice::new(&item.data))
            .collect()
    }

    /// Concatenate all responses into a single buffer.
    ///
    /// Use this for non-vectored I/O or when buffer coalescing is preferred.
    #[inline]
    pub fn concat(&self) -> Bytes {
        if self.items.len() == 1 {
            // Single response, just clone the Bytes (cheap)
            return self.items[0].data.clone();
        }

        let mut buf = BytesMut::with_capacity(self.total_size);
        for item in &self.items {
            buf.extend_from_slice(&item.data);
        }
        buf.freeze()
    }

    /// Take ownership of items.
    #[inline]
    pub fn into_items(self) -> Vec<ResponseItem> {
        self.items
    }
}

// ============================================================================
// Response Queue Statistics
// ============================================================================

/// Statistics for response queue operations.
#[derive(Debug, Default)]
pub struct ResponseQueueStats {
    /// Total responses pushed
    pushed: AtomicU64,
    /// Total responses popped
    popped: AtomicU64,
    /// Total bytes pushed
    bytes_pushed: AtomicU64,
    /// Total bytes popped
    bytes_popped: AtomicU64,
    /// Duplicate responses (already sent)
    duplicates: AtomicU64,
    /// Overflow (too far ahead)
    overflows: AtomicU64,
    /// Batch drains
    batch_drains: AtomicU64,
    /// Total batched responses
    batched_responses: AtomicU64,
}

impl ResponseQueueStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn record_push(&self, size: usize) {
        self.pushed.fetch_add(1, Ordering::Relaxed);
        self.bytes_pushed.fetch_add(size as u64, Ordering::Relaxed);
    }

    #[inline]
    fn record_pop(&self, size: usize) {
        self.popped.fetch_add(1, Ordering::Relaxed);
        self.bytes_popped.fetch_add(size as u64, Ordering::Relaxed);
    }

    #[inline]
    fn record_duplicate(&self) {
        self.duplicates.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_overflow(&self) {
        self.overflows.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_batch_drain(&self, count: usize, _size: usize) {
        self.batch_drains.fetch_add(1, Ordering::Relaxed);
        self.batched_responses
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    /// Get total pushed.
    pub fn pushed(&self) -> u64 {
        self.pushed.load(Ordering::Relaxed)
    }

    /// Get total popped.
    pub fn popped(&self) -> u64 {
        self.popped.load(Ordering::Relaxed)
    }

    /// Get bytes pushed.
    pub fn bytes_pushed(&self) -> u64 {
        self.bytes_pushed.load(Ordering::Relaxed)
    }

    /// Get bytes popped.
    pub fn bytes_popped(&self) -> u64 {
        self.bytes_popped.load(Ordering::Relaxed)
    }

    /// Get duplicate count.
    pub fn duplicates(&self) -> u64 {
        self.duplicates.load(Ordering::Relaxed)
    }

    /// Get overflow count.
    pub fn overflows(&self) -> u64 {
        self.overflows.load(Ordering::Relaxed)
    }

    /// Get batch drain count.
    pub fn batch_drains(&self) -> u64 {
        self.batch_drains.load(Ordering::Relaxed)
    }

    /// Get average batch size.
    pub fn avg_batch_size(&self) -> f64 {
        let drains = self.batch_drains();
        let batched = self.batched_responses.load(Ordering::Relaxed);
        if drains > 0 {
            batched as f64 / drains as f64
        } else {
            0.0
        }
    }
}

// ============================================================================
// Response Writer
// ============================================================================

/// Configuration for response writing.
#[derive(Debug, Clone)]
pub struct ResponseWriterConfig {
    /// Minimum responses to batch before flushing
    pub min_batch_size: usize,
    /// Maximum responses to batch before forcing flush
    pub max_batch_size: usize,
    /// Minimum bytes to batch before flushing
    pub min_batch_bytes: usize,
    /// Maximum bytes to batch before forcing flush
    pub max_batch_bytes: usize,
    /// Whether the connection's socket should be corked (via `TCP_CORK`,
    /// Linux only) while a batch is being written, so multiple queued
    /// responses coalesce into fewer TCP segments.
    ///
    /// This struct only *decides* the policy; it does not itself hold a
    /// socket, so nothing in this module applies the setsockopt call.
    /// Callers that own the connection's file descriptor should read this
    /// flag via [`ConnectionPipeline::config`] and apply it with
    /// [`ConnectionPipeline::apply_cork`] (which wraps
    /// [`crate::socket_batch::set_tcp_cork`]) immediately before writing a
    /// [`ResponseBatch`] and again to uncork once the write completes.
    pub use_tcp_cork: bool,
    /// Flush timeout in microseconds
    pub flush_timeout_us: u64,
}

impl Default for ResponseWriterConfig {
    fn default() -> Self {
        Self {
            min_batch_size: 1,
            max_batch_size: 64,
            min_batch_bytes: 1024,
            max_batch_bytes: 65536,
            use_tcp_cork: true,
            flush_timeout_us: 100,
        }
    }
}

impl ResponseWriterConfig {
    /// Create configuration for maximum throughput.
    pub fn high_throughput() -> Self {
        Self {
            min_batch_size: 8,
            max_batch_size: 128,
            min_batch_bytes: 8192,
            max_batch_bytes: 131072,
            use_tcp_cork: true,
            flush_timeout_us: 500,
        }
    }

    /// Create configuration for minimum latency.
    pub fn low_latency() -> Self {
        Self {
            min_batch_size: 1,
            max_batch_size: 16,
            min_batch_bytes: 512,
            max_batch_bytes: 16384,
            use_tcp_cork: false,
            flush_timeout_us: 10,
        }
    }
}

/// Writer statistics.
#[derive(Debug, Default)]
pub struct ResponseWriterStats {
    /// Total writes
    writes: AtomicU64,
    /// Total bytes written
    bytes_written: AtomicU64,
    /// Vectored writes (writev)
    vectored_writes: AtomicU64,
    /// Coalesced writes (buffer concat)
    coalesced_writes: AtomicU64,
}

impl ResponseWriterStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a write.
    #[inline]
    pub fn record_write(&self, bytes: usize, vectored: bool) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        self.bytes_written
            .fetch_add(bytes as u64, Ordering::Relaxed);
        if vectored {
            self.vectored_writes.fetch_add(1, Ordering::Relaxed);
        } else {
            self.coalesced_writes.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get total writes.
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }

    /// Get bytes written.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    /// Get vectored write count.
    pub fn vectored_writes(&self) -> u64 {
        self.vectored_writes.load(Ordering::Relaxed)
    }

    /// Get coalesced write count.
    pub fn coalesced_writes(&self) -> u64 {
        self.coalesced_writes.load(Ordering::Relaxed)
    }

    /// Get vectored write ratio.
    pub fn vectored_ratio(&self) -> f64 {
        let total = self.writes();
        let vectored = self.vectored_writes();
        if total > 0 {
            (vectored as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// Global response writer statistics.
static WRITER_STATS: ResponseWriterStats = ResponseWriterStats {
    writes: AtomicU64::new(0),
    bytes_written: AtomicU64::new(0),
    vectored_writes: AtomicU64::new(0),
    coalesced_writes: AtomicU64::new(0),
};

/// Get global writer statistics.
pub fn writer_stats() -> &'static ResponseWriterStats {
    &WRITER_STATS
}

// ============================================================================
// Connection Response Pipeline
// ============================================================================

/// A per-connection response pipeline.
///
/// Manages the full lifecycle of response pipelining for a single connection:
/// - Queue management
/// - Batch preparation
/// - Statistics tracking
#[derive(Debug)]
pub struct ConnectionPipeline {
    /// Response queue
    queue: ResponseQueue,
    /// Configuration
    config: ResponseWriterConfig,
    /// Sequence counter
    sequence_counter: u64,
    /// Connection ID (for logging)
    #[allow(dead_code)]
    connection_id: u64,
}

impl ConnectionPipeline {
    /// Create a new connection pipeline.
    pub fn new(connection_id: u64, queue_capacity: usize) -> Self {
        Self {
            queue: ResponseQueue::new(queue_capacity),
            config: ResponseWriterConfig::default(),
            sequence_counter: 0,
            connection_id,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(
        connection_id: u64,
        queue_capacity: usize,
        config: ResponseWriterConfig,
    ) -> Self {
        Self {
            queue: ResponseQueue::new(queue_capacity),
            config,
            sequence_counter: 0,
            connection_id,
        }
    }

    /// Get the next sequence number for a request.
    #[inline]
    pub fn next_sequence(&mut self) -> u64 {
        let seq = self.sequence_counter;
        self.sequence_counter += 1;
        seq
    }

    /// Queue a response.
    #[inline]
    pub fn queue_response(&mut self, sequence: u64, data: Bytes) -> bool {
        self.queue.push(ResponseItem::new(sequence, data))
    }

    /// Check if ready to flush.
    #[inline]
    pub fn should_flush(&self) -> bool {
        let count = self.queue.ready_count();
        let bytes = self.queue.ready_bytes();

        count >= self.config.min_batch_size
            || bytes >= self.config.min_batch_bytes
            || count >= self.config.max_batch_size
            || bytes >= self.config.max_batch_bytes
    }

    /// Check if a flush is required (max limits reached).
    #[inline]
    pub fn must_flush(&self) -> bool {
        let count = self.queue.ready_count();
        let bytes = self.queue.ready_bytes();

        count >= self.config.max_batch_size || bytes >= self.config.max_batch_bytes
    }

    /// Drain ready responses for sending.
    #[inline]
    pub fn drain_for_send(&mut self) -> ResponseBatch {
        self.queue.drain_ready()
    }

    /// Drain up to N ready responses.
    #[inline]
    pub fn drain_max(&mut self, max: usize) -> ResponseBatch {
        self.queue.drain_ready_max(max)
    }

    /// Get queue statistics.
    #[inline]
    pub fn queue_stats(&self) -> &ResponseQueueStats {
        self.queue.stats()
    }

    /// Check if there are pending responses.
    #[inline]
    pub fn has_pending(&self) -> bool {
        // O(1): `pending_count` is a maintained counter, not a scan.
        self.queue.pending_count() > 0
    }

    /// Check if there are ready responses.
    #[inline]
    pub fn has_ready(&self) -> bool {
        self.queue.has_ready()
    }

    /// Get ready count.
    #[inline]
    pub fn ready_count(&self) -> usize {
        self.queue.ready_count()
    }

    /// Get pending count.
    #[inline]
    pub fn pending_count(&self) -> usize {
        self.queue.pending_count()
    }

    /// Get total bytes ready.
    #[inline]
    pub fn ready_bytes(&self) -> usize {
        self.queue.ready_bytes()
    }

    /// Get configuration.
    #[inline]
    pub fn config(&self) -> &ResponseWriterConfig {
        &self.config
    }

    /// Apply (or clear) `TCP_CORK` on `fd` according to
    /// [`ResponseWriterConfig::use_tcp_cork`].
    ///
    /// This is a no-op that returns `Ok(())` immediately when corking is
    /// disabled in the config. Callers that own the connection's socket
    /// should invoke this with `cork = true` before writing a batch
    /// produced by [`Self::drain_for_send`] / [`Self::drain_max`], and with
    /// `cork = false` right after the write completes to flush any
    /// coalesced segments.
    #[cfg(unix)]
    pub fn apply_cork(&self, fd: std::os::unix::io::RawFd, cork: bool) -> std::io::Result<()> {
        if self.config.use_tcp_cork {
            crate::socket_batch::set_tcp_cork(fd, cork)
        } else {
            Ok(())
        }
    }

    /// Reset the pipeline (for connection reuse).
    pub fn reset(&mut self) {
        self.queue.reset();
        self.sequence_counter = 0;
    }
}

// ============================================================================
// Global Pipeline Statistics
// ============================================================================

/// Global statistics for response pipelining.
#[derive(Debug, Default)]
pub struct GlobalPipelineStats {
    /// Total responses queued
    responses_queued: AtomicU64,
    /// Total responses sent
    responses_sent: AtomicU64,
    /// Total batches sent
    batches_sent: AtomicU64,
    /// Total bytes sent
    bytes_sent: AtomicU64,
    /// Max batch size seen
    max_batch_size: AtomicUsize,
    /// Out-of-order responses (reordered)
    reordered: AtomicU64,
}

impl GlobalPipelineStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record responses queued.
    #[inline]
    pub fn record_queued(&self, count: usize) {
        self.responses_queued
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    /// Record a batch sent.
    #[inline]
    pub fn record_batch_sent(&self, responses: usize, bytes: usize) {
        self.responses_sent
            .fetch_add(responses as u64, Ordering::Relaxed);
        self.batches_sent.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent.fetch_add(bytes as u64, Ordering::Relaxed);
        self.max_batch_size.fetch_max(responses, Ordering::Relaxed);
    }

    /// Record a reorder event.
    #[inline]
    pub fn record_reorder(&self) {
        self.reordered.fetch_add(1, Ordering::Relaxed);
    }

    /// Get responses queued.
    pub fn responses_queued(&self) -> u64 {
        self.responses_queued.load(Ordering::Relaxed)
    }

    /// Get responses sent.
    pub fn responses_sent(&self) -> u64 {
        self.responses_sent.load(Ordering::Relaxed)
    }

    /// Get batches sent.
    pub fn batches_sent(&self) -> u64 {
        self.batches_sent.load(Ordering::Relaxed)
    }

    /// Get bytes sent.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get max batch size.
    pub fn max_batch_size(&self) -> usize {
        self.max_batch_size.load(Ordering::Relaxed)
    }

    /// Get reorder count.
    pub fn reordered(&self) -> u64 {
        self.reordered.load(Ordering::Relaxed)
    }

    /// Get average batch size.
    pub fn avg_batch_size(&self) -> f64 {
        let batches = self.batches_sent();
        let responses = self.responses_sent();
        if batches > 0 {
            responses as f64 / batches as f64
        } else {
            0.0
        }
    }

    /// Get average response size.
    pub fn avg_response_size(&self) -> f64 {
        let responses = self.responses_sent();
        let bytes = self.bytes_sent();
        if responses > 0 {
            bytes as f64 / responses as f64
        } else {
            0.0
        }
    }
}

/// Global pipeline statistics.
static GLOBAL_PIPELINE_STATS: GlobalPipelineStats = GlobalPipelineStats {
    responses_queued: AtomicU64::new(0),
    responses_sent: AtomicU64::new(0),
    batches_sent: AtomicU64::new(0),
    bytes_sent: AtomicU64::new(0),
    max_batch_size: AtomicUsize::new(0),
    reordered: AtomicU64::new(0),
};

/// Get global pipeline statistics.
pub fn global_pipeline_stats() -> &'static GlobalPipelineStats {
    &GLOBAL_PIPELINE_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_item_creation() {
        let item = ResponseItem::new(0, Bytes::from("hello"));
        assert_eq!(item.sequence, 0);
        assert_eq!(item.size, 5);
        assert_eq!(&item.data[..], b"hello");

        let item2 = ResponseItem::from_vec(1, vec![1, 2, 3]);
        assert_eq!(item2.sequence, 1);
        assert_eq!(item2.size, 3);

        let item3 = ResponseItem::from_static(2, b"static");
        assert_eq!(item3.sequence, 2);
        assert_eq!(item3.size, 6);
    }

    #[test]
    fn test_response_queue_in_order() {
        let mut queue = ResponseQueue::new(16);

        assert!(queue.push(ResponseItem::new(0, Bytes::from("first"))));
        assert!(queue.push(ResponseItem::new(1, Bytes::from("second"))));
        assert!(queue.push(ResponseItem::new(2, Bytes::from("third"))));

        assert_eq!(queue.ready_count(), 3);
        assert_eq!(queue.pending_count(), 0);

        let item0 = queue.pop_ready().unwrap();
        assert_eq!(item0.sequence, 0);

        let item1 = queue.pop_ready().unwrap();
        assert_eq!(item1.sequence, 1);

        let item2 = queue.pop_ready().unwrap();
        assert_eq!(item2.sequence, 2);

        assert!(queue.pop_ready().is_none());
    }

    #[test]
    fn test_response_queue_out_of_order() {
        let mut queue = ResponseQueue::new(16);

        // Push out of order: 2, 0, 1
        assert!(queue.push(ResponseItem::new(2, Bytes::from("third"))));
        assert_eq!(queue.ready_count(), 0);
        assert_eq!(queue.pending_count(), 1);

        assert!(queue.push(ResponseItem::new(0, Bytes::from("first"))));
        assert_eq!(queue.ready_count(), 1); // 0 is ready
        assert_eq!(queue.pending_count(), 1); // 2 still pending (waiting for 1)

        assert!(queue.push(ResponseItem::new(1, Bytes::from("second"))));
        assert_eq!(queue.ready_count(), 3); // All ready now
        assert_eq!(queue.pending_count(), 0);

        // Pop should be in order
        let item0 = queue.pop_ready().unwrap();
        assert_eq!(item0.sequence, 0);

        let item1 = queue.pop_ready().unwrap();
        assert_eq!(item1.sequence, 1);

        let item2 = queue.pop_ready().unwrap();
        assert_eq!(item2.sequence, 2);
    }

    #[test]
    fn test_response_queue_duplicate_rejected() {
        let mut queue = ResponseQueue::new(16);

        assert!(queue.push(ResponseItem::new(0, Bytes::from("first"))));
        queue.pop_ready(); // Consume it

        // Try to push sequence 0 again (should be rejected)
        assert!(!queue.push(ResponseItem::new(0, Bytes::from("duplicate"))));
        assert_eq!(queue.stats().duplicates(), 1);
    }

    #[test]
    fn test_response_queue_overflow_rejected() {
        let mut queue = ResponseQueue::new(4);

        // Push sequence 10 (too far ahead for capacity 4)
        assert!(!queue.push(ResponseItem::new(10, Bytes::from("too far"))));
        assert_eq!(queue.stats().overflows(), 1);
    }

    #[test]
    fn test_response_batch_drain() {
        let mut queue = ResponseQueue::new(16);

        queue.push(ResponseItem::new(0, Bytes::from("aaaa")));
        queue.push(ResponseItem::new(1, Bytes::from("bbbb")));
        queue.push(ResponseItem::new(2, Bytes::from("cccc")));

        let batch = queue.drain_ready();
        assert_eq!(batch.len(), 3);
        assert_eq!(batch.total_size, 12);

        assert_eq!(queue.ready_count(), 0);
    }

    #[test]
    fn test_response_batch_concat() {
        let mut queue = ResponseQueue::new(16);

        queue.push(ResponseItem::new(0, Bytes::from("aa")));
        queue.push(ResponseItem::new(1, Bytes::from("bb")));
        queue.push(ResponseItem::new(2, Bytes::from("cc")));

        let batch = queue.drain_ready();
        let concat = batch.concat();
        assert_eq!(&concat[..], b"aabbcc");
    }

    #[test]
    fn test_response_batch_io_slices() {
        let mut queue = ResponseQueue::new(16);

        queue.push(ResponseItem::new(0, Bytes::from("hello")));
        queue.push(ResponseItem::new(1, Bytes::from("world")));

        let batch = queue.drain_ready();
        let slices = batch.as_io_slices();
        assert_eq!(slices.len(), 2);
        assert_eq!(&*slices[0], b"hello");
        assert_eq!(&*slices[1], b"world");
    }

    #[test]
    fn test_connection_pipeline() {
        let mut pipeline = ConnectionPipeline::new(1, 32);

        let seq0 = pipeline.next_sequence();
        let seq1 = pipeline.next_sequence();
        let seq2 = pipeline.next_sequence();

        assert_eq!(seq0, 0);
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        // Queue out of order
        assert!(pipeline.queue_response(seq2, Bytes::from("c")));
        assert!(pipeline.queue_response(seq0, Bytes::from("a")));
        assert!(pipeline.queue_response(seq1, Bytes::from("b")));

        assert_eq!(pipeline.ready_count(), 3);
        assert_eq!(pipeline.ready_bytes(), 3);
    }

    #[test]
    fn test_connection_pipeline_should_flush() {
        let config = ResponseWriterConfig {
            min_batch_size: 2,
            max_batch_size: 10,
            min_batch_bytes: 100,
            max_batch_bytes: 1000,
            use_tcp_cork: true,
            flush_timeout_us: 100,
        };

        let mut pipeline = ConnectionPipeline::with_config(1, 32, config);

        // One response - not enough for flush
        pipeline.queue_response(0, Bytes::from("hello"));
        assert!(!pipeline.should_flush());

        // Two responses - meets min_batch_size
        pipeline.queue_response(1, Bytes::from("world"));
        assert!(pipeline.should_flush());
    }

    #[test]
    fn test_connection_pipeline_must_flush() {
        let config = ResponseWriterConfig {
            min_batch_size: 1,
            max_batch_size: 2,
            min_batch_bytes: 1,
            max_batch_bytes: 100,
            use_tcp_cork: true,
            flush_timeout_us: 100,
        };

        let mut pipeline = ConnectionPipeline::with_config(1, 32, config);

        pipeline.queue_response(0, Bytes::from("a"));
        assert!(!pipeline.must_flush());

        pipeline.queue_response(1, Bytes::from("b"));
        assert!(pipeline.must_flush()); // max_batch_size reached
    }

    #[test]
    fn test_response_writer_config_presets() {
        let high = ResponseWriterConfig::high_throughput();
        assert_eq!(high.min_batch_size, 8);
        assert!(high.use_tcp_cork);

        let low = ResponseWriterConfig::low_latency();
        assert_eq!(low.min_batch_size, 1);
        assert!(!low.use_tcp_cork);
    }

    #[test]
    fn test_response_queue_stats() {
        let mut queue = ResponseQueue::new(16);

        queue.push(ResponseItem::new(0, Bytes::from("hello")));
        queue.push(ResponseItem::new(1, Bytes::from("world")));

        assert_eq!(queue.stats().pushed(), 2);
        assert_eq!(queue.stats().bytes_pushed(), 10);

        queue.pop_ready();
        assert_eq!(queue.stats().popped(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_apply_cork_honors_config() {
        use std::os::unix::io::AsRawFd;

        // Corking disabled: apply_cork must not touch the socket at all. An
        // invalid fd proves it — a real setsockopt on -1 would fail EBADF.
        // (On non-Linux unix `set_tcp_cork` is an Ok(()) stub, so this only
        // demonstrates the short-circuit on Linux; it stays green elsewhere.)
        let quiet = ConnectionPipeline::with_config(1, 8, ResponseWriterConfig::low_latency());
        assert!(!quiet.config().use_tcp_cork);
        assert!(quiet.apply_cork(-1, true).is_ok());
        assert!(quiet.apply_cork(-1, false).is_ok());

        // Corking enabled: the option is really set on a connected socket,
        // both on the way in and on the way out.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let stream = std::net::TcpStream::connect(addr).expect("connect loopback");
        let _accepted = listener.accept().expect("accept");

        let corked = ConnectionPipeline::with_config(2, 8, ResponseWriterConfig::high_throughput());
        assert!(corked.config().use_tcp_cork);
        assert!(corked.apply_cork(stream.as_raw_fd(), true).is_ok());
        assert!(corked.apply_cork(stream.as_raw_fd(), false).is_ok());
    }

    #[test]
    fn test_global_pipeline_stats() {
        let stats = global_pipeline_stats();
        let _ = stats.responses_queued();
        let _ = stats.responses_sent();
        let _ = stats.avg_batch_size();
    }
}
