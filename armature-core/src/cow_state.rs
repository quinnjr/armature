//! Copy-on-Write State Management
//!
//! This module provides efficient state management patterns using Arc<T>
//! that avoid cloning data on read operations. Writes create new versions,
//! while reads simply clone the Arc pointer.
//!
//! # Key Types
//!
//! - [`CowState<T>`]: Copy-on-Write state with cheap reads
//! - [`Snapshot<T>`]: Immutable snapshot from a point in time
//! - [`VersionedState<T>`]: State with version tracking for cache invalidation
//! - [`AtomicState<T>`]: Lock-free state with atomic updates
//!
//! # Performance
//!
//! - **Read**: O(1) - Just Arc clone (atomic increment)
//! - **Write**: O(n) - Clone data + atomic swap
//! - **Memory**: One allocation per version
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_core::cow_state::{CowState, Snapshot};
//!
//! #[derive(Clone)]
//! struct Config {
//!     max_connections: usize,
//!     timeout_ms: u64,
//! }
//!
//! // Create mutable state
//! let state = CowState::new(Config {
//!     max_connections: 100,
//!     timeout_ms: 5000,
//! });
//!
//! // Cheap read (just Arc clone)
//! let snapshot: Snapshot<Config> = state.snapshot();
//! println!("Max: {}", snapshot.max_connections);
//!
//! // Update creates new version
//! state.update(|config| {
//!     config.max_connections = 200;
//! });
//!
//! // Old snapshot still valid
//! assert_eq!(snapshot.max_connections, 100);
//!
//! // New snapshot sees update
//! let new_snapshot = state.snapshot();
//! assert_eq!(new_snapshot.max_connections, 200);
//! ```

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

// ============================================================================
// Snapshot - Immutable View
// ============================================================================

/// An immutable snapshot of state at a point in time.
///
/// Snapshots are cheap to clone (just Arc increment) and provide
/// read-only access to the state. Multiple snapshots can exist
/// simultaneously, each seeing the state as it was when created.
///
/// # Example
///
/// ```rust,ignore
/// let state = CowState::new(vec![1, 2, 3]);
/// let snap1 = state.snapshot();
///
/// state.update(|v| v.push(4));
///
/// let snap2 = state.snapshot();
///
/// // snap1 still sees old state
/// assert_eq!(snap1.len(), 3);
/// // snap2 sees new state
/// assert_eq!(snap2.len(), 4);
/// ```
#[derive(Debug)]
pub struct Snapshot<T> {
    /// The actual data, wrapped in Arc for cheap cloning
    inner: Arc<T>,
    /// Version number when snapshot was taken
    version: u64,
}

impl<T> Snapshot<T> {
    /// Create a new snapshot from data.
    #[inline]
    pub fn new(data: T) -> Self {
        Self {
            inner: Arc::new(data),
            version: 0,
        }
    }

    /// Create from Arc with version.
    #[inline]
    pub(crate) fn from_arc(arc: Arc<T>, version: u64) -> Self {
        Self {
            inner: arc,
            version,
        }
    }

    /// Get the version number of this snapshot.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Get the inner Arc.
    #[inline]
    pub fn into_arc(self) -> Arc<T> {
        self.inner
    }

    /// Get reference to inner Arc.
    #[inline]
    pub fn as_arc(&self) -> &Arc<T> {
        &self.inner
    }

    /// Check if this snapshot is from the same source as another.
    #[inline]
    pub fn same_source(&self, other: &Snapshot<T>) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Get strong reference count.
    #[inline]
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }
}

impl<T> Clone for Snapshot<T> {
    #[inline]
    fn clone(&self) -> Self {
        COW_STATS.record_snapshot_clone();
        Self {
            inner: Arc::clone(&self.inner),
            version: self.version,
        }
    }
}

impl<T> Deref for Snapshot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> AsRef<T> for Snapshot<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

// ============================================================================
// CowState - Copy-on-Write State
// ============================================================================

