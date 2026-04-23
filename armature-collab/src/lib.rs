//! Real-time Collaboration Module for Armature Framework
//!
//! Provides CRDTs (Conflict-free Replicated Data Types) and collaboration
//! primitives for building real-time collaborative applications.
//!
//! ## Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Collaboration Architecture                    │
//! │                                                                  │
//! │  ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐  │
//! │  │ Client A │───▶│  CRDT    │───▶│  Sync    │◀───│ Client B │  │
//! │  └──────────┘    │  State   │    │  Engine  │    └──────────┘  │
//! │                  └──────────┘    └──────────┘                   │
//! │                       │               │                         │
//! │                       ▼               ▼                         │
//! │                  ┌──────────┐    ┌──────────┐                   │
//! │                  │ Document │    │ Presence │                   │
//! │                  │  State   │    │  State   │                   │
//! │                  └──────────┘    └──────────┘                   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_collab::{Document, TextCrdt, CollabSession};
//!
//! // Create a collaborative document
//! let doc = Document::new("doc-123");
//!
//! // Add a text field with CRDT
//! let text = doc.add_text("content");
//!
//! // Make edits (automatically synced)
//! text.insert(0, "Hello, ");
//! text.insert(7, "World!");
//!
//! // Subscribe to changes
//! doc.on_change(|change| {
//!     println!("Document updated: {:?}", change);
//! });
//! ```
//!
//! ## CRDT Types
//!
//! | Type | Use Case | Merge Strategy |
//! |------|----------|----------------|
//! | `LwwRegister` | Single values | Last-Writer-Wins |
//! | `GCounter` | Increment-only counters | Max per replica |
//! | `PnCounter` | Inc/dec counters | G-Counter pair |
//! | `GSet` | Append-only sets | Union |
//! | `OrSet` | Add/remove sets | Observed-Remove |
//! | `LwwMap` | Key-value stores | LWW per key |
//! | `RgaText` | Collaborative text | RGA algorithm |
//!
//! ## Features
//!
//! - **`text`** - Text CRDT with RGA algorithm (default)
//! - **`websocket`** - WebSocket sync integration
//! - **`full`** - All features

pub mod crdt;
pub mod document;
pub mod error;
pub mod presence;
pub mod session;
pub mod sync;

#[cfg(feature = "text")]
pub mod text;

pub use crdt::*;
pub use document::*;
pub use error::*;
pub use presence::*;
pub use session::*;
pub use sync::*;

#[cfg(feature = "text")]
pub use text::*;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a replica (client/node)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReplicaId(pub Uuid);

impl ReplicaId {
    /// Create a new random replica ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create from a UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for ReplicaId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Logical timestamp for ordering operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogicalClock {
    /// Counter value
    pub counter: u64,
    /// Replica that created this timestamp
    pub replica: ReplicaId,
}

impl LogicalClock {
    /// Create a new logical clock
    pub fn new(counter: u64, replica: ReplicaId) -> Self {
        Self { counter, replica }
    }

    /// Increment the clock
    pub fn tick(&mut self) -> Self {
        self.counter += 1;
        *self
    }

    /// Merge with another clock (take max)
    pub fn merge(&mut self, other: &Self) {
        self.counter = self.counter.max(other.counter);
    }
}

/// Vector clock for tracking causality
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorClock {
    clocks: std::collections::HashMap<ReplicaId, u64>,
}

impl VectorClock {
    /// Create a new empty vector clock
    pub fn new() -> Self {
        Self {
            clocks: std::collections::HashMap::new(),
        }
    }

    /// Increment the clock for a replica
    pub fn increment(&mut self, replica: ReplicaId) -> u64 {
        let counter = self.clocks.entry(replica).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Get the counter for a replica
    pub fn get(&self, replica: &ReplicaId) -> u64 {
        *self.clocks.get(replica).unwrap_or(&0)
    }

    /// Merge with another vector clock
    pub fn merge(&mut self, other: &Self) {
        for (replica, counter) in &other.clocks {
            let entry = self.clocks.entry(*replica).or_insert(0);
            *entry = (*entry).max(*counter);
        }
    }

    /// Check if this clock is concurrent with another
    pub fn is_concurrent(&self, other: &Self) -> bool {
        !self.happens_before(other) && !other.happens_before(self)
    }

    /// Check if this clock happens before another
    pub fn happens_before(&self, other: &Self) -> bool {
        let mut dominated = false;
        for (replica, &counter) in &self.clocks {
            let other_counter = other.get(replica);
            if counter > other_counter {
                return false;
            }
            if counter < other_counter {
                dominated = true;
            }
        }
        // Check for any replicas in other but not in self
        for replica in other.clocks.keys() {
            if !self.clocks.contains_key(replica) && other.get(replica) > 0 {
                dominated = true;
            }
        }
        dominated
    }
}

impl Default for VectorClock {
    fn default() -> Self {
        Self::new()
    }
}

/// An operation that can be applied to a CRDT
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation<T> {
    /// Unique operation ID
    pub id: Uuid,
    /// Replica that created the operation
    pub replica: ReplicaId,
    /// Logical timestamp
    pub timestamp: LogicalClock,
    /// The actual operation data
    pub data: T,
    /// Dependencies (operations this depends on)
    pub deps: Vec<Uuid>,
}

impl<T> Operation<T> {
    /// Create a new operation
    pub fn new(replica: ReplicaId, timestamp: LogicalClock, data: T) -> Self {
        Self {
            id: Uuid::new_v4(),
            replica,
            timestamp,
            data,
            deps: Vec::new(),
        }
    }

    /// Add a dependency
    pub fn with_dep(mut self, dep: Uuid) -> Self {
        self.deps.push(dep);
        self
    }

    /// Add multiple dependencies
    pub fn with_deps(mut self, deps: impl IntoIterator<Item = Uuid>) -> Self {
        self.deps.extend(deps);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replica_id() {
        let id1 = ReplicaId::new();
        let id2 = ReplicaId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_logical_clock() {
        let replica = ReplicaId::new();
        let mut clock = LogicalClock::new(0, replica);

        assert_eq!(clock.counter, 0);
        clock.tick();
        assert_eq!(clock.counter, 1);
    }

    #[test]
    fn test_vector_clock() {
        let replica1 = ReplicaId::new();
        let replica2 = ReplicaId::new();

        let mut vc1 = VectorClock::new();
        vc1.increment(replica1);
        vc1.increment(replica1);

        let mut vc2 = VectorClock::new();
        vc2.increment(replica2);

        assert!(vc1.is_concurrent(&vc2));

        vc1.merge(&vc2);
        assert_eq!(vc1.get(&replica1), 2);
        assert_eq!(vc1.get(&replica2), 1);
    }

    #[test]
    fn test_happens_before() {
        let replica = ReplicaId::new();

        let mut vc1 = VectorClock::new();
        vc1.increment(replica);

        let mut vc2 = vc1.clone();
        vc2.increment(replica);

        assert!(vc1.happens_before(&vc2));
        assert!(!vc2.happens_before(&vc1));
    }
}
