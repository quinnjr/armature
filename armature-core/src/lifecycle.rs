//! Lifecycle hook system for Armature framework.
//!
//! Provides lifecycle hooks for modules, controllers, and services similar to NestJS.
//!
//! ## Available Hooks
//!
//! - `OnModuleInit` - Called after module initialization
//! - `OnModuleDestroy` - Called before module destruction
//! - `OnApplicationBootstrap` - Called after full application bootstrap
//! - `OnApplicationShutdown` - Called during graceful shutdown
//!
//! ## Examples
//!
//! ```
//! use armature_core::lifecycle::{OnModuleInit, OnModuleDestroy};
//! use async_trait::async_trait;
//!
//! struct MyService {
//!     name: String,
//! }
//!
//! #[async_trait]
//! impl OnModuleInit for MyService {
//!     async fn on_module_init(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!         println!("Service {} initialized!", self.name);
//!         Ok(())
//!     }
//! }
//!
//! #[async_trait]
//! impl OnModuleDestroy for MyService {
//!     async fn on_module_destroy(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!         println!("Service {} destroyed!", self.name);
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ## Internal (`__`-prefixed) implementation details — no semver guarantee
//!
//! Items prefixed with `__` in this module (and anything they expand to,
//! e.g. via [`__armature_register_lifecycle_hooks`]) are internal
//! implementation details of the `#[module(...)]` / `provider_registration!`
//! macros. They carry **no semver stability guarantee** and may change or be
//! removed in any 0.x release without notice. Do not call them directly.

use async_trait::async_trait;
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Error type for lifecycle operations
pub type LifecycleResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Hook called after module dependencies are resolved
///
/// ## Example
///
/// ```
/// use armature_core::lifecycle::{OnModuleInit, LifecycleResult};
/// use async_trait::async_trait;
///
/// struct MyService;
///
/// #[async_trait]
/// impl OnModuleInit for MyService {
///     async fn on_module_init(&self) -> LifecycleResult {
///         println!("Service initialized!");
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait OnModuleInit: Send + Sync {
    /// Called once the module has been initialized
    async fn on_module_init(&self) -> LifecycleResult;
}

/// Hook called before module is destroyed
#[async_trait]
pub trait OnModuleDestroy: Send + Sync {
    /// Called before the module is destroyed
    async fn on_module_destroy(&self) -> LifecycleResult;
}

/// Hook called after all modules have been initialized
#[async_trait]
pub trait OnApplicationBootstrap: Send + Sync {
    /// Called once the application has fully started
    async fn on_application_bootstrap(&self) -> LifecycleResult;
}

/// Hook called during application shutdown
#[async_trait]
pub trait OnApplicationShutdown: Send + Sync {
    /// Called when the application is shutting down
    async fn on_application_shutdown(&self, signal: Option<String>) -> LifecycleResult;
}

/// Hook called before module initialization
#[async_trait]
pub trait BeforeApplicationShutdown: Send + Sync {
    /// Called before application shutdown hooks
    async fn before_application_shutdown(&self, signal: Option<String>) -> LifecycleResult;
}

/// Manages lifecycle hooks for all registered components
///
/// ## Example
///
/// ```
/// use armature_core::lifecycle::{LifecycleManager, OnModuleInit, LifecycleResult};
/// use async_trait::async_trait;
/// use std::sync::Arc;
///
/// struct TestService;
///
/// #[async_trait]
/// impl OnModuleInit for TestService {
///     async fn on_module_init(&self) -> LifecycleResult {
///         Ok(())
///     }
/// }
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let manager = LifecycleManager::new();
/// let service = Arc::new(TestService);
///
/// manager.register_on_init("TestService".to_string(), service).await;
/// manager.call_module_init_hooks().await.unwrap();
/// # });
/// ```
#[allow(clippy::type_complexity)]
pub struct LifecycleManager {
    init_hooks: Arc<RwLock<Vec<(String, Arc<dyn OnModuleInit>)>>>,
    destroy_hooks: Arc<RwLock<Vec<(String, Arc<dyn OnModuleDestroy>)>>>,
    bootstrap_hooks: Arc<RwLock<Vec<(String, Arc<dyn OnApplicationBootstrap>)>>>,
    shutdown_hooks: Arc<RwLock<Vec<(String, Arc<dyn OnApplicationShutdown>)>>>,
    before_shutdown_hooks: Arc<RwLock<Vec<(String, Arc<dyn BeforeApplicationShutdown>)>>>,
    hook_registry: Arc<RwLock<HashMap<TypeId, String>>>,
}

