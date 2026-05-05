//! Cache-Local State Management
//!
//! This module provides data structures and patterns for keeping frequently
//! accessed state in CPU cache, maximizing cache hits and minimizing cache
//! misses.
//!
//! # Key Concepts
//!
//! - **Cache Line Alignment**: Align hot data to cache line boundaries (64 bytes)
//! - **Hot/Cold Separation**: Keep frequently accessed data together
//! - **Padding**: Prevent false sharing between threads
//! - **Prefetching**: Hints for hardware prefetcher
//!
//! # Performance Impact
//!
//! - L1 cache hit: ~4 cycles
//! - L2 cache hit: ~10 cycles
//! - L3 cache hit: ~40 cycles
//! - Main memory: ~200+ cycles
//!
//! Keeping hot data in cache can provide 50x speedup over memory access.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

// ============================================================================
// Constants
// ============================================================================

/// Typical CPU cache line size (64 bytes on most x86/ARM)
pub const CACHE_LINE_SIZE: usize = 64;

/// L1 data cache typical size (32KB)
pub const L1_CACHE_SIZE: usize = 32 * 1024;

/// L2 cache typical size (256KB)
pub const L2_CACHE_SIZE: usize = 256 * 1024;

/// L3 cache typical size per core (2MB)
pub const L3_CACHE_SIZE_PER_CORE: usize = 2 * 1024 * 1024;

// ============================================================================
// Cache-Line Aligned Wrapper
// ============================================================================

/// A value aligned to a cache line boundary.
///
/// This prevents false sharing when the value is accessed from multiple
/// threads, as each thread will have the value in its own cache line.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::CacheAligned;
///
/// // Each counter in its own cache line - no false sharing
/// struct Counters {
///     reads: CacheAligned<AtomicU64>,
///     writes: CacheAligned<AtomicU64>,
/// }
/// ```
#[repr(C, align(64))]
#[derive(Debug)]
pub struct CacheAligned<T> {
    value: T,
    // Padding is implicit due to align(64)
}

impl<T> CacheAligned<T> {
    /// Create a new cache-aligned value.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self { value }
    }

    /// Get inner value.
    #[inline]
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Get reference to inner value.
    #[inline]
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Get mutable reference to inner value.
    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T: Default> Default for CacheAligned<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone> Clone for CacheAligned<T> {
    fn clone(&self) -> Self {
        Self::new(self.value.clone())
    }
}

impl<T> Deref for CacheAligned<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for CacheAligned<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

// ============================================================================
// Padded Value (for arrays)
// ============================================================================

/// A value with explicit padding to fill a cache line.
///
/// Use this in arrays where each element should have its own cache line.
/// Uses maximum padding (56 bytes) to ensure cache line isolation regardless
/// of the inner type size (up to 8 bytes).
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::Padded;
/// use std::sync::atomic::AtomicU64;
///
/// // Array of counters, each in its own cache line
/// let counters: [Padded<AtomicU64>; 8] = Default::default();
/// ```
#[repr(C, align(64))]
#[derive(Debug)]
pub struct Padded<T> {
    value: T,
}

impl<T> Padded<T> {
    /// Create a new padded value.
    #[inline]
    pub const fn new(value: T) -> Self {
        Self { value }
    }

    /// Get inner value.
    #[inline]
    pub fn into_inner(self) -> T {
        self.value
    }
}

impl<T: Default> Default for Padded<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone> Clone for Padded<T> {
    fn clone(&self) -> Self {
        Self::new(self.value.clone())
    }
}

impl<T> Deref for Padded<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for Padded<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

// ============================================================================
// Hot/Cold Separation
// ============================================================================

/// Container for separating hot (frequently accessed) and cold (rarely accessed) data.
///
/// Hot data is kept inline for cache locality, while cold data is boxed to
/// keep it out of the hot path's cache lines.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::HotCold;
///
/// // Hot: request count, last access time
/// // Cold: full request history, debug info
/// struct RequestState {
///     data: HotCold<HotData, ColdData>,
/// }
///
/// struct HotData {
///     count: u64,
///     last_access: Instant,
/// }
///
/// struct ColdData {
///     history: Vec<RequestInfo>,
///     debug_log: String,
/// }
/// ```
#[derive(Debug)]
pub struct HotCold<H, C> {
    /// Hot data - kept inline
    pub hot: H,
    /// Cold data - boxed to keep out of cache
    cold: Box<C>,
}

