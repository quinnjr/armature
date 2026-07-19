//! Event Sourcing for Armature
//!
//! This crate provides event sourcing capabilities with aggregates and event stores.
//!
//! ## Features
//!
//! - **Aggregates** - Event-sourced aggregate roots
//! - **Event Store** - Persistent event storage
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
//! `with_snapshots` only configures the snapshot *frequency* on the repository;
//! it does not by itself cause snapshots to be taken. The base [`AggregateRepository::save`]
//! method's internal snapshot step is a deliberate no-op (a generic `Aggregate` has
//! no serde bound, so it cannot be serialized). To actually persist periodic
//! snapshots, the aggregate must implement `Serialize + DeserializeOwned` and you
//! must call [`AggregateRepository::save_with_snapshot`] instead of `save`:
//!
//! ```rust,ignore
//! // Aggregate must derive Serialize + DeserializeOwned for snapshotting to work.
//! #[derive(Serialize, Deserialize)]
//! struct UserAggregate { /* ... */ }
//!
//! // Enable snapshots every 10 events.
//! let repo = AggregateRepository::<UserAggregate, _>::with_snapshots(store, 10);
//!
//! // Use `save_with_snapshot` (not `save`) so a real snapshot is persisted
//! // whenever the aggregate's version is a multiple of the configured frequency.
//! repo.save_with_snapshot(&mut user).await?;
//! ```
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
