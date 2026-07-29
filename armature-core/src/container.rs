//! Dependency injection container
//!
//! This module re-exports the DI container from `dependency-injector` and provides
//! framework-specific integration.

use crate::Error;
use crate::lifecycle::LifecycleManager;
use std::any::{Any, TypeId};
use std::sync::{Arc, OnceLock};

// Re-export the core DI types (excluding ProviderRegistration to avoid conflict with traits.rs)
pub use dependency_injector::{
    Container as DiContainer, DiError, Factory, Injectable, Lifetime, Provider, Scope,
    ScopeBuilder, ScopedContainer as DiScopedContainer,
};

/// The dependency injection container for Armature.
///
/// This is a thin wrapper around `dependency_injector::Container` that provides
/// error conversion to the framework's error type.
#[derive(Clone, Default)]
pub struct Container {
    inner: DiContainer,
    /// Lifecycle manager attached by [`Application::create`](crate::Application::create)
    /// (via [`Container::attach_lifecycle`]) so that provider registration
    /// can discover and register `OnModuleInit`/`OnModuleDestroy`/
    /// `OnApplicationBootstrap`/`OnApplicationShutdown` hooks. See
    /// [`Container::lifecycle_manager`]. Set at most once; unset by default.
    lifecycle: Arc<OnceLock<Arc<LifecycleManager>>>,
}

impl Container {
    /// Create a new empty container.
    #[inline]
    pub fn new() -> Self {
        Self {
            inner: DiContainer::new(),
            lifecycle: Arc::new(OnceLock::new()),
        }
    }

    /// Create with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: DiContainer::with_capacity(capacity),
            lifecycle: Arc::new(OnceLock::new()),
        }
    }

    /// Create a scoped child container.
    ///
    /// The child shares the parent's attached [`LifecycleManager`] (if any),
    /// since lifecycle wiring is a single application-wide concern set once
    /// at the root.
    #[inline]
    pub fn create_scope(&self) -> Self {
        Self {
            inner: self.inner.scope(),
            lifecycle: self.lifecycle.clone(),
        }
    }

    /// Attach a [`LifecycleManager`] to this container.
    ///
    /// Once attached, providers registered afterward through the provider
    /// registration path (see [`crate::module::provider_registration`] and
    /// `armature_proc_macro`'s `#[module(...)]` codegen) are probed for
    /// lifecycle hook trait implementations (`OnModuleInit`,
    /// `OnModuleDestroy`, `OnApplicationBootstrap`, `OnApplicationShutdown`)
    /// and registered with the manager automatically. Calling this more than
    /// once has no effect after the first call (first attachment wins).
    ///
    /// This is internal application wiring, called by
    /// [`Application::create`](crate::Application::create); most users never
    /// need to call it directly.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::Container;
    /// use armature_core::lifecycle::LifecycleManager;
    /// use std::sync::Arc;
    ///
    /// let container = Container::new();
    /// let manager = Arc::new(LifecycleManager::new());
    ///
    /// container.attach_lifecycle(&manager);
    ///
    /// assert!(container.lifecycle_manager().is_some());
    /// ```
    #[inline]
    pub fn attach_lifecycle(&self, lifecycle: &Arc<LifecycleManager>) {
        if self.lifecycle.set(lifecycle.clone()).is_err() {
            tracing::debug!(
                "attach_lifecycle called on a Container that already has a LifecycleManager attached; ignoring (first attachment wins)"
            );
        }
    }

    /// Returns the [`LifecycleManager`] attached via [`Container::attach_lifecycle`],
    /// if any.
    #[inline]
    pub fn lifecycle_manager(&self) -> Option<Arc<LifecycleManager>> {
        self.lifecycle.get().cloned()
    }

    /// Alias for create_scope.
    #[inline]
    pub fn scope(&self) -> Self {
        self.create_scope()
    }

    /// Register a singleton service.
    #[inline]
    pub fn register<T: Injectable>(&self, instance: T) {
        self.inner.singleton(instance);
    }

    /// Register a singleton service (explicit).
    #[inline]
    pub fn singleton<T: Injectable>(&self, instance: T) {
        self.inner.singleton(instance);
    }

    /// Register a lazy singleton.
    #[inline]
    pub fn lazy<T: Injectable, F>(&self, factory: F)
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        self.inner.lazy(factory);
    }

    /// Register a transient service.
    #[inline]
    pub fn transient<T: Injectable, F>(&self, factory: F)
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        self.inner.transient(factory);
    }

    /// Register a boxed service instance.
    #[inline]
    pub fn register_boxed<T: Injectable>(&self, instance: Box<T>) {
        self.inner.register_boxed(instance);
    }

    /// Register by TypeId directly.
    #[inline]
    pub fn register_by_id(&self, type_id: TypeId, instance: Arc<dyn Any + Send + Sync>) {
        self.inner.register_by_id(type_id, instance);
    }

    /// Register using a factory function.
    #[inline]
    pub fn register_factory<T: Injectable, F>(&self, factory: F)
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        self.inner.lazy(factory);
    }

    /// Resolve a service by type.
    #[inline]
    pub fn resolve<T: Injectable>(&self) -> Result<Arc<T>, Error> {
        self.inner
            .get::<T>()
            .map_err(|e| Error::ProviderNotFound(e.to_string()))
    }

    /// Alias for resolve.
    #[inline]
    pub fn get<T: Injectable>(&self) -> Result<Arc<T>, Error> {
        self.resolve::<T>()
    }

    /// Try to resolve, returning None if not found.
    #[inline]
    pub fn try_resolve<T: Injectable>(&self) -> Option<Arc<T>> {
        self.inner.try_get()
    }

    /// Try to get a service.
    #[inline]
    pub fn try_get<T: Injectable>(&self) -> Option<Arc<T>> {
        self.try_resolve::<T>()
    }

    /// Check if a service is registered.
    #[inline]
    pub fn has<T: Injectable>(&self) -> bool {
        self.inner.contains::<T>()
    }

    /// Alias for has.
    #[inline]
    pub fn contains<T: Injectable>(&self) -> bool {
        self.has::<T>()
    }

    /// Clear all services.
    #[inline]
    pub fn clear(&self) {
        self.inner.clear();
    }

    /// Get the number of registered services.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the container is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Lock the container.
    #[inline]
    pub fn lock(&self) {
        self.inner.lock();
    }

    /// Check if the container is locked.
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.inner.is_locked()
    }

    /// Get all registered type IDs.
    #[inline]
    pub fn registered_types(&self) -> Vec<TypeId> {
        self.inner.registered_types()
    }

    /// Get the scope depth.
    #[inline]
    pub fn depth(&self) -> u32 {
        self.inner.depth()
    }

    /// Get the inner DI container.
    #[inline]
    pub fn inner(&self) -> &DiContainer {
        &self.inner
    }

    // ============================================================================
    // Convenience Methods
    // ============================================================================

    /// Get a service or panic if not found.
    ///
    /// This is useful in tests or startup code where a missing service
    /// is a fatal error.
    ///
    /// # Panics
    ///
    /// Panics if the service is not registered.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::Container;
    ///
    /// #[derive(Clone)]
    /// struct MyService;
    ///
    /// let container = Container::new();
    /// container.register(MyService);
    ///
    /// let service = container.require::<MyService>(); // Won't panic
    /// ```
    #[inline]
    pub fn require<T: Injectable>(&self) -> Arc<T> {
        self.resolve::<T>().unwrap_or_else(|_| {
            panic!(
                "Required service {} not found in container",
                std::any::type_name::<T>()
            )
        })
    }

    /// Get a service or register a default value if not found.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::Container;
    ///
    /// #[derive(Clone, Default)]
    /// struct Config {
    ///     debug: bool,
    /// }
    ///
    /// let container = Container::new();
    /// let config = container.get_or_default::<Config>();
    /// ```
    #[inline]
    pub fn get_or_default<T: Injectable + Default>(&self) -> Arc<T> {
        self.try_get::<T>().unwrap_or_else(|| {
            self.register(T::default());
            self.resolve::<T>().unwrap()
        })
    }

    /// Register a service only if it's not already registered.
    ///
    /// Returns true if the service was registered, false if it already existed.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::Container;
    ///
    /// #[derive(Clone)]
    /// struct Config { value: i32 }
    ///
    /// let container = Container::new();
    ///
    /// // First registration succeeds
    /// assert!(container.register_if_missing(Config { value: 1 }));
    ///
    /// // Second registration is skipped
    /// assert!(!container.register_if_missing(Config { value: 2 }));
    ///
    /// // Original value is preserved
    /// assert_eq!(container.require::<Config>().value, 1);
    /// ```
    #[inline]
    pub fn register_if_missing<T: Injectable>(&self, instance: T) -> bool {
        if self.has::<T>() {
            false
        } else {
            self.register(instance);
            true
        }
    }
}