impl LifecycleManager {
    /// Create a new lifecycle manager
    ///
    /// ## Example
    ///
    /// ```
    /// use armature_core::lifecycle::LifecycleManager;
    ///
    /// let manager = LifecycleManager::new();
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let counts = manager.hook_counts().await;
    /// assert_eq!(counts.init, 0);
    /// # });
    /// ```
    pub fn new() -> Self {
        Self {
            init_hooks: Arc::new(RwLock::new(Vec::new())),
            destroy_hooks: Arc::new(RwLock::new(Vec::new())),
            bootstrap_hooks: Arc::new(RwLock::new(Vec::new())),
            shutdown_hooks: Arc::new(RwLock::new(Vec::new())),
            before_shutdown_hooks: Arc::new(RwLock::new(Vec::new())),
            hook_registry: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a type name for tracking purposes
    pub async fn register_type(&self, type_id: TypeId, name: String) {
        let mut registry = self.hook_registry.write().await;
        registry.insert(type_id, name);
    }

    /// Get the registered name for a type
    pub async fn get_type_name(&self, type_id: TypeId) -> Option<String> {
        let registry = self.hook_registry.read().await;
        registry.get(&type_id).cloned()
    }

    /// Register an OnModuleInit hook
    pub async fn register_on_init(&self, name: String, hook: Arc<dyn OnModuleInit>) {
        let mut hooks = self.init_hooks.write().await;
        hooks.push((name, hook));
    }

    /// Register an OnModuleDestroy hook
    pub async fn register_on_destroy(&self, name: String, hook: Arc<dyn OnModuleDestroy>) {
        let mut hooks = self.destroy_hooks.write().await;
        hooks.push((name, hook));
    }

    /// Register an OnApplicationBootstrap hook
    pub async fn register_on_bootstrap(&self, name: String, hook: Arc<dyn OnApplicationBootstrap>) {
        let mut hooks = self.bootstrap_hooks.write().await;
        hooks.push((name, hook));
    }

    /// Register an OnApplicationShutdown hook
    pub async fn register_on_shutdown(&self, name: String, hook: Arc<dyn OnApplicationShutdown>) {
        let mut hooks = self.shutdown_hooks.write().await;
        hooks.push((name, hook));
    }

    /// Register a BeforeApplicationShutdown hook
    pub async fn register_before_shutdown(
        &self,
        name: String,
        hook: Arc<dyn BeforeApplicationShutdown>,
    ) {
        let mut hooks = self.before_shutdown_hooks.write().await;
        hooks.push((name, hook));
    }

    /// Synchronous, best-effort variant of [`Self::register_on_init`].
    ///
    /// Provider registration (see [`crate::container::Container::attach_lifecycle`]
    /// and [`crate::module::provider_registration`]) runs synchronously during
    /// [`crate::Application::create`], so it cannot `.await` the methods
    /// above. Registration happens during single-threaded application
    /// startup, before this manager is shared with any other task, so the
    /// lock is never contended in practice; `try_write` is used instead of
    /// blocking so a pathological caller can never deadlock startup. On the
    /// vanishingly unlikely event it can't acquire the lock, the hook is
    /// skipped rather than blocking, and a `tracing::warn!` is emitted so the
    /// dropped registration leaves a diagnostic trail.
    pub fn register_on_init_sync(&self, name: String, hook: Arc<dyn OnModuleInit>) {
        match self.init_hooks.try_write() {
            Ok(mut hooks) => hooks.push((name, hook)),
            Err(_) => tracing::warn!(
                hook = %name,
                "Failed to register OnModuleInit hook: lock contended, hook was NOT registered"
            ),
        }
    }

    /// Synchronous, best-effort variant of [`Self::register_on_destroy`]. See
    /// [`Self::register_on_init_sync`] for why this exists.
    pub fn register_on_destroy_sync(&self, name: String, hook: Arc<dyn OnModuleDestroy>) {
        match self.destroy_hooks.try_write() {
            Ok(mut hooks) => hooks.push((name, hook)),
            Err(_) => tracing::warn!(
                hook = %name,
                "Failed to register OnModuleDestroy hook: lock contended, hook was NOT registered"
            ),
        }
    }

    /// Synchronous, best-effort variant of [`Self::register_on_bootstrap`].
    /// See [`Self::register_on_init_sync`] for why this exists.
    pub fn register_on_bootstrap_sync(&self, name: String, hook: Arc<dyn OnApplicationBootstrap>) {
        match self.bootstrap_hooks.try_write() {
            Ok(mut hooks) => hooks.push((name, hook)),
            Err(_) => tracing::warn!(
                hook = %name,
                "Failed to register OnApplicationBootstrap hook: lock contended, hook was NOT registered"
            ),
        }
    }

    /// Synchronous, best-effort variant of [`Self::register_on_shutdown`].
    /// See [`Self::register_on_init_sync`] for why this exists.
    pub fn register_on_shutdown_sync(&self, name: String, hook: Arc<dyn OnApplicationShutdown>) {
        match self.shutdown_hooks.try_write() {
            Ok(mut hooks) => hooks.push((name, hook)),
            Err(_) => tracing::warn!(
                hook = %name,
                "Failed to register OnApplicationShutdown hook: lock contended, hook was NOT registered"
            ),
        }
    }

    /// Execute all OnModuleInit hooks
    pub async fn call_module_init_hooks(
        &self,
    ) -> Result<(), Vec<(String, Box<dyn std::error::Error + Send + Sync>)>> {
        tracing::info!("🔄 Calling module initialization hooks...");
        // Snapshot the registrations and drop the read guard before awaiting
        // any hook. `tokio::sync::RwLock` is write-preferring, so holding the
        // guard across the whole loop would stall every concurrent
        // `register_on_init` for the full duration of every hook, deadlock a
        // hook that registers another hook on this same manager, and cause
        // `register_on_init_sync`'s `try_write` to fail -- silently dropping
        // that registration. The entries are `(String, Arc<dyn ..>)`, so the
        // clone is just a string copy plus a refcount bump.
        let hooks: Vec<_> = self.init_hooks.read().await.clone();
        let mut errors = Vec::new();

        for (name, hook) in hooks.iter() {
            match hook.on_module_init().await {
                Ok(_) => {
                    tracing::info!("  ✓ {}: onModuleInit() completed", name);
                }
                Err(e) => {
                    tracing::error!("  ✗ {}: onModuleInit() failed: {}", name, e);
                    errors.push((name.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Execute all OnModuleDestroy hooks
    pub async fn call_module_destroy_hooks(
        &self,
    ) -> Result<(), Vec<(String, Box<dyn std::error::Error + Send + Sync>)>> {
        tracing::info!("🔄 Calling module destruction hooks...");
        // Snapshot first, then drop the guard -- see `call_module_init_hooks`
        // for why the read guard must not be held across hook `.await`s.
        let hooks: Vec<_> = self.destroy_hooks.read().await.clone();
        let mut errors = Vec::new();

        // Call in reverse order (LIFO)
        for (name, hook) in hooks.iter().rev() {
            match hook.on_module_destroy().await {
                Ok(_) => {
                    tracing::info!("  ✓ {}: onModuleDestroy() completed", name);
                }
                Err(e) => {
                    tracing::error!("  ✗ {}: onModuleDestroy() failed: {}", name, e);
                    errors.push((name.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Execute all OnApplicationBootstrap hooks
    pub async fn call_bootstrap_hooks(
        &self,
    ) -> Result<(), Vec<(String, Box<dyn std::error::Error + Send + Sync>)>> {
        tracing::info!("🚀 Calling application bootstrap hooks...");
        // Snapshot first, then drop the guard -- see `call_module_init_hooks`
        // for why the read guard must not be held across hook `.await`s.
        let hooks: Vec<_> = self.bootstrap_hooks.read().await.clone();
        let mut errors = Vec::new();

        for (name, hook) in hooks.iter() {
            match hook.on_application_bootstrap().await {
                Ok(_) => {
                    tracing::info!("  ✓ {}: onApplicationBootstrap() completed", name);
                }
                Err(e) => {
                    tracing::error!("  ✗ {}: onApplicationBootstrap() failed: {}", name, e);
                    errors.push((name.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Execute BeforeApplicationShutdown hooks
    pub async fn call_before_shutdown_hooks(
        &self,
        signal: Option<String>,
    ) -> Result<(), Vec<(String, Box<dyn std::error::Error + Send + Sync>)>> {
        tracing::info!("⚠️  Calling before shutdown hooks...");
        // Snapshot first, then drop the guard -- see `call_module_init_hooks`
        // for why the read guard must not be held across hook `.await`s.
        let hooks: Vec<_> = self.before_shutdown_hooks.read().await.clone();
        let mut errors = Vec::new();

        for (name, hook) in hooks.iter() {
            match hook.before_application_shutdown(signal.clone()).await {
                Ok(_) => {
                    tracing::info!("  ✓ {}: beforeApplicationShutdown() completed", name);
                }
                Err(e) => {
                    tracing::error!("  ✗ {}: beforeApplicationShutdown() failed: {}", name, e);
                    errors.push((name.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Execute all OnApplicationShutdown hooks
    ///
    /// ## Example
    ///
    /// ```
    /// use armature_core::lifecycle::{LifecycleManager, OnApplicationShutdown, LifecycleResult};
    /// use async_trait::async_trait;
    /// use std::sync::Arc;
    ///
    /// struct ShutdownService;
    ///
    /// #[async_trait]
    /// impl OnApplicationShutdown for ShutdownService {
    ///     async fn on_application_shutdown(&self, signal: Option<String>) -> LifecycleResult {
    ///         println!("Shutting down with signal: {:?}", signal);
    ///         Ok(())
    ///     }
    /// }
    ///
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// let manager = LifecycleManager::new();
    /// let service = Arc::new(ShutdownService);
    ///
    /// manager.register_on_shutdown("ShutdownService".to_string(), service).await;
    /// manager.call_shutdown_hooks(Some("SIGTERM".to_string())).await.unwrap();
    /// # });
    /// ```
    pub async fn call_shutdown_hooks(
        &self,
        signal: Option<String>,
    ) -> Result<(), Vec<(String, Box<dyn std::error::Error + Send + Sync>)>> {
        tracing::info!("🛑 Calling application shutdown hooks...");
        // Snapshot first, then drop the guard -- see `call_module_init_hooks`
        // for why the read guard must not be held across hook `.await`s.
        let hooks: Vec<_> = self.shutdown_hooks.read().await.clone();
        let mut errors = Vec::new();

        // Call in reverse order (LIFO)
        for (name, hook) in hooks.iter().rev() {
            match hook.on_application_shutdown(signal.clone()).await {
                Ok(_) => {
                    tracing::info!("  ✓ {}: onApplicationShutdown() completed", name);
                }
                Err(e) => {
                    tracing::error!("  ✗ {}: onApplicationShutdown() failed: {}", name, e);
                    errors.push((name.clone(), e));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Get the number of registered hooks of each type
    pub async fn hook_counts(&self) -> LifecycleHookCounts {
        LifecycleHookCounts {
            init: self.init_hooks.read().await.len(),
            destroy: self.destroy_hooks.read().await.len(),
            bootstrap: self.bootstrap_hooks.read().await.len(),
            shutdown: self.shutdown_hooks.read().await.len(),
            before_shutdown: self.before_shutdown_hooks.read().await.len(),
        }
    }

    /// Clear all registered hooks
    pub async fn clear(&self) {
        self.init_hooks.write().await.clear();
        self.destroy_hooks.write().await.clear();
        self.bootstrap_hooks.write().await.clear();
        self.shutdown_hooks.write().await.clear();
        self.before_shutdown_hooks.write().await.clear();
        self.hook_registry.write().await.clear();
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about registered lifecycle hooks
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleHookCounts {
    pub init: usize,
    pub destroy: usize,
    pub bootstrap: usize,
    pub shutdown: usize,
    pub before_shutdown: usize,
}

// ============================================================================
// Automatic lifecycle hook discovery
// ============================================================================
//
// Providers are registered into the DI container behind a plain
// `fn(&Container)` pointer (`ProviderRegistration::register_fn` in
// `traits.rs`), generated once per concrete provider type by
// `armature_proc_macro`'s `#[module(...)]` codegen (and by the
// [`crate::module::provider_registration`] macro, for hand-built modules).
// That closure is the only place in the framework where a provider's
// *concrete* type is still known: once it's registered, the instance is
// erased into `Arc<dyn Any + Send + Sync>` and nothing can recover which
// lifecycle traits (if any) it implements.
//
// [`__armature_register_lifecycle_hooks`] is invoked from inside that
// closure (which is monomorphic per provider type, since the macro
// substitutes the concrete type textually) to conditionally register the
// instance against each lifecycle trait it implements, using "autoref-based
// stable specialization": for each trait `X`, a `__ViaXSpecific` impl exists
// only for `&__LifecycleProbe<T> where T: X`, while a `__ViaXFallback` impl
// (no-op) exists unconditionally for `__LifecycleProbe<T>`. Calling
// `(&&probe).method()` prefers the `&__LifecycleProbe<T>`-level impl (found
// one deref step earlier in method resolution) when it exists, and falls
// back to the no-op otherwise.
//
// Crucially, this only works when the call site is monomorphic (a concrete
// `T`, not a generic type parameter still under a `where` bound): inside a
// generic function, method resolution can only use the bounds declared on
// that function, not whatever `T` is eventually monomorphized to, so the
// specific impl would never be selected. Do not wrap this macro in a
// generic helper function for that reason -- always invoke it directly from
// a concrete, per-type call site (i.e. from within macro-generated code
// where the provider type has already been substituted).

// NOTE: everything below this point (`__LifecycleProbe`, the `__ViaX*`
// traits, and `__armature_register_lifecycle_hooks!`) is `__`-prefixed and
// carries NO semver stability guarantee -- see the "Internal implementation
// details" section of the module-level docs at the top of this file (not
// itself `#[doc(hidden)]`, so it renders normally in rustdoc/IDE tooltips
// for the `lifecycle` module) for the full policy statement. Do not call
// these directly.

/// Wrapper used by the autoref-specialization trick below. Not part of the
/// public API.
#[doc(hidden)]
pub struct __LifecycleProbe<T>(pub Arc<T>);

// Generates one `__ViaXFallback`/`__ViaXSpecific` trait pair (see the
// autoref-specialization explanation above) for a single lifecycle trait.
// The four invocations below are structurally identical modulo the
// substituted identifiers -- this macro exists purely to avoid hand-copying
// that boilerplate four times; it produces the same code the hand-written
// pairs used to.
macro_rules! define_lifecycle_probe_via {
    ($fallback:ident, $specific:ident, $bound:ident, $method:ident, $register_fn:ident) => {
        #[doc(hidden)]
        pub trait $fallback {
            fn $method(&self, _manager: &LifecycleManager, _name: &str) {}
        }
        impl<T> $fallback for __LifecycleProbe<T> {}

        #[doc(hidden)]
        pub trait $specific {
            fn $method(&self, manager: &LifecycleManager, name: &str);
        }
        impl<T: $bound + 'static> $specific for &__LifecycleProbe<T> {
            fn $method(&self, manager: &LifecycleManager, name: &str) {
                manager.$register_fn(name.to_string(), self.0.clone() as Arc<dyn $bound>);
            }
        }
    };
}

define_lifecycle_probe_via!(
    __ViaOnModuleInitFallback,
    __ViaOnModuleInitSpecific,
    OnModuleInit,
    __maybe_register_on_init,
    register_on_init_sync
);

define_lifecycle_probe_via!(
    __ViaOnModuleDestroyFallback,
    __ViaOnModuleDestroySpecific,
    OnModuleDestroy,
    __maybe_register_on_destroy,
    register_on_destroy_sync
);

define_lifecycle_probe_via!(
    __ViaOnApplicationBootstrapFallback,
    __ViaOnApplicationBootstrapSpecific,
    OnApplicationBootstrap,
    __maybe_register_on_bootstrap,
    register_on_bootstrap_sync
);

define_lifecycle_probe_via!(
    __ViaOnApplicationShutdownFallback,
    __ViaOnApplicationShutdownSpecific,
    OnApplicationShutdown,
    __maybe_register_on_shutdown,
    register_on_shutdown_sync
);

/// Probe a provider instance (`Arc<T>`) for lifecycle hook trait
/// implementations and register any it implements with `$manager`. See the
/// module-level comment above for how this works and why it must be
/// invoked from a monomorphic call site.
///
/// Not part of the public API; used by
/// [`crate::module::provider_registration`] and by `armature_proc_macro`'s
/// `#[module(...)]` codegen.
#[doc(hidden)]
#[macro_export]
macro_rules! __armature_register_lifecycle_hooks {
    ($manager:expr, $name:expr, $instance:expr) => {{
        #[allow(unused_imports)]
        use $crate::lifecycle::{
            __ViaOnApplicationBootstrapFallback as _, __ViaOnApplicationBootstrapSpecific as _,
            __ViaOnApplicationShutdownFallback as _, __ViaOnApplicationShutdownSpecific as _,
            __ViaOnModuleDestroyFallback as _, __ViaOnModuleDestroySpecific as _,
            __ViaOnModuleInitFallback as _, __ViaOnModuleInitSpecific as _,
        };
        let __armature_probe = $crate::lifecycle::__LifecycleProbe($instance.clone());
        (&&__armature_probe).__maybe_register_on_init($manager, $name);
        (&&__armature_probe).__maybe_register_on_destroy($manager, $name);
        (&&__armature_probe).__maybe_register_on_bootstrap($manager, $name);
        (&&__armature_probe).__maybe_register_on_shutdown($manager, $name);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[allow(dead_code)]
    struct TestService {
        name: String,
        init_called: Arc<RwLock<bool>>,
        destroy_called: Arc<RwLock<bool>>,
    }

    #[async_trait]
    impl OnModuleInit for TestService {
        async fn on_module_init(&self) -> LifecycleResult {
            *self.init_called.write().await = true;
            Ok(())
        }
    }

    #[async_trait]
    impl OnModuleDestroy for TestService {
        async fn on_module_destroy(&self) -> LifecycleResult {
            *self.destroy_called.write().await = true;
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_lifecycle_manager_registration() {
        let manager = LifecycleManager::new();
        let init_called = Arc::new(RwLock::new(false));
        let destroy_called = Arc::new(RwLock::new(false));

        let service = Arc::new(TestService {
            name: "TestService".to_string(),
            init_called: init_called.clone(),
            destroy_called: destroy_called.clone(),
        });

        manager
            .register_on_init("TestService".to_string(), service.clone())
            .await;
        manager
            .register_on_destroy("TestService".to_string(), service.clone())
            .await;

        let counts = manager.hook_counts().await;
        assert_eq!(counts.init, 1);
        assert_eq!(counts.destroy, 1);
    }

    #[tokio::test]
    async fn test_lifecycle_hooks_execution() {
        let manager = LifecycleManager::new();
        let init_called = Arc::new(RwLock::new(false));
        let destroy_called = Arc::new(RwLock::new(false));

        let service = Arc::new(TestService {
            name: "TestService".to_string(),
            init_called: init_called.clone(),
            destroy_called: destroy_called.clone(),
        });

        manager
            .register_on_init("TestService".to_string(), service.clone())
            .await;
        manager
            .register_on_destroy("TestService".to_string(), service.clone())
            .await;

        // Execute init hooks
        manager.call_module_init_hooks().await.unwrap();
        assert!(*init_called.read().await);

        // Execute destroy hooks
        manager.call_module_destroy_hooks().await.unwrap();
        assert!(*destroy_called.read().await);
    }

    #[tokio::test]
    async fn test_lifecycle_hook_order() {
        let manager = LifecycleManager::new();
        let order = Arc::new(RwLock::new(Vec::new()));

        struct OrderService {
            id: usize,
            order: Arc<RwLock<Vec<usize>>>,
        }

        #[async_trait]
        impl OnModuleInit for OrderService {
            async fn on_module_init(&self) -> LifecycleResult {
                self.order.write().await.push(self.id);
                Ok(())
            }
        }

        for i in 1..=3 {
            let service = Arc::new(OrderService {
                id: i,
                order: order.clone(),
            });
            manager
                .register_on_init(format!("Service{}", i), service)
                .await;
        }

        manager.call_module_init_hooks().await.unwrap();

        let execution_order = order.read().await.clone();
        assert_eq!(execution_order, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_destroy_hooks_run_in_reverse_registration_order() {
        // Teardown must unwind in LIFO order: a provider registered later may
        // depend on one registered earlier, so it has to be torn down first.
        let manager = LifecycleManager::new();
        let order = Arc::new(RwLock::new(Vec::new()));

        struct OrderService {
            id: usize,
            order: Arc<RwLock<Vec<usize>>>,
        }

        #[async_trait]
        impl OnModuleDestroy for OrderService {
            async fn on_module_destroy(&self) -> LifecycleResult {
                self.order.write().await.push(self.id);
                Ok(())
            }
        }

        for i in 1..=3 {
            let service = Arc::new(OrderService {
                id: i,
                order: order.clone(),
            });
            manager
                .register_on_destroy(format!("Service{}", i), service)
                .await;
        }

        manager.call_module_destroy_hooks().await.unwrap();

        let execution_order = order.read().await.clone();
        assert_eq!(execution_order, vec![3, 2, 1]);
    }

    #[tokio::test]
    async fn test_shutdown_hooks_run_in_reverse_registration_order() {
        // Same LIFO contract as destroy: shutdown unwinds registration order.
        let manager = LifecycleManager::new();
        let order = Arc::new(RwLock::new(Vec::new()));

        struct OrderService {
            id: usize,
            order: Arc<RwLock<Vec<usize>>>,
        }

        #[async_trait]
        impl OnApplicationShutdown for OrderService {
            async fn on_application_shutdown(&self, _signal: Option<String>) -> LifecycleResult {
                self.order.write().await.push(self.id);
                Ok(())
            }
        }

        for i in 1..=3 {
            let service = Arc::new(OrderService {
                id: i,
                order: order.clone(),
            });
            manager
                .register_on_shutdown(format!("Service{}", i), service)
                .await;
        }

        manager
            .call_shutdown_hooks(Some("SIGTERM".to_string()))
            .await
            .unwrap();

        let execution_order = order.read().await.clone();
        assert_eq!(execution_order, vec![3, 2, 1]);
    }

    #[tokio::test]
    async fn test_failing_init_hook_is_reported_by_name_and_does_not_abort_the_rest() {
        // The `Vec<(String, Box<dyn Error>)>` error channel only earns its
        // keep if the failing hook's *registered name* comes back with it --
        // that name is the only handle an operator has on which provider
        // broke. Hook execution is also continue-on-error by construction, so
        // a hook registered after the failure must still run.
        let manager = LifecycleManager::new();
        let later_ran = Arc::new(AtomicBool::new(false));

        struct FailingService;

        #[async_trait]
        impl OnModuleInit for FailingService {
            async fn on_module_init(&self) -> LifecycleResult {
                Err("boom".into())
            }
        }

        struct FlagService {
            ran: Arc<AtomicBool>,
        }

        #[async_trait]
        impl OnModuleInit for FlagService {
            async fn on_module_init(&self) -> LifecycleResult {
                self.ran.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        manager
            .register_on_init("FailingService".to_string(), Arc::new(FailingService))
            .await;
        manager
            .register_on_init(
                "FlagService".to_string(),
                Arc::new(FlagService {
                    ran: later_ran.clone(),
                }),
            )
            .await;

        let errors = manager
            .call_module_init_hooks()
            .await
            .expect_err("a failing hook must surface as Err");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "FailingService");
        assert!(
            later_ran.load(Ordering::SeqCst),
            "hooks registered after a failing one must still run"
        );
    }

    // ---- sync registration variants (used by the provider registration
    // path, which cannot `.await`) --------------------------------------

    #[tokio::test]
    async fn test_sync_register_methods_are_visible_to_async_call_hooks() {
        struct SyncService {
            init_called: Arc<AtomicBool>,
            destroy_called: Arc<AtomicBool>,
            bootstrap_called: Arc<AtomicBool>,
            shutdown_called: Arc<AtomicBool>,
        }

        #[async_trait]
        impl OnModuleInit for SyncService {
            async fn on_module_init(&self) -> LifecycleResult {
                self.init_called.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
        #[async_trait]
        impl OnModuleDestroy for SyncService {
            async fn on_module_destroy(&self) -> LifecycleResult {
                self.destroy_called.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
        #[async_trait]
        impl OnApplicationBootstrap for SyncService {
            async fn on_application_bootstrap(&self) -> LifecycleResult {
                self.bootstrap_called.store(true, Ordering::SeqCst);
                Ok(())
            }
        }
        #[async_trait]
        impl OnApplicationShutdown for SyncService {
            async fn on_application_shutdown(&self, _signal: Option<String>) -> LifecycleResult {
                self.shutdown_called.store(true, Ordering::SeqCst);
                Ok(())
            }
        }

        let init_called = Arc::new(AtomicBool::new(false));
        let destroy_called = Arc::new(AtomicBool::new(false));
        let bootstrap_called = Arc::new(AtomicBool::new(false));
        let shutdown_called = Arc::new(AtomicBool::new(false));
        let service = Arc::new(SyncService {
            init_called: init_called.clone(),
            destroy_called: destroy_called.clone(),
            bootstrap_called: bootstrap_called.clone(),
            shutdown_called: shutdown_called.clone(),
        });

        let manager = LifecycleManager::new();
        manager.register_on_init_sync("SyncService".to_string(), service.clone());
        manager.register_on_destroy_sync("SyncService".to_string(), service.clone());
        manager.register_on_bootstrap_sync("SyncService".to_string(), service.clone());
        manager.register_on_shutdown_sync("SyncService".to_string(), service.clone());

        let counts = manager.hook_counts().await;
        assert_eq!(counts.init, 1);
        assert_eq!(counts.destroy, 1);
        assert_eq!(counts.bootstrap, 1);
        assert_eq!(counts.shutdown, 1);

        manager.call_module_init_hooks().await.unwrap();
        manager.call_bootstrap_hooks().await.unwrap();
        manager.call_shutdown_hooks(None).await.unwrap();
        manager.call_module_destroy_hooks().await.unwrap();

        assert!(init_called.load(Ordering::SeqCst));
        assert!(destroy_called.load(Ordering::SeqCst));
        assert!(bootstrap_called.load(Ordering::SeqCst));
        assert!(shutdown_called.load(Ordering::SeqCst));
    }

    // ---- autoref-specialization probe (Finding 1) -----------------------

    struct ProbeHasInitAndDestroy {
        init: Arc<AtomicBool>,
        destroy: Arc<AtomicBool>,
    }
    #[async_trait]
    impl OnModuleInit for ProbeHasInitAndDestroy {
        async fn on_module_init(&self) -> LifecycleResult {
            self.init.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    #[async_trait]
    impl OnModuleDestroy for ProbeHasInitAndDestroy {
        async fn on_module_destroy(&self) -> LifecycleResult {
            self.destroy.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    struct ProbePlain;

    #[tokio::test]
    async fn test_lifecycle_probe_macro_registers_only_implemented_traits() {
        let manager = LifecycleManager::new();

        let init = Arc::new(AtomicBool::new(false));
        let destroy = Arc::new(AtomicBool::new(false));
        let full_instance = Arc::new(ProbeHasInitAndDestroy {
            init: init.clone(),
            destroy: destroy.clone(),
        });
        // This call site is monomorphic (concrete `ProbeHasInitAndDestroy`),
        // exactly like a macro-generated `register_fn` closure body.
        crate::__armature_register_lifecycle_hooks!(
            &manager,
            "ProbeHasInitAndDestroy",
            full_instance
        );

        let plain_instance = Arc::new(ProbePlain);
        crate::__armature_register_lifecycle_hooks!(&manager, "ProbePlain", plain_instance);

        let counts = manager.hook_counts().await;
        assert_eq!(
            counts.init, 1,
            "only the OnModuleInit-implementing provider should register an init hook"
        );
        assert_eq!(
            counts.destroy, 1,
            "only the OnModuleDestroy-implementing provider should register a destroy hook"
        );
        assert_eq!(counts.bootstrap, 0);
        assert_eq!(counts.shutdown, 0);
    }
}
