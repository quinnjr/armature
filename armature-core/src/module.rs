//! Module system for organizing and composing application components.
//!
//! Modules are the building blocks of Armature applications. They encapsulate
//! providers, controllers, guards, and other components, and can be composed
//! through imports and exports.
//!
//! ## Module Basics
//!
//! ```rust
//! use armature_core::{Module, ProviderRegistration, ControllerRegistration, GuardRegistration};
//! use std::any::TypeId;
//!
//! struct UsersModule;
//!
//! impl Module for UsersModule {
//!     fn providers(&self) -> Vec<ProviderRegistration> {
//!         vec![]  // User service, repository, etc.
//!     }
//!
//!     fn controllers(&self) -> Vec<ControllerRegistration> {
//!         vec![]  // User controller
//!     }
//!
//!     fn imports(&self) -> Vec<Box<dyn Module>> {
//!         vec![]  // Other modules this depends on
//!     }
//!
//!     fn exports(&self) -> Vec<TypeId> {
//!         vec![]  // Providers to expose to parent modules
//!     }
//! }
//! ```
//!
//! ## Module Composition
//!
//! When Module A imports Module B:
//! - B's providers are registered in the container
//! - B's controllers are registered with the router
//! - B's exported providers become available to A
//! - B's guards become available to A's controllers (if exported)
//!
//! ```rust
//! use armature_core::Module;
//!
//! // Database module exports DatabaseService
//! struct DatabaseModule;
//!
//! // Users module imports Database module and uses DatabaseService
//! struct UsersModule;
//!
//! // App module imports both
//! struct AppModule;
//! ```
//!
//! ## Global Modules
//!
//! Global modules are automatically available to all other modules without
//! explicit imports. Use sparingly for truly cross-cutting concerns.
//!
//! ```rust
//! use armature_core::ModuleMetadata;
//!
//! // A module can be marked as global in its metadata
//! // Global modules are registered once and available everywhere
//! ```
//!
//! ## Dynamic Modules
//!
//! Dynamic modules can be configured at runtime:
//!
//! ```ignore
//! use armature_core::{DynamicModule, ModuleBuilder};
//!
//! struct DatabaseModule;
//!
//! impl DatabaseModule {
//!     pub fn for_root(connection_string: &str) -> DynamicModule {
//!         DynamicModule::new("DatabaseModule")
//!             .with_provider(/* database connection */)
//!             .export_all()
//!     }
//!
//!     pub fn for_feature(schema: &str) -> DynamicModule {
//!         DynamicModule::new("DatabaseModule")
//!             .with_provider(/* schema-specific repository */)
//!     }
//! }
//! ```

use crate::{Container, ControllerRegistration, Error, Guard, ProviderRegistration, Router};
use std::any::TypeId;
use std::collections::HashSet;
use std::sync::Arc;

/// Registration information for a guard
#[derive(Clone)]
pub struct GuardRegistration {
    /// TypeId of the guard
    pub type_id: TypeId,
    /// Human-readable type name
    pub type_name: &'static str,
    /// Factory function to create the guard
    pub factory: fn(&Container) -> Result<Arc<dyn Guard>, Error>,
}

impl std::fmt::Debug for GuardRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuardRegistration")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .finish()
    }
}

/// Metadata about a module
#[derive(Debug, Clone, Default)]
pub struct ModuleMetadata {
    /// Whether this module is global (auto-imported everywhere)
    pub global: bool,
    /// Human-readable module name
    pub name: Option<String>,
    /// Module version
    pub version: Option<String>,
    /// Description
    pub description: Option<String>,
}

impl ModuleMetadata {
    /// Create new metadata
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark module as global
    pub fn global(mut self) -> Self {
        self.global = true;
        self
    }

    /// Set module name
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    /// Set module version
    pub fn version(mut self, version: &str) -> Self {
        self.version = Some(version.to_string());
        self
    }

    /// Set module description
    pub fn description(mut self, description: &str) -> Self {
        self.description = Some(description.to_string());
        self
    }
}