/// Copy-on-Write state container.
///
/// Provides efficient read access through snapshots (Arc clones) while
/// writes create new versions. This is ideal for configuration, caches,
/// and other read-heavy state.
///
/// # Thread Safety
///
/// - Reads (snapshot): Lock-free, concurrent
/// - Writes (update): Serialized through RwLock
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cow_state::CowState;
///
/// let state = CowState::new(vec!["a", "b", "c"]);
///
/// // Multiple threads can read concurrently
/// let handles: Vec<_> = (0..10).map(|_| {
///     let snap = state.snapshot();
///     std::thread::spawn(move || {
///         println!("{:?}", *snap);
///     })
/// }).collect();
///
/// // Update (serialized)
/// state.update(|v| v.push("d"));
/// ```
pub struct CowState<T> {
    /// Current state wrapped in Arc for cheap cloning
    current: RwLock<Arc<T>>,
    /// Version counter
    version: AtomicU64,
}

impl<T> CowState<T> {
    /// Create new CoW state.
    pub fn new(value: T) -> Self {
        COW_STATS.record_state_created();
        Self {
            current: RwLock::new(Arc::new(value)),
            version: AtomicU64::new(1),
        }
    }

    /// Get a snapshot of the current state.
    ///
    /// This is very cheap - just an Arc clone.
    #[inline]
    pub fn snapshot(&self) -> Snapshot<T> {
        let arc = self.current.read().unwrap();
        let version = self.version.load(Ordering::Acquire);
        COW_STATS.record_snapshot_taken();
        Snapshot::from_arc(Arc::clone(&arc), version)
    }

    /// Get current version number.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Check if version matches.
    #[inline]
    pub fn is_version(&self, version: u64) -> bool {
        self.version() == version
    }
}

impl<T: Clone> CowState<T> {
    /// Update state by applying a function.
    ///
    /// This clones the current state, applies the function, then
    /// atomically swaps to the new version.
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        let mut guard = self.current.write().unwrap();
        let mut new_value = (**guard).clone();
        f(&mut new_value);
        *guard = Arc::new(new_value);
        self.version.fetch_add(1, Ordering::Release);
        COW_STATS.record_update();
    }

    /// Replace state entirely.
    pub fn replace(&self, value: T) {
        let mut guard = self.current.write().unwrap();
        *guard = Arc::new(value);
        self.version.fetch_add(1, Ordering::Release);
        COW_STATS.record_update();
    }

    /// Update state only if predicate is true.
    ///
    /// Returns true if update was applied.
    pub fn update_if<P, F>(&self, predicate: P, f: F) -> bool
    where
        P: FnOnce(&T) -> bool,
        F: FnOnce(&mut T),
    {
        let mut guard = self.current.write().unwrap();
        if predicate(&guard) {
            let mut new_value = (**guard).clone();
            f(&mut new_value);
            *guard = Arc::new(new_value);
            self.version.fetch_add(1, Ordering::Release);
            COW_STATS.record_update();
            true
        } else {
            false
        }
    }

    /// Compare-and-swap: update only if version matches.
    ///
    /// Returns Ok(new_version) if successful, Err(current_version) if version mismatch.
    pub fn compare_and_swap<F>(&self, expected_version: u64, f: F) -> Result<u64, u64>
    where
        F: FnOnce(&mut T),
    {
        let mut guard = self.current.write().unwrap();
        let current = self.version.load(Ordering::Acquire);
        if current != expected_version {
            return Err(current);
        }
        let mut new_value = (**guard).clone();
        f(&mut new_value);
        *guard = Arc::new(new_value);
        let new_version = self.version.fetch_add(1, Ordering::Release) + 1;
        COW_STATS.record_update();
        Ok(new_version)
    }
}

impl<T: Clone + Default> Default for CowState<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for CowState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snap = self.current.read().unwrap();
        f.debug_struct("CowState")
            .field("value", &**snap)
            .field("version", &self.version.load(Ordering::Relaxed))
            .finish()
    }
}

// ============================================================================
// VersionedState - State with Cache Invalidation
// ============================================================================

/// State with version tracking for cache invalidation.
///
/// Useful when you need to know if state has changed since
/// you last read it (e.g., for caching derived data).
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cow_state::VersionedState;
///
/// let state = VersionedState::new(Config::default());
///
/// // Cache with version
/// let cached = state.read();
/// let cache_version = state.version();
///
/// // Later, check if cache is stale
/// if state.version() != cache_version {
///     // Re-read and update cache
/// }
/// ```
pub struct VersionedState<T> {
    /// The state value
    inner: CowState<T>,
    /// Name for debugging
    #[allow(dead_code)]
    name: &'static str,
}

