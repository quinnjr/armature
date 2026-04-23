//! Core CRDT (Conflict-free Replicated Data Type) implementations
//!
//! Provides fundamental CRDT types for building collaborative applications.

use crate::{LogicalClock, ReplicaId};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Trait for CRDTs that can be merged
pub trait Crdt: Clone {
    /// Merge with another CRDT state
    fn merge(&mut self, other: &Self);
}

/// Last-Writer-Wins Register
///
/// A register where concurrent writes are resolved by timestamp.
/// The write with the highest timestamp wins.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{LwwRegister, ReplicaId, LogicalClock};
///
/// let replica = ReplicaId::new();
/// let mut reg = LwwRegister::new("initial", LogicalClock::new(1, replica));
///
/// // Update with higher timestamp wins
/// reg.set("updated", LogicalClock::new(2, replica));
/// assert_eq!(reg.get(), &"updated");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwwRegister<T> {
    value: T,
    timestamp: LogicalClock,
}

impl<T: Clone> LwwRegister<T> {
    /// Create a new LWW register
    pub fn new(value: T, timestamp: LogicalClock) -> Self {
        Self { value, timestamp }
    }

    /// Get the current value
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> LogicalClock {
        self.timestamp
    }

    /// Set a new value if the timestamp is higher
    pub fn set(&mut self, value: T, timestamp: LogicalClock) -> bool {
        if timestamp > self.timestamp {
            self.value = value;
            self.timestamp = timestamp;
            true
        } else {
            false
        }
    }
}

impl<T: Clone> Crdt for LwwRegister<T> {
    fn merge(&mut self, other: &Self) {
        if other.timestamp > self.timestamp {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
        }
    }
}

/// Grow-only Counter
///
/// A counter that can only be incremented. Each replica maintains
/// its own count, and the total is the sum of all replica counts.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{GCounter, ReplicaId};
///
/// let replica = ReplicaId::new();
/// let mut counter = GCounter::new();
///
/// counter.increment(replica);
/// counter.increment(replica);
/// assert_eq!(counter.value(), 2);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GCounter {
    counts: HashMap<ReplicaId, u64>,
}

impl GCounter {
    /// Create a new grow-only counter
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    /// Increment the counter for a replica
    pub fn increment(&mut self, replica: ReplicaId) {
        *self.counts.entry(replica).or_insert(0) += 1;
    }

    /// Increment by a specific amount
    pub fn increment_by(&mut self, replica: ReplicaId, amount: u64) {
        *self.counts.entry(replica).or_insert(0) += amount;
    }

    /// Get the total count
    pub fn value(&self) -> u64 {
        self.counts.values().sum()
    }

    /// Get the count for a specific replica
    pub fn get_replica_count(&self, replica: &ReplicaId) -> u64 {
        *self.counts.get(replica).unwrap_or(&0)
    }
}

impl Crdt for GCounter {
    fn merge(&mut self, other: &Self) {
        for (replica, count) in &other.counts {
            let entry = self.counts.entry(*replica).or_insert(0);
            *entry = (*entry).max(*count);
        }
    }
}

/// Positive-Negative Counter
///
/// A counter that can be both incremented and decremented.
/// Implemented as a pair of G-Counters.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{PnCounter, ReplicaId};
///
/// let replica = ReplicaId::new();
/// let mut counter = PnCounter::new();
///
/// counter.increment(replica);
/// counter.increment(replica);
/// counter.decrement(replica);
/// assert_eq!(counter.value(), 1);
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PnCounter {
    positive: GCounter,
    negative: GCounter,
}

impl PnCounter {
    /// Create a new PN counter
    pub fn new() -> Self {
        Self {
            positive: GCounter::new(),
            negative: GCounter::new(),
        }
    }

    /// Increment the counter
    pub fn increment(&mut self, replica: ReplicaId) {
        self.positive.increment(replica);
    }

    /// Decrement the counter
    pub fn decrement(&mut self, replica: ReplicaId) {
        self.negative.increment(replica);
    }

    /// Get the current value
    pub fn value(&self) -> i64 {
        self.positive.value() as i64 - self.negative.value() as i64
    }
}

impl Crdt for PnCounter {
    fn merge(&mut self, other: &Self) {
        self.positive.merge(&other.positive);
        self.negative.merge(&other.negative);
    }
}

