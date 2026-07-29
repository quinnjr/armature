//! Aggregate repository

use crate::aggregate::{Aggregate, AggregateError, Snapshot};
use crate::store::EventStore;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::debug;

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

        // The store's optimistic-concurrency check compares `expected_version`
        // against the number of events already persisted. `aggregate.version()`
        // reflects the version *after* applying the staged (uncommitted) events,
        // so the expected version must be the base version prior to those
        // events, not the post-apply version.
        let base_version = aggregate.version().saturating_sub(events.len() as u64);

        // Save events with optimistic concurrency
        self.store
            .save_events(aggregate.aggregate_id(), events, Some(base_version))
            .await?;

        // Mark events as committed
        aggregate.mark_events_committed();

        // Check if we should create a snapshot
        if let Some(frequency) = self.snapshot_frequency
            && aggregate.version().is_multiple_of(frequency)
        {
            // A snapshot store + frequency are configured, so the caller clearly
            // expects snapshots to be persisted here. `create_snapshot` on this
            // generic (non-serde-bounded) path is a deliberate no-op (see its
            // doc comment), so make the footgun visible instead of silently
            // doing nothing: `save_with_snapshot` is required for real
            // snapshot persistence.
            debug!(
                aggregate_id = aggregate.aggregate_id(),
                version = aggregate.version(),
                "snapshot_frequency configured but base save() does not persist \
                 snapshots; use AggregateRepository::save_with_snapshot instead"
            );
            self.create_snapshot(aggregate).await?;
        }

        Ok(())
    }

    /// Create snapshot for aggregate
    ///
    /// Note: the generic path cannot serialize an arbitrary `A` (the [`Aggregate`]
    /// trait has no serde bound), so this remains a no-op. Aggregates that also
    /// implement [`serde::Serialize`] + [`serde::de::DeserializeOwned`] get real
    /// snapshot persistence via [`AggregateRepository::save_snapshot`] and
    /// [`AggregateRepository::save_with_snapshot`] in the serde-bounded impl below.
    async fn create_snapshot(&self, _aggregate: &A) -> Result<(), AggregateError> {
        // Snapshot creation requires custom serialization logic per aggregate type.
        // Users should implement their own snapshot creation if needed.
        Ok(())
    }
}