impl<T> VersionedState<T> {
    /// Create new versioned state.
    pub fn new(value: T) -> Self {
        Self {
            inner: CowState::new(value),
            name: std::any::type_name::<T>(),
        }
    }

    /// Create with custom name.
    pub fn with_name(value: T, name: &'static str) -> Self {
        Self {
            inner: CowState::new(value),
            name,
        }
    }

    /// Get current version.
    #[inline]
    pub fn version(&self) -> u64 {
        self.inner.version()
    }

    /// Get a snapshot.
    #[inline]
    pub fn read(&self) -> Snapshot<T> {
        self.inner.snapshot()
    }

    /// Get snapshot with version.
    #[inline]
    pub fn read_versioned(&self) -> (Snapshot<T>, u64) {
        let snap = self.inner.snapshot();
        let version = snap.version();
        (snap, version)
    }

    /// Check if state has changed since version.
    #[inline]
    pub fn changed_since(&self, version: u64) -> bool {
        self.version() != version
    }
}

impl<T: Clone> VersionedState<T> {
    /// Update state.
    pub fn write<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.inner.update(f);
    }

    /// Replace state entirely.
    pub fn set(&self, value: T) {
        self.inner.replace(value);
    }

    /// Conditional update based on version.
    pub fn write_if_version<F>(&self, version: u64, f: F) -> Result<u64, u64>
    where
        F: FnOnce(&mut T),
    {
        self.inner.compare_and_swap(version, f)
    }
}

impl<T: Clone + Default> Default for VersionedState<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for VersionedState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VersionedState")
            .field("inner", &self.inner)
            .field("name", &self.name)
            .finish()
    }
}

// ============================================================================
// AtomicState - Lock-Free Updates
// ============================================================================

/// Lock-free state using atomic pointer swaps.
///
/// Provides even lower contention than CowState by using atomic
/// operations instead of RwLock. Best for very high read frequency.
///
/// # Caution
///
/// Multiple concurrent writes may "lose" updates. Use when:
/// - Writes are rare
/// - Lost updates are acceptable
/// - Or use compare_exchange pattern
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cow_state::AtomicState;
/// use std::sync::Arc;
///
/// let state = AtomicState::new(Arc::new(Config::default()));
///
/// // Very fast reads
/// let config = state.load();
///
/// // Atomic update
/// state.store(Arc::new(new_config));
/// ```
pub struct AtomicState<T> {
    /// Current value as atomic Arc
    current: std::sync::atomic::AtomicPtr<Arc<T>>,
    /// Version counter
    version: AtomicU64,
}

impl<T> AtomicState<T> {
    /// Create new atomic state.
    pub fn new(value: Arc<T>) -> Self {
        let boxed = Box::new(value);
        let ptr = Box::into_raw(boxed);
        COW_STATS.record_state_created();
        Self {
            current: std::sync::atomic::AtomicPtr::new(ptr),
            version: AtomicU64::new(1),
        }
    }

    /// Load current value.
    ///
    /// Very fast - just atomic load and Arc clone.
    #[inline]
    pub fn load(&self) -> Arc<T> {
        let ptr = self.current.load(Ordering::Acquire);
        // SAFETY: ptr always points to valid Box<Arc<T>>
        let arc_ref = unsafe { &*ptr };
        COW_STATS.record_snapshot_taken();
        Arc::clone(arc_ref)
    }

    /// Store new value.
    ///
    /// Atomically swaps to new value, dropping old.
    pub fn store(&self, value: Arc<T>) {
        let new_box = Box::new(value);
        let new_ptr = Box::into_raw(new_box);
        let old_ptr = self.current.swap(new_ptr, Ordering::AcqRel);
        self.version.fetch_add(1, Ordering::Release);
        // SAFETY: old_ptr was created by Box::into_raw
        let _old = unsafe { Box::from_raw(old_ptr) };
        COW_STATS.record_update();
    }