/// Extended module trait with guards and metadata support
pub trait ModuleExt: Send + Sync + 'static {
    /// Returns the `TypeId` of the concrete type implementing this trait.
    ///
    /// Do not override this: like [`crate::traits::Module::module_type_id`],
    /// the default implementation is monomorphized per concrete `Self`, so
    /// its vtable entry always reports the *concrete* module type, even
    /// when called through a `&dyn ModuleExt` trait object. Used by
    /// [`ModuleRegistry::register_module`] to dedupe diamond-imported
    /// modules and guard against cyclic import graphs.
    fn module_type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }

    /// Returns the key used to deduplicate this module during registration,
    /// or `None` to opt this module out of dedup entirely.
    ///
    /// The default forwards to [`ModuleExt::module_type_id`], preserving the
    /// existing dedup-by-concrete-type behavior for ordinary modules. Types
    /// like [`DynamicModule`] where multiple distinct, differently
    /// configured instances of the same concrete type are an expected,
    /// legitimate pattern (e.g. the `for_root()`/`for_feature()`
    /// configurable-module pattern) override this to return `None` so each
    /// instance registers independently instead of every instance
    /// collapsing into the first one registered.
    fn dedup_key(&self) -> Option<std::any::TypeId> {
        Some(self.module_type_id())
    }

    /// Returns the list of provider types to register
    fn providers(&self) -> Vec<ProviderRegistration> {
        vec![]
    }

    /// Returns the list of controller types to register
    fn controllers(&self) -> Vec<ControllerRegistration> {
        vec![]
    }

    /// Returns the list of guard types to register
    fn guards(&self) -> Vec<GuardRegistration> {
        vec![]
    }

    /// Returns the list of imported modules
    fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
        vec![]
    }

    /// Returns the list of exported provider TypeIds
    fn exports(&self) -> Vec<TypeId> {
        vec![]
    }

    /// Returns the list of re-exported modules
    ///
    /// When a module re-exports another module, the re-exported module's
    /// exports become available to anyone importing this module.
    fn re_exports(&self) -> Vec<Box<dyn ModuleExt>> {
        vec![]
    }

    /// Returns module metadata
    fn metadata(&self) -> ModuleMetadata {
        ModuleMetadata::default()
    }

    /// Called when the module is initialized
    ///
    /// Override to perform setup tasks like registering middleware.
    fn on_module_init(&self, _container: &Container) -> Result<(), Error> {
        Ok(())
    }

    /// Called when the module is destroyed
    fn on_module_destroy(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// A dynamically configured module
#[derive(Clone)]
pub struct DynamicModule {
    /// Module name
    pub name: String,
    providers: Vec<ProviderRegistration>,
    controllers: Vec<ControllerRegistration>,
    guards: Vec<GuardRegistration>,
    imports: Vec<Arc<dyn ModuleExt>>,
    exports: Vec<TypeId>,
    re_exports: Vec<Arc<dyn ModuleExt>>,
    metadata: ModuleMetadata,
    export_all: bool,
}

impl DynamicModule {
    /// Create a new dynamic module with the given name
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            providers: vec![],
            controllers: vec![],
            guards: vec![],
            imports: vec![],
            exports: vec![],
            re_exports: vec![],
            metadata: ModuleMetadata::default().name(name),
            export_all: false,
        }
    }

    /// Add a provider registration
    pub fn with_provider(mut self, provider: ProviderRegistration) -> Self {
        self.providers.push(provider);
        self
    }

    /// Add multiple provider registrations
    pub fn with_providers(mut self, providers: Vec<ProviderRegistration>) -> Self {
        self.providers.extend(providers);
        self
    }

    /// Add a controller registration
    pub fn with_controller(mut self, controller: ControllerRegistration) -> Self {
        self.controllers.push(controller);
        self
    }

    /// Add multiple controller registrations
    pub fn with_controllers(mut self, controllers: Vec<ControllerRegistration>) -> Self {
        self.controllers.extend(controllers);
        self
    }

    /// Add a guard registration
    pub fn with_guard(mut self, guard: GuardRegistration) -> Self {
        self.guards.push(guard);
        self
    }

    /// Add multiple guard registrations
    pub fn with_guards(mut self, guards: Vec<GuardRegistration>) -> Self {
        self.guards.extend(guards);
        self
    }

    /// Import another module
    pub fn import<M: ModuleExt + Clone + 'static>(mut self, module: M) -> Self {
        self.imports.push(Arc::new(module));
        self
    }

    /// Import a boxed module
    pub fn import_boxed(mut self, module: Box<dyn ModuleExt>) -> Self {
        self.imports.push(Arc::from(module));
        self
    }

    /// Export a provider type
    pub fn export<T: 'static>(mut self) -> Self {
        self.exports.push(TypeId::of::<T>());
        self
    }

    /// Export a specific TypeId
    pub fn export_id(mut self, type_id: TypeId) -> Self {
        self.exports.push(type_id);
        self
    }

    /// Export all registered providers
    pub fn export_all(mut self) -> Self {
        self.export_all = true;
        self
    }

    /// Re-export another module (forward its exports)
    pub fn re_export<M: ModuleExt + Clone + 'static>(mut self, module: M) -> Self {
        self.re_exports.push(Arc::new(module));
        self
    }

    /// Mark as global module
    pub fn global(mut self) -> Self {
        self.metadata.global = true;
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: ModuleMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

impl ModuleExt for DynamicModule {
    // Multiple differently-configured `DynamicModule` instances (the
    // `for_root()`/`for_feature()` configurable-module pattern) all share
    // the same concrete type, so `module_type_id()` alone can't distinguish
    // them. Opt out of dedup entirely rather than collapsing every instance
    // into whichever one registers first.
    fn dedup_key(&self) -> Option<std::any::TypeId> {
        None
    }

    fn providers(&self) -> Vec<ProviderRegistration> {
        self.providers.clone()
    }

    fn controllers(&self) -> Vec<ControllerRegistration> {
        self.controllers.clone()
    }

    fn guards(&self) -> Vec<GuardRegistration> {
        self.guards.clone()
    }

    fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
        self.imports
            .iter()
            .map(|m| Box::new(ArcModule(m.clone())) as Box<dyn ModuleExt>)
            .collect()
    }

    fn exports(&self) -> Vec<TypeId> {
        if self.export_all {
            self.providers.iter().map(|p| p.type_id).collect()
        } else {
            self.exports.clone()
        }
    }

    fn re_exports(&self) -> Vec<Box<dyn ModuleExt>> {
        self.re_exports
            .iter()
            .map(|m| Box::new(ArcModule(m.clone())) as Box<dyn ModuleExt>)
            .collect()
    }

    fn metadata(&self) -> ModuleMetadata {
        self.metadata.clone()
    }
}

