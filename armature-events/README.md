# armature-events

Event system for the Armature framework.

## Features

- **Event Bus** - Publish/subscribe events
- **Async Handlers** - Non-blocking event processing

## Installation

```toml
[dependencies]
armature-events = "0.1"
```

## Quick Start

```rust,ignore
use armature_events::{Event, EventBus, EventHandler, EventHandlerError, EventMetadata, TypedEventHandler};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::any::Any;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct UserCreated {
    metadata: EventMetadata,
    user_id: String,
    email: String,
}

impl Event for UserCreated {
    fn event_name(&self) -> &str { "user_created" }
    fn event_id(&self) -> Uuid { self.metadata.id }
    fn timestamp(&self) -> DateTime<Utc> { self.metadata.timestamp }
    fn as_any(&self) -> &dyn Any { self }
    fn clone_event(&self) -> Box<dyn Event> { Box::new(self.clone()) }
}

#[derive(Clone)]
struct EmailHandler;

#[async_trait]
impl EventHandler<UserCreated> for EmailHandler {
    async fn handle(&self, event: &UserCreated) -> Result<(), EventHandlerError> {
        send_welcome_email(&event.email).await
    }
}

let bus = EventBus::new();

// Subscribe: two type params (event type, handler type) wrapped in TypedEventHandler
bus.subscribe::<UserCreated, _>(TypedEventHandler::new(EmailHandler));

// Publish
bus.publish(UserCreated {
    metadata: EventMetadata::new("user_created"),
    user_id: "123".into(),
    email: "alice@example.com".into(),
})
.await?;
```

## License

MIT OR Apache-2.0