    /// Get current version.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Load with version.
    #[inline]
    pub fn load_versioned(&self) -> (Arc<T>, u64) {
        let value = self.load();
        let version = self.version();
        (value, version)
    }
}

impl<T: Clone> AtomicState<T> {
    /// Update using function (read-modify-write).
    ///
    /// Note: Not atomic - another update may happen between read and write.
    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&T) -> T,
    {
        let current = self.load();
        let new_value = f(&current);
        self.store(Arc::new(new_value));
    }
}

impl<T> Drop for AtomicState<T> {
    fn drop(&mut self) {
        let ptr = self.current.load(Ordering::Acquire);
        if !ptr.is_null() {
            // SAFETY: ptr was created by Box::into_raw
            let _old = unsafe { Box::from_raw(ptr) };
        }
    }
}

// SAFETY: AtomicState uses atomic operations for all accesses
unsafe impl<T: Send + Sync> Send for AtomicState<T> {}
unsafe impl<T: Send + Sync> Sync for AtomicState<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for AtomicState<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.load();
        f.debug_struct("AtomicState")
            .field("value", &*value)
            .field("version", &self.version())
            .finish()
    }
}

// ============================================================================
// Cached Value with Expiry
// ============================================================================

/// A cached value that automatically invalidates.
///
/// Useful for caching expensive computations with a TTL.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::cow_state::CachedValue;
/// use std::time::Duration;
///
/// let cache: CachedValue<Vec<User>> = CachedValue::new(Duration::from_secs(60));
///
/// // Get or compute
/// let users = cache.get_or_compute(|| {
///     db.query("SELECT * FROM users")
/// });
/// ```
pub struct CachedValue<T> {
    /// Cached value
    value: RwLock<Option<CacheEntry<T>>>,
    /// TTL
    ttl: std::time::Duration,
}

#[derive(Clone)]
struct CacheEntry<T> {
    value: Arc<T>,
    created_at: std::time::Instant,
    version: u64,
}

impl<T> CachedValue<T> {
    /// Create new cached value with TTL.
    pub fn new(ttl: std::time::Duration) -> Self {
        Self {
            value: RwLock::new(None),
            ttl,
        }
    }