/// Wrapper to allow Arc<dyn ModuleExt> to be used as Box<dyn ModuleExt>
struct ArcModule(Arc<dyn ModuleExt>);

impl ModuleExt for ArcModule {
    // Forward to the wrapped module's own identity instead of using the
    // default (which would report `ArcModule`'s own TypeId -- the same for
    // every wrapped module, collapsing all of them together under the
    // dedup logic in `ModuleRegistry::register_module`).
    fn module_type_id(&self) -> std::any::TypeId {
        self.0.module_type_id()
    }

    // Forward the wrapped module's own dedup opt-out/key for the same
    // reason `module_type_id` is forwarded above: without this, a
    // `DynamicModule` reached through `.import()`/`.re_export()` (which
    // wrap it in `ArcModule`) would fall back to the default `dedup_key`
    // impl here and lose its `None` opt-out, silently re-collapsing
    // distinct configured instances nested under another module.
    fn dedup_key(&self) -> Option<std::any::TypeId> {
        self.0.dedup_key()
    }

    fn providers(&self) -> Vec<ProviderRegistration> {
        self.0.providers()
    }

    fn controllers(&self) -> Vec<ControllerRegistration> {
        self.0.controllers()
    }

    fn guards(&self) -> Vec<GuardRegistration> {
        self.0.guards()
    }

    fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
        self.0.imports()
    }

    fn exports(&self) -> Vec<TypeId> {
        self.0.exports()
    }

    fn re_exports(&self) -> Vec<Box<dyn ModuleExt>> {
        self.0.re_exports()
    }

    fn metadata(&self) -> ModuleMetadata {
        self.0.metadata()
    }
}

/// Collects all exports from a module including re-exports
pub fn collect_all_exports(module: &dyn ModuleExt) -> HashSet<TypeId> {
    let mut exports = HashSet::new();

    // Add direct exports
    for type_id in module.exports() {
        exports.insert(type_id);
    }

    // Add re-exported module exports
    for re_export in module.re_exports() {
        for type_id in collect_all_exports(re_export.as_ref()) {
            exports.insert(type_id);
        }
    }

    exports
}

