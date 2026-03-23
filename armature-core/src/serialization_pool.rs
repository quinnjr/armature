//! Serialization Buffer Pool
//!
//! This module provides an integrated API for zero-copy response serialization
//! with automatic buffer pooling. It combines the capabilities of:
//!
//! - `body.rs` - Zero-copy `Bytes` handling
//! - `response_buffer.rs` - Pre-allocated `BytesMut` buffers
//! - `buffer_pool.rs` - Thread-local buffer reuse
//!
//! # Performance
//!
//! By combining these features, we achieve:
//!
//! - **Zero-copy responses**: `Bytes` passed directly to Hyper
//! - **No reallocations**: Pre-sized buffers match typical response sizes
//! - **Buffer reuse**: Thread-local pools eliminate allocation overhead
//! - **Adaptive sizing**: Learns optimal buffer sizes from usage patterns
//!
//! # Usage
//!
//! ```rust,ignore
//! use armature_core::serialization_pool::{serialize_json, serialize_bytes};
//!
//! // Serialize JSON with automatic buffer management
//! let response = serialize_json(&data)?;
//!
//! // Or use a specific buffer size
//! let response = serialize_json_with_size(&data, SerializationSize::Medium)?;
//! ```

use crate::buffer_pool::{BufferSize, PooledBuffer, acquire_buffer};
#[cfg(feature = "simd-json")]
use crate::json::Json;
use bytes::Bytes;
use lru::LruCache;
use serde::Serialize;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Serialization Size Estimation
// ============================================================================

/// Size categories for serialization buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SerializationSize {
    /// Tiny (256B) - status responses, simple acks
    Tiny,
    /// Small (1KB) - simple JSON objects
    Small,
    /// Medium (4KB) - typical API responses
    Medium,
    /// Large (16KB) - paginated lists, moderate data
    Large,
    /// XLarge (64KB) - bulk data, large collections
    XLarge,
}

impl SerializationSize {
    /// Get buffer capacity for this size.
    #[inline]
    pub const fn capacity(self) -> usize {
        match self {
            Self::Tiny => 256,
            Self::Small => 1024,
            Self::Medium => 4096,
            Self::Large => 16384,
            Self::XLarge => 65536,
        }
    }

    /// Convert to BufferSize for pool acquisition.
    #[inline]
    pub fn to_buffer_size(self) -> BufferSize {
        match self {
            Self::Tiny => BufferSize::Tiny,
            Self::Small => BufferSize::Small,
            Self::Medium => BufferSize::Small, // 4KB pool for 1-4KB
            Self::Large => BufferSize::Medium,
            Self::XLarge => BufferSize::Large,
        }
    }

    /// Estimate size from type size hint.
    pub fn estimate_from_hint(size_hint: usize) -> Self {
        if size_hint <= 256 {
            Self::Tiny
        } else if size_hint <= 1024 {
            Self::Small
        } else if size_hint <= 4096 {
            Self::Medium
        } else if size_hint <= 16384 {
            Self::Large
        } else {
            Self::XLarge
        }
    }
}

// ============================================================================
// Serialization Functions
// ============================================================================

/// Serialize a value to JSON using pooled buffers.
///
/// Returns `Bytes` for zero-copy response handling.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::serialization_pool::serialize_json;
///
/// #[derive(Serialize)]
/// struct User { name: String, age: u32 }
///
/// let user = User { name: "John".into(), age: 30 };
/// let bytes = serialize_json(&user)?;
/// // bytes is ready for zero-copy response
/// ```
#[inline]
pub fn serialize_json<T: Serialize>(value: &T) -> Result<Bytes, SerializationError> {
    serialize_json_with_size(value, SerializationSize::Medium)
}

/// Serialize JSON with a specific buffer size hint.
pub fn serialize_json_with_size<T: Serialize>(
    value: &T,
    size: SerializationSize,
) -> Result<Bytes, SerializationError> {
    SERIALIZATION_STATS.record_serialization();

    // Acquire buffer from pool
    let mut buffer = acquire_buffer(size.to_buffer_size());

    // Serialize directly into buffer
    let result = serde_json::to_writer(buffer.as_writer(), value);

    match result {
        Ok(()) => {
            let len = buffer.len();
            SERIALIZATION_STATS.record_bytes(len);

            // Convert to Bytes (zero-copy if buffer not reallocated)
            Ok(buffer.freeze())
        }
        Err(e) => {
            SERIALIZATION_STATS.record_error();
            Err(SerializationError::Json(e.to_string()))
        }
    }
}