impl<H, C> HotCold<H, C> {
    /// Create new hot/cold container.
    pub fn new(hot: H, cold: C) -> Self {
        Self {
            hot,
            cold: Box::new(cold),
        }
    }

    /// Access hot data (cache-friendly).
    #[inline(always)]
    pub fn hot(&self) -> &H {
        &self.hot
    }

    /// Access hot data mutably.
    #[inline(always)]
    pub fn hot_mut(&mut self) -> &mut H {
        &mut self.hot
    }

    /// Access cold data (may cause cache miss).
    #[inline]
    pub fn cold(&self) -> &C {
        &self.cold
    }

    /// Access cold data mutably.
    #[inline]
    pub fn cold_mut(&mut self) -> &mut C {
        &mut self.cold
    }

    /// Decompose into parts.
    pub fn into_parts(self) -> (H, C) {
        (self.hot, *self.cold)
    }
}

impl<H: Clone, C: Clone> Clone for HotCold<H, C> {
    fn clone(&self) -> Self {
        Self::new(self.hot.clone(), (*self.cold).clone())
    }
}

// ============================================================================
// Local State Cache
// ============================================================================

/// Thread-local cache for frequently accessed state.
///
/// Provides O(1) access to cached values with automatic invalidation
/// based on a global version counter.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::LocalStateCache;
///
/// thread_local! {
///     static CONFIG_CACHE: LocalStateCache<Config> = LocalStateCache::new();
/// }
///
/// fn get_config(global: &GlobalConfig) -> Config {
///     CONFIG_CACHE.with(|cache| {
///         cache.get_or_refresh(global.version(), || global.snapshot())
///     })
/// }
/// ```
pub struct LocalStateCache<T> {
    /// Cached value
    value: UnsafeCell<Option<T>>,
    /// Version when cached
    version: UnsafeCell<u64>,
    /// Hit count
    hits: UnsafeCell<u64>,
    /// Miss count
    misses: UnsafeCell<u64>,
}

impl<T> LocalStateCache<T> {
    /// Create a new empty cache.
    pub const fn new() -> Self {
        Self {
            value: UnsafeCell::new(None),
            version: UnsafeCell::new(0),
            hits: UnsafeCell::new(0),
            misses: UnsafeCell::new(0),
        }
    }

    /// Get cached value or refresh if stale.
    ///
    /// # Safety
    ///
    /// Must only be called from the owning thread (thread_local).
    #[inline]
    pub fn get_or_refresh<F>(&self, current_version: u64, refresh: F) -> &T
    where
        F: FnOnce() -> T,
    {
        // SAFETY: Only called from owning thread
        unsafe {
            let cached_version = *self.version.get();

            if cached_version == current_version
                && let Some(ref value) = *self.value.get()
            {
                *self.hits.get() += 1;
                LOCALITY_STATS.record_cache_hit();
                return value;
            }

            // Cache miss - refresh
            *self.misses.get() += 1;
            LOCALITY_STATS.record_cache_miss();

            let new_value = refresh();
            *self.value.get() = Some(new_value);
            *self.version.get() = current_version;

            (*self.value.get()).as_ref().unwrap()
        }
    }

    /// Invalidate the cache.
    #[inline]
    pub fn invalidate(&self) {
        // SAFETY: Only called from owning thread
        unsafe {
            *self.value.get() = None;
            *self.version.get() = 0;
        }
    }

    /// Get cache statistics.
    pub fn stats(&self) -> LocalCacheStats {
        // SAFETY: Only called from owning thread
        unsafe {
            LocalCacheStats {
                hits: *self.hits.get(),
                misses: *self.misses.get(),
            }
        }
    }
}

impl<T> Default for LocalStateCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: LocalStateCache is only accessed from owning thread
unsafe impl<T: Send> Send for LocalStateCache<T> {}

/// Statistics for a local cache.
#[derive(Debug, Clone, Copy)]
pub struct LocalCacheStats {
    /// Cache hits
    pub hits: u64,
    /// Cache misses
    pub misses: u64,
}

impl LocalCacheStats {
    /// Get hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total > 0 {
            self.hits as f64 / total as f64
        } else {
            0.0
        }
    }
}

// ============================================================================
// Compact State
// ============================================================================

