//! Type-safe request extensions for zero-cost state extraction.
//!
//! This module provides a way to attach typed data to requests without
//! runtime type checking overhead. Unlike the DI container which uses
//! `Any` downcasting, extensions use a type-erased map that only requires
//! type checks at the point of insertion, not retrieval.
//!
//! # Performance
//!
//! - **Insertion**: a linear scan over at most a handful of `TypeId`s
//! - **Retrieval**: the same scan, then a pointer cast (no runtime type check)
//! - **Memory**: a `SmallVec` with eight inline slots and one `Arc<T>` per
//!   extension type, so a realistic request never allocates a table
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_core::{Extensions, State};
//!
//! // Application state
//! #[derive(Clone)]
//! struct AppState {
//!     db_pool: Pool,
//! }
//!
//! // Insert state at startup
//! let mut extensions = Extensions::new();
//! extensions.insert(AppState { db_pool });
//!
//! // Extract in handler (zero-cost after setup)
//! async fn handler(state: State<AppState>) -> Result<HttpResponse, Error> {
//!     let pool = &state.db_pool;
//!     // ...
//! }
//! ```

use smallvec::SmallVec;
use std::any::{Any, TypeId};
use std::sync::Arc;

/// Eight inline slots. A linear scan comparing `TypeId`s — one `u128` compare
/// each — beats hashing one, and it never allocates for a realistic request.
type Slots = SmallVec<[(TypeId, Arc<dyn Any + Send + Sync>); 8]>;

/// Type-safe extensions container.
///
/// Stores typed values keyed by `TypeId`, found by scanning rather than
/// hashing, with no runtime type checking after insertion.
#[derive(Clone, Default)]
pub struct Extensions {
    slots: Slots,
}

impl Extensions {
    /// Create a new empty extensions container.
    #[inline]
    pub fn new() -> Self {
        Self {
            slots: Slots::new(),
        }
    }