/// Serialize JSON with SIMD acceleration (if available).
#[cfg(feature = "simd-json")]
pub fn serialize_json_simd<T: Serialize>(value: &T) -> Result<Bytes, SerializationError> {
    serialize_json_simd_with_size(value, SerializationSize::Medium)
}

/// Serialize JSON with SIMD and specific size.
#[cfg(feature = "simd-json")]
pub fn serialize_json_simd_with_size<T: Serialize>(
    value: &T,
    size: SerializationSize,
) -> Result<Bytes, SerializationError> {
    SERIALIZATION_STATS.record_serialization();

    let vec = Json::to_vec(value)?;
    SERIALIZATION_STATS.record_bytes(vec.len());

    Ok(Bytes::from(vec))
}

/// Serialize bytes with optional compression.
pub fn serialize_bytes(data: &[u8]) -> Bytes {
    SERIALIZATION_STATS.record_serialization();
    SERIALIZATION_STATS.record_bytes(data.len());
    Bytes::copy_from_slice(data)
}

/// Serialize bytes from a Vec (zero-copy).
#[inline]
pub fn serialize_bytes_from_vec(data: Vec<u8>) -> Bytes {
    SERIALIZATION_STATS.record_serialization();
    SERIALIZATION_STATS.record_bytes(data.len());
    Bytes::from(data)
}

/// Serialize static bytes (zero-copy).
#[inline]
pub fn serialize_static(data: &'static [u8]) -> Bytes {
    SERIALIZATION_STATS.record_serialization();
    SERIALIZATION_STATS.record_bytes(data.len());
    Bytes::from_static(data)
}

// ============================================================================
// Pooled Serializer
// ============================================================================

/// A serializer that reuses buffers across serialization calls.
///
/// Use this when serializing multiple values in sequence to maximize
/// buffer reuse efficiency.
#[derive(Debug)]
pub struct PooledSerializer {
    /// Default size for new serializations
    default_size: SerializationSize,
    /// Track sizes for adaptive sizing
    size_tracker: SizeTracker,
}

impl PooledSerializer {
    /// Create new serializer with default size.
    pub fn new() -> Self {
        Self {
            default_size: SerializationSize::Medium,
            size_tracker: SizeTracker::new(),
        }
    }

    /// Create with specific default size.
    pub fn with_size(size: SerializationSize) -> Self {
        Self {
            default_size: size,
            size_tracker: SizeTracker::new(),
        }
    }

    /// Serialize JSON value.
    pub fn serialize<T: Serialize>(&mut self, value: &T) -> Result<Bytes, SerializationError> {
        let size = self
            .size_tracker
            .recommended_size()
            .unwrap_or(self.default_size);
        let bytes = serialize_json_with_size(value, size)?;

        // Track actual size for future recommendations
        self.size_tracker.record_size(bytes.len());

        Ok(bytes)
    }

    /// Serialize with type name for adaptive sizing.
    pub fn serialize_typed<T: Serialize>(
        &mut self,
        value: &T,
        type_name: &str,
    ) -> Result<Bytes, SerializationError> {
        let size = self
            .size_tracker
            .recommended_size_for_type(type_name)
            .unwrap_or(self.default_size);

        let bytes = serialize_json_with_size(value, size)?;

        // Track size by type
        self.size_tracker
            .record_size_for_type(type_name, bytes.len());

        Ok(bytes)
    }

    /// Get size tracker statistics.
    pub fn stats(&self) -> &SizeTracker {
        &self.size_tracker
    }
}

impl Default for PooledSerializer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Size Tracking for Adaptive Sizing
// ============================================================================

