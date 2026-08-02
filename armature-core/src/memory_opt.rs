//! Memory Optimization Utilities
//!
//! This module provides memory-optimized alternatives to standard collections
//! for HTTP request/response handling:
//!
//! - `SmallHeaders`: Stack-allocated headers using SmallVec
//! - `CompactPath`: Short string optimization for paths
//! - `PreSizedBuffer`: Pre-allocated response buffers
//! - `ObjectPool`: Reusable request/response objects
//!
//! # Performance Impact
//!
//! - SmallHeaders: Avoids heap allocation for ≤16 headers
//! - CompactPath: Inline storage for paths ≤24 bytes
//! - PreSizedBuffer: Eliminates reallocations during response building
//! - ObjectPool: Zero-allocation request reuse

use bytes::{Bytes, BytesMut};
use compact_str::CompactString;
use smallvec::SmallVec;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// SmallVec Headers
// ============================================================================

/// Maximum inline header count before heap allocation.
pub const INLINE_HEADER_COUNT: usize = 16;

/// A header key-value pair using compact strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallHeader {
    /// Header name (e.g., "Content-Type")
    pub name: CompactString,
    /// Header value
    pub value: CompactString,
}

impl SmallHeader {
    /// Create new header.
    #[inline]
    pub fn new(name: impl Into<CompactString>, value: impl Into<CompactString>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Get name as str.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get value as str.
    #[inline]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Stack-allocated header collection for typical HTTP requests.
///
/// Uses SmallVec to store up to 16 headers inline (on stack),
/// only allocating on heap for requests with many headers.
#[derive(Debug, Clone, Default)]
pub struct SmallHeaders {
    headers: SmallVec<[SmallHeader; INLINE_HEADER_COUNT]>,
}

impl SmallHeaders {
    /// Create empty headers.
    #[inline]
    pub fn new() -> Self {
        Self {
            headers: SmallVec::new(),
        }
    }

    /// Create with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            headers: SmallVec::with_capacity(capacity),
        }
    }

    /// Insert a header.
    #[inline]
    pub fn insert(&mut self, name: impl Into<CompactString>, value: impl Into<CompactString>) {
        let name = name.into();
        // Check if header exists and update
        for header in &mut self.headers {
            if header.name.eq_ignore_ascii_case(&name) {
                header.value = value.into();
                return;
            }
        }
        // Add new header
        self.headers.push(SmallHeader::new(name, value));
        MEMORY_STATS.record_header_insert(self.is_inline());
    }

    /// Get header value by name (case-insensitive).
    #[inline]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    /// Check if header exists.
    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Remove a header.
    pub fn remove(&mut self, name: &str) -> Option<CompactString> {
        if let Some(pos) = self
            .headers
            .iter()
            .position(|h| h.name.eq_ignore_ascii_case(name))
        {
            Some(self.headers.remove(pos).value)
        } else {
            None
        }
    }

    /// Get number of headers.
    #[inline]
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// Check if headers are stored inline (no heap allocation).
    #[inline]
    pub fn is_inline(&self) -> bool {
        !self.headers.spilled()
    }

    /// Iterate over headers.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
    }

    /// Clear all headers.
    #[inline]
    pub fn clear(&mut self) {
        self.headers.clear();
    }

    /// Get Content-Type header.
    #[inline]
    pub fn content_type(&self) -> Option<&str> {
        self.get("content-type")
    }

    /// Get Content-Length header.
    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.get("content-length")?.parse().ok()
    }

    /// Convert to HashMap for compatibility.
    pub fn to_hash_map(&self) -> std::collections::HashMap<String, String> {
        self.headers
            .iter()
            .map(|h| (h.name.to_string(), h.value.to_string()))
            .collect()
    }
}

// ============================================================================
// CompactPath
// ============================================================================

/// Maximum inline path length before heap allocation.
pub const INLINE_PATH_LENGTH: usize = 24;

/// A compact string optimized for URL paths.
///
/// Stores paths up to 24 bytes inline, avoiding heap allocation
/// for most common paths like "/api/users", "/health", etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompactPath(CompactString);

impl CompactPath {
    /// Create from string.
    #[inline]
    pub fn new(path: impl Into<CompactString>) -> Self {
        let path = path.into();
        MEMORY_STATS.record_path_create(path.is_heap_allocated());
        Self(path)
    }

