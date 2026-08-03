//! Event Sourcing for Armature
//!
//! This crate provides event sourcing capabilities with aggregates and event stores.
//!
//! ## Features
//!
//! - **Aggregates** - Event-sourced aggregate roots
//! - **Event Store** - A pluggable [`EventStore`] trait, with an in-memory
//!   implementation ([`InMemoryEventStore`]) provided for tests and local
//!   development. **No durable store ships in this crate**: `InMemoryEventStore`
//!   keeps everything in process memory and loses it on exit, so persistence
//!   means implementing [`EventStore`] against your own database.
//! - **Snapshots** - Aggregate snapshots for performance
//! - **Repository** - Load/save aggregates
//! - **Optimistic Concurrency** - Version-based conflict detection
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_eventsourcing::*;
//! use armature_events::DomainEvent;
//! use async_trait::async_trait;
//! use serde::{Deserialize, Serialize};
//!
//! // Define aggregate state
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct UserState {
//!     email: String,
//!     active: bool,
//! }
//!
//! // Define aggregate
//! #[derive(Debug, Clone, Serialize, Deserialize)]
//! struct UserAggregate {
//!     #[serde(flatten)]
//!     root: AggregateRoot<UserState>,
//! }
//!
//! #[async_trait]
//! impl Aggregate for UserAggregate {
//!     fn aggregate_id(&self) -> &str { &self.root.id }
//!     fn aggregate_type() -> &'static str { "User" }
//!     fn version(&self) -> u64 { self.root.version }
//!
//!     fn apply_event(&mut self, event: &DomainEvent) -> Result<(), AggregateError> {
//!         match event.metadata.name.as_str() {
//!             "user_created" => {
//!                 self.root.state.email = event.payload["email"].as_str().unwrap().to_string();
//!                 self.root.state.active = true;
//!                 self.root.increment_version();
//!             }
//!             "user_deactivated" => {
//!                 self.root.state.active = false;
//!                 self.root.increment_version();
//!             }
//!             _ => {}
//!         }
//!         Ok(())
//!     }
//!
//!     fn uncommitted_events(&self) -> &[DomainEvent] { &self.root.uncommitted_events }
//!     fn mark_events_committed(&mut self) { self.root.uncommitted_events.clear(); }
//!
//!     fn new_instance(id: String) -> Self {
//!         Self {
//!             root: AggregateRoot::new(id, UserState {
//!                 email: String::new(),
//!                 active: false,
//!             }),
//!         }
//!     }
//! }
//!
//! // Use the repository
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create event store
//!     let store = Arc::new(InMemoryEventStore::new());
//!
//!     // Create repository
//!     let repo = AggregateRepository::<UserAggregate, _>::new(store);
//!
//!     // Create new aggregate
//!     let mut user = UserAggregate::new_instance("user-123".to_string());
//!
//!     // Add event
//!     user.root.add_event(DomainEvent::new(
//!         "user_created",
//!         "user-123",
//!         "User",
//!         serde_json::json!({"email": "alice@example.com"}),
//!     ));
//!
//!     // Apply event
//!     let event = user.root.uncommitted_events[0].clone();
//!     user.apply_event(&event)?;
//!
//!     // Save aggregate
//!     repo.save(&mut user).await?;
//!
//!     // Load aggregate
//!     let loaded = repo.load("user-123").await?;
//!     println!("Loaded user: {:?}", loaded);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Snapshots
//!
//! [`AggregateRepository::with_snapshots`] requires `A: Serialize` and captures the
//! serializer up front, so the ordinary [`AggregateRepository::save`] persists a
//! snapshot whenever the aggregate's version is a multiple of the configured
//! frequency. Configuring a frequency and then getting no snapshots is not a
//! reachable state.
//!
//! ```rust,ignore
//! // Aggregate must derive Serialize (plus Deserialize to read snapshots back).
//! #[derive(Serialize, Deserialize)]
//! struct UserAggregate { /* ... */ }
//!
//! // Enable snapshots every 10 versions.
//! let repo = AggregateRepository::<UserAggregate, _>::with_snapshots(store, 10);
//!
//! // Plain `save` persists the snapshot when version % 10 == 0.
//! repo.save(&mut user).await?;
//! ```
//!
//! [`AggregateRepository::save_with_snapshot`] remains as an alias of `save` for
//! existing callers.
//!
//! A repository built with [`AggregateRepository::new`] never snapshots.
//!
//! Snapshot-aware reads use [`AggregateRepository::load_snapshotted`], which seeds
//! the aggregate from the latest snapshot (if any) and replays only the events
//! recorded after it, falling back to a full replay via [`AggregateRepository::load`]
//! when no snapshot exists.

pub mod aggregate;
pub mod repository;
pub mod store;

pub use aggregate::{Aggregate, AggregateError, AggregateRoot, Snapshot};
pub use repository::AggregateRepository;
pub use store::{EventStore, EventStoreError, InMemoryEventStore};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