/// Tracks serialization sizes for adaptive buffer allocation.
#[derive(Debug)]
pub struct SizeTracker {
    /// Recent sizes (ring buffer)
    recent_sizes: Vec<usize>,
    /// Current index in ring buffer
    index: usize,
    /// Size by type name (bounded LRU cache to prevent unbounded growth)
    type_sizes: LruCache<String, TypeSizeInfo>,
    /// Total serializations
    total_count: u64,
}

impl SizeTracker {
    const HISTORY_SIZE: usize = 64;
    /// Maximum number of type entries to track (prevents unbounded growth)
    const MAX_TYPE_ENTRIES: usize = 256;

    /// Create new tracker.
    pub fn new() -> Self {
        Self {
            recent_sizes: Vec::with_capacity(Self::HISTORY_SIZE),
            index: 0,
            type_sizes: LruCache::new(NonZeroUsize::new(Self::MAX_TYPE_ENTRIES).unwrap()),
            total_count: 0,
        }
    }

    /// Record a serialization size.
    pub fn record_size(&mut self, size: usize) {
        if self.recent_sizes.len() < Self::HISTORY_SIZE {
            self.recent_sizes.push(size);
        } else {
            self.recent_sizes[self.index] = size;
            self.index = (self.index + 1) % Self::HISTORY_SIZE;
        }
        self.total_count += 1;
    }

    /// Record size for a specific type.
    pub fn record_size_for_type(&mut self, type_name: &str, size: usize) {
        self.record_size(size);

        // LruCache handles bounded eviction automatically when capacity exceeded
        let info = self
            .type_sizes
            .get_or_insert_mut(type_name.to_string(), TypeSizeInfo::new);

        info.record(size);
    }

    /// Get recommended size based on recent history.
    pub fn recommended_size(&self) -> Option<SerializationSize> {
        if self.recent_sizes.is_empty() {
            return None;
        }

        // Use 90th percentile
        let mut sizes = self.recent_sizes.clone();
        sizes.sort_unstable();
        let p90_idx = (sizes.len() * 90 / 100).max(0);
        let p90_size = sizes[p90_idx];

        Some(SerializationSize::estimate_from_hint(p90_size))
    }

    /// Get recommended size for a specific type.
    pub fn recommended_size_for_type(&self, type_name: &str) -> Option<SerializationSize> {
        self.type_sizes
            .peek(type_name)
            .map(|info| SerializationSize::estimate_from_hint(info.p90_size()))
    }

    /// Get total serializations.
    pub fn total_count(&self) -> u64 {
        self.total_count
    }

    /// Get average size.
    pub fn average_size(&self) -> usize {
        if self.recent_sizes.is_empty() {
            0
        } else {
            self.recent_sizes.iter().sum::<usize>() / self.recent_sizes.len()
        }
    }
}

impl Default for SizeTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Size information for a specific type.
#[derive(Debug)]
pub struct TypeSizeInfo {
    /// Minimum observed size
    min: usize,
    /// Maximum observed size
    max: usize,
    /// Sum for average
    sum: u64,
    /// Count
    count: u64,
    /// Recent sizes for percentile
    recent: Vec<usize>,
}

impl TypeSizeInfo {
    fn new() -> Self {
        Self {
            min: usize::MAX,
            max: 0,
            sum: 0,
            count: 0,
            recent: Vec::with_capacity(32),
        }
    }

    fn record(&mut self, size: usize) {
        self.min = self.min.min(size);
        self.max = self.max.max(size);
        self.sum += size as u64;
        self.count += 1;

        if self.recent.len() < 32 {
            self.recent.push(size);
        } else {
            let idx = (self.count as usize) % 32;
            self.recent[idx] = size;
        }
    }

    fn p90_size(&self) -> usize {
        if self.recent.is_empty() {
            return 0;
        }

        let mut sorted = self.recent.clone();
        sorted.sort_unstable();
        let idx = (sorted.len() * 90 / 100).max(0);
        sorted[idx]
    }

    /// Get average size.
    pub fn average(&self) -> usize {
        self.sum
            .checked_div(self.count)
            .map(|v| v as usize)
            .unwrap_or(0)
    }

    /// Get minimum size.
    pub fn min(&self) -> usize {
        if self.min == usize::MAX { 0 } else { self.min }
    }