/// Grow-only Set
///
/// A set that only supports adding elements.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::GSet;
///
/// let mut set = GSet::new();
/// set.add("apple");
/// set.add("banana");
/// assert!(set.contains(&"apple"));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GSet<T: Eq + Hash + Clone> {
    elements: HashSet<T>,
}

impl<T: Eq + Hash + Clone> GSet<T> {
    /// Create a new grow-only set
    pub fn new() -> Self {
        Self {
            elements: HashSet::new(),
        }
    }

    /// Add an element to the set
    pub fn add(&mut self, element: T) {
        self.elements.insert(element);
    }

    /// Check if the set contains an element
    pub fn contains(&self, element: &T) -> bool {
        self.elements.contains(element)
    }

    /// Get the number of elements
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Iterate over elements
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.elements.iter()
    }
}

impl<T: Eq + Hash + Clone> Crdt for GSet<T> {
    fn merge(&mut self, other: &Self) {
        self.elements.extend(other.elements.iter().cloned());
    }
}

/// Observed-Remove Set (OR-Set)
///
/// A set that supports both adding and removing elements.
/// Each element is tagged with unique identifiers to handle concurrent
/// add and remove operations.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{OrSet, ReplicaId, LogicalClock};
///
/// let replica = ReplicaId::new();
/// let mut clock = LogicalClock::new(0, replica);
///
/// let mut set = OrSet::new();
/// set.add("apple", clock.tick());
/// set.add("banana", clock.tick());
/// set.remove("apple");
/// assert!(!set.contains(&"apple"));
/// assert!(set.contains(&"banana"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrSet<T: Eq + Hash + Clone> {
    /// Elements with their tags (element -> set of tags)
    elements: HashMap<T, HashSet<LogicalClock>>,
    /// Tombstones (removed tags)
    tombstones: HashSet<LogicalClock>,
}

impl<T: Eq + Hash + Clone> OrSet<T> {
    /// Create a new OR-Set
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
            tombstones: HashSet::new(),
        }
    }

    /// Add an element with a tag
    pub fn add(&mut self, element: T, tag: LogicalClock) {
        if !self.tombstones.contains(&tag) {
            self.elements.entry(element).or_default().insert(tag);
        }
    }

    /// Remove an element (marks all its tags as tombstones)
    pub fn remove(&mut self, element: &T) {
        if let Some(tags) = self.elements.remove(element) {
            self.tombstones.extend(tags);
        }
    }

    /// Check if an element is in the set
    pub fn contains(&self, element: &T) -> bool {
        self.elements
            .get(element)
            .map(|tags| !tags.is_empty())
            .unwrap_or(false)
    }

    /// Get all elements
    pub fn elements(&self) -> impl Iterator<Item = &T> {
        self.elements
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(e, _)| e)
    }

    /// Get the number of elements
    pub fn len(&self) -> usize {
        self.elements
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .count()
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Eq + Hash + Clone> Default for OrSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Eq + Hash + Clone> Crdt for OrSet<T> {
    fn merge(&mut self, other: &Self) {
        // Merge tombstones first
        self.tombstones.extend(other.tombstones.iter().cloned());

        // Merge elements, filtering out tombstoned tags
        for (element, tags) in &other.elements {
            let entry = self.elements.entry(element.clone()).or_default();
            for tag in tags {
                if !self.tombstones.contains(tag) {
                    entry.insert(*tag);
                }
            }
        }

        // Remove tombstoned tags from existing elements
        for tags in self.elements.values_mut() {
            tags.retain(|tag| !self.tombstones.contains(tag));
        }
    }
}

/// Last-Writer-Wins Map
///
/// A map where each key has an associated LWW register.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{LwwMap, ReplicaId, LogicalClock};
///
/// let replica = ReplicaId::new();
/// let mut clock = LogicalClock::new(0, replica);
///
/// let mut map = LwwMap::new();
/// map.set("name", "Alice", clock.tick());
/// map.set("age", "30", clock.tick());
/// assert_eq!(map.get(&"name"), Some(&"Alice"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LwwMap<K: Eq + Hash + Clone, V: Clone> {
    entries: HashMap<K, LwwRegister<Option<V>>>,
}