    /// Check if cache is valid.
    pub fn is_valid(&self) -> bool {
        let guard = self.value.read().unwrap();
        guard
            .as_ref()
            .map(|e| e.created_at.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    /// Get cached value if valid.
    pub fn get(&self) -> Option<Arc<T>> {
        let guard = self.value.read().unwrap();
        guard.as_ref().and_then(|e| {
            if e.created_at.elapsed() < self.ttl {
                COW_STATS.record_cache_hit();
                Some(Arc::clone(&e.value))
            } else {
                COW_STATS.record_cache_miss();
                None
            }
        })
    }

    /// Set cached value.
    pub fn set(&self, value: T) {
        let mut guard = self.value.write().unwrap();
        let version = guard.as_ref().map(|e| e.version + 1).unwrap_or(1);
        *guard = Some(CacheEntry {
            value: Arc::new(value),
            created_at: std::time::Instant::now(),
            version,
        });
    }

    /// Invalidate cache.
    pub fn invalidate(&self) {
        let mut guard = self.value.write().unwrap();
        *guard = None;
    }

    /// Get TTL.
    pub fn ttl(&self) -> std::time::Duration {
        self.ttl
    }

    /// Get remaining TTL.
    pub fn remaining_ttl(&self) -> Option<std::time::Duration> {
        let guard = self.value.read().unwrap();
        guard.as_ref().and_then(|e| {
            let elapsed = e.created_at.elapsed();
            if elapsed < self.ttl {
                Some(self.ttl - elapsed)
            } else {
                None
            }
        })
    }
}

impl<T: Clone> CachedValue<T> {
    /// Get cached value or compute it.
    pub fn get_or_compute<F>(&self, f: F) -> Arc<T>
    where
        F: FnOnce() -> T,
    {
        // Fast path: check read lock first
        {
            let guard = self.value.read().unwrap();
            if let Some(ref entry) = *guard
                && entry.created_at.elapsed() < self.ttl
            {
                COW_STATS.record_cache_hit();
                return Arc::clone(&entry.value);
            }
        }

        // Slow path: compute and store
        COW_STATS.record_cache_miss();
        let mut guard = self.value.write().unwrap();

        // Double-check after acquiring write lock
        if let Some(ref entry) = *guard
            && entry.created_at.elapsed() < self.ttl
        {
            return Arc::clone(&entry.value);
        }

        let value = Arc::new(f());
        let version = guard.as_ref().map(|e| e.version + 1).unwrap_or(1);
        *guard = Some(CacheEntry {
            value: Arc::clone(&value),
            created_at: std::time::Instant::now(),
            version,
        });
        value
    }

    /// Get cached value or compute it asynchronously.
    pub async fn get_or_compute_async<F, Fut>(&self, f: F) -> Arc<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        // Fast path
        {
            let guard = self.value.read().unwrap();
            if let Some(ref entry) = *guard
                && entry.created_at.elapsed() < self.ttl
            {
                COW_STATS.record_cache_hit();
                return Arc::clone(&entry.value);
            }
        }

        // Slow path
        COW_STATS.record_cache_miss();
        let value = Arc::new(f().await);

        let mut guard = self.value.write().unwrap();
        let version = guard.as_ref().map(|e| e.version + 1).unwrap_or(1);
        *guard = Some(CacheEntry {
            value: Arc::clone(&value),
            created_at: std::time::Instant::now(),
            version,
        });
        value
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for CachedValue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let is_valid = self.is_valid();
        let remaining = self.remaining_ttl();
        f.debug_struct("CachedValue")
            .field("is_valid", &is_valid)
            .field("remaining_ttl", &remaining)
            .field("ttl", &self.ttl)
            .finish()
    }
}

// ============================================================================
// Global Statistics
// ============================================================================

/// Statistics for CoW state operations.
#[derive(Debug, Default)]
pub struct CowStats {
    /// States created
    states_created: AtomicU64,
    /// Snapshots taken
    snapshots_taken: AtomicU64,
    /// Snapshot clones
    snapshot_clones: AtomicU64,
    /// Updates performed
    updates: AtomicU64,
    /// Cache hits
    cache_hits: AtomicU64,
    /// Cache misses
    cache_misses: AtomicU64,
}

impl CowStats {
    fn record_state_created(&self) {
        self.states_created.fetch_add(1, Ordering::Relaxed);
    }

    fn record_snapshot_taken(&self) {
        self.snapshots_taken.fetch_add(1, Ordering::Relaxed);
    }

    fn record_snapshot_clone(&self) {
        self.snapshot_clones.fetch_add(1, Ordering::Relaxed);
    }

    fn record_update(&self) {
        self.updates.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get states created count.
    pub fn states_created(&self) -> u64 {
        self.states_created.load(Ordering::Relaxed)
    }

    /// Get snapshots taken count.
    pub fn snapshots_taken(&self) -> u64 {
        self.snapshots_taken.load(Ordering::Relaxed)
    }

    /// Get snapshot clones count.
    pub fn snapshot_clones(&self) -> u64 {
        self.snapshot_clones.load(Ordering::Relaxed)
    }

    /// Get updates count.
    pub fn updates(&self) -> u64 {
        self.updates.load(Ordering::Relaxed)
    }

    /// Get cache hits.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Get cache misses.
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    /// Get cache hit ratio.
    pub fn cache_hit_ratio(&self) -> f64 {
        let hits = self.cache_hits() as f64;
        let total = hits + self.cache_misses() as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }

    /// Get read/write ratio.
    pub fn read_write_ratio(&self) -> f64 {
        let reads = self.snapshots_taken() as f64;
        let writes = self.updates() as f64;
        if writes > 0.0 {
            reads / writes
        } else {
            f64::INFINITY
        }
    }
}

/// Global CoW statistics.
static COW_STATS: CowStats = CowStats {
    states_created: AtomicU64::new(0),
    snapshots_taken: AtomicU64::new(0),
    snapshot_clones: AtomicU64::new(0),
    updates: AtomicU64::new(0),
    cache_hits: AtomicU64::new(0),
    cache_misses: AtomicU64::new(0),
};

/// Get global CoW statistics.
pub fn cow_stats() -> &'static CowStats {
    &COW_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_basic() {
        let snap = Snapshot::new(42);
        assert_eq!(*snap, 42);
        assert_eq!(snap.version(), 0);
    }