    /// Get maximum size.
    pub fn max(&self) -> usize {
        self.max
    }
}

// ============================================================================
// Response Builder with Pooling
// ============================================================================

/// Builder for creating responses with pooled buffers.
#[derive(Debug)]
pub struct PooledResponseBuilder {
    buffer: PooledBuffer,
    content_type: Option<&'static str>,
}

impl PooledResponseBuilder {
    /// Create new builder with default size.
    pub fn new() -> Self {
        Self {
            buffer: acquire_buffer(BufferSize::Small),
            content_type: None,
        }
    }

    /// Create with specific size.
    pub fn with_size(size: SerializationSize) -> Self {
        Self {
            buffer: acquire_buffer(size.to_buffer_size()),
            content_type: None,
        }
    }

    /// Set content type.
    pub fn content_type(mut self, content_type: &'static str) -> Self {
        self.content_type = Some(content_type);
        self
    }

    /// Write JSON body.
    pub fn json<T: Serialize>(mut self, value: &T) -> Result<Self, SerializationError> {
        serde_json::to_writer(self.buffer.as_writer(), value)
            .map_err(|e| SerializationError::Json(e.to_string()))?;
        self.content_type = Some("application/json");
        Ok(self)
    }

    /// Write raw bytes.
    pub fn bytes(mut self, data: &[u8]) -> Self {
        self.buffer.extend_from_slice(data);
        self
    }

    /// Write string.
    pub fn string(mut self, s: &str) -> Self {
        self.buffer.extend_from_slice(s.as_bytes());
        self
    }

    /// Build into Bytes.
    pub fn build(self) -> Bytes {
        self.buffer.freeze()
    }

    /// Get content type.
    pub fn get_content_type(&self) -> Option<&'static str> {
        self.content_type
    }
}

impl Default for PooledResponseBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Error Types
// ============================================================================

/// Serialization error.
#[derive(Debug, Clone)]
pub enum SerializationError {
    /// JSON serialization error
    Json(String),
    /// Buffer overflow
    BufferOverflow,
    /// Other error
    Other(String),
}

impl std::fmt::Display for SerializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "JSON serialization error: {}", e),
            Self::BufferOverflow => write!(f, "Buffer overflow"),
            Self::Other(e) => write!(f, "Serialization error: {}", e),
        }
    }
}

impl std::error::Error for SerializationError {}

impl From<serde_json::Error> for SerializationError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Global serialization statistics.
#[derive(Debug, Default)]
pub struct SerializationStats {
    serializations: AtomicU64,
    bytes_serialized: AtomicU64,
    errors: AtomicU64,
}

impl SerializationStats {
    fn record_serialization(&self) {
        self.serializations.fetch_add(1, Ordering::Relaxed);
    }

    fn record_bytes(&self, bytes: usize) {
        self.bytes_serialized
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total serializations.
    pub fn serializations(&self) -> u64 {
        self.serializations.load(Ordering::Relaxed)
    }

    /// Get total bytes serialized.
    pub fn bytes_serialized(&self) -> u64 {
        self.bytes_serialized.load(Ordering::Relaxed)
    }

    /// Get error count.
    pub fn errors(&self) -> u64 {
        self.errors.load(Ordering::Relaxed)
    }

    /// Get average serialization size.
    pub fn average_size(&self) -> usize {
        self.bytes_serialized()
            .checked_div(self.serializations())
            .map(|v| v as usize)
            .unwrap_or(0)
    }
}

static SERIALIZATION_STATS: SerializationStats = SerializationStats {
    serializations: AtomicU64::new(0),
    bytes_serialized: AtomicU64::new(0),
    errors: AtomicU64::new(0),
};

/// Get global serialization statistics.
pub fn serialization_stats() -> &'static SerializationStats {
    &SERIALIZATION_STATS
}

// ============================================================================
// Pooled Buffer Writer Adapter
// ============================================================================

/// Extension trait for PooledBuffer to work with serde.
trait PooledBufferExt {
    fn as_writer(&mut self) -> PooledBufferWriter<'_>;
}

