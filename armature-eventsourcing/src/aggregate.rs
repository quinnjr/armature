//! Aggregate root for event sourcing

use armature_events::DomainEvent;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Aggregate trait
///
/// Aggregates are the core building blocks of event sourcing.
/// They apply events to change state and generate new events for commands.
#[async_trait]
pub trait Aggregate: Send + Sync + Debug + Clone {
    /// Get aggregate ID
    fn aggregate_id(&self) -> &str;

    /// Get aggregate type name
    fn aggregate_type() -> &'static str
    where
        Self: Sized;

    /// Get current version
    ///
    /// # Invariant
    ///
    /// The version MUST equal the number of events applied to this aggregate: a
    /// fresh instance starts at 0 and every successful [`Aggregate::apply_event`]
    /// advances it by exactly one, including events replayed from the store.
    ///
    /// This is not a stylistic preference. [`crate::EventStore`] treats versions
    /// as positional indices into the aggregate's event log — `load_events`
    /// interprets `from_version` as a count of events to skip, and `save_events`
    /// compares `expected_version` against the number of events already stored.
    /// An aggregate that advances its version by anything other than one per
    /// event will silently replay from the wrong offset and produce bogus
    /// optimistic-concurrency conflicts.
    fn version(&self) -> u64;

    /// Apply an event to the aggregate
    ///
    /// Implementations must advance [`Aggregate::version`] by exactly one on
    /// success (see the invariant documented there).
    fn apply_event(&mut self, event: &DomainEvent) -> Result<(), AggregateError>;

    /// Get uncommitted events
    fn uncommitted_events(&self) -> &[DomainEvent];

    /// Mark all events as committed
    fn mark_events_committed(&mut self);

    /// Create a new instance (default state)
    fn new_instance(id: String) -> Self
    where
        Self: Sized;
}

/// Base aggregate implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateRoot<T: Clone + Debug> {
    /// Aggregate ID
    pub id: String,

    /// Current version
    pub version: u64,

    /// Aggregate state
    pub state: T,

    /// Uncommitted events
    #[serde(skip)]
    uncommitted_events: Vec<DomainEvent>,
}

impl<T: Clone + Debug> AggregateRoot<T> {
    /// Create new aggregate
    pub fn new(id: String, state: T) -> Self {
        Self {
            id,
            version: 0,
            state,
            uncommitted_events: Vec::new(),
        }
    }

    /// Add uncommitted event
    pub fn add_event(&mut self, event: DomainEvent) {
        self.uncommitted_events.push(event);
    }

    /// Get state reference
    pub fn state(&self) -> &T {
        &self.state
    }

    /// Get uncommitted events
    pub fn uncommitted_events(&self) -> &[DomainEvent] {
        &self.uncommitted_events
    }

    /// Clear uncommitted events
    pub fn clear_uncommitted_events(&mut self) {
        self.uncommitted_events.clear();
    }

    /// Get mutable state reference
    pub fn state_mut(&mut self) -> &mut T {
        &mut self.state
    }

    /// Increment version
    pub fn increment_version(&mut self) {
        self.version += 1;
    }
}

/// Aggregate error
#[derive(Debug, thiserror::Error)]
pub enum AggregateError {
    #[error("Event application failed: {0}")]
    EventApplicationFailed(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Aggregate not found: {0}")]
    NotFound(String),

    #[error("Version conflict: expected {expected}, got {actual}")]
    VersionConflict { expected: u64, actual: u64 },

    #[error("Serialization error: {0}")]
    SerializationError(String),
}

/// Aggregate snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Aggregate ID
    pub aggregate_id: String,

    /// Aggregate type
    pub aggregate_type: String,

    /// Version at snapshot
    pub version: u64,

    /// Snapshot timestamp
    pub timestamp: DateTime<Utc>,

    /// Serialized state
    pub state: serde_json::Value,
}

impl Snapshot {
    /// Create new snapshot
    pub fn new(
        aggregate_id: String,
        aggregate_type: String,
        version: u64,
        state: serde_json::Value,
    ) -> Self {
        Self {
            aggregate_id,
            aggregate_type,
            version,
            timestamp: Utc::now(),
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestState {
        count: u32,
    }

    #[test]
    fn test_aggregate_root() {
        let state = TestState { count: 0 };
        let mut aggregate = AggregateRoot::new("test-1".to_string(), state);

        assert_eq!(aggregate.id, "test-1");
        assert_eq!(aggregate.version, 0);
        assert_eq!(aggregate.state().count, 0);

        aggregate.state_mut().count += 1;
        assert_eq!(aggregate.state().count, 1);

        aggregate.increment_version();
        assert_eq!(aggregate.version, 1);
    }

    #[test]
    fn test_snapshot() {
        let state = serde_json::json!({"count": 42});
        let snapshot = Snapshot::new(
            "test-1".to_string(),
            "TestAggregate".to_string(),
            10,
            state.clone(),
        );

        assert_eq!(snapshot.aggregate_id, "test-1");
        assert_eq!(snapshot.version, 10);
        assert_eq!(snapshot.state, state);
    }
}