    /// Create empty path.
    #[inline]
    pub fn empty() -> Self {
        Self(CompactString::new(""))
    }

    /// Get as str.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check if stored inline.
    #[inline]
    pub fn is_inline(&self) -> bool {
        !self.0.is_heap_allocated()
    }

    /// Get length.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Check if path starts with prefix.
    #[inline]
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix)
    }

    /// Check if path ends with suffix.
    #[inline]
    pub fn ends_with(&self, suffix: &str) -> bool {
        self.0.ends_with(suffix)
    }

    /// Split path into segments.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|s| !s.is_empty())
    }

    /// Get path without query string.
    pub fn without_query(&self) -> &str {
        self.0.split('?').next().unwrap_or(&self.0)
    }
}

impl std::fmt::Display for CompactPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for CompactPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for CompactPath {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for CompactPath {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// ============================================================================
// Pre-sized Response Buffer
// ============================================================================

/// Default response buffer capacity.
pub const DEFAULT_RESPONSE_BUFFER: usize = 512;
/// Medium response buffer capacity.
pub const MEDIUM_RESPONSE_BUFFER: usize = 4096;
/// Large response buffer capacity.
pub const LARGE_RESPONSE_BUFFER: usize = 65536;

/// Pre-sized buffer for response building.
///
/// Avoids reallocations by pre-allocating based on expected response size.
#[derive(Debug)]
pub struct PreSizedBuffer {
    buffer: BytesMut,
    initial_capacity: usize,
}

impl PreSizedBuffer {
    /// Create with default capacity (512 bytes).
    #[inline]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RESPONSE_BUFFER)
    }

    /// Create with specific capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        MEMORY_STATS.record_buffer_alloc(capacity);
        Self {
            buffer: BytesMut::with_capacity(capacity),
            initial_capacity: capacity,
        }
    }

    /// Create for small responses (512 bytes).
    #[inline]
    pub fn small() -> Self {
        Self::with_capacity(DEFAULT_RESPONSE_BUFFER)
    }

    /// Create for medium responses (4KB).
    #[inline]
    pub fn medium() -> Self {
        Self::with_capacity(MEDIUM_RESPONSE_BUFFER)
    }

    /// Create for large responses (64KB).
    #[inline]
    pub fn large() -> Self {
        Self::with_capacity(LARGE_RESPONSE_BUFFER)
    }

    /// Write bytes to buffer.
    #[inline]
    pub fn write(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Write string to buffer.
    #[inline]
    pub fn write_str(&mut self, s: &str) {
        self.buffer.extend_from_slice(s.as_bytes());
    }

    /// Get current length.
    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Get remaining capacity before reallocation.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.buffer.capacity() - self.buffer.len()
    }

    /// Check if buffer has grown beyond initial capacity.
    #[inline]
    pub fn has_grown(&self) -> bool {
        self.buffer.capacity() > self.initial_capacity
    }

    /// Freeze into immutable Bytes.
    #[inline]
    pub fn freeze(self) -> Bytes {
        self.buffer.freeze()
    }

    /// Get as slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }

    /// Clear buffer, keeping capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Reset to initial capacity.
    pub fn reset(&mut self) {
        self.buffer.clear();
        if self.buffer.capacity() > self.initial_capacity * 2 {
            // Shrink if grown too large
            self.buffer = BytesMut::with_capacity(self.initial_capacity);
        }
    }
}

impl Default for PreSizedBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::io::Write for PreSizedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ============================================================================
// Object Pool
// ============================================================================

/// Configuration for object pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum pool size
    pub max_size: usize,
    /// Initial pool size
    pub initial_size: usize,
    /// Pre-warm pool on creation
    pub pre_warm: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 1024,
            initial_size: 64,
            pre_warm: true,
        }
    }
}

impl PoolConfig {
    /// Create new config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set max size.
    pub fn max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set initial size.
    pub fn initial_size(mut self, size: usize) -> Self {
        self.initial_size = size;
        self
    }

    /// Enable/disable pre-warming.
    pub fn pre_warm(mut self, enabled: bool) -> Self {
        self.pre_warm = enabled;
        self
    }
}