/// Module registry for tracking registered modules
#[derive(Default)]
pub struct ModuleRegistry {
    /// Set of registered module TypeIds to prevent duplicate registration
    registered: HashSet<TypeId>,
    /// Global modules that should be auto-imported
    global_modules: Vec<Arc<dyn ModuleExt>>,
    /// Guards registered by all modules
    guards: Vec<Arc<dyn Guard>>,
}

impl ModuleRegistry {
    /// Create a new module registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a module type has been registered
    pub fn is_registered<M: 'static>(&self) -> bool {
        self.registered.contains(&TypeId::of::<M>())
    }

    /// Mark a module type as registered
    pub fn mark_registered<M: 'static>(&mut self) {
        self.registered.insert(TypeId::of::<M>());
    }

    /// Register a module and its dependencies.
    ///
    /// Each module registers at most once: registration is deduped by the
    /// *concrete* module's `TypeId` (see [`ModuleExt::module_type_id`]), so
    /// a module reachable via two different import paths (a "diamond"
    /// import) is registered only the first time it's reached, and a
    /// cyclic import graph does not recurse forever.
    pub fn register_module(
        &mut self,
        container: &Container,
        router: &mut Router,
        module: &dyn ModuleExt,
    ) -> Result<(), Error> {
        let module_name = module
            .metadata()
            .name
            .unwrap_or_else(|| "UnnamedModule".to_string());

        // Mirrors the dedup pattern in `Application::register_module`
        // (armature-core/src/application.rs) — if this logic changes, check
        // whether that implementation needs the same fix.
        //
        // Dedup by `module.dedup_key()` rather than the bare
        // `module_type_id()`: most modules key on their concrete `TypeId`
        // (so diamond imports/cycles collapse to one registration), but
        // `DynamicModule` returns `None` from `dedup_key()` to opt out,
        // since multiple distinct, differently-configured instances of that
        // same concrete type are an expected, legitimate pattern.
        if let Some(module_id) = module.dedup_key()
            && !self.registered.insert(module_id)
        {
            tracing::debug!(
                module = %module_name,
                "Module already registered, skipping"
            );
            return Ok(());
        }

        tracing::debug!(module = %module_name, "Registering module");

        // Register imported modules first
        for imported in module.imports() {
            self.register_module(container, router, imported.as_ref())?;
        }

        // Register global modules if not already registered
        let globals: Vec<Arc<dyn ModuleExt>> = self.global_modules.clone();
        for global in globals {
            self.register_module(container, router, global.as_ref())?;
        }

        // Register providers
        for provider_reg in module.providers() {
            (provider_reg.register_fn)(container);
            tracing::trace!(
                module = %module_name,
                provider = provider_reg.type_name,
                "Provider registered"
            );
        }

        // Register guards
        for guard_reg in module.guards() {
            match (guard_reg.factory)(container) {
                Ok(guard) => {
                    self.guards.push(guard);
                    tracing::trace!(
                        module = %module_name,
                        guard = guard_reg.type_name,
                        "Guard registered"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        module = %module_name,
                        guard = guard_reg.type_name,
                        error = %e,
                        "Failed to register guard"
                    );
                }
            }
        }

        // Register controllers
        for controller_reg in module.controllers() {
            match (controller_reg.factory)(container) {
                Ok(controller_instance) => {
                    if let Err(e) =
                        (controller_reg.route_registrar)(container, router, controller_instance)
                    {
                        tracing::warn!(
                            module = %module_name,
                            controller = controller_reg.type_name,
                            error = %e,
                            "Failed to register controller routes"
                        );
                    } else {
                        tracing::trace!(
                            module = %module_name,
                            controller = controller_reg.type_name,
                            base_path = controller_reg.base_path,
                            "Controller registered"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        module = %module_name,
                        controller = controller_reg.type_name,
                        error = %e,
                        "Failed to instantiate controller"
                    );
                }
            }
        }

        // Call module init hook
        module.on_module_init(container)?;

        tracing::debug!(module = %module_name, "Module registration complete");
        Ok(())
    }

    /// Add a global module
    pub fn add_global_module<M: ModuleExt + 'static>(&mut self, module: M) {
        self.global_modules.push(Arc::new(module));
    }

    /// Add a boxed global module
    pub fn add_global_module_boxed(&mut self, module: Box<dyn ModuleExt>) {
        self.global_modules.push(Arc::from(module));
    }

    /// Get all registered guards
    pub fn guards(&self) -> &[Arc<dyn Guard>] {
        &self.guards
    }
}

