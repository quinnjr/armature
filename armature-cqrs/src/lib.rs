//! CQRS (Command Query Responsibility Segregation) for Armature
//!
//! This crate provides CQRS pattern implementation with command/query separation.
//!
//! ## Features
//!
//! - **Command Bus** - Execute commands (writes)
//! - **Query Bus** - Execute queries (reads)
//! - **Projections** - Build read models from events
//! - **Type-safe** - Strong typing with compile-time safety
//! - **Async** - Full async/await support
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_cqrs::*;
//! use async_trait::async_trait;
//!
//! // Define a command
//! struct CreateUserCommand {
//!     email: String,
//! }
//!
//! impl Command for CreateUserCommand {
//!     type Result = String; // User ID
//! }
//!
//! // Define command handler
//! struct CreateUserHandler;
//!
//! #[async_trait]
//! impl CommandHandler<CreateUserCommand> for CreateUserHandler {
//!     async fn handle(&self, command: CreateUserCommand) -> Result<String, CommandError> {
//!         // Business logic here
//!         Ok(format!("user-{}", uuid::Uuid::new_v4()))
//!     }
//! }
//!
//! // Define a query
//! struct GetUserQuery {
//!     user_id: String,
//! }
//!
//! impl Query for GetUserQuery {
//!     type Result = User;
//! }
//!
//! // Define query handler
//! struct GetUserHandler;
//!
//! #[async_trait]
//! impl QueryHandler<GetUserQuery> for GetUserHandler {
//!     async fn handle(&self, query: GetUserQuery) -> Result<User, QueryError> {
//!         // Fetch from read model
//!         Ok(User {
//!             id: query.user_id,
//!             email: "alice@example.com".to_string(),
//!         })
//!     }
//! }
//!
//! // Use the buses
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create buses
//!     let command_bus = CommandBus::new();
//!     let query_bus = QueryBus::new();
//!
//!     // Register handlers
//!     command_bus.register::<CreateUserCommand, _>(CreateUserHandler);
//!     query_bus.register::<GetUserQuery, _>(GetUserHandler);
//!
//!     // Execute command
//!     let user_id = command_bus.execute(CreateUserCommand {
//!         email: "alice@example.com".to_string(),
//!     }).await?;
//!
//!     // Execute query
//!     let user = query_bus.execute(GetUserQuery { user_id }).await?;
//!     println!("User: {:?}", user);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Projections
//!
//! ```rust,ignore
//! use armature_events::Event;
//! use async_trait::async_trait;
//! use std::sync::Mutex;
//!
//! struct UserListProjection {
//!     // Read model storage
//!     users: Mutex<Vec<String>>,
//! }
//!
//! #[async_trait]
//! impl Projection for UserListProjection {
//!     async fn project(&self, event: &dyn Event) -> Result<(), ProjectionError> {
//!         match event.event_name() {
//!             "user_created" => {
//!                 // Update read model
//!             }
//!             "user_deleted" => {
//!                 // Update read model
//!             }
//!             _ => {}
//!         }
//!         Ok(())
//!     }
//!
//!     // Any projection that holds state MUST override `reset`, clearing
//!     // that state, so that `rebuild`/`rebuild_all` (which call `reset`
//!     // before replaying events) don't double-apply events on top of it.
//!     async fn reset(&self) -> Result<(), ProjectionError> {
//!         self.users.lock().unwrap().clear();
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ## Integration with Event Sourcing
//!
//! ```rust,ignore
//! use armature_eventsourcing::*;
//! use armature_events::EventBus;
//!
//! // Command handler uses aggregate repository
//! #[async_trait]
//! impl CommandHandler<CreateUserCommand> for CreateUserHandler {
//!     async fn handle(&self, command: CreateUserCommand) -> Result<String, CommandError> {
//!         let mut user = UserAggregate::new_instance(uuid::Uuid::new_v4().to_string());
//!
//!         // Add domain event
//!         user.create(command.email)?;
//!
//!         // Save aggregate (persists events)
//!         self.repository.save(&mut user).await?;
//!
//!         // Publish events to event bus for projections
//!         for event in user.uncommitted_events() {
//!             self.event_bus.publish(event.clone()).await?;
//!         }
//!
//!         Ok(user.aggregate_id().to_string())
//!     }
//! }
//! ```

pub mod command;
pub mod projection;
pub mod query;

pub use command::{Command, CommandBus, CommandError, CommandHandler};
pub use projection::{Projection, ProjectionError, ProjectionEventHandler, ProjectionManager};
pub use query::{Query, QueryBus, QueryError, QueryHandler};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
