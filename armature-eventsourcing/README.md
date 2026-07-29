# armature-eventsourcing

Event sourcing support for the Armature framework.

## Features

- **Event Store** - `EventStore` trait with an `InMemoryEventStore` implementation (for
  testing/development; a persistent backend can be plugged in by implementing the trait)
- **Aggregates** - Domain-driven aggregates via the `Aggregate` trait and `AggregateRoot`
- **Repository** - `AggregateRepository` to load/save aggregates against an `EventStore`
- **Snapshots** - Performance optimization via `save_with_snapshot`/`load_snapshotted`

> Read model / projection support lives in `armature-cqrs`, not in this crate.

## Installation

```toml
[dependencies]
armature-eventsourcing = "0.1"
```

## Quick Start

```rust,ignore
use armature_eventsourcing::*;
use armature_events::DomainEvent;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Define aggregate state
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserState {
    email: String,
    active: bool,
}

// Define aggregate
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserAggregate {
    #[serde(flatten)]
    root: AggregateRoot<UserState>,
}

#[async_trait]
impl Aggregate for UserAggregate {
    fn aggregate_id(&self) -> &str { &self.root.id }
    fn aggregate_type() -> &'static str { "User" }
    fn version(&self) -> u64 { self.root.version }

    fn apply_event(&mut self, event: &DomainEvent) -> Result<(), AggregateError> {
        match event.metadata.name.as_str() {
            "user_created" => {
                self.root.state.email = event.payload["email"].as_str().unwrap().to_string();
                self.root.state.active = true;
                self.root.increment_version();
            }
            "user_deactivated" => {
                self.root.state.active = false;
                self.root.increment_version();
            }
            _ => {}
        }
        Ok(())
    }

    fn uncommitted_events(&self) -> &[DomainEvent] { &self.root.uncommitted_events }
    fn mark_events_committed(&mut self) { self.root.uncommitted_events.clear(); }

    fn new_instance(id: String) -> Self {
        Self {
            root: AggregateRoot::new(id, UserState {
                email: String::new(),
                active: false,
            }),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create an event store (in-memory; swap in your own EventStore impl for persistence)
    let store = Arc::new(InMemoryEventStore::new());

    // Create repository
    let repo = AggregateRepository::<UserAggregate, _>::new(store);

    // Create new aggregate
    let mut user = UserAggregate::new_instance("user-123".to_string());

    // Add event
    user.root.add_event(DomainEvent::new(
        "user_created",
        "user-123",
        "User",
        serde_json::json!({"email": "alice@example.com"}),
    ));

    // Apply event
    let event = user.root.uncommitted_events[0].clone();
    user.apply_event(&event)?;

    // Save aggregate (persists uncommitted events via EventStore::save_events)
    repo.save(&mut user).await?;

    // Load aggregate (replays events via EventStore::load_events)
    let loaded = repo.load("user-123").await?;
    println!("Loaded user: {:?}", loaded);

    Ok(())
}
```

## License

MIT OR Apache-2.0