/// Snapshot-aware operations for aggregates that are (de)serializable.
///
/// These are fully additive: they layer snapshot support on top of the base
/// [`AggregateRepository`] without changing the behavior of [`load`] or [`save`].
/// The extra `Serialize + DeserializeOwned` bounds are applied only to this impl
/// block, so aggregates that do not implement serde still compile and work with
/// the base API unchanged.
///
/// [`load`]: AggregateRepository::load
/// [`save`]: AggregateRepository::save
impl<A, S> AggregateRepository<A, S>
where
    A: Aggregate + Serialize + DeserializeOwned,
    S: EventStore,
{
    /// Snapshot-aware load.
    ///
    /// Loads the latest snapshot (if any), rehydrates the aggregate from it, then
    /// applies only the events recorded *after* the snapshot version via
    /// `load_events(id, Some(snapshot.version))`. When no snapshot exists this
    /// falls back to a full replay identical to [`AggregateRepository::load`].
    pub async fn load_snapshotted(&self, aggregate_id: &str) -> Result<A, AggregateError> {
        match self.store.load_snapshot(aggregate_id).await? {
            Some(snapshot) => {
                // Seed the aggregate from the persisted snapshot state.
                let mut aggregate: A = serde_json::from_value(snapshot.state.clone())
                    .map_err(|e| AggregateError::SerializationError(e.to_string()))?;

                // Apply only events newer than the snapshot version.
                let events = self
                    .store
                    .load_events(aggregate_id, Some(snapshot.version))
                    .await?;

                for event in events {
                    aggregate.apply_event(&event)?;
                }

                Ok(aggregate)
            }
            None => self.load(aggregate_id).await,
        }
    }

    /// Persist a snapshot of the aggregate's current state.
    ///
    /// Serializes the whole aggregate to JSON and stores it via
    /// [`EventStore::save_snapshot`] tagged with the aggregate's current version.
    pub async fn save_snapshot(&self, aggregate: &A) -> Result<(), AggregateError> {
        let state = serde_json::to_value(aggregate)
            .map_err(|e| AggregateError::SerializationError(e.to_string()))?;

        let snapshot = Snapshot::new(
            aggregate.aggregate_id().to_string(),
            A::aggregate_type().to_string(),
            aggregate.version(),
            state,
        );

        self.store.save_snapshot(&snapshot).await?;
        Ok(())
    }

    /// Save like [`AggregateRepository::save`], additionally persisting a real
    /// snapshot every `snapshot_frequency` events (when configured via
    /// [`AggregateRepository::with_snapshots`]).
    ///
    /// This is the serde-enabled counterpart to the base `save`, whose generic
    /// `create_snapshot` is necessarily a no-op.
    pub async fn save_with_snapshot(&self, aggregate: &mut A) -> Result<(), AggregateError> {
        let events = aggregate.uncommitted_events();

        if events.is_empty() {
            return Ok(());
        }

        // See `save` above: the store compares `expected_version` against the
        // number of already-persisted events, so we must pass the base version
        // (before the staged events were applied), not `aggregate.version()`.
        let base_version = aggregate.version().saturating_sub(events.len() as u64);

        self.store
            .save_events(aggregate.aggregate_id(), events, Some(base_version))
            .await?;

        aggregate.mark_events_committed();

        if let Some(frequency) = self.snapshot_frequency
            && frequency > 0
            && aggregate.version().is_multiple_of(frequency)
        {
            self.save_snapshot(aggregate).await?;
        }

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
    async fn save_after_applying_events_succeeds() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        // Load a fresh (non-existent) aggregate: version 0, no stored events.
        let mut aggregate = repo.load("agg-apply").await.unwrap();
        assert_eq!(aggregate.version(), 0);

        // Apply one event (bumps version to 1) and stage it as uncommitted.
        let event = increment_event("agg-apply", 5);
        aggregate.apply_event(&event).unwrap();
        aggregate.root.add_event(event);
        assert_eq!(aggregate.version(), 1);

        // Saving must succeed: the store has 0 stored events, matching the base
        // version (version() - uncommitted count = 1 - 1 = 0), not version() itself.
        repo.save(&mut aggregate)
            .await
            .expect("save should not raise a false VersionConflict");
    }

    #[tokio::test]
    async fn save_with_snapshot_after_applying_events_succeeds() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        let mut aggregate = repo.load("agg-apply-snap").await.unwrap();
        assert_eq!(aggregate.version(), 0);

        let event = increment_event("agg-apply-snap", 5);
        aggregate.apply_event(&event).unwrap();
        aggregate.root.add_event(event);
        assert_eq!(aggregate.version(), 1);

        repo.save_with_snapshot(&mut aggregate)
            .await
            .expect("save_with_snapshot should not raise a false VersionConflict");
    }

    #[tokio::test]
    async fn save_detects_genuine_stale_write() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        // Two independent loads of the same (empty) aggregate at base version 0.
        let mut first = repo.load("agg-stale").await.unwrap();
        let mut second = repo.load("agg-stale").await.unwrap();

        let event_a = increment_event("agg-stale", 1);
        first.apply_event(&event_a).unwrap();
        first.root.add_event(event_a);

        let event_b = increment_event("agg-stale", 2);
        second.apply_event(&event_b).unwrap();
        second.root.add_event(event_b);

        // First save succeeds and advances the store.
        repo.save(&mut first).await.unwrap();

        // Second save is stale: it was loaded before the first save landed, so its
        // base version (0) no longer matches the store's current length (1).
        let result = repo.save(&mut second).await;
        assert!(
            matches!(result, Err(AggregateError::VersionConflict { .. })),
            "expected a genuine VersionConflict, got: {result:?}"
        );
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

    // A counter aggregate whose `apply_event` actually mutates state and version,
    // so snapshot-aware loading is observable.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CounterState {
        count: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct CounterAggregate {
        #[serde(flatten)]
        root: AggregateRoot<CounterState>,
    }

    #[async_trait]
    impl Aggregate for CounterAggregate {
        fn aggregate_id(&self) -> &str {
            &self.root.id
        }

        fn aggregate_type() -> &'static str {
            "CounterAggregate"
        }

        fn version(&self) -> u64 {
            self.root.version
        }

        fn apply_event(&mut self, event: &DomainEvent) -> Result<(), AggregateError> {
            let amount = event.payload["amount"].as_u64().unwrap_or(0);
            self.root.state.count += amount;
            self.root.increment_version();
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
                root: AggregateRoot::new(id, CounterState { count: 0 }),
            }
        }
    }

    fn increment_event(id: &str, amount: u64) -> DomainEvent {
        DomainEvent::new(
            "increment",
            id,
            "CounterAggregate",
            serde_json::json!({ "amount": amount }),
        )
    }

    #[tokio::test]
    async fn load_snapshotted_seeds_from_snapshot_and_applies_only_newer_events() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        // Four persisted events (+10 each).
        store
            .save_events(
                "agg-1",
                &[
                    increment_event("agg-1", 10),
                    increment_event("agg-1", 10),
                    increment_event("agg-1", 10),
                    increment_event("agg-1", 10),
                ],
                None,
            )
            .await
            .unwrap();

        // Snapshot at version 2 with a distinctive base count that a replay-from-zero
        // could never produce.
        let mut snap_agg = CounterAggregate::new_instance("agg-1".to_string());
        snap_agg.root.version = 2;
        snap_agg.root.state.count = 1000;
        repo.save_snapshot(&snap_agg).await.unwrap();

        // Snapshot-aware load: 1000 (snapshot) + 2 newer events * 10 = 1020, version 4.
        let loaded = repo.load_snapshotted("agg-1").await.unwrap();
        assert_eq!(
            loaded.root.state.count, 1020,
            "must seed from snapshot base and apply only events after the snapshot version"
        );
        assert_eq!(
            loaded.version(),
            4,
            "version = snapshot(2) + 2 newer events"
        );

        // Full replay from zero for contrast: 4 events * 10 = 40.
        let full = repo.load("agg-1").await.unwrap();
        assert_eq!(full.root.state.count, 40);
    }

    #[tokio::test]
    async fn load_snapshotted_falls_back_to_full_replay_without_snapshot() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        store
            .save_events(
                "agg-2",
                &[increment_event("agg-2", 5), increment_event("agg-2", 7)],
                None,
            )
            .await
            .unwrap();

        let loaded = repo.load_snapshotted("agg-2").await.unwrap();
        assert_eq!(loaded.root.state.count, 12);
        assert_eq!(loaded.version(), 2);
    }

    #[tokio::test]
    async fn save_with_snapshot_persists_snapshot_every_frequency() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::with_snapshots(store.clone(), 2);

        // One event already persisted (store length 1 == the aggregate's base
        // version before the newly staged/applied event below).
        store
            .save_events("c1", &[increment_event("c1", 10)], None)
            .await
            .unwrap();

        // Aggregate reflecting the persisted state (version 1, count 10), with a
        // new event both applied (version -> 2, count -> 20) and staged to commit,
        // matching the real apply-then-save flow.
        let mut agg = CounterAggregate::new_instance("c1".to_string());
        agg.root.version = 1;
        agg.root.state.count = 10;
        let event = increment_event("c1", 10);
        agg.apply_event(&event).unwrap();
        agg.root.add_event(event);

        repo.save_with_snapshot(&mut agg).await.unwrap();

        // version 2 is a multiple of the frequency, so a snapshot is persisted.
        let snapshot = store.load_snapshot("c1").await.unwrap();
        assert!(
            snapshot.is_some(),
            "snapshot should be persisted at version 2"
        );
        let snapshot = snapshot.unwrap();
        assert_eq!(snapshot.version, 2);

        let restored: CounterAggregate = serde_json::from_value(snapshot.state).unwrap();
        assert_eq!(restored.root.state.count, 20);
    }

    #[tokio::test]
    async fn save_snapshot_round_trips_through_store() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        let mut agg = CounterAggregate::new_instance("c2".to_string());
        agg.root.version = 3;
        agg.root.state.count = 99;

        repo.save_snapshot(&agg).await.unwrap();

        let snapshot = store.load_snapshot("c2").await.unwrap().unwrap();
        assert_eq!(snapshot.version, 3);
        assert_eq!(snapshot.aggregate_type, "CounterAggregate");

        let restored: CounterAggregate = serde_json::from_value(snapshot.state).unwrap();
        assert_eq!(restored.root.state.count, 99);
    }

    #[tokio::test]
    async fn load_snapshotted_matches_full_replay_after_real_incremental_save() {
        let store = Arc::new(InMemoryEventStore::new());
        let repo = AggregateRepository::<CounterAggregate, _>::new(store.clone());

        // Build the aggregate through several real load -> apply -> save cycles
        // (the same flow a caller would actually drive), rather than
        // hand-constructing state.
        let mut aggregate = repo.load("agg-equiv").await.unwrap();
        for amount in [3, 5, 7] {
            let event = increment_event("agg-equiv", amount);
            aggregate.apply_event(&event).unwrap();
            aggregate.root.add_event(event);
            repo.save(&mut aggregate).await.unwrap();
        }
        assert_eq!(aggregate.version(), 3);
        assert_eq!(aggregate.root.state.count, 15);

        // Take a REAL snapshot mid-stream, reflecting genuinely replayed state
        // (not a fabricated value) at the current version (3).
        repo.save_snapshot(&aggregate).await.unwrap();
        assert!(
            store.load_snapshot("agg-equiv").await.unwrap().is_some(),
            "snapshot must actually be persisted so load_snapshotted exercises \
             the snapshot path, not the no-snapshot fallback"
        );

        // Continue applying more real events after the snapshot.
        for amount in [11, 13] {
            let event = increment_event("agg-equiv", amount);
            aggregate.apply_event(&event).unwrap();
            aggregate.root.add_event(event);
            repo.save(&mut aggregate).await.unwrap();
        }
        assert_eq!(aggregate.version(), 5);
        assert_eq!(aggregate.root.state.count, 39);

        // The invariant under test: snapshot + replay of newer events must
        // produce exactly the same state as a full replay of the identical
        // event log.
        let snapshotted = repo.load_snapshotted("agg-equiv").await.unwrap();
        let full = repo.load("agg-equiv").await.unwrap();

        assert_eq!(snapshotted.version(), full.version());
        assert_eq!(snapshotted.root.state.count, full.root.state.count);

        // Byte-for-byte: compare the full serialized state rather than only
        // the fields we thought to check individually.
        let snapshotted_json = serde_json::to_value(&snapshotted).unwrap();
        let full_json = serde_json::to_value(&full).unwrap();
        assert_eq!(
            snapshotted_json, full_json,
            "snapshot+replay state must exactly equal full-replay state"
        );
    }
}
