# armature-cqrs

CQRS (Command Query Responsibility Segregation) for the Armature framework.

## Features

- **Commands** - Write operations, dispatched through a `CommandBus`
- **Queries** - Read operations, dispatched through a `QueryBus`
- **Handlers** - Async `CommandHandler<C>` / `QueryHandler<Q>` implementations
- **Projections** - Build read models from events, with rebuild support

## Installation

```toml
[dependencies]
armature-cqrs = "0.1"
```

## Quick Start

`Command` and `Query` are plain traits you implement by hand (there is no
derive macro) — they just declare the type returned by a successful
handler:

```rust,ignore
use armature_cqrs::{Command, CommandBus, CommandError, CommandHandler, Query, QueryBus, QueryError, QueryHandler};
use async_trait::async_trait;

struct CreateUser {
    name: String,
}

impl Command for CreateUser {
    type Result = String; // user ID
}

struct CreateUserHandler;

#[async_trait]
impl CommandHandler<CreateUser> for CreateUserHandler {
    async fn handle(&self, command: CreateUser) -> Result<String, CommandError> {
        Ok(format!("user-{}", command.name))
    }
}

struct GetUser {
    id: String,
}

impl Query for GetUser {
    type Result = String;
}

struct GetUserHandler;

#[async_trait]
impl QueryHandler<GetUser> for GetUserHandler {
    async fn handle(&self, query: GetUser) -> Result<String, QueryError> {
        Ok(format!("user {}", query.id))
    }
}

#[tokio::main]
async fn main() {
    let command_bus = CommandBus::new();
    command_bus.register::<CreateUser, _>(CreateUserHandler);

    let query_bus = QueryBus::new();
    query_bus.register::<GetUser, _>(GetUserHandler);

    // Execute a command
    let user_id = command_bus
        .execute(CreateUser { name: "Alice".into() })
        .await
        .unwrap();

    // Execute a query
    let user = query_bus
        .execute(GetUser { id: user_id })
        .await
        .unwrap();

    println!("{user}");
}
```

If no handler is registered for a command/query type, `execute` returns
`CommandError::HandlerNotFound` / `QueryError::HandlerNotFound`.

## Projections

Projections build read models by folding events. Implement `project` to
apply a single event; override `reset` if your projection holds any state,
so `rebuild`/`rebuild_all` can clear it before replaying (the default
`reset` is a no-op, so a projection that skips it will double-apply events
on rebuild):

```rust,ignore
use armature_cqrs::{Projection, ProjectionError};
use armature_events::Event;
use async_trait::async_trait;

struct UserListProjection {
    // read model storage
}

#[async_trait]
impl Projection for UserListProjection {
    async fn project(&self, event: &dyn Event) -> Result<(), ProjectionError> {
        match event.event_name() {
            "user_created" => {
                // update read model
            }
            "user_deleted" => {
                // update read model
            }
            _ => {}
        }
        Ok(())
    }

    async fn reset(&self) -> Result<(), ProjectionError> {
        // clear read model storage here
        Ok(())
    }
}
```

## License

MIT OR Apache-2.0