/// Builder for creating modules with a fluent API
pub struct ModuleBuilder {
    inner: DynamicModule,
}

impl ModuleBuilder {
    /// Create a new module builder
    pub fn new(name: &str) -> Self {
        Self {
            inner: DynamicModule::new(name),
        }
    }

    /// Add a provider
    pub fn provider(mut self, provider: ProviderRegistration) -> Self {
        self.inner = self.inner.with_provider(provider);
        self
    }

    /// Add a controller
    pub fn controller(mut self, controller: ControllerRegistration) -> Self {
        self.inner = self.inner.with_controller(controller);
        self
    }

    /// Add a guard
    pub fn guard(mut self, guard: GuardRegistration) -> Self {
        self.inner = self.inner.with_guard(guard);
        self
    }

    /// Import a module
    pub fn import<M: ModuleExt + Clone + 'static>(mut self, module: M) -> Self {
        self.inner = self.inner.import(module);
        self
    }

    /// Export a type
    pub fn export<T: 'static>(mut self) -> Self {
        self.inner = self.inner.export::<T>();
        self
    }

    /// Export all providers
    pub fn export_all(mut self) -> Self {
        self.inner = self.inner.export_all();
        self
    }

    /// Re-export a module
    pub fn re_export<M: ModuleExt + Clone + 'static>(mut self, module: M) -> Self {
        self.inner = self.inner.re_export(module);
        self
    }

    /// Mark as global
    pub fn global(mut self) -> Self {
        self.inner = self.inner.global();
        self
    }

    /// Build the module
    pub fn build(self) -> DynamicModule {
        self.inner
    }
}

/// Helper macro to create provider registrations
///
/// If the constructed provider implements any of the lifecycle hook traits
/// (`OnModuleInit`, `OnModuleDestroy`, `OnApplicationBootstrap`,
/// `OnApplicationShutdown`), and a [`crate::lifecycle::LifecycleManager`]
/// has been attached to the container (see
/// [`crate::container::Container::attach_lifecycle`], done automatically by
/// [`crate::Application::create`]), those hooks are registered
/// automatically. See the `lifecycle` module docs for details.
#[macro_export]
macro_rules! provider_registration {
    ($type:ty, $factory:expr) => {
        $crate::ProviderRegistration {
            type_id: std::any::TypeId::of::<$type>(),
            type_name: std::any::type_name::<$type>(),
            register_fn: |container| {
                let instance: $type = $factory;
                let instance = ::std::sync::Arc::new(instance);
                if let Some(__armature_lifecycle) = container.lifecycle_manager() {
                    $crate::__armature_register_lifecycle_hooks!(
                        &__armature_lifecycle,
                        std::any::type_name::<$type>(),
                        instance
                    );
                }
                container.register_by_id(
                    std::any::TypeId::of::<$type>(),
                    instance as ::std::sync::Arc<dyn ::std::any::Any + Send + Sync>,
                );
            },
        }
    };
}

