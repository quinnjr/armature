//! Projections for read models

use armature_events::{Event, EventHandler, EventHandlerError};
use async_trait::async_trait;
use std::sync::Arc;
use thiserror::Error;

/// Projection trait
///
/// Projections build read models from events.
#[async_trait]
pub trait Projection: Send + Sync {
    /// Project an event to update the read model
    async fn project(&self, event: &dyn Event) -> Result<(), ProjectionError>;

    /// Reset the read model to its initial (empty) state.
    ///
    /// Called by the default [`Projection::rebuild`] implementation before
    /// replaying events, so that rebuilding a projection that has already
    /// processed events does not double-apply them.
    ///
    /// The default implementation is a no-op. Projections that hold any
    /// state MUST override this method with logic that actually clears
    /// that state, otherwise `rebuild`/`rebuild_all` will double-apply
    /// events on top of the existing read model.
    async fn reset(&self) -> Result<(), ProjectionError> {
        Ok(())
    }

    /// Rebuild the projection from scratch
    ///
    /// Calls [`Projection::reset`] first to clear any existing state, then
    /// replays every event through [`Projection::project`].
    async fn rebuild(&self, events: &[&dyn Event]) -> Result<(), ProjectionError> {
        self.reset().await?;
        for event in events {
            self.project(*event).await?;
        }
        Ok(())
    }
}

/// Projection error
#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("Projection failed: {0}")]
    ProjectionFailed(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Event handling error: {0}")]
    EventHandlingError(String),
}

/// Projection manager
///
/// Manages multiple projections and rebuilds.
pub struct ProjectionManager {
    projections: Vec<Arc<dyn Projection>>,
}

impl ProjectionManager {
    /// Create new projection manager
    pub fn new() -> Self {
        Self {
            projections: Vec::new(),
        }
    }

    /// Add a projection
    pub fn add_projection(&mut self, projection: Arc<dyn Projection>) {
        self.projections.push(projection);
    }

    /// Project event to all projections
    pub async fn project_event(&self, event: &dyn Event) -> Result<(), ProjectionError> {
        for projection in &self.projections {
            projection.project(event).await?;
        }
        Ok(())
    }

    /// Rebuild all projections
    pub async fn rebuild_all(&self, events: &[&dyn Event]) -> Result<(), ProjectionError> {
        for projection in &self.projections {
            projection.rebuild(events).await?;
        }
        Ok(())
    }
}

impl Default for ProjectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Event handler wrapper for projections
pub struct ProjectionEventHandler<P: Projection> {
    projection: Arc<P>,
}

impl<P: Projection> ProjectionEventHandler<P> {
    pub fn new(projection: Arc<P>) -> Self {
        Self { projection }
    }
}

impl<P: Projection> Clone for ProjectionEventHandler<P> {
    fn clone(&self) -> Self {
        Self {
            projection: self.projection.clone(),
        }
    }
}

#[async_trait]
impl<E: Event, P: Projection> EventHandler<E> for ProjectionEventHandler<P> {
    async fn handle(&self, event: &E) -> Result<(), EventHandlerError> {
        self.projection
            .project(event)
            .await
            .map_err(|e| EventHandlerError::ProcessingError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_events::EventMetadata;
    use chrono::Utc;
    use std::any::Any;
    use std::sync::atomic::{AtomicU32, Ordering};
    use uuid::Uuid;

    #[derive(Debug, Clone)]
    struct TestEvent {
        metadata: EventMetadata,
    }

    impl Event for TestEvent {
        fn event_name(&self) -> &str {
            &self.metadata.name
        }

        fn event_id(&self) -> Uuid {
            self.metadata.id
        }

        fn timestamp(&self) -> chrono::DateTime<Utc> {
            self.metadata.timestamp
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn clone_event(&self) -> Box<dyn Event> {
            Box::new(self.clone())
        }
    }

    struct TestProjection {
        counter: AtomicU32,
    }

    #[async_trait]
    impl Projection for TestProjection {
        async fn project(&self, _event: &dyn Event) -> Result<(), ProjectionError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn reset(&self) -> Result<(), ProjectionError> {
            self.counter.store(0, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_projection() {
        let projection = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });

        let event = TestEvent {
            metadata: EventMetadata::new("test_event"),
        };

        projection.project(&event).await.unwrap();
        assert_eq!(projection.counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_projection_manager() {
        let projection1 = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });
        let projection2 = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });

        let mut manager = ProjectionManager::new();
        manager.add_projection(projection1.clone() as Arc<dyn Projection>);
        manager.add_projection(projection2.clone() as Arc<dyn Projection>);

        let event = TestEvent {
            metadata: EventMetadata::new("test_event"),
        };

        manager.project_event(&event).await.unwrap();
        assert_eq!(projection1.counter.load(Ordering::SeqCst), 1);
        assert_eq!(projection2.counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_rebuild_resets_before_replay() {
        let projection = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });

        let event = TestEvent {
            metadata: EventMetadata::new("test_event"),
        };
        let events: Vec<&dyn Event> = vec![&event, &event, &event];

        // Populate the projection first, simulating a projection that has
        // already processed events before a rebuild is requested.
        for e in &events {
            projection.project(*e).await.unwrap();
        }
        assert_eq!(projection.counter.load(Ordering::SeqCst), 3);

        // Rebuilding with the same 3 events must reset first, so the final
        // count equals a single replay (3), not a double application (6).
        projection.rebuild(&events).await.unwrap();
        assert_eq!(projection.counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_rebuild_all_resets_before_replay() {
        let projection1 = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });
        let projection2 = Arc::new(TestProjection {
            counter: AtomicU32::new(0),
        });

        let mut manager = ProjectionManager::new();
        manager.add_projection(projection1.clone() as Arc<dyn Projection>);
        manager.add_projection(projection2.clone() as Arc<dyn Projection>);

        let event = TestEvent {
            metadata: EventMetadata::new("test_event"),
        };
        let events: Vec<&dyn Event> = vec![&event, &event];

        manager.project_event(&event).await.unwrap();
        manager.project_event(&event).await.unwrap();
        assert_eq!(projection1.counter.load(Ordering::SeqCst), 2);
        assert_eq!(projection2.counter.load(Ordering::SeqCst), 2);

        manager.rebuild_all(&events).await.unwrap();
        assert_eq!(projection1.counter.load(Ordering::SeqCst), 2);
        assert_eq!(projection2.counter.load(Ordering::SeqCst), 2);
    }

    /// A projection that does not override `reset`, demonstrating the
    /// documented default no-op behavior of `Projection::reset`.
    struct NoResetProjection {
        counter: AtomicU32,
    }

    #[async_trait]
    impl Projection for NoResetProjection {
        async fn project(&self, _event: &dyn Event) -> Result<(), ProjectionError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_default_reset_is_noop() {
        let projection = NoResetProjection {
            counter: AtomicU32::new(5),
        };

        projection.reset().await.unwrap();

        // Default `reset` does nothing; state carries over unless a
        // projection overrides it.
        assert_eq!(projection.counter.load(Ordering::SeqCst), 5);
    }
}