/// Compact state that fits in a single cache line.
///
/// Packs multiple small values into 64 bytes for efficient access.
/// All fields are accessed together with a single cache line fetch.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::CompactState;
///
/// let state = CompactState::<4>::new();
/// state.set(0, 42);
/// state.set(1, 100);
/// let val = state.get(0);
/// ```
#[repr(C, align(64))]
pub struct CompactState<const N: usize> {
    /// Values packed into cache line
    values: [AtomicU64; N],
}

impl<const N: usize> CompactState<N> {
    /// Create new compact state.
    pub fn new() -> Self {
        // Ensure we fit in a cache line
        const {
            assert!(
                std::mem::size_of::<[AtomicU64; N]>() <= CACHE_LINE_SIZE,
                "CompactState exceeds cache line size"
            );
        }

        Self {
            values: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    /// Get value at index.
    #[inline]
    pub fn get(&self, index: usize) -> u64 {
        self.values[index].load(Ordering::Relaxed)
    }

    /// Set value at index.
    #[inline]
    pub fn set(&self, index: usize, value: u64) {
        self.values[index].store(value, Ordering::Relaxed);
    }

    /// Increment value at index.
    #[inline]
    pub fn increment(&self, index: usize) -> u64 {
        self.values[index].fetch_add(1, Ordering::Relaxed)
    }

    /// Add to value at index.
    #[inline]
    pub fn add(&self, index: usize, delta: u64) -> u64 {
        self.values[index].fetch_add(delta, Ordering::Relaxed)
    }

    /// Get all values as array.
    pub fn all(&self) -> [u64; N] {
        std::array::from_fn(|i| self.get(i))
    }
}

impl<const N: usize> Default for CompactState<N> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Prefetch Hints
// ============================================================================

/// Prefetch hint level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchLevel {
    /// Prefetch to L1 cache (fastest, smallest)
    L1,
    /// Prefetch to L2 cache
    L2,
    /// Prefetch to L3 cache (slowest, largest)
    L3,
    /// Non-temporal (don't pollute cache)
    NonTemporal,
}

/// Prefetch data into cache.
///
/// This is a hint to the CPU - it may be ignored.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cache_local::{prefetch, PrefetchLevel};
///
/// let data = vec![0u64; 1000];
///
/// // Prefetch before processing
/// for i in (0..1000).step_by(8) {
///     prefetch(&data[i], PrefetchLevel::L1);
/// }
/// ```
#[inline]
pub fn prefetch<T>(ptr: &T, level: PrefetchLevel) {
    let addr = ptr as *const T as *const u8;
    prefetch_ptr(addr, level);
}

/// Prefetch raw pointer.
#[inline]
pub fn prefetch_ptr(ptr: *const u8, level: PrefetchLevel) {
    // Use platform-specific prefetch instructions where available
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::*;
        unsafe {
            match level {
                PrefetchLevel::L1 => _mm_prefetch(ptr as *const i8, _MM_HINT_T0),
                PrefetchLevel::L2 => _mm_prefetch(ptr as *const i8, _MM_HINT_T1),
                PrefetchLevel::L3 => _mm_prefetch(ptr as *const i8, _MM_HINT_T2),
                PrefetchLevel::NonTemporal => _mm_prefetch(ptr as *const i8, _MM_HINT_NTA),
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // ARM prefetch
        unsafe {
            match level {
                PrefetchLevel::L1 => {
                    std::arch::asm!("prfm pldl1keep, [{x}]", x = in(reg) ptr, options(readonly, nostack));
                }
                PrefetchLevel::L2 => {
                    std::arch::asm!("prfm pldl2keep, [{x}]", x = in(reg) ptr, options(readonly, nostack));
                }
                PrefetchLevel::L3 => {
                    std::arch::asm!("prfm pldl3keep, [{x}]", x = in(reg) ptr, options(readonly, nostack));
                }
                PrefetchLevel::NonTemporal => {
                    std::arch::asm!("prfm pldl1strm, [{x}]", x = in(reg) ptr, options(readonly, nostack));
                }
            }
        }
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // Fallback: do nothing, let hardware handle it
        let _ = (ptr, level);
    }
}

/// Prefetch a range of memory.
#[inline]
pub fn prefetch_range<T>(slice: &[T], level: PrefetchLevel) {
    let ptr = slice.as_ptr() as *const u8;
    let len = std::mem::size_of_val(slice);

    for offset in (0..len).step_by(CACHE_LINE_SIZE) {
        prefetch_ptr(unsafe { ptr.add(offset) }, level);
    }
}

// ============================================================================
// Struct of Arrays (SoA) Layout
// ============================================================================

/// Struct-of-Arrays storage for better cache locality.
///
/// Instead of Array-of-Structs (AoS) which interleaves fields,
/// SoA groups same fields together for sequential access.
///
/// # Example
///
/// ```rust,ignore
/// // AoS (bad cache utilization when only accessing one field):
/// // [Point{x,y,z}, Point{x,y,z}, Point{x,y,z}...]
///
/// // SoA (good cache utilization):
/// // [x, x, x...], [y, y, y...], [z, z, z...]
///
/// let points = SoaStorage::<3, 1000>::new();
/// points.set_field(0, 42, 1.0);  // Set x[42] = 1.0
/// ```
pub struct SoaStorage<const FIELDS: usize, const CAPACITY: usize> {
    /// Storage for each field
    data: [Box<[MaybeUninit<f64>; CAPACITY]>; FIELDS],
    /// Number of elements
    len: AtomicUsize,
}

impl<const FIELDS: usize, const CAPACITY: usize> SoaStorage<FIELDS, CAPACITY> {
    /// Create new SoA storage.
    pub fn new() -> Self {
        Self {
            data: std::array::from_fn(|_| {
                Box::new(unsafe {
                    MaybeUninit::<[MaybeUninit<f64>; CAPACITY]>::uninit().assume_init()
                })
            }),
            len: AtomicUsize::new(0),
        }
    }

    /// Get current length.
    #[inline]
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    /// Get field value at index.
    #[inline]
    pub fn get_field(&self, field: usize, index: usize) -> f64 {
        assert!(field < FIELDS && index < self.len());
        unsafe { self.data[field][index].assume_init() }
    }

    /// Set field value at index.
    #[inline]
    pub fn set_field(&self, field: usize, index: usize, value: f64) {
        assert!(field < FIELDS && index < CAPACITY);
        unsafe {
            let ptr = self.data[field].as_ptr() as *mut MaybeUninit<f64>;
            (*ptr.add(index)).write(value);
        }
    }

    /// Push a new element (all fields must be set separately).
    pub fn push(&self) -> Option<usize> {
        let index = self.len.fetch_add(1, Ordering::Relaxed);
        if index < CAPACITY {
            Some(index)
        } else {
            self.len.fetch_sub(1, Ordering::Relaxed);
            None
        }
    }

    /// Get a field slice for sequential access.
    ///
    /// This is cache-friendly for operations on a single field.
    pub fn field_slice(&self, field: usize) -> &[f64] {
        assert!(field < FIELDS);
        let len = self.len();
        unsafe { std::slice::from_raw_parts(self.data[field].as_ptr() as *const f64, len) }
    }

    /// Prefetch a field for upcoming access.
    #[inline]
    pub fn prefetch_field(&self, field: usize, level: PrefetchLevel) {
        assert!(field < FIELDS);
        prefetch_range(self.field_slice(field), level);
    }
}

impl<const FIELDS: usize, const CAPACITY: usize> Default for SoaStorage<FIELDS, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Global Statistics
// ============================================================================

/// Statistics for cache locality operations.
#[derive(Debug, Default)]
pub struct LocalityStats {
    /// Local cache hits
    cache_hits: AtomicU64,
    /// Local cache misses
    cache_misses: AtomicU64,
    /// Prefetch operations
    prefetches: AtomicU64,
}

impl LocalityStats {
    fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn record_prefetch(&self) {
        self.prefetches.fetch_add(1, Ordering::Relaxed);
    }

    /// Get cache hits.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Get cache misses.
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    /// Get prefetch count.
    pub fn prefetches(&self) -> u64 {
        self.prefetches.load(Ordering::Relaxed)
    }

    /// Get cache hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let hits = self.cache_hits() as f64;
        let total = hits + self.cache_misses() as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }
}

/// Global locality statistics.
static LOCALITY_STATS: LocalityStats = LocalityStats {
    cache_hits: AtomicU64::new(0),
    cache_misses: AtomicU64::new(0),
    prefetches: AtomicU64::new(0),
};

/// Get global locality statistics.
pub fn locality_stats() -> &'static LocalityStats {
    &LOCALITY_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_aligned_size() {
        assert_eq!(std::mem::align_of::<CacheAligned<u64>>(), CACHE_LINE_SIZE);
        assert!(std::mem::size_of::<CacheAligned<u64>>() >= CACHE_LINE_SIZE);
    }

