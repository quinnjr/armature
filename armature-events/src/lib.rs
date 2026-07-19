//! Event-Driven Architecture support for Armature
//!
//! This crate provides in-process event publishing and handling.
//!
//! ## Features
//!
//! - **Event Bus** - Publish/subscribe event system
//! - **Event Handlers** - Trait-based event handling: implement [`EventHandler<E>`] for your
//!   handler type, then register it with [`EventBus::subscribe`] by wrapping it in
//!   [`TypedEventHandler::new`]
//! - **Type-safe** - Strong typing with compile-time safety
//! - **Async** - Full async/await support
//! - **Flexible** - Sync and async handler execution
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_events::*;
//! use async_trait::async_trait;
//!
//! // Define an event
//! #[derive(Debug, Clone)]
//! struct UserCreatedEvent {
//!     metadata: EventMetadata,
//!     user_id: String,
//!     email: String,
//! }
//!
//! impl Event for UserCreatedEvent {
//!     fn event_name(&self) -> &str { "user_created" }
//!     fn event_id(&self) -> Uuid { self.metadata.id }
//!     fn timestamp(&self) -> DateTime<Utc> { self.metadata.timestamp }
//!     fn as_any(&self) -> &dyn Any { self }
//!     fn clone_event(&self) -> Box<dyn Event> { Box::new(self.clone()) }
//! }
//!
//! // Define a handler
//! #[derive(Clone)]
//! struct EmailHandler;
//!
//! #[async_trait]
//! impl EventHandler<UserCreatedEvent> for EmailHandler {
//!     async fn handle(&self, event: &UserCreatedEvent) -> Result<(), EventHandlerError> {
//!         println!("Sending welcome email to {}", event.email);
//!         Ok(())
//!     }
//! }
//!
//! // Use the event bus
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let bus = EventBus::new();
//!
//!     // Subscribe handler
//!     bus.subscribe::<UserCreatedEvent, _>(TypedEventHandler::new(EmailHandler));
//!
//!     // Publish event
//!     let event = UserCreatedEvent {
//!         metadata: EventMetadata::new("user_created"),
//!         user_id: "123".to_string(),
//!         email: "alice@example.com".to_string(),
//!     };
//!
//!     bus.publish(event).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Multiple Handlers
//!
//! ```rust,ignore
//! // Subscribe multiple handlers
//! bus.subscribe::<UserCreatedEvent, _>(TypedEventHandler::new(EmailHandler));
//! bus.subscribe::<UserCreatedEvent, _>(TypedEventHandler::new(AnalyticsHandler));
//! bus.subscribe::<UserCreatedEvent, _>(TypedEventHandler::new(AuditHandler));
//!
//! // All handlers will be invoked
//! bus.publish(event).await?;
//! ```
//!
//! ## Configuration
//!
//! ```rust,ignore
//! let bus = EventBusBuilder::new()
//!     .async_handling(true)           // Run handlers concurrently
//!     .continue_on_error(true)        // Don't stop on handler errors
//!     .enable_logging(true)           // Log events
//!     .build();
//! ```
//!
//! ## Error Handling
//!
//! ```rust,ignore
//! let bus = EventBusBuilder::new()
//!     .continue_on_error(false)  // Stop on first error
//!     .build();
//!
//! match bus.publish(event).await {
//!     Ok(()) => println!("All handlers succeeded"),
//!     Err(EventBusError::HandlersFailed(errors)) => {
//!         eprintln!("Some handlers failed: {:?}", errors);
//!     }
//!     Err(e) => eprintln!("Publish error: {}", e),
//! }
//! ```

pub mod bus;
pub mod event;

pub use bus::{EventBus, EventBusBuilder, EventBusConfig, EventBusError};
pub use event::{
    DomainEvent, DynEventHandler, Event, EventHandler, EventHandlerError, EventMetadata,
    TypedEventHandler,
};

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