/// Trait for poolable objects.
pub trait Poolable: Send + Sync {
    /// Create new instance.
    fn create() -> Self;
    /// Reset for reuse.
    fn reset(&mut self);
}

/// Generic object pool for reusing allocations.
#[derive(Debug)]
pub struct ObjectPool<T: Poolable> {
    pool: Mutex<VecDeque<T>>,
    config: PoolConfig,
    stats: PoolStats,
}

impl<T: Poolable> ObjectPool<T> {
    /// Create new pool with config.
    pub fn new(config: PoolConfig) -> Self {
        let pool = if config.pre_warm {
            let mut items = VecDeque::with_capacity(config.initial_size);
            for _ in 0..config.initial_size {
                items.push_back(T::create());
            }
            items
        } else {
            VecDeque::with_capacity(config.initial_size)
        };

        Self {
            pool: Mutex::new(pool),
            config,
            stats: PoolStats::default(),
        }
    }

    /// Create with default config.
    pub fn default_pool() -> Self {
        Self::new(PoolConfig::default())
    }

    /// Acquire object from pool.
    pub fn acquire(&self) -> PooledObject<'_, T> {
        let obj = {
            let mut pool = self.pool.lock().unwrap();
            pool.pop_front()
        };

        let obj = match obj {
            Some(obj) => {
                self.stats.record_hit();
                MEMORY_STATS.record_pool_hit();
                obj
            }
            None => {
                self.stats.record_miss();
                MEMORY_STATS.record_pool_miss();
                T::create()
            }
        };

        PooledObject {
            obj: Some(obj),
            pool: self,
        }
    }

    /// Return object to pool.
    fn release(&self, mut obj: T) {
        obj.reset();

        let mut pool = self.pool.lock().unwrap();
        if pool.len() < self.config.max_size {
            pool.push_back(obj);
            self.stats.record_return();
        } else {
            self.stats.record_drop();
        }
    }

    /// Get pool statistics.
    pub fn stats(&self) -> &PoolStats {
        &self.stats
    }

    /// Get current pool size.
    pub fn size(&self) -> usize {
        self.pool.lock().unwrap().len()
    }

    /// Clear the pool.
    pub fn clear(&self) {
        self.pool.lock().unwrap().clear();
    }
}

/// RAII guard for pooled object.
pub struct PooledObject<'a, T: Poolable> {
    obj: Option<T>,
    pool: &'a ObjectPool<T>,
}

impl<'a, T: Poolable> std::ops::Deref for PooledObject<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.obj.as_ref().unwrap()
    }
}

impl<'a, T: Poolable> std::ops::DerefMut for PooledObject<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.obj.as_mut().unwrap()
    }
}

impl<'a, T: Poolable> Drop for PooledObject<'a, T> {
    fn drop(&mut self) {
        if let Some(obj) = self.obj.take() {
            self.pool.release(obj);
        }
    }
}

/// Pool statistics.
#[derive(Debug, Default)]
pub struct PoolStats {
    hits: AtomicU64,
    misses: AtomicU64,
    returns: AtomicU64,
    drops: AtomicU64,
}

impl PoolStats {
    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    fn record_return(&self) {
        self.returns.fetch_add(1, Ordering::Relaxed);
    }

    fn record_drop(&self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }

    /// Get hit count.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get miss count.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get return count.
    pub fn returns(&self) -> u64 {
        self.returns.load(Ordering::Relaxed)
    }

    /// Get drop count.
    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }

    /// Get hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let hits = self.hits() as f64;
        let total = hits + self.misses() as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }
}

// ============================================================================
// Request/Response Pool Types
// ============================================================================

/// Poolable request builder.
#[derive(Debug)]
pub struct PooledRequest {
    pub method: CompactString,
    pub path: CompactPath,
    pub headers: SmallHeaders,
    pub body: PreSizedBuffer,
}

impl Poolable for PooledRequest {
    fn create() -> Self {
        Self {
            method: CompactString::new("GET"),
            path: CompactPath::empty(),
            headers: SmallHeaders::new(),
            body: PreSizedBuffer::new(),
        }
    }