    #[test]
    fn test_cache_aligned_basic() {
        let aligned = CacheAligned::new(42u64);
        assert_eq!(*aligned, 42);
    }

    #[test]
    fn test_cache_aligned_deref() {
        let mut aligned = CacheAligned::new(vec![1, 2, 3]);
        aligned.push(4);
        assert_eq!(&*aligned, &vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_padded_size() {
        assert!(std::mem::size_of::<Padded<u64>>() >= CACHE_LINE_SIZE);
    }

    #[test]
    fn test_padded_basic() {
        let padded = Padded::new(100u64);
        assert_eq!(*padded, 100);
    }

    #[test]
    fn test_hot_cold_separation() {
        let data: HotCold<u64, Vec<String>> = HotCold::new(42, vec!["cold".into()]);

        assert_eq!(*data.hot(), 42);
        assert_eq!(data.cold().len(), 1);
    }

    #[test]
    fn test_hot_cold_mutation() {
        let mut data: HotCold<u64, Vec<u64>> = HotCold::new(0, vec![]);

        *data.hot_mut() = 100;
        data.cold_mut().push(1);

        assert_eq!(*data.hot(), 100);
        assert_eq!(data.cold().len(), 1);
    }

    #[test]
    fn test_local_state_cache() {
        let cache: LocalStateCache<u64> = LocalStateCache::new();

        // First access - miss
        let val1 = cache.get_or_refresh(1, || 42);
        assert_eq!(*val1, 42);

        // Same version - hit
        let val2 = cache.get_or_refresh(1, || 100);
        assert_eq!(*val2, 42); // Still 42, not 100

        // Different version - miss
        let val3 = cache.get_or_refresh(2, || 100);
        assert_eq!(*val3, 100);
    }

    #[test]
    fn test_local_cache_stats() {
        let cache: LocalStateCache<u64> = LocalStateCache::new();

        cache.get_or_refresh(1, || 1); // miss
        cache.get_or_refresh(1, || 2); // hit
        cache.get_or_refresh(1, || 3); // hit

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_ratio() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_compact_state() {
        let state = CompactState::<4>::new();

        state.set(0, 100);
        state.set(1, 200);
        assert_eq!(state.get(0), 100);
        assert_eq!(state.get(1), 200);

        state.increment(0);
        assert_eq!(state.get(0), 101);

        state.add(1, 50);
        assert_eq!(state.get(1), 250);
    }

    #[test]
    fn test_compact_state_all() {
        let state = CompactState::<4>::new();
        state.set(0, 1);
        state.set(1, 2);
        state.set(2, 3);
        state.set(3, 4);

        let all = state.all();
        assert_eq!(all, [1, 2, 3, 4]);
    }

    #[test]
    fn test_soa_storage() {
        let storage = SoaStorage::<3, 100>::new();

        let idx = storage.push().unwrap();
        storage.set_field(0, idx, 1.0);
        storage.set_field(1, idx, 2.0);
        storage.set_field(2, idx, 3.0);

        assert_eq!(storage.get_field(0, idx), 1.0);
        assert_eq!(storage.get_field(1, idx), 2.0);
        assert_eq!(storage.get_field(2, idx), 3.0);
    }

    #[test]
    fn test_soa_field_slice() {
        let storage = SoaStorage::<2, 100>::new();

        for _ in 0..10 {
            let idx = storage.push().unwrap();
            storage.set_field(0, idx, idx as f64);
        }

        let slice = storage.field_slice(0);
        assert_eq!(slice.len(), 10);
    }

    #[test]
    fn test_prefetch_levels() {
        // Just test that prefetch doesn't crash
        let data = vec![0u64; 100];
        prefetch(&data[0], PrefetchLevel::L1);
        prefetch(&data[50], PrefetchLevel::L2);
        prefetch(&data[99], PrefetchLevel::L3);
    }

    #[test]
    fn test_locality_stats() {
        let stats = locality_stats();
        let _ = stats.cache_hits();
        let _ = stats.cache_misses();
        let _ = stats.hit_ratio();
    }
}
