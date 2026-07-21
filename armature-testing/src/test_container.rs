// Test Container for Dependency Injection

use armature_core::{Container, Provider};

/// Test container with enhanced testing capabilities
pub struct TestContainer {
    container: Container,
}

impl TestContainer {
    /// Create a new test container
    pub fn new() -> Self {
        Self {
            container: Container::new(),
        }
    }

    /// Register a provider
    pub fn register<T: Provider + Clone + 'static>(&self, provider: T) {
        self.container.register(provider);
    }

    /// Register a mock provider
    pub fn register_mock<T: Provider + Clone + 'static>(&self, mock: T) {
        self.container.register(mock);
    }

    /// Get a provider from the container.
    ///
    /// Delegates to the underlying `armature_core::Container`. Returns
    /// `None` if no provider of type `T` was registered via `register`/
    /// `register_mock`.
    pub fn get<T: Provider + Clone + 'static>(&self) -> Option<T> {
        self.container.get::<T>().ok().map(|arc| (*arc).clone())
    }

    /// Clear all providers
    pub fn clear(&mut self) {
        self.container = Container::new();
    }

    /// Get the underlying container
    pub fn inner(&self) -> &Container {
        &self.container
    }
}

impl Default for TestContainer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_creation() {
        let _container = TestContainer::new();
    }

    #[derive(Clone)]
    struct Widget {
        value: u32,
    }

    #[test]
    fn registered_provider_is_retrievable() {
        let container = TestContainer::new();
        container.register(Widget { value: 5 });

        let widget = container
            .get::<Widget>()
            .expect("Widget should be registered");
        assert_eq!(widget.value, 5);
    }

    #[test]
    fn mock_provider_is_retrievable() {
        let container = TestContainer::new();
        container.register_mock(Widget { value: 11 });

        let widget = container
            .get::<Widget>()
            .expect("Widget mock should be registered");
        assert_eq!(widget.value, 11);
    }

    #[test]
    fn unregistered_provider_returns_none() {
        let container = TestContainer::new();
        assert!(container.get::<Widget>().is_none());
    }

    #[test]
    fn clear_removes_previously_registered_providers() {
        let mut container = TestContainer::new();
        container.register(Widget { value: 1 });
        assert!(container.get::<Widget>().is_some());

        container.clear();
        assert!(container.get::<Widget>().is_none());
    }
}