    fn reset(&mut self) {
        self.method = CompactString::new("GET");
        self.path = CompactPath::empty();
        self.headers.clear();
        self.body.reset();
    }
}

impl PooledRequest {
    /// Set method.
    pub fn method(&mut self, method: impl Into<CompactString>) -> &mut Self {
        self.method = method.into();
        self
    }

    /// Set path.
    pub fn path(&mut self, path: impl Into<CompactPath>) -> &mut Self {
        self.path = path.into();
        self
    }

    /// Add header.
    pub fn header(
        &mut self,
        name: impl Into<CompactString>,
        value: impl Into<CompactString>,
    ) -> &mut Self {
        self.headers.insert(name, value);
        self
    }

    /// Set body.
    pub fn body(&mut self, data: &[u8]) -> &mut Self {
        self.body.write(data);
        self
    }
}

/// Poolable response builder.
#[derive(Debug)]
pub struct PooledResponse {
    pub status: u16,
    pub headers: SmallHeaders,
    pub body: PreSizedBuffer,
}

impl Poolable for PooledResponse {
    fn create() -> Self {
        Self {
            status: 200,
            headers: SmallHeaders::new(),
            body: PreSizedBuffer::new(),
        }
    }

    fn reset(&mut self) {
        self.status = 200;
        self.headers.clear();
        self.body.reset();
    }
}

impl PooledResponse {
    /// Set status.
    pub fn status(&mut self, code: u16) -> &mut Self {
        self.status = code;
        self
    }

    /// Add header.
    pub fn header(
        &mut self,
        name: impl Into<CompactString>,
        value: impl Into<CompactString>,
    ) -> &mut Self {
        self.headers.insert(name, value);
        self
    }

    /// Set body.
    pub fn body(&mut self, data: &[u8]) -> &mut Self {
        self.body.write(data);
        self
    }

    /// Set JSON body.
    pub fn json<T: serde::Serialize>(&mut self, value: &T) -> Result<&mut Self, serde_json::Error> {
        self.headers.insert("content-type", "application/json");
        serde_json::to_writer(&mut self.body, value)?;
        Ok(self)
    }
}

// ============================================================================
// Global Pools
// ============================================================================

/// Global request pool.
static REQUEST_POOL: std::sync::OnceLock<ObjectPool<PooledRequest>> = std::sync::OnceLock::new();

/// Global response pool.
static RESPONSE_POOL: std::sync::OnceLock<ObjectPool<PooledResponse>> = std::sync::OnceLock::new();

/// Initialize global pools with the given configuration.
///
/// Returns `true` if both pools were freshly initialized with `config`.
/// Returns `false` if one or both pools were already initialized (e.g. via
/// a prior call to [`init_pools`], or lazily via [`acquire_request`]/
/// [`acquire_response`]) — in that case `config` was silently ignored for
/// the pool(s) that were already set.
pub fn init_pools(config: PoolConfig) -> bool {
    let request_initialized = REQUEST_POOL.set(ObjectPool::new(config.clone())).is_ok();
    if !request_initialized {
        tracing::warn!(
            "init_pools: REQUEST_POOL was already initialized; the provided PoolConfig was ignored"
        );
    }

    let response_initialized = RESPONSE_POOL.set(ObjectPool::new(config)).is_ok();
    if !response_initialized {
        tracing::warn!(
            "init_pools: RESPONSE_POOL was already initialized; the provided PoolConfig was ignored"
        );
    }

    request_initialized && response_initialized
}

/// Acquire request from global pool.
pub fn acquire_request() -> PooledObject<'static, PooledRequest> {
    REQUEST_POOL.get_or_init(ObjectPool::default_pool).acquire()
}

/// Acquire response from global pool.
pub fn acquire_response() -> PooledObject<'static, PooledResponse> {
    RESPONSE_POOL
        .get_or_init(ObjectPool::default_pool)
        .acquire()
}

/// Get request pool stats.
pub fn request_pool_stats() -> Option<&'static PoolStats> {
    REQUEST_POOL.get().map(|p| p.stats())
}

/// Get response pool stats.
pub fn response_pool_stats() -> Option<&'static PoolStats> {
    RESPONSE_POOL.get().map(|p| p.stats())
}

// ============================================================================
// Global Statistics
// ============================================================================