    #[test]
    fn test_snapshot_clone_is_cheap() {
        let snap = Snapshot::new(vec![1, 2, 3, 4, 5]);
        assert_eq!(snap.ref_count(), 1);

        let snap2 = snap.clone();
        assert_eq!(snap.ref_count(), 2);
        assert_eq!(snap2.ref_count(), 2);

        assert!(snap.same_source(&snap2));
    }

    #[test]
    fn test_cow_state_basic() {
        let state = CowState::new(10);
        assert_eq!(*state.snapshot(), 10);
        assert_eq!(state.version(), 1);
    }

    #[test]
    fn test_cow_state_update() {
        let state = CowState::new(vec![1, 2, 3]);
        let snap1 = state.snapshot();
        assert_eq!(state.version(), 1);

        state.update(|v| v.push(4));
        assert_eq!(state.version(), 2);

        let snap2 = state.snapshot();

        // Old snapshot unchanged
        assert_eq!(*snap1, vec![1, 2, 3]);
        // New snapshot has update
        assert_eq!(*snap2, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_cow_state_replace() {
        let state = CowState::new("old".to_string());
        state.replace("new".to_string());
        assert_eq!(*state.snapshot(), "new");
    }

    #[test]
    fn test_cow_state_update_if() {
        let state = CowState::new(5);

        // Should update (predicate true)
        let updated = state.update_if(|&v| v < 10, |v| *v += 1);
        assert!(updated);
        assert_eq!(*state.snapshot(), 6);

        // Should not update (predicate false)
        let updated = state.update_if(|&v| v > 100, |v| *v += 1);
        assert!(!updated);
        assert_eq!(*state.snapshot(), 6);
    }

    #[test]
    fn test_cow_state_compare_and_swap() {
        let state = CowState::new(100);
        let version = state.version();

        // Should succeed
        let result = state.compare_and_swap(version, |v| *v += 1);
        assert!(result.is_ok());
        assert_eq!(*state.snapshot(), 101);

        // Should fail (wrong version)
        let result = state.compare_and_swap(version, |v| *v += 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_versioned_state() {
        let state = VersionedState::new(vec!["a", "b"]);
        let v1 = state.version();

        state.write(|v| v.push("c"));
        let v2 = state.version();

        assert_ne!(v1, v2);
        assert!(state.changed_since(v1));
        assert!(!state.changed_since(v2));
    }

    #[test]
    fn test_atomic_state() {
        let state = AtomicState::new(Arc::new(42));
        assert_eq!(*state.load(), 42);

        state.store(Arc::new(100));
        assert_eq!(*state.load(), 100);
    }

    #[test]
    fn test_atomic_state_update() {
        let state = AtomicState::new(Arc::new(10));
        state.update(|&v| v * 2);
        assert_eq!(*state.load(), 20);
    }

    #[test]
    fn test_cached_value() {
        let cache: CachedValue<i32> = CachedValue::new(std::time::Duration::from_secs(60));

        assert!(!cache.is_valid());
        assert!(cache.get().is_none());

        cache.set(42);
        assert!(cache.is_valid());
        assert_eq!(*cache.get().unwrap(), 42);
    }

    #[test]
    fn test_cached_value_compute() {
        let cache: CachedValue<i32> = CachedValue::new(std::time::Duration::from_secs(60));
        let mut computed_count = 0;

        let value = cache.get_or_compute(|| {
            computed_count += 1;
            42
        });
        assert_eq!(*value, 42);

        // Second call should use cache
        let value2 = cache.get_or_compute(|| {
            computed_count += 1;
            99
        });
        assert_eq!(*value2, 42); // Still 42, not recomputed
    }

    #[test]
    fn test_cow_stats() {
        let stats = cow_stats();
        let _ = stats.states_created();
        let _ = stats.snapshots_taken();
        let _ = stats.updates();
        let _ = stats.cache_hit_ratio();
        let _ = stats.read_write_ratio();
    }
}