    /// Create with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Slots::with_capacity(capacity),
        }
    }

    /// Whether the storage has spilled to the heap.
    #[inline]
    pub fn spilled(&self) -> bool {
        self.slots.spilled()
    }

    /// Insert a typed value into the extensions.
    ///
    /// If a value of this type already exists, it is replaced.
    ///
    /// # Example
    ///
    /// ```rust
    /// use armature_core::Extensions;
    ///
    /// let mut ext = Extensions::new();
    /// ext.insert(42i32);
    /// ext.insert("hello");
    /// ```
    #[inline]
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        self.insert_arc(Arc::new(value));
    }

    /// Insert an Arc-wrapped value directly.
    ///
    /// This is more efficient when you already have an Arc.
    #[inline]
    pub fn insert_arc<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        let type_id = TypeId::of::<T>();
        let erased: Arc<dyn Any + Send + Sync> = value;
        if let Some(slot) = self.slots.iter_mut().find(|(k, _)| *k == type_id) {
            slot.1 = erased;
            return;
        }
        self.slots.push((type_id, erased));
    }

    /// Get a reference to a typed value.
    ///
    /// Returns `None` if no value of this type exists.
    ///
    /// # Performance
    ///
    /// A scan over at most a handful of `TypeId`s followed by a pointer cast
    /// (no runtime type checking).
    ///
    /// # Example
    ///
    /// ```rust
    /// use armature_core::Extensions;
    ///
    /// let mut ext = Extensions::new();
    /// ext.insert(42i32);
    ///
    /// assert_eq!(ext.get::<i32>(), Some(&42));
    /// assert_eq!(ext.get::<String>(), None);
    /// ```
    #[inline]
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        let type_id = TypeId::of::<T>();
        self.slots
            .iter()
            .find(|(k, _)| *k == type_id)
            .and_then(|(_, v)| v.downcast_ref::<T>())
    }

    /// Get an Arc reference to a typed value.
    ///
    /// This is useful when you need to clone the Arc for async operations.
    #[inline]
    pub fn get_arc<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        self.slots
            .iter()
            .find(|(k, _)| *k == type_id)
            .and_then(|(_, v)| v.clone().downcast::<T>().ok())
    }

    /// Check if a value of this type exists.
    #[inline]
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.slots.iter().any(|(k, _)| *k == type_id)
    }

    /// Remove a typed value from the extensions.
    ///
    /// Returns true if the value existed and was removed.
    #[inline]
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        match self.slots.iter().position(|(k, _)| *k == type_id) {
            Some(i) => {
                self.slots.remove(i);
                true
            }
            None => false,
        }
    }

    /// Clear all extensions.
    #[inline]
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Get the number of extensions.
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Check if extensions is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Merge another extensions container into this one.
    ///
    /// Values from `other` will overwrite values in `self` for the same type.
    pub fn extend(&mut self, other: Extensions) {
        for (id, value) in other.slots {
            if let Some(slot) = self.slots.iter_mut().find(|(k, _)| *k == id) {
                slot.1 = value;
            } else {
                self.slots.push((id, value));
            }
        }
    }
}

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extensions")
            .field("count", &self.slots.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_get() {
        let mut ext = Extensions::new();

        ext.insert(42i32);
        ext.insert("hello".to_string());

        assert_eq!(ext.get::<i32>(), Some(&42));
        assert_eq!(ext.get::<String>(), Some(&"hello".to_string()));
        assert_eq!(ext.get::<f64>(), None);
    }

    #[test]
    fn test_insert_replaces() {
        let mut ext = Extensions::new();

        ext.insert(42i32);
        ext.insert(100i32);

        assert_eq!(ext.get::<i32>(), Some(&100));
    }

    #[test]
    fn test_contains() {
        let mut ext = Extensions::new();

        assert!(!ext.contains::<i32>());
        ext.insert(42i32);
        assert!(ext.contains::<i32>());
    }

    #[test]
    fn test_remove() {
        let mut ext = Extensions::new();
        ext.insert(42i32);

        let removed = ext.remove::<i32>();
        assert!(removed);
        assert!(!ext.contains::<i32>());
    }

    #[test]
    fn test_arc_insert() {
        let mut ext = Extensions::new();
        let arc = Arc::new(42i32);

        ext.insert_arc(arc.clone());

        let retrieved = ext.get_arc::<i32>().unwrap();
        assert_eq!(*retrieved, 42);
    }

    #[test]
    fn eight_extensions_stay_inline() {
        let mut ext = Extensions::new();
        ext.insert(1u8);
        ext.insert(2u16);
        ext.insert(3u32);
        ext.insert(4u64);
        ext.insert(5i8);
        ext.insert(6i16);
        ext.insert(7i32);
        ext.insert(8i64);
        assert_eq!(ext.len(), 8);
        assert!(!ext.spilled(), "eight extensions must not allocate a table");
        assert_eq!(ext.get::<u32>(), Some(&3u32));
    }

    #[test]
    fn insert_replaces_the_same_type() {
        let mut ext = Extensions::new();
        ext.insert(1u32);
        ext.insert(2u32);
        assert_eq!(ext.len(), 1);
        assert_eq!(ext.get::<u32>(), Some(&2u32));
    }

    #[test]
    fn extend_overwrites_colliding_types_and_keeps_the_rest() {
        let mut a = Extensions::new();
        a.insert(1u32);
        a.insert("keep");
        let mut b = Extensions::new();
        b.insert(2u32);
        a.extend(b);
        assert_eq!(a.get::<u32>(), Some(&2u32));
        assert_eq!(a.get::<&str>(), Some(&"keep"));
    }

    #[test]
    fn test_clone() {
        let mut ext = Extensions::new();
        ext.insert(42i32);

        let cloned = ext.clone();
        assert_eq!(cloned.get::<i32>(), Some(&42));
    }
}