/// Global memory optimization statistics.
#[derive(Debug, Default)]
pub struct MemoryStats {
    /// Headers stored inline
    headers_inline: AtomicU64,
    /// Headers stored on heap
    headers_heap: AtomicU64,
    /// Paths stored inline
    paths_inline: AtomicU64,
    /// Paths stored on heap
    paths_heap: AtomicU64,
    /// Buffer allocations
    buffer_allocs: AtomicU64,
    /// Total buffer bytes allocated
    buffer_bytes: AtomicU64,
    /// Pool hits
    pool_hits: AtomicU64,
    /// Pool misses
    pool_misses: AtomicU64,
}

impl MemoryStats {
    fn record_header_insert(&self, inline: bool) {
        if inline {
            self.headers_inline.fetch_add(1, Ordering::Relaxed);
        } else {
            self.headers_heap.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_path_create(&self, heap: bool) {
        if heap {
            self.paths_heap.fetch_add(1, Ordering::Relaxed);
        } else {
            self.paths_inline.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_buffer_alloc(&self, size: usize) {
        self.buffer_allocs.fetch_add(1, Ordering::Relaxed);
        self.buffer_bytes.fetch_add(size as u64, Ordering::Relaxed);
    }

    fn record_pool_hit(&self) {
        self.pool_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_pool_miss(&self) {
        self.pool_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get inline header count.
    pub fn headers_inline(&self) -> u64 {
        self.headers_inline.load(Ordering::Relaxed)
    }

    /// Get heap header count.
    pub fn headers_heap(&self) -> u64 {
        self.headers_heap.load(Ordering::Relaxed)
    }

    /// Get inline header ratio.
    pub fn headers_inline_ratio(&self) -> f64 {
        let inline = self.headers_inline() as f64;
        let total = inline + self.headers_heap() as f64;
        if total > 0.0 { inline / total } else { 0.0 }
    }

    /// Get inline path count.
    pub fn paths_inline(&self) -> u64 {
        self.paths_inline.load(Ordering::Relaxed)
    }

    /// Get heap path count.
    pub fn paths_heap(&self) -> u64 {
        self.paths_heap.load(Ordering::Relaxed)
    }

    /// Get inline path ratio.
    pub fn paths_inline_ratio(&self) -> f64 {
        let inline = self.paths_inline() as f64;
        let total = inline + self.paths_heap() as f64;
        if total > 0.0 { inline / total } else { 0.0 }
    }

    /// Get buffer allocation count.
    pub fn buffer_allocs(&self) -> u64 {
        self.buffer_allocs.load(Ordering::Relaxed)
    }

    /// Get total buffer bytes.
    pub fn buffer_bytes(&self) -> u64 {
        self.buffer_bytes.load(Ordering::Relaxed)
    }

    /// Get pool hit ratio.
    pub fn pool_hit_ratio(&self) -> f64 {
        let hits = self.pool_hits.load(Ordering::Relaxed) as f64;
        let total = hits + self.pool_misses.load(Ordering::Relaxed) as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }
}

/// Global statistics.
static MEMORY_STATS: MemoryStats = MemoryStats {
    headers_inline: AtomicU64::new(0),
    headers_heap: AtomicU64::new(0),
    paths_inline: AtomicU64::new(0),
    paths_heap: AtomicU64::new(0),
    buffer_allocs: AtomicU64::new(0),
    buffer_bytes: AtomicU64::new(0),
    pool_hits: AtomicU64::new(0),
    pool_misses: AtomicU64::new(0),
};

/// Get global memory statistics.
pub fn memory_stats() -> &'static MemoryStats {
    &MEMORY_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_headers_inline() {
        let mut headers = SmallHeaders::new();

        // Add fewer than 16 headers
        for i in 0..10 {
            headers.insert(format!("Header-{}", i), format!("Value-{}", i));
        }

        assert!(headers.is_inline());
        assert_eq!(headers.len(), 10);
    }

    #[test]
    fn test_small_headers_get() {
        let mut headers = SmallHeaders::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("X-Custom", "value");

        assert_eq!(headers.get("content-type"), Some("application/json"));
        assert_eq!(headers.get("x-custom"), Some("value"));
        assert_eq!(headers.get("missing"), None);
    }

    #[test]
    fn test_small_headers_case_insensitive() {
        let mut headers = SmallHeaders::new();
        headers.insert("Content-Type", "text/plain");

        assert_eq!(headers.get("content-type"), Some("text/plain"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain"));
        assert_eq!(headers.get("Content-type"), Some("text/plain"));
    }

    #[test]
    fn test_small_headers_update() {
        let mut headers = SmallHeaders::new();
        headers.insert("Content-Type", "text/plain");
        headers.insert("Content-Type", "application/json");

        assert_eq!(headers.get("content-type"), Some("application/json"));
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn test_compact_path() {
        let short = CompactPath::new("/api/users");
        assert!(short.is_inline());
        assert_eq!(short.as_str(), "/api/users");

        let long = CompactPath::new("/api/very/long/path/that/exceeds/inline/storage/capacity");
        assert!(!long.is_inline());
    }

    #[test]
    fn test_compact_path_segments() {
        let path = CompactPath::new("/api/users/123");
        let segments: Vec<_> = path.segments().collect();
        assert_eq!(segments, vec!["api", "users", "123"]);
    }

    #[test]
    fn test_compact_path_without_query() {
        let path = CompactPath::new("/api/users?page=1&limit=10");
        assert_eq!(path.without_query(), "/api/users");
    }

    #[test]
    fn test_pre_sized_buffer() {
        let mut buf = PreSizedBuffer::new();
        assert_eq!(buf.remaining_capacity(), DEFAULT_RESPONSE_BUFFER);

        buf.write(b"Hello, ");
        buf.write_str("World!");

        assert_eq!(buf.as_slice(), b"Hello, World!");
        assert!(!buf.has_grown());
    }

    #[test]
    fn test_pre_sized_buffer_freeze() {
        let mut buf = PreSizedBuffer::new();
        buf.write(b"test data");

        let bytes = buf.freeze();
        assert_eq!(&bytes[..], b"test data");
    }

    #[test]
    fn test_object_pool_acquire_release() {
        let pool: ObjectPool<PooledRequest> = ObjectPool::new(PoolConfig::new().pre_warm(false));

        // First acquire should miss
        {
            let _req = pool.acquire();
            assert_eq!(pool.stats().misses(), 1);
        }

        // Object returned to pool

        // Second acquire should hit
        {
            let _req = pool.acquire();
            assert_eq!(pool.stats().hits(), 1);
        }
    }

    #[test]
    fn test_pooled_request() {
        let pool: ObjectPool<PooledRequest> = ObjectPool::default_pool();
        let mut req = pool.acquire();

        req.method("POST")
            .path("/api/users")
            .header("Content-Type", "application/json")
            .body(b"{\"name\":\"test\"}");

        assert_eq!(req.method.as_str(), "POST");
        assert_eq!(req.path.as_str(), "/api/users");
        assert!(req.headers.get("content-type").is_some());
    }

    #[test]
    fn test_pooled_response() {
        let pool: ObjectPool<PooledResponse> = ObjectPool::default_pool();
        let mut resp = pool.acquire();

        resp.status(201)
            .header("Content-Type", "application/json")
            .body(b"{\"id\":1}");

        assert_eq!(resp.status, 201);
        assert!(resp.headers.get("content-type").is_some());
    }

    #[test]
    fn test_global_pools() {
        init_pools(PoolConfig::new().initial_size(10));

        let req = acquire_request();
        assert!(req.path.is_empty());

        let resp = acquire_response();
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn test_pool_stats() {
        let pool: ObjectPool<PooledRequest> = ObjectPool::new(PoolConfig::new().pre_warm(false));

        {
            let _r1 = pool.acquire();
            let _r2 = pool.acquire();
        }

        {
            let _r3 = pool.acquire();
        }

        assert_eq!(pool.stats().misses(), 2);
        assert_eq!(pool.stats().hits(), 1);
        assert!(pool.stats().hit_ratio() > 0.0);
    }

    #[test]
    fn test_memory_stats() {
        let stats = memory_stats();
        let _ = stats.headers_inline();
        let _ = stats.paths_inline();
        let _ = stats.buffer_allocs();
        let _ = stats.pool_hit_ratio();
    }
}