impl std::fmt::Debug for Container {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Container")
            .field("inner", &self.inner)
            .finish()
    }
}

impl From<DiContainer> for Container {
    fn from(inner: DiContainer) -> Self {
        Self {
            inner,
            lifecycle: Arc::new(OnceLock::new()),
        }
    }
}

impl AsRef<DiContainer> for Container {
    fn as_ref(&self) -> &DiContainer {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestService {
        value: String,
    }

    #[test]
    fn test_container_creation() {
        let container = Container::new();
        assert!(container.is_empty());
    }

    #[test]
    fn test_register_and_resolve() {
        let container = Container::new();
        container.register(TestService {
            value: "test".to_string(),
        });

        let resolved = container.resolve::<TestService>().unwrap();
        assert_eq!(resolved.value, "test");
    }

    #[test]
    fn test_scoped_container() {
        let parent = Container::new();
        parent.register(TestService {
            value: "parent".to_string(),
        });

        let child = parent.create_scope();

        // Child can resolve from parent
        assert!(child.has::<TestService>());
        let resolved = child.resolve::<TestService>().unwrap();
        assert_eq!(resolved.value, "parent");
    }

    #[test]
    fn test_create_scope_shares_lifecycle_manager() {
        let parent = Container::new();
        let manager = Arc::new(LifecycleManager::new());
        parent.attach_lifecycle(&manager);

        let scope = parent.create_scope();

        let scope_manager = scope
            .lifecycle_manager()
            .expect("scope should inherit the parent's attached LifecycleManager");
        assert!(
            Arc::ptr_eq(&manager, &scope_manager),
            "scope's LifecycleManager should be the same Arc instance as the parent's"
        );
    }

    #[test]
    fn test_lazy_singleton() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static CREATED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct LazyService;

        let container = Container::new();
        container.lazy(|| {
            CREATED.store(true, Ordering::SeqCst);
            LazyService
        });

        assert!(!CREATED.load(Ordering::SeqCst));

        let _ = container.get::<LazyService>().unwrap();
        assert!(CREATED.load(Ordering::SeqCst));
    }

    #[test]
    fn test_transient() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static COUNTER: AtomicU32 = AtomicU32::new(0);

        #[derive(Clone)]
        struct Counter(u32);

        let container = Container::new();
        container.transient(|| Counter(COUNTER.fetch_add(1, Ordering::SeqCst)));

        let c1 = container.get::<Counter>().unwrap();
        let c2 = container.get::<Counter>().unwrap();

        assert_ne!(c1.0, c2.0);
    }
}