/// Helper macro to create guard registrations
#[macro_export]
macro_rules! guard_registration {
    ($type:ty, $factory:expr) => {
        $crate::module::GuardRegistration {
            type_id: std::any::TypeId::of::<$type>(),
            type_name: std::any::type_name::<$type>(),
            factory: |_container| {
                let guard = $factory;
                Ok(std::sync::Arc::new(guard) as std::sync::Arc<dyn $crate::Guard>)
            },
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    #[derive(Clone)]
    struct TestModule;

    impl ModuleExt for TestModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![]
        }

        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }

        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![]
        }

        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }

        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("TestModule")
        }
    }

    #[test]
    fn test_dynamic_module_creation() {
        let module = DynamicModule::new("TestModule");
        assert_eq!(module.name, "TestModule");
        assert!(module.providers.is_empty());
        assert!(module.controllers.is_empty());
    }

    #[test]
    fn test_dynamic_module_export_all() {
        let module = DynamicModule::new("TestModule").export_all();
        assert!(module.export_all);
    }

    #[test]
    fn test_module_builder() {
        let module = ModuleBuilder::new("MyModule").export_all().global().build();

        assert!(module.export_all);
        assert!(module.metadata.global);
    }

    #[test]
    fn test_module_metadata() {
        let metadata = ModuleMetadata::new()
            .name("MyModule")
            .version("1.0.0")
            .description("A test module")
            .global();

        assert_eq!(metadata.name, Some("MyModule".to_string()));
        assert_eq!(metadata.version, Some("1.0.0".to_string()));
        assert_eq!(metadata.description, Some("A test module".to_string()));
        assert!(metadata.global);
    }

    #[test]
    fn test_collect_all_exports() {
        #[derive(Clone)]
        struct InnerModule;
        impl ModuleExt for InnerModule {
            fn exports(&self) -> Vec<TypeId> {
                vec![TypeId::of::<String>()]
            }
        }

        #[derive(Clone)]
        struct OuterModule;
        impl ModuleExt for OuterModule {
            fn exports(&self) -> Vec<TypeId> {
                vec![TypeId::of::<i32>()]
            }
            fn re_exports(&self) -> Vec<Box<dyn ModuleExt>> {
                vec![Box::new(InnerModule)]
            }
        }

        let exports = collect_all_exports(&OuterModule);
        assert!(exports.contains(&TypeId::of::<String>()));
        assert!(exports.contains(&TypeId::of::<i32>()));
    }

    #[test]
    fn test_module_registry_creation() {
        let registry = ModuleRegistry::new();
        assert!(registry.guards().is_empty());
    }

    #[test]
    fn test_guard_registration_debug() {
        let reg = GuardRegistration {
            type_id: TypeId::of::<String>(),
            type_name: "TestGuard",
            factory: |_| Err(Error::Internal("test".to_string())),
        };

        let debug_str = format!("{:?}", reg);
        assert!(debug_str.contains("TestGuard"));
    }

    // ---- ModuleRegistry::register_module diamond/cycle dedup (Finding 2) --

    static REGISTRY_SHARED_PROVIDER_INIT_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    struct RegistrySharedProvider;

    struct RegistrySharedModule;
    impl ModuleExt for RegistrySharedModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![ProviderRegistration {
                type_id: TypeId::of::<RegistrySharedProvider>(),
                type_name: "RegistrySharedProvider",
                register_fn: |c| {
                    REGISTRY_SHARED_PROVIDER_INIT_COUNT
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    c.register(RegistrySharedProvider);
                },
            }]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistrySharedModule")
        }
    }

    struct RegistryLeftModule;
    impl ModuleExt for RegistryLeftModule {
        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![Box::new(RegistrySharedModule)]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistryLeftModule")
        }
    }

    struct RegistryRightModule;
    impl ModuleExt for RegistryRightModule {
        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![Box::new(RegistrySharedModule)]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistryRightModule")
        }
    }

    struct RegistryDiamondRootModule;
    impl ModuleExt for RegistryDiamondRootModule {
        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![Box::new(RegistryLeftModule), Box::new(RegistryRightModule)]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistryDiamondRootModule")
        }
    }

    #[test]
    fn test_module_registry_diamond_import_registers_shared_module_once() {
        REGISTRY_SHARED_PROVIDER_INIT_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);

        let container = Container::new();
        let mut router = Router::new();
        let mut registry = ModuleRegistry::new();

        registry
            .register_module(&container, &mut router, &RegistryDiamondRootModule)
            .unwrap();

        assert!(
            container.has::<RegistrySharedProvider>(),
            "shared module reachable via a diamond must still register"
        );
        assert_eq!(
            REGISTRY_SHARED_PROVIDER_INIT_COUNT.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "diamond-imported module (reached via two different parents) must \
             register exactly once, not zero (dropped) or two (duplicated)"
        );
    }

    /// A module that (indirectly) imports itself. Without cycle dedup,
    /// `register_module` recurses forever and overflows the stack.
    struct RegistryCyclicModuleA;
    struct RegistryCyclicModuleB;

    impl ModuleExt for RegistryCyclicModuleA {
        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![Box::new(RegistryCyclicModuleB)]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistryCyclicModuleA")
        }
    }
    impl ModuleExt for RegistryCyclicModuleB {
        fn imports(&self) -> Vec<Box<dyn ModuleExt>> {
            vec![Box::new(RegistryCyclicModuleA)]
        }
        fn metadata(&self) -> ModuleMetadata {
            ModuleMetadata::new().name("RegistryCyclicModuleB")
        }
    }

    #[test]
    fn test_module_registry_cyclic_import_does_not_recurse_forever() {
        let container = Container::new();
        let mut router = Router::new();
        let mut registry = ModuleRegistry::new();

        // Must return (not stack-overflow) once each module in the cycle has
        // been visited exactly once.
        registry
            .register_module(&container, &mut router, &RegistryCyclicModuleA)
            .unwrap();
    }

    // ---- DynamicModule dedup opt-out (Finding 1) ----

    struct DynamicModuleProviderA;
    struct DynamicModuleProviderB;

    /// Two distinctly-configured `DynamicModule` instances (different
    /// `.name`/providers) share the same concrete `DynamicModule` type, so
    /// keying dedup on the bare `module_type_id()` alone would make the
    /// second one silently no-op as a "duplicate" of the first. `DynamicModule`
    /// opts out of dedup via `dedup_key() -> None`, so both must register.
    #[test]
    fn test_dynamic_module_instances_do_not_collapse_under_dedup() {
        let container = Container::new();
        let mut router = Router::new();
        let mut registry = ModuleRegistry::new();

        let module_a = DynamicModule::new("FeatureA").with_provider(ProviderRegistration {
            type_id: TypeId::of::<DynamicModuleProviderA>(),
            type_name: "DynamicModuleProviderA",
            register_fn: |c| c.register(DynamicModuleProviderA),
        });

        let module_b = DynamicModule::new("FeatureB").with_provider(ProviderRegistration {
            type_id: TypeId::of::<DynamicModuleProviderB>(),
            type_name: "DynamicModuleProviderB",
            register_fn: |c| c.register(DynamicModuleProviderB),
        });

        registry
            .register_module(&container, &mut router, &module_a)
            .unwrap();
        registry
            .register_module(&container, &mut router, &module_b)
            .unwrap();

        assert!(
            container.has::<DynamicModuleProviderA>(),
            "first DynamicModule instance's providers must register"
        );
        assert!(
            container.has::<DynamicModuleProviderB>(),
            "second, distinctly-configured DynamicModule instance must also \
             register -- it must not be dropped as a 'duplicate' of the \
             first just because both share the same concrete DynamicModule \
             type"
        );
    }

    /// The same opt-out must survive being reached through `.import()`,
    /// which wraps the imported module in `ArcModule`. If `ArcModule` didn't
    /// forward `dedup_key()`, it would fall back to the default (keyed on
    /// `module_type_id()`), and the second nested `DynamicModule` would
    /// collapse into the first even though the top-level test above passes.
    #[test]
    fn test_dynamic_module_instances_do_not_collapse_when_imported() {
        let container = Container::new();
        let mut router = Router::new();
        let mut registry = ModuleRegistry::new();

        let feature_a = DynamicModule::new("FeatureA").with_provider(ProviderRegistration {
            type_id: TypeId::of::<DynamicModuleProviderA>(),
            type_name: "DynamicModuleProviderA",
            register_fn: |c| c.register(DynamicModuleProviderA),
        });

        let feature_b = DynamicModule::new("FeatureB").with_provider(ProviderRegistration {
            type_id: TypeId::of::<DynamicModuleProviderB>(),
            type_name: "DynamicModuleProviderB",
            register_fn: |c| c.register(DynamicModuleProviderB),
        });

        let root = DynamicModule::new("Root")
            .import(feature_a)
            .import(feature_b);

        registry
            .register_module(&container, &mut router, &root)
            .unwrap();

        assert!(
            container.has::<DynamicModuleProviderA>(),
            "first imported DynamicModule instance's providers must register"
        );
        assert!(
            container.has::<DynamicModuleProviderB>(),
            "second imported DynamicModule instance must also register \
             through the ArcModule wrapper, not collapse into the first"
        );
    }
}
