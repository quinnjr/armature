//! Event Store implementation

use crate::aggregate::{AggregateError, Snapshot};
use armature_events::DomainEvent;
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

/// Event store trait
///
/// Implement this trait to provide custom event storage (e.g., PostgreSQL, EventStoreDB).
/// No durable implementation ships with this crate; [`InMemoryEventStore`] is
/// provided for tests and local development only.
///
/// # Version invariant
///
/// Versions are positions in an aggregate's event log, not opaque tokens: version
/// `n` means "`n` events have been applied". This holds only because
/// [`crate::Aggregate::version`] advances by exactly one per applied event, and
/// both methods below depend on it.
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Save events for an aggregate
    ///
    /// `expected_version` is the number of events already stored for
    /// `aggregate_id` as of the caller's last read. Implementations must reject
    /// the write with [`EventStoreError::VersionConflict`] when the stored event
    /// count differs, which is what makes optimistic concurrency work.
    async fn save_events(
        &self,
        aggregate_id: &str,
        events: &[DomainEvent],
        expected_version: Option<u64>,
    ) -> Result<(), EventStoreError>;

    /// Load events for an aggregate
    ///
    /// `from_version` is a count of leading events to skip, so passing a
    /// snapshot's version yields exactly the events recorded after that snapshot.
    /// `None` replays the full log.
    async fn load_events(
        &self,
        aggregate_id: &str,
        from_version: Option<u64>,
    ) -> Result<Vec<DomainEvent>, EventStoreError>;

    /// Save snapshot
    async fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), EventStoreError>;

    /// Load snapshot
    async fn load_snapshot(&self, aggregate_id: &str) -> Result<Option<Snapshot>, EventStoreError>;
}

/// In-memory event store (for testing/development)
///
/// Backed by in-process `DashMap`s: everything it holds is lost when the process
/// exits. It is not a durable store and must not be used in production.
#[derive(Clone)]
pub struct InMemoryEventStore {
    /// Events indexed by aggregate ID
    events: Arc<DashMap<String, Vec<DomainEvent>>>,

    /// Snapshots indexed by aggregate ID
    snapshots: Arc<DashMap<String, Snapshot>>,
}

impl InMemoryEventStore {
    /// Create new in-memory event store
    pub fn new() -> Self {
        Self {
            events: Arc::new(DashMap::new()),
            snapshots: Arc::new(DashMap::new()),
        }
    }

    /// Get all events (for testing)
    pub fn all_events(&self) -> Vec<DomainEvent> {
        self.events
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// Clear all data
    pub fn clear(&self) {
        self.events.clear();
        self.snapshots.clear();
    }
}

impl Default for InMemoryEventStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn save_events(
        &self,
        aggregate_id: &str,
        events: &[DomainEvent],
        expected_version: Option<u64>,
    ) -> Result<(), EventStoreError> {
        let mut entry = self.events.entry(aggregate_id.to_string()).or_default();

        // Check version if specified
        if let Some(expected) = expected_version {
            let current_version = entry.len() as u64;
            if current_version != expected {
                return Err(EventStoreError::VersionConflict {
                    expected,
                    actual: current_version,
                });
            }
        }

        // Append events
        entry.extend_from_slice(events);
        Ok(())
    }

    async fn load_events(
        &self,
        aggregate_id: &str,
        from_version: Option<u64>,
    ) -> Result<Vec<DomainEvent>, EventStoreError> {
        match self.events.get(aggregate_id) {
            Some(events) => {
                let start = from_version.unwrap_or(0) as usize;
                Ok(events.iter().skip(start).cloned().collect())
            }
            None => Ok(Vec::new()),
        }
    }

    async fn save_snapshot(&self, snapshot: &Snapshot) -> Result<(), EventStoreError> {
        self.snapshots
            .insert(snapshot.aggregate_id.clone(), snapshot.clone());
        Ok(())
    }

    async fn load_snapshot(&self, aggregate_id: &str) -> Result<Option<Snapshot>, EventStoreError> {
        Ok(self.snapshots.get(aggregate_id).map(|s| s.clone()))
    }
}

/// Event store error
#[derive(Debug, thiserror::Error)]
pub enum EventStoreError {
    #[error("Version conflict: expected {expected}, got {actual}")]
    VersionConflict { expected: u64, actual: u64 },

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Aggregate not found: {0}")]
    NotFound(String),
}

impl From<EventStoreError> for AggregateError {
    fn from(err: EventStoreError) -> Self {
        match err {
            EventStoreError::VersionConflict { expected, actual } => {
                AggregateError::VersionConflict { expected, actual }
            }
            EventStoreError::NotFound(id) => AggregateError::NotFound(id),
            EventStoreError::SerializationError(msg) => AggregateError::SerializationError(msg),
            EventStoreError::StorageError(msg) => AggregateError::EventApplicationFailed(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_store() {
        let store = InMemoryEventStore::new();

        let event = DomainEvent::new(
            "test_event",
            "agg-1",
            "TestAggregate",
            serde_json::json!({"data": "value"}),
        );

        // Save event
        store
            .save_events("agg-1", std::slice::from_ref(&event), None)
            .await
            .unwrap();

        // Load events
        let events = store.load_events("agg-1", None).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].aggregate_id, "agg-1");
    }

    #[tokio::test]
    async fn test_version_conflict() {
        let store = InMemoryEventStore::new();

        let event = DomainEvent::new(
            "test_event",
            "agg-1",
            "TestAggregate",
            serde_json::json!({}),
        );

        // Save first event
        store
            .save_events("agg-1", std::slice::from_ref(&event), Some(0))
            .await
            .unwrap();

        // Try to save with wrong version
        let result = store.save_events("agg-1", &[event], Some(0)).await;
        assert!(matches!(
            result,
            Err(EventStoreError::VersionConflict { .. })
        ));
    }

    #[tokio::test]
    async fn test_snapshot() {
        let store = InMemoryEventStore::new();

        let snapshot = Snapshot::new(
            "agg-1".to_string(),
            "TestAggregate".to_string(),
            10,
            serde_json::json!({"count": 42}),
        );

        // Save snapshot
        store.save_snapshot(&snapshot).await.unwrap();

        // Load snapshot
        let loaded = store.load_snapshot("agg-1").await.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().version, 10);
    }
}
