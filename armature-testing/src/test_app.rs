// Test Application Builder

use armature_core::{Application, Container, Module, Provider, Router};
use std::sync::Arc;

/// Test application for integration testing
pub struct TestApp {
    pub app: Application,
    pub container: Arc<Container>,
}

impl TestApp {
    /// Create a new test application
    ///
    /// `armature_core::Container` clones share the same underlying
    /// (`Arc`-backed) storage, so cloning the caller-supplied container into
    /// `Application::new` — rather than handing the `Application` a brand
    /// new, empty `Container::new()` — keeps `self.container` and the
    /// container the running `Application` resolves against in sync:
    /// anything registered through either handle is visible through both.
    pub fn new(container: Container, router: Router) -> Self {
        let app = Application::new(container.clone(), router);
        Self {
            app,
            container: Arc::new(container),
        }
    }

    /// Get a service from the container.
    ///
    /// Delegates to the stored `armature_core::Container`. Returns `None`
    /// if no provider of type `T` was registered (via `TestAppBuilder`,
    /// `add_module`, or directly on the container).
    pub fn get<T: Provider + Clone + 'static>(&self) -> Option<T> {
        self.container.get::<T>().ok().map(|arc| (*arc).clone())
    }

    /// Create a test client for making requests
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_testing::TestAppBuilder;
    /// use armature_core::HttpResponse;
    ///
    /// # tokio_test::block_on(async {
    /// let app = TestAppBuilder::new()
    ///     .with_route("/test", |_req| async {
    ///         Ok(HttpResponse::ok().with_body(b"OK".to_vec()))
    ///     })
    ///     .build();
    ///
    /// let client = app.client();
    /// let response = client.get("/test").await;
    /// assert_eq!(response.status(), Some(200));
    /// # });
    /// ```
    pub fn client(&self) -> crate::TestClient {
        crate::TestClient::new(self.app.router.clone())
    }
}

/// Builder for test applications.
///
/// Provides a fluent API for creating test applications with custom
/// routes, services, and configuration.
///
/// # Examples
///
/// Basic test app:
///
/// ```no_run
/// use armature_testing::TestAppBuilder;
/// use armature_core::{HttpResponse, Error};
///
/// # tokio_test::block_on(async {
/// let app = TestAppBuilder::new()
///     .with_route("/test", |_req| async {
///         Ok(HttpResponse::ok().with_body(b"test".to_vec()))
///     })
///     .build();
///
/// let client = app.client();
/// let response = client.get("/test").await;
/// assert_eq!(response.status(), Some(200));
/// # });
/// ```
pub struct TestAppBuilder {
    container: Container,
    router: Router,
}

/// Maximum `imports()`/`re_exports()` nesting depth `wire_module` will
/// recurse through before panicking.
///
/// `wire_module` dedups modules by concrete `TypeId`
/// (`Module::module_type_id`), so a self-importing or diamond-shaped module
/// graph no longer recurses unboundedly on its own: the cyclic/repeated
/// module is skipped the second time it's reached, terminating the walk.
/// This constant is only a defense-in-depth backstop for a pathological
/// *non-cyclic* graph — e.g. a long linear chain of distinct modules each
/// importing the next — that could otherwise still recurse arbitrarily deep
/// and overflow the stack (which would abort the whole test binary rather
/// than fail a single test). 64 levels is far beyond any real module
/// hierarchy while still panicking well before the stack is at risk.
const WIRE_MODULE_MAX_DEPTH: usize = 64;

impl TestAppBuilder {
    /// Create a new test app builder
    pub fn new() -> Self {
        Self {
            container: Container::new(),
            router: Router::new(),
        }
    }