impl PooledBufferExt for PooledBuffer {
    fn as_writer(&mut self) -> PooledBufferWriter<'_> {
        PooledBufferWriter { buffer: self }
    }
}

/// Writer adapter for PooledBuffer.
struct PooledBufferWriter<'a> {
    buffer: &'a mut PooledBuffer,
}

impl<'a> std::io::Write for PooledBufferWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestUser {
        name: String,
        age: u32,
    }

    #[test]
    fn test_serialization_size_capacity() {
        assert_eq!(SerializationSize::Tiny.capacity(), 256);
        assert_eq!(SerializationSize::Small.capacity(), 1024);
        assert_eq!(SerializationSize::Medium.capacity(), 4096);
    }

    #[test]
    fn test_serialize_json() {
        let user = TestUser {
            name: "John".to_string(),
            age: 30,
        };

        let bytes = serialize_json(&user).unwrap();
        assert!(!bytes.is_empty());

        // Verify it's valid JSON
        let parsed: TestUser = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, user);
    }

    #[test]
    fn test_serialize_json_with_size() {
        let user = TestUser {
            name: "Jane".to_string(),
            age: 25,
        };

        let bytes = serialize_json_with_size(&user, SerializationSize::Tiny).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_serialize_bytes() {
        let data = b"Hello, World!";
        let bytes = serialize_bytes(data);
        assert_eq!(&bytes[..], data);
    }

    #[test]
    fn test_serialize_bytes_from_vec() {
        let data = vec![1, 2, 3, 4, 5];
        let bytes = serialize_bytes_from_vec(data.clone());
        assert_eq!(&bytes[..], &data[..]);
    }

    #[test]
    fn test_serialize_static() {
        let bytes = serialize_static(b"static data");
        assert_eq!(&bytes[..], b"static data");
    }

    #[test]
    fn test_pooled_serializer() {
        let mut serializer = PooledSerializer::new();

        let user = TestUser {
            name: "Test".to_string(),
            age: 20,
        };

        // Serialize multiple times
        for _ in 0..10 {
            let bytes = serializer.serialize(&user).unwrap();
            assert!(!bytes.is_empty());
        }

        // Check stats
        assert!(serializer.stats().total_count() >= 10);
    }

    #[test]
    fn test_size_tracker() {
        let mut tracker = SizeTracker::new();

        // Record some sizes
        for size in &[100, 200, 300, 400, 500] {
            tracker.record_size(*size);
        }

        assert_eq!(tracker.total_count(), 5);
        assert_eq!(tracker.average_size(), 300);
    }

    #[test]
    fn test_size_tracker_by_type() {
        let mut tracker = SizeTracker::new();

        tracker.record_size_for_type("User", 100);
        tracker.record_size_for_type("User", 150);
        tracker.record_size_for_type("Post", 500);

        let user_size = tracker.recommended_size_for_type("User");
        assert!(user_size.is_some());
    }

    #[test]
    fn test_pooled_response_builder() {
        let builder = PooledResponseBuilder::new()
            .content_type("text/plain")
            .string("Hello, World!");

        assert_eq!(builder.get_content_type(), Some("text/plain"));

        let bytes = builder.build();
        assert_eq!(&bytes[..], b"Hello, World!");
    }

    #[test]
    fn test_pooled_response_builder_json() {
        let user = TestUser {
            name: "Builder".to_string(),
            age: 42,
        };

        let builder = PooledResponseBuilder::new().json(&user).unwrap();

        assert_eq!(builder.get_content_type(), Some("application/json"));

        let bytes = builder.build();
        let parsed: TestUser = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed, user);
    }

    #[test]
    fn test_serialization_stats() {
        let stats = serialization_stats();

        // Stats should be non-negative
        let _ = stats.serializations();
        let _ = stats.bytes_serialized();
        let _ = stats.errors();
        let _ = stats.average_size();
    }

    #[test]
    fn test_serialization_error_display() {
        let err = SerializationError::Json("test error".to_string());
        assert!(err.to_string().contains("JSON"));

        let err = SerializationError::BufferOverflow;
        assert!(err.to_string().contains("overflow"));
    }
}
