//! Event definitions and traits

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::fmt::Debug;
use uuid::Uuid;

/// Event trait
///
/// All events must implement this trait to be published through the event bus.
pub trait Event: Send + Sync + Debug + 'static {
    /// Get event name
    fn event_name(&self) -> &str;

    /// Get event ID
    fn event_id(&self) -> Uuid;

    /// Get event timestamp
    fn timestamp(&self) -> DateTime<Utc>;

    /// Cast to Any for downcasting
    fn as_any(&self) -> &dyn Any;

    /// Clone the event (box clone pattern)
    fn clone_event(&self) -> Box<dyn Event>;
}

/// Base event metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Unique event ID
    pub id: Uuid,

    /// Event name/type
    pub name: String,

    /// Timestamp when event was created
    pub timestamp: DateTime<Utc>,

    /// Optional correlation ID for tracing
    pub correlation_id: Option<Uuid>,

    /// Optional causation ID (ID of the event that caused this event)
    pub causation_id: Option<Uuid>,

    /// Custom metadata
    pub metadata: serde_json::Value,
}

impl EventMetadata {
    /// Create new event metadata
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            timestamp: Utc::now(),
            correlation_id: None,
            causation_id: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Set correlation ID
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set causation ID
    pub fn with_causation_id(mut self, id: Uuid) -> Self {
        self.causation_id = Some(id);
        self
    }
}

/// Domain event (for event sourcing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    /// Event metadata
    #[serde(flatten)]
    pub metadata: EventMetadata,

    /// Aggregate ID
    pub aggregate_id: String,

    /// Aggregate type
    pub aggregate_type: String,

    /// Event version (for schema evolution)
    pub version: u32,

    /// Event payload
    pub payload: serde_json::Value,
}

impl DomainEvent {
    /// Create new domain event
    pub fn new(
        event_name: impl Into<String>,
        aggregate_id: impl Into<String>,
        aggregate_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            metadata: EventMetadata::new(event_name),
            aggregate_id: aggregate_id.into(),
            aggregate_type: aggregate_type.into(),
            version: 1,
            payload,
        }
    }
}

/// Event handler trait
#[async_trait]
pub trait EventHandler<E: Event>: Send + Sync {
    /// Handle the event
    async fn handle(&self, event: &E) -> Result<(), EventHandlerError>;
}

/// Event handler error
#[derive(Debug, thiserror::Error)]
pub enum EventHandlerError {
    #[error("Handler failed: {0}")]
    HandlerFailed(String),

    #[error("Event processing error: {0}")]
    ProcessingError(String),

    #[error("Handler not found for event: {0}")]
    HandlerNotFound(String),
}

/// Type-erased event handler
#[async_trait]
pub trait DynEventHandler: Send + Sync {
    /// Handle event (type-erased)
    async fn handle_dyn(&self, event: &dyn Event) -> Result<(), EventHandlerError>;

    /// Clone the handler
    fn clone_handler(&self) -> Box<dyn DynEventHandler>;
}

/// Wrapper for typed event handlers
pub struct TypedEventHandler<E: Event, H: EventHandler<E>> {
    handler: H,
    _phantom: std::marker::PhantomData<E>,
}

impl<E: Event, H: EventHandler<E>> TypedEventHandler<E, H> {
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<E: Event, H: EventHandler<E> + Clone> Clone for TypedEventHandler<E, H> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            _phantom: std::marker::PhantomData,
        }
    }
}

/// A `TypedEventHandler` is itself a handler for `E`, so wrapping is idempotent.
/// This keeps `EventBus::subscribe` — which now requires `H: EventHandler<E>` so
/// the event type cannot be stated wrongly — usable with both a bare handler and
/// an already-wrapped one.
#[async_trait]
impl<E: Event, H: EventHandler<E>> EventHandler<E> for TypedEventHandler<E, H> {
    async fn handle(&self, event: &E) -> Result<(), EventHandlerError> {
        self.handler.handle(event).await
    }
}

#[async_trait]
impl<E: Event + Clone, H: EventHandler<E> + Clone + 'static> DynEventHandler
    for TypedEventHandler<E, H>
{
    async fn handle_dyn(&self, event: &dyn Event) -> Result<(), EventHandlerError> {
        if let Some(typed_event) = event.as_any().downcast_ref::<E>() {
            self.handler.handle(typed_event).await
        } else {
            Err(EventHandlerError::HandlerFailed(
                "Type mismatch".to_string(),
            ))
        }
    }

    fn clone_handler(&self) -> Box<dyn DynEventHandler> {
        Box::new(Self {
            handler: self.handler.clone(),
            _phantom: std::marker::PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct TestEvent {
        metadata: EventMetadata,
        data: String,
    }

    impl TestEvent {
        #[allow(dead_code)]
        fn new(data: String) -> Self {
            Self {
                metadata: EventMetadata::new("test_event"),
                data,
            }
        }
    }

    impl Event for TestEvent {
        fn event_name(&self) -> &str {
            &self.metadata.name
        }

        fn event_id(&self) -> Uuid {
            self.metadata.id
        }

        fn timestamp(&self) -> DateTime<Utc> {
            self.metadata.timestamp
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn clone_event(&self) -> Box<dyn Event> {
            Box::new(self.clone())
        }
    }

    #[test]
    fn test_event_metadata() {
        let metadata = EventMetadata::new("test_event").with_correlation_id(Uuid::new_v4());

        assert_eq!(metadata.name, "test_event");
        assert!(metadata.correlation_id.is_some());
    }

    #[test]
    fn test_domain_event() {
        let event = DomainEvent::new(
            "user_created",
            "user-123",
            "User",
            serde_json::json!({"name": "Alice"}),
        );

        assert_eq!(event.metadata.name, "user_created");
        assert_eq!(event.aggregate_id, "user-123");
        assert_eq!(event.aggregate_type, "User");
    }
}