    /// Register a provider
    pub fn register<T: Provider + Clone + 'static>(self, provider: T) -> Self {
        self.container.register(provider);
        self
    }

    /// Add a module.
    ///
    /// Registers the module's providers and controllers (recursively,
    /// through `imports()`/`re_exports()`) via the same
    /// `ProviderRegistration` / `ControllerRegistration` function pointers
    /// `Application` itself calls, so DI and routes wired by the module are
    /// real — not a no-op.
    ///
    /// Like `Application::register_module`, this deduplicates
    /// diamond-imported modules: `Module::module_type_id()` is monomorphized
    /// per concrete module type, so it genuinely identifies the concrete
    /// module behind a `&dyn Module` reference (unlike
    /// `std::any::type_name_of_val`, which only ever reports `"dyn
    /// Module"`, the trait object's own type, for every module). A module
    /// reachable via two distinct import paths — e.g. a root importing `A`
    /// and `B`, both of which import shared module `S` — registers its
    /// providers and controllers exactly once.
    ///
    /// Module guards are **not** applied: `TestApp`'s router dispatch (via
    /// `TestClient`) routes directly against `Router`, without guard
    /// evaluation, so there is nothing here for a guard to scope against.
    ///
    /// # Panics
    ///
    /// Panics if a controller factory or route registrar returns an error —
    /// a misconfigured module should fail the test loudly rather than
    /// silently register nothing. Also panics if the import graph nests
    /// deeper than `WIRE_MODULE_MAX_DEPTH` levels — see that constant's doc
    /// comment; with dedup in place this is only a defense-in-depth backstop
    /// for pathological non-cyclic graphs, since true cycles now terminate
    /// via the visited set.
    pub fn add_module<M: Module>(mut self, module: M) -> Self {
        let mut visited = std::collections::HashSet::new();
        Self::wire_module(&self.container, &mut self.router, &mut visited, &module, 0);
        self
    }

    /// Recursively register a module's providers and controllers into
    /// `container`/`router`. Mirrors `armature_core::Application`'s private
    /// `register_module` wiring (minus guard scoping; see `add_module`'s doc
    /// comment for the guard-scoping difference).
    ///
    /// `visited` is keyed on `Module::module_type_id()` and carried across
    /// the whole recursion (not per-branch), so a module reached via two
    /// different import paths — a diamond — registers only on the first
    /// visit, exactly like `Application::register_module`.
    ///
    /// `depth` counts levels of `imports()`/`re_exports()` nesting below the
    /// module passed to `add_module` (which is depth 0). With `visited` in
    /// place, genuine cycles (including self-imports) terminate on their own
    /// — the repeated module is already in `visited` — so `depth` now exists
    /// solely as a defense-in-depth cap on pathological non-cyclic graphs;
    /// see `WIRE_MODULE_MAX_DEPTH`.
    fn wire_module(
        container: &Container,
        router: &mut Router,
        visited: &mut std::collections::HashSet<std::any::TypeId>,
        module: &dyn Module,
        depth: usize,
    ) {
        if depth >= WIRE_MODULE_MAX_DEPTH {
            panic!("module import graph exceeds depth {WIRE_MODULE_MAX_DEPTH} — cyclic imports?");
        }

        // Each module registers once: diamond imports must not duplicate
        // providers/routes, and cyclic imports (including self-imports)
        // must not recurse forever. Mirrors
        // `Application::register_module`'s visited-set semantics.
        if !visited.insert(module.module_type_id()) {
            return;
        }

        for imported in module.imports() {
            Self::wire_module(container, router, visited, imported.as_ref(), depth + 1);
        }
        for re_exported in module.re_exports() {
            Self::wire_module(container, router, visited, re_exported.as_ref(), depth + 1);
        }

        for provider_reg in module.providers() {
            (provider_reg.register_fn)(container);
        }

        for controller_reg in module.controllers() {
            let instance = (controller_reg.factory)(container).unwrap_or_else(|e| {
                panic!(
                    "TestAppBuilder::add_module: failed to instantiate controller `{}`: {}",
                    controller_reg.type_name, e
                )
            });
            (controller_reg.route_registrar)(container, router, instance).unwrap_or_else(|e| {
                panic!(
                    "TestAppBuilder::add_module: failed to register routes for controller `{}`: {}",
                    controller_reg.type_name, e
                )
            });
        }
    }

    /// Set custom container
    pub fn with_container(self, container: Container) -> Self {
        Self { container, ..self }
    }

    /// Set custom router
    pub fn with_router(self, router: Router) -> Self {
        Self { router, ..self }
    }

    /// Add a test route with a handler
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_testing::TestAppBuilder;
    /// use armature_core::HttpResponse;
    ///
    /// let app = TestAppBuilder::new()
    ///     .with_route("/api/health", |_req| async {
    ///         Ok(HttpResponse::ok().with_body(b"OK".to_vec()))
    ///     })
    ///     .build();
    /// ```
    pub fn with_route<F, Fut>(mut self, path: &str, handler: F) -> Self
    where
        F: Fn(armature_core::HttpRequest) -> Fut + Send + Sync + Clone + 'static,
        Fut: std::future::Future<Output = Result<armature_core::HttpResponse, armature_core::Error>>
            + Send
            + 'static,
    {
        use armature_core::{HttpMethod, Route};
        // Use the optimized Route::new which enables handler monomorphization
        self.router
            .add_route(Route::new(HttpMethod::GET, path, handler));
        self
    }

    /// Build the test application
    pub fn build(self) -> TestApp {
        TestApp::new(self.container, self.router)
    }
}