impl<K: Eq + Hash + Clone, V: Clone> LwwMap<K, V> {
    /// Create a new LWW map
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Set a value for a key
    pub fn set(&mut self, key: K, value: V, timestamp: LogicalClock) {
        match self.entries.get_mut(&key) {
            Some(register) => {
                register.set(Some(value), timestamp);
            }
            None => {
                self.entries
                    .insert(key, LwwRegister::new(Some(value), timestamp));
            }
        }
    }

    /// Remove a key
    pub fn remove(&mut self, key: &K, timestamp: LogicalClock) {
        if let Some(register) = self.entries.get_mut(key) {
            register.set(None, timestamp);
        }
    }

    /// Get a value
    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key).and_then(|r| r.get().as_ref())
    }

    /// Check if a key exists
    pub fn contains_key(&self, key: &K) -> bool {
        self.entries
            .get(key)
            .map(|r| r.get().is_some())
            .unwrap_or(false)
    }

    /// Get all keys
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries
            .iter()
            .filter(|(_, r)| r.get().is_some())
            .map(|(k, _)| k)
    }

    /// Get all key-value pairs
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries
            .iter()
            .filter_map(|(k, r)| r.get().as_ref().map(|v| (k, v)))
    }

    /// Get the number of entries
    pub fn len(&self) -> usize {
        self.entries
            .iter()
            .filter(|(_, r)| r.get().is_some())
            .count()
    }

    /// Check if the map is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Default for LwwMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Eq + Hash + Clone, V: Clone> Crdt for LwwMap<K, V> {
    fn merge(&mut self, other: &Self) {
        for (key, register) in &other.entries {
            match self.entries.get_mut(key) {
                Some(existing) => {
                    existing.merge(register);
                }
                None => {
                    self.entries.insert(key.clone(), register.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lww_register() {
        let replica = ReplicaId::new();
        let mut reg = LwwRegister::new("a", LogicalClock::new(1, replica));

        assert_eq!(reg.get(), &"a");

        // Higher timestamp wins
        reg.set("b", LogicalClock::new(2, replica));
        assert_eq!(reg.get(), &"b");

        // Lower timestamp loses
        reg.set("c", LogicalClock::new(1, replica));
        assert_eq!(reg.get(), &"b");
    }

    #[test]
    fn test_gcounter() {
        let replica1 = ReplicaId::new();
        let replica2 = ReplicaId::new();

        let mut counter1 = GCounter::new();
        counter1.increment(replica1);
        counter1.increment(replica1);

        let mut counter2 = GCounter::new();
        counter2.increment(replica2);

        counter1.merge(&counter2);
        assert_eq!(counter1.value(), 3);
    }

    #[test]
    fn test_pncounter() {
        let replica = ReplicaId::new();
        let mut counter = PnCounter::new();

        counter.increment(replica);
        counter.increment(replica);
        counter.decrement(replica);

        assert_eq!(counter.value(), 1);
    }

    #[test]
    fn test_gset() {
        let mut set1 = GSet::new();
        set1.add("a");
        set1.add("b");

        let mut set2 = GSet::new();
        set2.add("b");
        set2.add("c");

        set1.merge(&set2);
        assert!(set1.contains(&"a"));
        assert!(set1.contains(&"b"));
        assert!(set1.contains(&"c"));
    }

    #[test]
    fn test_orset() {
        let replica = ReplicaId::new();
        let mut clock = LogicalClock::new(0, replica);

        let mut set = OrSet::new();
        set.add("a", clock.tick());
        set.add("b", clock.tick());

        assert!(set.contains(&"a"));
        assert!(set.contains(&"b"));

        set.remove(&"a");
        assert!(!set.contains(&"a"));
        assert!(set.contains(&"b"));
    }

    #[test]
    fn test_lwwmap() {
        let replica = ReplicaId::new();
        let mut clock = LogicalClock::new(0, replica);

        let mut map = LwwMap::new();
        map.set("name", "Alice", clock.tick());
        map.set("age", "30", clock.tick());

        assert_eq!(map.get(&"name"), Some(&"Alice"));
        assert_eq!(map.get(&"age"), Some(&"30"));

        map.remove(&"age", clock.tick());
        assert_eq!(map.get(&"age"), None);
    }
}
