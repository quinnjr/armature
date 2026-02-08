//! Aggregate repository

use crate::aggregate::{Aggregate, AggregateError};
use crate::store::EventStore;
use std::marker::PhantomData;
use std::sync::Arc;

/// Aggregate repository
///
/// Provides load/save operations for aggregates with event sourcing.
pub struct AggregateRepository<A, S>
where
    A: Aggregate,
    S: EventStore,
{
    store: Arc<S>,
    snapshot_frequency: Option<u64>,
    _phantom: PhantomData<A>,
}

impl<A, S> AggregateRepository<A, S>
where
    A: Aggregate,
    S: EventStore,
{
    /// Create new repository
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            snapshot_frequency: None,
            _phantom: PhantomData,
        }
    }

    /// Create repository with snapshotting
    pub fn with_snapshots(store: Arc<S>, frequency: u64) -> Self {
        Self {
            store,
            snapshot_frequency: Some(frequency),
            _phantom: PhantomData,
        }
    }

    /// Load aggregate by ID
    pub async fn load(&self, aggregate_id: &str) -> Result<A, AggregateError> {
        // Create new aggregate instance
        let mut aggregate = A::new_instance(aggregate_id.to_string());

        // Load and apply all events (snapshot loading would require complex serde traits)
        let events = self.store.load_events(aggregate_id, None).await?;

        for event in events {
            aggregate.apply_event(&event)?;
        }

        Ok(aggregate)
    }

    /// Save aggregate
    pub async fn save(&self, aggregate: &mut A) -> Result<(), AggregateError> {
        let events = aggregate.uncommitted_events();

        if events.is_empty() {
            return Ok(());
        }

        // Save events with optimistic concurrency
        self.store
            .save_events(aggregate.aggregate_id(), events, Some(aggregate.version()))
            .await?;

        // Mark events as committed
        aggregate.mark_events_committed();

        // Check if we should create a snapshot
        if let Some(frequency) = self.snapshot_frequency
            && aggregate.version().is_multiple_of(frequency)
        {
            self.create_snapshot(aggregate).await?;
        }

        Ok(())
    }

    /// Create snapshot for aggregate
    ///
    /// Note: To enable snapshotting, implement custom snapshot logic in your aggregate.
    async fn create_snapshot(&self, _aggregate: &A) -> Result<(), AggregateError> {
        // Snapshot creation requires custom serialization logic per aggregate type.
        // Users should implement their own snapshot creation if needed.
        Ok(())
    }
}

impl<A, S> Clone for AggregateRepository<A, S>
where
    A: Aggregate,
    S: EventStore,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            snapshot_frequency: self.snapshot_frequency,
            _phantom: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::AggregateRoot;
    use crate::store::InMemoryEventStore;
    use armature_events::DomainEvent;
    use async_trait::async_trait;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestState {
        count: u32,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestAggregate {
        #[serde(flatten)]
        root: AggregateRoot<TestState>,
    }

    #[async_trait]
    impl Aggregate for TestAggregate {
        fn aggregate_id(&self) -> &str {
            &self.root.id
        }

        fn aggregate_type() -> &'static str {
            "TestAggregate"
        }

        fn version(&self) -> u64 {
            self.root.version
        }

        fn apply_event(&mut self, _event: &DomainEvent) -> Result<(), AggregateError> {
            Ok(())
        }

        fn uncommitted_events(&self) -> &[DomainEvent] {
            self.root.uncommitted_events()
        }

        fn mark_events_committed(&mut self) {
            self.root.clear_uncommitted_events();
        }

        fn new_instance(id: String) -> Self {
            Self {
                root: AggregateRoot::new(id, TestState { count: 0 }),
            }
        }
    }

    #[tokio::test]
    async fn test_repository_load_save() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<TestAggregate, _>::new(store);

        let mut aggregate = TestAggregate::new_instance("test-1".to_string());
        aggregate.root.add_event(DomainEvent::new(
            "test_event",
            "test-1",
            "TestAggregate",
            serde_json::json!({}),
        ));

        repo.save(&mut aggregate).await.unwrap();
        assert_eq!(aggregate.uncommitted_events().len(), 0);
    }
}