impl Default for TestAppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_creation() {
        let _builder = TestAppBuilder::new();
    }

    #[test]
    fn test_app_creation() {
        let builder = TestAppBuilder::new();
        let _app = builder.build();
    }

    #[derive(Clone)]
    struct Widget {
        value: u32,
    }

    #[test]
    fn registered_provider_is_retrievable_via_test_app_get() {
        let app = TestAppBuilder::new().register(Widget { value: 7 }).build();

        let widget = app.get::<Widget>().expect("Widget should be registered");
        assert_eq!(widget.value, 7);
    }

    #[test]
    fn unregistered_provider_returns_none() {
        let app = TestAppBuilder::new().build();
        assert!(app.get::<Widget>().is_none());
    }

    #[test]
    fn with_container_flows_into_test_app() {
        let container = Container::new();
        container.register(Widget { value: 99 });

        let app = TestAppBuilder::new().with_container(container).build();

        let widget = app.get::<Widget>().expect("Widget should be registered");
        assert_eq!(widget.value, 99);
    }

    // -- add_module: real provider/controller wiring, not a no-op --------

    use armature_core::{
        ControllerRegistration, Error, HttpMethod, HttpRequest, HttpResponse, ProviderRegistration,
    };
    use std::any::{Any, TypeId};

    #[derive(Clone)]
    struct Counter {
        value: u32,
    }

    fn register_counter(container: &Container) {
        container.register(Counter { value: 42 });
    }

    struct CounterController {
        count: Arc<Counter>,
    }

    fn counter_factory(container: &Container) -> Result<Box<dyn Any + Send + Sync>, Error> {
        let count = container.resolve::<Counter>()?;
        Ok(Box::new(CounterController { count }))
    }

    fn counter_route_registrar(
        _container: &Container,
        router: &mut Router,
        instance: Box<dyn Any + Send + Sync>,
    ) -> Result<(), Error> {
        let controller = instance
            .downcast::<CounterController>()
            .expect("instance should be CounterController");
        let count = controller.count.clone();
        router.add_route(armature_core::Route::new(
            HttpMethod::GET,
            "/counter",
            move |_req: HttpRequest| {
                let count = count.clone();
                async move { Ok(HttpResponse::ok().with_body(count.value.to_string().into_bytes())) }
            },
        ));
        Ok(())
    }

    struct CounterModule;
    impl Module for CounterModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![ProviderRegistration {
                type_id: TypeId::of::<Counter>(),
                type_name: "Counter",
                register_fn: register_counter,
            }]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![ControllerRegistration {
                type_id: TypeId::of::<CounterController>(),
                type_name: "CounterController",
                base_path: "/counter",
                factory: counter_factory,
                route_registrar: counter_route_registrar,
            }]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    #[tokio::test]
    async fn add_module_registers_providers_and_controllers_for_real() {
        let app = TestAppBuilder::new().add_module(CounterModule).build();

        // The module's provider is retrievable directly...
        let counter = app
            .get::<Counter>()
            .expect("Counter should be registered by the module");
        assert_eq!(counter.value, 42);

        // ...and reaches the controller's handler via DI, proving the
        // container the module registered into is the same one the running
        // Application resolves against.
        let response = app.client().get("/counter").await;
        assert_eq!(response.status(), Some(200));
        assert_eq!(response.body_string(), Some("42".to_string()));
    }

    #[derive(Clone)]
    struct Pinger;

    fn register_pinger(container: &Container) {
        container.register(Pinger);
    }

    struct PingController;

    fn ping_factory(container: &Container) -> Result<Box<dyn Any + Send + Sync>, Error> {
        container.resolve::<Pinger>()?;
        Ok(Box::new(PingController))
    }

    fn ping_route_registrar(
        _container: &Container,
        router: &mut Router,
        instance: Box<dyn Any + Send + Sync>,
    ) -> Result<(), Error> {
        instance
            .downcast::<PingController>()
            .expect("instance should be PingController");
        router.add_route(armature_core::Route::new(
            HttpMethod::GET,
            "/ping",
            |_req: HttpRequest| async { Ok(HttpResponse::ok().with_body(b"pong".to_vec())) },
        ));
        Ok(())
    }

    struct PingModule;
    impl Module for PingModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![ProviderRegistration {
                type_id: TypeId::of::<Pinger>(),
                type_name: "Pinger",
                register_fn: register_pinger,
            }]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![ControllerRegistration {
                type_id: TypeId::of::<PingController>(),
                type_name: "PingController",
                base_path: "/ping",
                factory: ping_factory,
                route_registrar: ping_route_registrar,
            }]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    /// A parent module importing two *distinct* submodules. This guards the
    /// other side of `wire_module`'s `module_type_id()`-keyed dedup (see
    /// `add_module_dedups_diamond_imported_module_registers_shared_provider_once`
    /// below for the diamond/same-module case): two genuinely different
    /// modules must never be mistaken for the same visited entry, so both
    /// must still register even though both are reached from the same
    /// parent.
    struct ParentModule;
    impl Module for ParentModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(CounterModule), Box::new(PingModule)]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    #[tokio::test]
    async fn add_module_registers_all_distinct_imported_modules() {
        let app = TestAppBuilder::new().add_module(ParentModule).build();

        // Both imported modules' providers must be registered...
        assert!(app.get::<Counter>().is_some());
        assert!(app.get::<Pinger>().is_some());

        // ...and both imported modules' routes must be reachable.
        let counter_response = app.client().get("/counter").await;
        assert_eq!(counter_response.status(), Some(200));
        assert_eq!(counter_response.body_string(), Some("42".to_string()));

        let ping_response = app.client().get("/ping").await;
        assert_eq!(ping_response.status(), Some(200));
        assert_eq!(ping_response.body_string(), Some("pong".to_string()));
    }

    // -- wire_module: diamond-imported modules must register exactly once -

    /// Counts calls to `register_shared`, proving how many times
    /// `SharedModule`'s provider registration actually ran — independent of
    /// whatever the container ends up holding (a second `register` call
    /// would just overwrite the first, masking a double-registration).
    static SHARED_REGISTRATIONS: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);

    #[derive(Clone)]
    struct Shared;

    fn register_shared(container: &Container) {
        SHARED_REGISTRATIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        container.register(Shared);
    }

    struct SharedModule;
    impl Module for SharedModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![ProviderRegistration {
                type_id: TypeId::of::<Shared>(),
                type_name: "Shared",
                register_fn: register_shared,
            }]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    /// Imports `SharedModule` — the left branch of the diamond.
    struct DiamondLeftModule;
    impl Module for DiamondLeftModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(SharedModule)]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    /// Also imports `SharedModule` — the right branch of the diamond.
    struct DiamondRightModule;
    impl Module for DiamondRightModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(SharedModule)]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    /// Imports both `DiamondLeftModule` and `DiamondRightModule`, so
    /// `SharedModule` is reachable via two distinct import paths — a classic
    /// diamond. `Application::register_module` dedups this by the concrete
    /// module's `TypeId` (`Module::module_type_id`); `wire_module` must do
    /// the same so a test app's module graph wires identically to the real
    /// `Application`'s.
    struct DiamondRootModule;
    impl Module for DiamondRootModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(DiamondLeftModule), Box::new(DiamondRightModule)]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    #[test]
    fn add_module_dedups_diamond_imported_module_registers_shared_provider_once() {
        let app = TestAppBuilder::new().add_module(DiamondRootModule).build();

        assert_eq!(
            SHARED_REGISTRATIONS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "SharedModule is reachable via both DiamondLeftModule and \
             DiamondRightModule; its provider must register exactly once"
        );
        assert!(app.get::<Shared>().is_some());
    }

    // -- wire_module: self-importing modules dedup via the visited set ---

    /// Counts calls to `register_self_importing`, proving
    /// `SelfImportingModule`'s provider registers exactly once despite the
    /// module importing itself (same rationale as `SHARED_REGISTRATIONS`
    /// above: counting the registration call itself, not container state
    /// afterward, is what actually proves single-registration).
    static SELF_IMPORT_REGISTRATIONS: std::sync::atomic::AtomicU32 =
        std::sync::atomic::AtomicU32::new(0);

    #[derive(Clone)]
    struct SelfImportMarker;

    fn register_self_importing(container: &Container) {
        SELF_IMPORT_REGISTRATIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        container.register(SelfImportMarker);
    }

    /// A module that imports itself.
    ///
    /// Before `wire_module` deduped by `Module::module_type_id()`, wiring
    /// this in recursed unboundedly and overflowed the stack; the depth cap
    /// (`WIRE_MODULE_MAX_DEPTH`) existed specifically to turn that into a
    /// clean panic instead of aborting the whole test binary. Now the
    /// module is inserted into the visited set on its very first (root)
    /// entry, so when its own `imports()` yields another
    /// `SelfImportingModule`, that recursive call finds the type already
    /// visited and returns immediately — the cycle terminates on its own,
    /// one level deep, without ever approaching the depth cap.
    struct SelfImportingModule;
    impl Module for SelfImportingModule {
        fn providers(&self) -> Vec<ProviderRegistration> {
            vec![ProviderRegistration {
                type_id: TypeId::of::<SelfImportMarker>(),
                type_name: "SelfImportMarker",
                register_fn: register_self_importing,
            }]
        }
        fn controllers(&self) -> Vec<ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(SelfImportingModule)]
        }
        fn exports(&self) -> Vec<TypeId> {
            vec![]
        }
    }

    #[test]
    fn add_module_dedups_self_importing_module_registers_once_without_panicking() {
        let app = TestAppBuilder::new()
            .add_module(SelfImportingModule)
            .build();

        assert_eq!(
            SELF_IMPORT_REGISTRATIONS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a self-importing module is visited on its first (root) entry, \
             so its own self-import is skipped as already-visited — its \
             provider must register exactly once, not zero or many"
        );
        assert!(app.get::<SelfImportMarker>().is_some());
    }
}
