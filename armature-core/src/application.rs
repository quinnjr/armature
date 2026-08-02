// Application bootstrapper and HTTP server

use crate::epoll_tuning::EpollConfig;
use crate::exception_filter::{ExceptionFilter, ExceptionFilterChain};
use crate::guard::{Guard, GuardContext};
use crate::http2::{Http2Builder, Http2Config, Http2Stats};
use crate::http3::{Http3Config, Http3Stats};
use crate::logging::{debug, error, info, trace, warn};
use crate::pipeline::{PipelineConfig, PipelineStats, PipelinedHttp1Builder};
use crate::route_cache::OptimizedRouter;
use crate::{
    Container, Error, HttpRequest, HttpResponse, HttpsConfig, LifecycleManager, Module, Router,
    TlsConfig,
};
use http_body_util::{BodyExt, Full, Limited};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, body::Incoming as IncomingBody};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// The main application struct
pub struct Application {
    pub container: Container,
    pub router: Arc<Router>,
    pub lifecycle: Arc<LifecycleManager>,
    /// HTTP/1.1 pipelining configuration
    pipeline_config: PipelineConfig,
    /// Shared pipeline statistics
    pipeline_stats: Arc<PipelineStats>,
    /// HTTP/2 configuration
    http2_config: Http2Config,
    /// Shared HTTP/2 statistics
    http2_stats: Arc<Http2Stats>,
    /// HTTP/3 (QUIC) configuration
    http3_config: Http3Config,
    /// Shared HTTP/3 statistics
    http3_stats: Arc<Http3Stats>,
    /// Optional CORS configuration applied to every response
    cors_config: Option<Arc<CorsConfig>>,
    /// Guards evaluated before routing. Each is scoped to a URL path prefix:
    /// module guards to their declaring module's controller base paths, and
    /// manually-added guards (via [`Application::with_guard`]) to the empty
    /// (all-matching) prefix. See [`ScopedGuard`].
    guards: Vec<ScopedGuard>,
    /// Maximum request body size in bytes; larger bodies are rejected with 413
    max_body_size: usize,
    /// Optional socket tuning applied to listener and accepted sockets
    #[cfg_attr(not(unix), allow(dead_code))]
    epoll_config: Option<EpollConfig>,
    /// Optional global exception filter chain (see [`Application::use_global_filter`]).
    /// When unset, errors are converted via [`Error::to_client_response`]
    /// exactly as before this field existed.
    filter_chain: Option<ExceptionFilterChain>,
}

/// Default maximum request body size (10 MB).
pub const DEFAULT_MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

/// A guard paired with the URL path prefix it applies to.
///
/// Module guards are *not* global: a guard declared by a module is scoped to
/// the base paths of the controllers registered by that **same** module (see
/// [`Application::register_module`]), so it runs only for requests whose path
/// falls under one of those base paths. Guards added manually via
/// [`Application::with_guard`] use an empty prefix and therefore run for every
/// request (a genuinely global guard).
#[derive(Clone)]
struct ScopedGuard {
    /// URL path prefix this guard applies to. An empty prefix (or `"/"`)
    /// matches every request path.
    prefix: String,
    /// The guard to evaluate for matching requests.
    guard: Arc<dyn Guard>,
}

impl ScopedGuard {
    /// Returns `true` if this guard should run for the given request path.
    ///
    /// Matching is path-segment aware: prefix `/admin` matches `/admin` and
    /// `/admin/...` but **not** `/administrators`. An empty or `/` prefix
    /// matches every path (a global guard).
    fn matches(&self, path: &str) -> bool {
        let prefix = self.prefix.trim_end_matches('/');
        if prefix.is_empty() {
            return true;
        }
        path == prefix || path.starts_with(&format!("{}/", prefix))
    }
}

/// Shared state captured by every connection's request handler.
///
/// Routing dispatches through the O(1) [`OptimizedRouter`] (static-HashMap fast
/// path + compiled patterns + LRU cache), compiled once from the fully
/// populated linear [`Router`] at server startup (see
/// [`Application::serve_state`]). The linear router remains the registration
/// target; only per-request dispatch is accelerated.
#[derive(Clone)]
struct ServeState {
    router: Arc<OptimizedRouter>,
    cors: Option<Arc<CorsConfig>>,
    guards: Arc<[ScopedGuard]>,
    max_body_size: usize,
    /// Global exception filter chain (see [`Application::use_global_filter`]).
    /// `None` preserves the original behavior: errors go straight to
    /// [`Error::to_client_response`] via [`error_response`].
    filter_chain: Option<Arc<ExceptionFilterChain>>,
}

/// CORS configuration for the application.
#[derive(Debug, Clone)]
pub struct CorsConfig {
    pub allow_origin: String,
    pub allow_methods: String,
    pub allow_headers: String,
    pub allow_credentials: bool,
    pub max_age: u32,
}

impl CorsConfig {
    pub fn new(origin: impl Into<String>) -> Self {
        Self {
            allow_origin: origin.into(),
            allow_methods: "GET, POST, PUT, DELETE, OPTIONS, PATCH".to_string(),
            allow_headers: "Content-Type, Authorization, Accept, X-Requested-With".to_string(),
            allow_credentials: false,
            max_age: 86400,
        }
    }

    pub fn with_credentials(mut self) -> Self {
        self.allow_credentials = true;
        self
    }

    pub fn allow_headers(mut self, headers: impl Into<String>) -> Self {
        self.allow_headers = headers.into();
        self
    }
}

impl Application {
    /// Create an application with a container and router
    pub fn new(container: Container, router: Router) -> Self {
        Self {
            container,
            router: Arc::new(router),
            lifecycle: Arc::new(LifecycleManager::new()),
            pipeline_config: PipelineConfig::default(),
            pipeline_stats: Arc::new(PipelineStats::new()),
            http2_config: Http2Config::default(),
            http2_stats: Arc::new(Http2Stats::new()),
            http3_config: Http3Config::default(),
            http3_stats: Arc::new(Http3Stats::new()),
            cors_config: None,
            guards: Vec::new(),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            epoll_config: None,
            filter_chain: None,
        }
    }

    /// Register a global exception filter.
    ///
    /// Filters run in priority order (highest first); the first filter
    /// whose `catch()` returns `Some(response)` wins and its response is
    /// returned to the client. Wired into every HTTP/1.1 and HTTP/2 request
    /// path: every `service_fn` closure across [`Application::listen`],
    /// [`Application::listen_on`], HTTP/2 cleartext, and HTTPS/TLS+ALPN
    /// listeners funnels through the same shared `handle_request`, so both
    /// the guard-rejection error path and the routing/handler error path try
    /// the filter chain before falling back to
    /// [`Error::to_client_response`]. HTTP/3 ([`Application::listen_h3`] /
    /// [`Application::listen_dual_stack`]'s QUIC side) does **not** yet go
    /// through the filter chain -- it's served by a separate `Http3Server`
    /// code path (see `http3.rs`) that doesn't call `handle_request`.
    ///
    /// A registered filter's `catch()` runs with panic and timeout
    /// isolation (see `respond_to_error`): a panicking or hanging filter
    /// falls back to the same response [`Error::to_client_response`] would
    /// have produced, rather than taking down the request or connection.
    ///
    /// Calling this repeatedly adds more filters to the same chain. Errors
    /// not claimed by any filter fall back to the chain's own default
    /// transformer (production-mode [`crate::error_transform::ErrorTransformer`]),
    /// *not* [`Error::to_client_response`] -- this matches
    /// [`crate::exception_filter::ExceptionFilterChain`]'s own documented
    /// behavior. Without any call to this method, errors are converted via
    /// [`Error::to_client_response`] exactly as before this method existed.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::{Application, Container, Router};
    /// use armature_core::exception_filter::AllExceptionsFilter;
    ///
    /// let app = Application::new(Container::new(), Router::new())
    ///     .use_global_filter(AllExceptionsFilter::new());
    /// # let _ = app;
    /// ```
    pub fn use_global_filter<F: ExceptionFilter>(mut self, filter: F) -> Self {
        let chain = self.filter_chain.take().unwrap_or_default();
        self.filter_chain = Some(chain.add_filter(filter));
        self
    }

    /// Configure CORS for the application. Handles preflight OPTIONS
    /// requests automatically and adds CORS headers to every response.
    pub fn with_cors(mut self, config: CorsConfig) -> Self {
        self.cors_config = Some(Arc::new(config));
        self
    }

    /// Add a **global** guard evaluated for every request before routing.
    ///
    /// Manually-added guards use an empty (all-matching) path prefix, so they
    /// run for every request path regardless of which controller handles it.
    /// This is different from guards declared by a module, which are scoped to
    /// the base paths of that module's own controllers (see
    /// [`Application::register_module`]). Module guards are registered
    /// automatically during [`Application::create`]; use this to add global
    /// guards manually.
    pub fn with_guard(mut self, guard: Arc<dyn Guard>) -> Self {
        self.guards.push(ScopedGuard {
            prefix: String::new(),
            guard,
        });
        self
    }

    /// Set the maximum request body size in bytes.
    ///
    /// Requests with larger bodies are rejected with `413 Payload Too Large`
    /// before the body is buffered in memory. Defaults to
    /// [`DEFAULT_MAX_BODY_SIZE`] (10 MB).
    pub fn with_max_body_size(mut self, bytes: usize) -> Self {
        self.max_body_size = bytes;
        self
    }

    /// Apply low-level socket tuning to server sockets.
    ///
    /// When set, [`crate::epoll_tuning::configure_socket`] is applied to
    /// every accepted connection socket (TCP_NODELAY, TCP_QUICKACK, buffer
    /// sizes, keepalive) before the connection is served, and to the
    /// listener socket right after binding. Failures are logged as warnings
    /// and never abort the accept loop.
    ///
    /// # Limitations
    ///
    /// The server binds its listener via `TcpListener::bind`, so options
    /// that must be set *before* bind to have any effect — notably
    /// `SO_REUSEPORT` and `SO_REUSEADDR` — are applied too late to influence
    /// binding semantics. Setting them here succeeds but is effectively a
    /// no-op at the listener level; only options that still matter post-bind
    /// (e.g. buffer sizes, which accepted sockets inherit) take effect
    /// there. To use `SO_REUSEPORT` for multi-worker load balancing, create
    /// and bind the socket yourself with the option set before binding.
    ///
    /// Only effective on Unix platforms; the full option set requires Linux.
    /// The epoll flag settings in the config (`edge_triggered`, `oneshot`,
    /// `exclusive`) are advisory and are not applied by the built-in server
    /// (tokio owns its epoll registration).
    ///
    /// See also [`crate::connection_tuning::TcpConfig`] for the related
    /// per-workload TCP tuning API.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, epoll_tuning::EpollConfig};
    ///
    /// let app = Application::new(container, router)
    ///     .with_socket_tuning(EpollConfig::low_latency());
    /// ```
    pub fn with_socket_tuning(mut self, config: EpollConfig) -> Self {
        self.epoll_config = Some(config);
        self
    }

    /// Build the shared per-connection serving state.
    ///
    /// The linear [`Router`] is compiled once into an [`OptimizedRouter`] here,
    /// after all modules have registered their routes, so per-request routing
    /// uses the O(1) fast path instead of an O(n) linear scan. Called once per
    /// `listen*` entry point (server startup), so the compilation cost is paid
    /// a single time.
    fn serve_state(&self, cors: Option<Arc<CorsConfig>>) -> ServeState {
        ServeState {
            router: Arc::new(OptimizedRouter::from_router(&self.router)),
            cors,
            guards: self.guards.clone().into(),
            max_body_size: self.max_body_size,
            filter_chain: self.filter_chain.clone().map(Arc::new),
        }
    }

    /// Set the pipeline configuration for HTTP/1.1 pipelining
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, pipeline::{PipelineConfig, PipelineMode}};
    ///
    /// let app = Application::new(container, router)
    ///     .with_pipeline_config(PipelineConfig::high_performance());
    /// ```
    pub fn with_pipeline_config(mut self, config: PipelineConfig) -> Self {
        self.pipeline_config = config;
        self
    }

    /// Get the pipeline statistics
    ///
    /// Use this to monitor pipeline performance at runtime.
    pub fn pipeline_stats(&self) -> Arc<PipelineStats> {
        Arc::clone(&self.pipeline_stats)
    }

    /// Get the pipeline configuration
    pub fn pipeline_config(&self) -> &PipelineConfig {
        &self.pipeline_config
    }

    /// Set the HTTP/2 configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, Http2Config};
    ///
    /// let app = Application::new(container, router)
    ///     .with_http2_config(Http2Config::high_throughput());
    /// ```
    pub fn with_http2_config(mut self, config: Http2Config) -> Self {
        self.http2_config = config;
        self
    }

    /// Get the HTTP/2 statistics
    ///
    /// Use this to monitor HTTP/2 connection and stream metrics at runtime.
    pub fn http2_stats(&self) -> Arc<Http2Stats> {
        Arc::clone(&self.http2_stats)
    }

    /// Get the HTTP/2 configuration
    pub fn http2_config(&self) -> &Http2Config {
        &self.http2_config
    }

    /// Set the HTTP/3 (QUIC) configuration
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, Http3Config};
    ///
    /// let app = Application::new(container, router)
    ///     .with_http3_config(Http3Config::low_latency());
    /// ```
    pub fn with_http3_config(mut self, config: Http3Config) -> Self {
        self.http3_config = config;
        self
    }

    /// Get the HTTP/3 (QUIC) statistics
    ///
    /// Use this to monitor HTTP/3 connection, stream, and transfer metrics.
    pub fn http3_stats(&self) -> Arc<Http3Stats> {
        Arc::clone(&self.http3_stats)
    }

    /// Get the HTTP/3 (QUIC) configuration
    pub fn http3_config(&self) -> &Http3Config {
        &self.http3_config
    }

    /// Create a new application from a root module with lifecycle support
    ///
    /// # Lifecycle hook failures are fail-open
    ///
    /// `OnModuleInit` and `OnApplicationBootstrap` hooks run automatically as
    /// part of bootstrap (see below). If one or more hooks return an `Err`,
    /// this is **not** fatal: the failures are logged (`warn!`/`error!`) and
    /// startup continues to completion, returning a fully constructed
    /// `Application` regardless. This is an intentional, documented design
    /// choice -- not a bug -- so a single misbehaving provider's init hook
    /// can't unconditionally prevent the process from starting. Callers that
    /// need boot to abort on hook failure should inspect
    /// [`LifecycleManager::call_module_init_hooks`]/
    /// [`LifecycleManager::call_bootstrap_hooks`] results themselves (e.g. by
    /// driving lifecycle manually instead of via `create`) or check logs/
    /// metrics for hook failures after `create` returns.
    pub async fn create<M: Module + Default>() -> Self {
        info!("Bootstrapping Armature application");
        debug!(
            module_type = std::any::type_name::<M>(),
            "Creating application from root module"
        );

        let container = Container::new();
        debug!("DI container initialized");

        let mut router = Router::new();
        debug!("Router initialized");

        let lifecycle = Arc::new(LifecycleManager::new());
        debug!("Lifecycle manager initialized");

        // Attach the lifecycle manager to the container *before* any
        // provider is registered: the provider registration path (see
        // `Container::attach_lifecycle`) probes each provider instance for
        // lifecycle hook trait implementations at the moment it's
        // registered, so this must happen before `register_module` below.
        container.attach_lifecycle(&lifecycle);

        // Initialize the root module
        let root_module = M::default();
        debug!("Root module instantiated");

        info!("Registering modules and dependencies");

        // Register all providers and controllers from the module tree
        let mut guards: Vec<ScopedGuard> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        Self::register_module(
            &container,
            &mut router,
            &mut guards,
            &mut visited,
            &root_module,
        );

        info!("Executing lifecycle hooks");

        // Fail-open: hook errors below are logged, not propagated. See the
        // "Lifecycle hook failures are fail-open" section on this method's
        // doc comment.

        // Call module init hooks
        debug!("Calling OnModuleInit hooks");
        if let Err(errors) = lifecycle.call_module_init_hooks().await {
            warn!(error_count = errors.len(), "Some module init hooks failed");
            for (name, error) in errors {
                error!(hook_name = %name, error = %error, "Module init hook failed");
            }
        } else {
            debug!("All OnModuleInit hooks completed successfully");
        }

        // Call bootstrap hooks
        debug!("Calling OnApplicationBootstrap hooks");
        if let Err(errors) = lifecycle.call_bootstrap_hooks().await {
            warn!(error_count = errors.len(), "Some bootstrap hooks failed");
            for (name, error) in errors {
                error!(hook_name = %name, error = %error, "Bootstrap hook failed");
            }
        } else {
            debug!("All OnApplicationBootstrap hooks completed successfully");
        }

        info!("Application bootstrap complete");

        Self {
            container,
            router: Arc::new(router),
            lifecycle,
            pipeline_config: PipelineConfig::default(),
            pipeline_stats: Arc::new(PipelineStats::new()),
            http2_config: Http2Config::default(),
            http2_stats: Arc::new(Http2Stats::new()),
            http3_config: Http3Config::default(),
            http3_stats: Arc::new(Http3Stats::new()),
            cors_config: None,
            guards,
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            epoll_config: None,
            filter_chain: None,
        }
    }

    /// Get a reference to the lifecycle manager
    pub fn lifecycle(&self) -> &Arc<LifecycleManager> {
        &self.lifecycle
    }

    /// Gracefully shutdown the application
    pub async fn shutdown(
        &self,
        signal: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(signal = ?signal, "Gracefully shutting down application");

        // Call before shutdown hooks
        debug!("Calling BeforeApplicationShutdown hooks");
        if let Err(errors) = self
            .lifecycle
            .call_before_shutdown_hooks(signal.clone())
            .await
        {
            warn!(
                error_count = errors.len(),
                "Some before shutdown hooks failed"
            );
            for (name, error) in errors {
                error!(hook_name = %name, error = %error, "Before shutdown hook failed");
            }
        } else {
            debug!("All BeforeApplicationShutdown hooks completed successfully");
        }

        // Call shutdown hooks
        debug!("Calling OnApplicationShutdown hooks");
        if let Err(errors) = self.lifecycle.call_shutdown_hooks(signal.clone()).await {
            warn!(error_count = errors.len(), "Some shutdown hooks failed");
            for (name, error) in errors {
                error!(hook_name = %name, error = %error, "Shutdown hook failed");
            }
        } else {
            debug!("All OnApplicationShutdown hooks completed successfully");
        }

        // Call module destroy hooks
        debug!("Calling OnModuleDestroy hooks");
        if let Err(errors) = self.lifecycle.call_module_destroy_hooks().await {
            warn!(
                error_count = errors.len(),
                "Some module destroy hooks failed"
            );
            for (name, error) in errors {
                error!(hook_name = %name, error = %error, "Module destroy hook failed");
            }
        } else {
            debug!("All OnModuleDestroy hooks completed successfully");
        }

        info!("Application shutdown complete");
        Ok(())
    }

    /// Initialize logging with default configuration
    ///
    /// This is a convenience method that initializes JSON logging to STDOUT.
    /// For more control, use `LogConfig` directly.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::Application;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let _guard = Application::init_logging();
    ///     // Application code...
    /// }
    /// ```
    pub fn init_logging() -> Option<crate::logging::tracing_appender::non_blocking::WorkerGuard> {
        crate::logging::LogConfig::default().init()
    }

    /// Initialize logging with custom configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::{Application, LogConfig, LogLevel, LogFormat};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let config = LogConfig::new()
    ///         .level(LogLevel::Debug)
    ///         .format(LogFormat::Pretty);
    ///
    ///     let _guard = Application::init_logging_with_config(config);
    ///     // Application code...
    /// }
    /// ```
    pub fn init_logging_with_config(
        config: crate::logging::LogConfig,
    ) -> Option<crate::logging::tracing_appender::non_blocking::WorkerGuard> {
        config.init()
    }

    /// Register a module and its imports recursively.
    ///
    /// # Guard scoping
    ///
    /// Guards declared by a module are **not** application-global. Each
    /// module's guards are scoped to the base paths of the controllers that the
    /// **same** module registers (via `module.controllers()`): a guard `G` in a
    /// module whose controllers have base paths `[P1, P2]` is stored once per
    /// base path and runs only for requests whose path falls under `P1` or
    /// `P2`. Recursion does not widen this: an imported or child module's guards
    /// scope to that child's own controllers, never to the parent's. A module
    /// that declares guards but registers no controllers has nothing to scope
    /// to, so its guards are inert (a warning is emitted). Manually-added guards
    /// (see [`Application::with_guard`]) use an empty prefix and stay global.
    fn register_module(
        container: &Container,
        router: &mut Router,
        guards: &mut Vec<ScopedGuard>,
        visited: &mut std::collections::HashSet<std::any::TypeId>,
        module: &dyn Module,
    ) {
        // Dedup by the *concrete* module's `TypeId` (`Module::module_type_id`),
        // not `std::any::type_name_of_val(module)`. The latter resolves its
        // type parameter from the *static* type of the `module: &dyn Module`
        // parameter, so it always evaluates to the trait object's own type
        // name (the same string for every module) rather than the concrete
        // type behind the vtable. Keyed that way, the very first module
        // `register_module` ever touches (the root) claims the one shared
        // key, and every module reached afterwards — any import, re-export,
        // or sibling, not just true diamond re-imports — collides with it
        // and is silently skipped.
        let module_id = module.module_type_id();
        let module_type = module.module_type_name();

        // Each module registers once: diamond imports must not duplicate
        // providers/routes, and cyclic imports must not recurse forever.
        if !visited.insert(module_id) {
            debug!(
                module_type = module_type,
                "Module already registered, skipping"
            );
            return;
        }
        debug!(module_type = module_type, "Registering module");

        // First, recursively register imported modules
        let imports = module.imports();
        if !imports.is_empty() {
            debug!(
                module_type = module_type,
                import_count = imports.len(),
                "Registering imported modules"
            );
            for imported_module in imports {
                Self::register_module(container, router, guards, visited, imported_module.as_ref());
            }
        }

        // Register re-exported modules (they need to be registered too)
        let re_exports = module.re_exports();
        if !re_exports.is_empty() {
            debug!(
                module_type = module_type,
                re_export_count = re_exports.len(),
                "Registering re-exported modules"
            );
            for re_exported_module in re_exports {
                Self::register_module(
                    container,
                    router,
                    guards,
                    visited,
                    re_exported_module.as_ref(),
                );
            }
        }

        // Register all providers
        let providers = module.providers();
        debug!(
            module_type = module_type,
            provider_count = providers.len(),
            "Registering providers"
        );
        for provider_reg in providers {
            // Call the registration function which will register the provider in the container
            (provider_reg.register_fn)(container);
            debug!(
                module_type = module_type,
                provider = provider_reg.type_name,
                "Provider registered"
            );
        }

        // Register all guards.
        //
        // Module guards are scoped to the base paths of the controllers that
        // THIS SAME module registers, so a guard runs only for requests to its
        // own module's controllers — not for every request, and not for
        // controllers belonging to imported/child modules. A module that
        // declares guards but has no controllers has nothing to scope to, so
        // those guards are inert and a warning is emitted.
        let guard_regs = module.guards();
        if !guard_regs.is_empty() {
            // Base paths of this module's own controllers to scope guards to.
            let controller_paths: Vec<&'static str> =
                module.controllers().iter().map(|c| c.base_path).collect();
            debug!(
                module_type = module_type,
                guard_count = guard_regs.len(),
                controller_count = controller_paths.len(),
                "Registering guards"
            );
            for guard_reg in guard_regs {
                match (guard_reg.factory)(container) {
                    Ok(guard) => {
                        if controller_paths.is_empty() {
                            warn!(
                                module_type = module_type,
                                guard = guard_reg.type_name,
                                "Module declares a guard but registers no controllers; \
                                 the guard is inert and will not run for any request"
                            );
                        } else {
                            for base_path in &controller_paths {
                                guards.push(ScopedGuard {
                                    prefix: base_path.to_string(),
                                    guard: guard.clone(),
                                });
                            }
                            debug!(
                                module_type = module_type,
                                guard = guard_reg.type_name,
                                scoped_to = ?controller_paths,
                                "Guard registered (scoped to module's controller base paths)"
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            module_type = module_type,
                            guard = guard_reg.type_name,
                            error = %e,
                            "Failed to instantiate guard"
                        );
                    }
                }
            }
        }

        // Register all controllers
        let controllers = module.controllers();
        debug!(
            module_type = module_type,
            controller_count = controllers.len(),
            "Registering controllers"
        );
        for controller_reg in controllers {
            // Instantiate controller with DI
            match (controller_reg.factory)(container) {
                Ok(controller_instance) => {
                    // Register routes for this controller
                    if let Err(e) =
                        (controller_reg.route_registrar)(container, router, controller_instance)
                    {
                        error!(
                            module_type = module_type,
                            controller = controller_reg.type_name,
                            error = %e,
                            "Failed to register routes for controller"
                        );
                    } else {
                        debug!(
                            module_type = module_type,
                            controller = controller_reg.type_name,
                            base_path = controller_reg.base_path,
                            "Controller registered"
                        );
                    }
                }
                Err(e) => {
                    error!(
                        module_type = module_type,
                        controller = controller_reg.type_name,
                        error = %e,
                        "Failed to instantiate controller"
                    );
                }
            }
        }

        debug!(module_type = module_type, "Module registration complete");
    }

    /// Start the HTTP server on the specified port
    ///
    /// Uses HTTP/1.1 pipelining for improved throughput. Configure pipelining
    /// behavior with `with_pipeline_config()` before calling this method.
    ///
    /// # Pipelining
    ///
    /// HTTP/1.1 pipelining allows clients to send multiple requests on the
    /// same connection without waiting for responses. This significantly
    /// improves throughput, especially on high-latency connections.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, pipeline::PipelineConfig};
    ///
    /// let app = Application::new(container, router)
    ///     .with_pipeline_config(PipelineConfig::high_performance());
    ///
    /// app.listen(8080).await?;
    /// ```
    ///
    /// This binds to all interfaces (`0.0.0.0`) on the given port. To bind to a
    /// specific address (e.g. loopback only, or an ephemeral `:0` port), use
    /// [`Application::listen_on`].
    pub async fn listen(self, port: u16) -> Result<(), Error> {
        self.listen_on((std::net::Ipv4Addr::UNSPECIFIED, port))
            .await
    }

    /// Start the HTTP server on the specified socket address.
    ///
    /// This is the address-accepting counterpart to [`Application::listen`],
    /// which binds to all interfaces on a port. `listen_on` accepts anything
    /// convertible into a [`SocketAddr`], allowing binds to a specific
    /// interface, IPv6, or an OS-assigned ephemeral port (`:0`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::Application;
    /// use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    ///
    /// # async fn example(app: Application) -> Result<(), Box<dyn std::error::Error>> {
    /// // Loopback only, port 8080
    /// app.listen_on(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_on(self, addr: impl Into<SocketAddr>) -> Result<(), Error> {
        let addr = addr.into();

        debug!(address = %addr, "Binding to address");
        let listener = TcpListener::bind(addr).await?;

        #[cfg(unix)]
        let socket_tuning = self.epoll_config.clone();
        #[cfg(unix)]
        if let Some(ref tuning) = socket_tuning {
            use std::os::unix::io::AsRawFd;
            apply_socket_tuning(listener.as_raw_fd(), tuning, "listener");
        }

        info!(
            address = %addr,
            pipeline_mode = ?self.pipeline_config.mode,
            pipeline_flush = self.pipeline_config.pipeline_flush,
            max_concurrent = self.pipeline_config.max_concurrent,
            "HTTP server listening with pipelining enabled"
        );

        let state = self.serve_state(self.cors_config.clone());
        let pipeline_builder = PipelinedHttp1Builder::with_stats(
            self.pipeline_config.clone(),
            Arc::clone(&self.pipeline_stats),
        );
        let pipeline_stats = Arc::clone(&self.pipeline_stats);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            trace!(client_address = %client_addr, "Connection accepted");

            // Apply TCP_NODELAY if configured
            if pipeline_builder.config().tcp_nodelay
                && let Err(e) = stream.set_nodelay(true)
            {
                trace!(error = %e, "Failed to set TCP_NODELAY");
            }

            // Apply opt-in socket tuning to the accepted socket
            #[cfg(unix)]
            if let Some(ref tuning) = socket_tuning {
                use std::os::unix::io::AsRawFd;
                apply_socket_tuning(stream.as_raw_fd(), tuning, "accepted connection");
            }

            let io = TokioIo::new(stream);
            let state = state.clone();
            let http_builder = pipeline_builder.configure_hyper_builder();
            let stats = Arc::clone(&pipeline_stats);

            // Track connection
            stats.connection_opened();

            tokio::spawn(async move {
                let stats_for_close = Arc::clone(&stats);
                let service = service_fn(move |req: Request<IncomingBody>| {
                    let state = state.clone();
                    let stats = Arc::clone(&stats);
                    async move {
                        stats.request_processed();
                        handle_request(req, state).await
                    }
                });

                if let Err(err) = http_builder.serve_connection(io, service).await {
                    error!(error = %err, client = %client_addr, "Error serving connection");
                }

                // Track connection close
                stats_for_close.connection_closed();
            });
        }
    }

    /// Start the HTTPS server with TLS
    ///
    /// # Example
    ///
    /// ```ignore
    /// use armature_core::{Application, TlsConfig, Module};
    ///
    /// #[derive(Clone)]
    /// struct AppModule;
    /// impl Module for AppModule {
    ///     fn name(&self) -> &str { "AppModule" }
    ///     fn controllers(&self) -> Vec<Box<dyn Controller>> { vec![] }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut app = Application::new();
    /// let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
    /// app.listen_https(443, tls).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_https(self, port: u16, tls_config: TlsConfig) -> Result<(), Error> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        debug!(address = %addr, "Binding to address (HTTPS)");
        let listener = TcpListener::bind(addr).await?;

        #[cfg(unix)]
        let socket_tuning = self.epoll_config.clone();
        #[cfg(unix)]
        if let Some(ref tuning) = socket_tuning {
            use std::os::unix::io::AsRawFd;
            apply_socket_tuning(listener.as_raw_fd(), tuning, "listener");
        }

        info!(
            address = %addr,
            pipeline_mode = ?self.pipeline_config.mode,
            pipeline_flush = self.pipeline_config.pipeline_flush,
            "HTTPS server listening with pipelining enabled"
        );

        let acceptor = TlsAcceptor::from(tls_config.server_config);
        let state = self.serve_state(None);
        let pipeline_builder = PipelinedHttp1Builder::with_stats(
            self.pipeline_config.clone(),
            Arc::clone(&self.pipeline_stats),
        );
        let pipeline_stats = Arc::clone(&self.pipeline_stats);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            trace!(client_address = %client_addr, "HTTPS connection accepted");

            // Apply TCP_NODELAY if configured
            if pipeline_builder.config().tcp_nodelay
                && let Err(e) = stream.set_nodelay(true)
            {
                trace!(error = %e, "Failed to set TCP_NODELAY");
            }

            // Apply opt-in socket tuning to the accepted socket
            #[cfg(unix)]
            if let Some(ref tuning) = socket_tuning {
                use std::os::unix::io::AsRawFd;
                apply_socket_tuning(stream.as_raw_fd(), tuning, "accepted connection");
            }

            let acceptor = acceptor.clone();
            let state = state.clone();
            let http_builder = pipeline_builder.configure_hyper_builder();
            let stats = Arc::clone(&pipeline_stats);

            // Track connection
            stats.connection_opened();

            tokio::spawn(async move {
                let stats_for_close = Arc::clone(&stats);
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        debug!(client = %client_addr, "TLS handshake successful");
                        let io = TokioIo::new(tls_stream);

                        let service = service_fn(move |req: Request<IncomingBody>| {
                            let state = state.clone();
                            let stats = Arc::clone(&stats);
                            async move {
                                stats.request_processed();
                                handle_request(req, state).await
                            }
                        });

                        if let Err(err) = http_builder.serve_connection(io, service).await {
                            error!(error = %err, client = %client_addr, "Error serving HTTPS connection");
                        }
                    }
                    Err(err) => {
                        error!(error = %err, client = %client_addr, "TLS handshake failed");
                    }
                }

                // Track connection close
                stats_for_close.connection_closed();
            });
        }
    }

    /// Start HTTPS server with optional HTTP to HTTPS redirect
    ///
    /// This method starts both an HTTPS server and optionally an HTTP server that redirects
    /// all traffic to HTTPS.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use armature_core::{Application, HttpsConfig, TlsConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut app = Application::new();
    /// let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
    /// let https_config = HttpsConfig::new("0.0.0.0:443", tls)
    ///     .with_http_redirect("0.0.0.0:80");
    /// app.listen_with_config(https_config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_with_config(self, config: HttpsConfig) -> Result<(), Error> {
        let state = self.serve_state(None);

        // Start HTTP redirect server if configured
        if let Some(ref http_addr) = config.http_redirect_addr {
            let https_port = config
                .https_addr
                .split(':')
                .next_back()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(443);

            let http_addr = http_addr.clone();
            tokio::spawn(async move {
                if let Err(e) = start_http_redirect_server(&http_addr, https_port).await {
                    eprintln!("HTTP redirect server failed: {}", e);
                }
            });
        }

        // Parse HTTPS address
        let https_addr: SocketAddr = config
            .https_addr
            .parse()
            .map_err(|e| Error::Internal(format!("Invalid HTTPS address: {}", e)))?;

        let listener = TcpListener::bind(https_addr).await?;

        #[cfg(unix)]
        let socket_tuning = self.epoll_config.clone();
        #[cfg(unix)]
        if let Some(ref tuning) = socket_tuning {
            use std::os::unix::io::AsRawFd;
            apply_socket_tuning(listener.as_raw_fd(), tuning, "listener");
        }

        println!("🔒 HTTPS Server listening on https://{}", https_addr);
        if config.http_redirect_addr.is_some() {
            println!("↪️  HTTP redirect server enabled");
        }

        let acceptor = TlsAcceptor::from(config.tls.server_config);

        loop {
            let (stream, _) = listener.accept().await?;

            // Apply opt-in socket tuning to the accepted socket
            #[cfg(unix)]
            if let Some(ref tuning) = socket_tuning {
                use std::os::unix::io::AsRawFd;
                apply_socket_tuning(stream.as_raw_fd(), tuning, "accepted connection");
            }

            let acceptor = acceptor.clone();
            let state = state.clone();

            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let io = TokioIo::new(tls_stream);

                        let service = service_fn(move |req: Request<IncomingBody>| {
                            let state = state.clone();
                            async move { handle_request(req, state).await }
                        });

                        if let Err(err) = http1::Builder::new().serve_connection(io, service).await
                        {
                            eprintln!("Error serving HTTPS connection: {:?}", err);
                        }
                    }
                    Err(err) => {
                        eprintln!("TLS handshake failed: {:?}", err);
                    }
                }
            });
        }
    }

    /// Start HTTP/2 cleartext server (h2c)
    ///
    /// **Warning**: HTTP/2 cleartext (h2c) is not recommended for production.
    /// Use `listen_https_h2` for TLS-secured HTTP/2.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::Application;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = Application::new(container, router);
    /// app.listen_h2c(8080).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_h2c(self, port: u16) -> Result<(), Error> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        debug!(address = %addr, "Binding to address (HTTP/2 cleartext)");
        let listener = TcpListener::bind(addr).await?;

        #[cfg(unix)]
        let socket_tuning = self.epoll_config.clone();
        #[cfg(unix)]
        if let Some(ref tuning) = socket_tuning {
            use std::os::unix::io::AsRawFd;
            apply_socket_tuning(listener.as_raw_fd(), tuning, "listener");
        }

        info!(
            address = %addr,
            max_concurrent_streams = self.http2_config.max_concurrent_streams,
            "HTTP/2 cleartext server listening (h2c)"
        );
        warn!("HTTP/2 cleartext (h2c) is not recommended for production. Use HTTPS.");

        let state = self.serve_state(None);
        let h2_builder =
            Http2Builder::with_stats(self.http2_config.clone(), Arc::clone(&self.http2_stats));
        let h2_stats = Arc::clone(&self.http2_stats);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            trace!(client_address = %client_addr, "HTTP/2 connection accepted");

            // Apply opt-in socket tuning to the accepted socket
            #[cfg(unix)]
            if let Some(ref tuning) = socket_tuning {
                use std::os::unix::io::AsRawFd;
                apply_socket_tuning(stream.as_raw_fd(), tuning, "accepted connection");
            }

            let io = TokioIo::new(stream);
            let state = state.clone();
            let http_builder = h2_builder.configure_hyper_builder();
            let stats = Arc::clone(&h2_stats);

            // Track connection
            stats.connection_opened();

            tokio::spawn(async move {
                let stats_for_close = Arc::clone(&stats);
                let service = service_fn(move |req: Request<IncomingBody>| {
                    let state = state.clone();
                    let stats = Arc::clone(&stats);
                    async move {
                        stats.request_processed();
                        handle_request(req, state).await
                    }
                });

                if let Err(err) = http_builder.serve_connection(io, service).await {
                    error!(error = %err, client = %client_addr, "Error serving HTTP/2 connection");
                }

                // Track connection close
                stats_for_close.connection_closed();
            });
        }
    }

    /// Start HTTPS server with HTTP/2 support (ALPN negotiation)
    ///
    /// This method automatically negotiates the best protocol:
    /// - If client supports HTTP/2 and advertises "h2" via ALPN, use HTTP/2
    /// - Otherwise, fall back to HTTP/1.1
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, TlsConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = Application::new(container, router);
    /// let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
    /// app.listen_https_h2(443, tls).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn listen_https_h2(self, port: u16, tls_config: TlsConfig) -> Result<(), Error> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        debug!(address = %addr, "Binding to address (HTTPS with HTTP/2)");
        let listener = TcpListener::bind(addr).await?;

        #[cfg(unix)]
        let socket_tuning = self.epoll_config.clone();
        #[cfg(unix)]
        if let Some(ref tuning) = socket_tuning {
            use std::os::unix::io::AsRawFd;
            apply_socket_tuning(listener.as_raw_fd(), tuning, "listener");
        }

        info!(
            address = %addr,
            max_concurrent_streams = self.http2_config.max_concurrent_streams,
            pipeline_mode = ?self.pipeline_config.mode,
            "HTTPS server listening with HTTP/2 and HTTP/1.1 (ALPN)"
        );

        let acceptor = TlsAcceptor::from(tls_config.server_config);
        let state = self.serve_state(None);
        let h1_builder = PipelinedHttp1Builder::with_stats(
            self.pipeline_config.clone(),
            Arc::clone(&self.pipeline_stats),
        );
        let h2_builder =
            Http2Builder::with_stats(self.http2_config.clone(), Arc::clone(&self.http2_stats));
        let h1_stats = Arc::clone(&self.pipeline_stats);
        let h2_stats = Arc::clone(&self.http2_stats);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            trace!(client_address = %client_addr, "Connection accepted, starting TLS handshake");

            // Apply opt-in socket tuning to the accepted socket
            #[cfg(unix)]
            if let Some(ref tuning) = socket_tuning {
                use std::os::unix::io::AsRawFd;
                apply_socket_tuning(stream.as_raw_fd(), tuning, "accepted connection");
            }

            let acceptor = acceptor.clone();
            let state = state.clone();
            let h1_builder_ref = h1_builder.configure_hyper_builder();
            let h2_builder_ref = h2_builder.configure_hyper_builder();
            let h1_stats = Arc::clone(&h1_stats);
            let h2_stats = Arc::clone(&h2_stats);

            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        // Check negotiated ALPN protocol
                        let (_, session) = tls_stream.get_ref();
                        let protocol = session.alpn_protocol();

                        let is_h2 = protocol.map(|p| p == b"h2").unwrap_or(false);

                        if is_h2 {
                            debug!(client = %client_addr, "Using HTTP/2 (ALPN negotiated h2)");
                            h2_stats.connection_opened();

                            let io = TokioIo::new(tls_stream);
                            let stats = Arc::clone(&h2_stats);

                            let service = service_fn(move |req: Request<IncomingBody>| {
                                let state = state.clone();
                                let stats = Arc::clone(&stats);
                                async move {
                                    stats.request_processed();
                                    handle_request(req, state).await
                                }
                            });

                            if let Err(err) = h2_builder_ref.serve_connection(io, service).await {
                                error!(error = %err, client = %client_addr, "Error serving HTTP/2 connection");
                            }

                            h2_stats.connection_closed();
                        } else {
                            debug!(client = %client_addr, "Using HTTP/1.1 (ALPN fallback)");
                            h1_stats.connection_opened();

                            let io = TokioIo::new(tls_stream);
                            let stats = Arc::clone(&h1_stats);

                            let service = service_fn(move |req: Request<IncomingBody>| {
                                let state = state.clone();
                                let stats = Arc::clone(&stats);
                                async move {
                                    stats.request_processed();
                                    handle_request(req, state).await
                                }
                            });

                            if let Err(err) = h1_builder_ref.serve_connection(io, service).await {
                                error!(error = %err, client = %client_addr, "Error serving HTTP/1.1 connection");
                            }

                            h1_stats.connection_closed();
                        }
                    }
                    Err(err) => {
                        error!(error = %err, client = %client_addr, "TLS handshake failed");
                    }
                }
            });
        }
    }

    /// Start HTTP/3 (QUIC) server
    ///
    /// HTTP/3 uses QUIC (UDP) instead of TCP, providing:
    /// - 0-RTT connection establishment
    /// - No head-of-line blocking
    /// - Connection migration (mobile-friendly)
    /// - Built-in encryption (TLS 1.3)
    ///
    /// **Note**: Requires the `http3` feature to be enabled.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, TlsConfig, Http3Config};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = Application::new(container, router)
    ///     .with_http3_config(Http3Config::low_latency());
    ///
    /// let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
    /// app.listen_h3(443, tls).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "http3")]
    pub async fn listen_h3(self, port: u16, tls_config: TlsConfig) -> Result<(), Error> {
        use crate::http3::Http3Server;

        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        info!(
            address = %addr,
            max_concurrent_streams = self.http3_config.max_concurrent_bidi_streams,
            enable_0rtt = self.http3_config.enable_0rtt,
            "Starting HTTP/3 (QUIC) server"
        );

        // Compile the linear router into the O(1) optimized router once,
        // matching the TCP serve paths.
        let optimized = Arc::new(crate::route_cache::OptimizedRouter::from_router(
            &self.router,
        ));
        let server = Http3Server::new(self.http3_config.clone(), optimized);

        server.listen(addr, tls_config.server_config).await
    }

    /// Start dual-stack server: HTTP/3 (QUIC/UDP) + HTTPS (TCP)
    ///
    /// This runs both servers on the same port number (different protocols):
    /// - HTTP/3 on UDP port (for modern clients)
    /// - HTTPS with HTTP/2/HTTP/1.1 on TCP port (for compatibility)
    ///
    /// Add `Alt-Svc` header to responses to advertise HTTP/3:
    /// ```text
    /// Alt-Svc: h3=":443"; ma=86400
    /// ```
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use armature_core::{Application, TlsConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let app = Application::new(container, router);
    /// let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
    ///
    /// // Runs both HTTP/3 (UDP) and HTTPS (TCP) on port 443
    /// app.listen_dual_stack(443, tls).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "http3")]
    pub async fn listen_dual_stack(self, port: u16, tls_config: TlsConfig) -> Result<(), Error> {
        use crate::http3::Http3Server;

        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        info!(
            address = %addr,
            "Starting dual-stack server (HTTP/3 + HTTPS)"
        );

        // Clone for the two servers
        let tls_config_h3 = tls_config.clone();
        let router_h3 = Arc::new(crate::route_cache::OptimizedRouter::from_router(
            &self.router,
        ));
        let http3_config = self.http3_config.clone();

        // Start HTTP/3 server (UDP)
        let h3_handle = tokio::spawn(async move {
            let server = Http3Server::new(http3_config, router_h3);
            if let Err(e) = server.listen(addr, tls_config_h3.server_config).await {
                error!(error = %e, "HTTP/3 server error");
            }
        });

        // Start HTTPS server with HTTP/2 (TCP)
        let https_handle = tokio::spawn(async move {
            if let Err(e) = self.listen_https_h2(port, tls_config).await {
                error!(error = %e, "HTTPS server error");
            }
        });

        // Wait for either to finish (usually they run forever)
        tokio::select! {
            _ = h3_handle => {
                warn!("HTTP/3 server stopped");
            }
            _ = https_handle => {
                warn!("HTTPS server stopped");
            }
        }

        Ok(())
    }

    /// Get a reference to the DI container
    pub fn container(&self) -> &Container {
        &self.container
    }
}

/// Apply the configured socket tuning options to a raw fd, logging a
/// warning on failure. Never fails the caller.
#[cfg(unix)]
fn apply_socket_tuning(fd: std::os::unix::io::RawFd, config: &EpollConfig, socket: &'static str) {
    if let Err(e) = crate::epoll_tuning::configure_socket(fd, config) {
        warn!(error = %e, socket, "Failed to apply socket tuning");
    }
}

/// Start HTTP server that redirects all requests to HTTPS
async fn start_http_redirect_server(addr: &str, https_port: u16) -> Result<(), Error> {
    let addr: SocketAddr = addr
        .parse()
        .map_err(|e| Error::Internal(format!("Invalid HTTP redirect address: {}", e)))?;

    let listener = TcpListener::bind(addr).await?;

    println!("↪️  HTTP redirect server listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<IncomingBody>| async move {
                // Redirect to HTTPS
                let host = req
                    .headers()
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("localhost");

                // Remove port from host if present
                let host_without_port = host.split(':').next().unwrap_or(host);

                let location = if https_port == 443 {
                    format!("https://{}{}", host_without_port, req.uri().path())
                } else {
                    format!(
                        "https://{}:{}{}",
                        host_without_port,
                        https_port,
                        req.uri().path()
                    )
                };

                let response = Response::builder()
                    .status(301)
                    .header("Location", location)
                    .body(Full::new(bytes::Bytes::from("Redirecting to HTTPS...")))
                    .unwrap();

                Ok::<_, hyper::Error>(response)
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("Error serving HTTP redirect: {:?}", err);
            }
        });
    }
}

/// Handle an incoming HTTP request
async fn handle_request(
    req: Request<IncomingBody>,
    state: ServeState,
) -> Result<Response<Full<bytes::Bytes>>, hyper::Error> {
    use std::time::Instant;

    let start = Instant::now();

    // Convert hyper request to our HttpRequest
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_owned);

    trace!(method = %method, path = %path, "Incoming request");

    if method == "OPTIONS"
        && let Some(ref cors) = state.cors
    {
        let mut builder = Response::builder().status(204);
        builder = builder.header("Access-Control-Allow-Origin", &cors.allow_origin);
        builder = builder.header("Access-Control-Allow-Methods", &cors.allow_methods);
        builder = builder.header("Access-Control-Allow-Headers", &cors.allow_headers);
        builder = builder.header("Access-Control-Max-Age", cors.max_age.to_string());
        if cors.allow_credentials {
            builder = builder.header("Access-Control-Allow-Credentials", "true");
        }
        return Ok(builder.body(Full::new(bytes::Bytes::new())).unwrap());
    }

    let mut armature_req = HttpRequest::new(method.clone(), path.clone());

    // Parse query parameters (percent-decoded)
    if let Some(ref q) = query {
        armature_req.query_params = crate::simd_parser::parse_query_string_decoded(q);
    }

    // Copy headers
    let header_count = req.headers().len();
    for (name, value) in req.headers() {
        if let Ok(value_str) = value.to_str() {
            armature_req.headers.insert(name, value_str);
        }
    }
    trace!(header_count = header_count, "Headers parsed");

    // Fast-path rejection: if the client declares a Content-Length larger than
    // the configured limit, reject with 413 before buffering any body bytes.
    // The streaming `Limited` wrapper below still enforces the limit for
    // chunked or undeclared bodies.
    if let Some(declared_len) = armature_req
        .headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        && !body_within_limit(declared_len, state.max_body_size)
    {
        warn!(
            method = %method,
            path = %path,
            limit = state.max_body_size,
            declared_len,
            "Request Content-Length exceeds configured limit"
        );
        return Ok(to_hyper_response(
            payload_too_large_response(),
            state.cors.as_deref(),
        ));
    }

    // Read body into Bytes (zero-copy after this point), enforcing the
    // configured size limit before the body is buffered in memory.
    let limited = Limited::new(req.into_body(), state.max_body_size);
    let body_bytes = match limited.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) if err.is::<http_body_util::LengthLimitError>() => {
            warn!(
                method = %method,
                path = %path,
                limit = state.max_body_size,
                "Request body exceeds configured limit"
            );
            return Ok(to_hyper_response(
                payload_too_large_response(),
                state.cors.as_deref(),
            ));
        }
        Err(err) => match err.downcast::<hyper::Error>() {
            Ok(hyper_err) => return Err(*hyper_err),
            Err(other) => {
                warn!(method = %method, path = %path, error = %other, "Failed to read request body");
                return Ok(to_hyper_response(
                    HttpResponse::new(400),
                    state.cors.as_deref(),
                ));
            }
        },
    };
    let body_size = body_bytes.len();

    // Use zero-copy body storage
    if body_size > 0 {
        armature_req.set_body_bytes(body_bytes);
        trace!(body_size = body_size, "Request body received (zero-copy)");
    }

    // Only needed when a global exception filter chain is configured: a
    // filter's `catch()` receives the original request for context (path,
    // headers, request id, ...), matching `ExceptionContext::from_request`.
    // Guard evaluation and routing below each consume `armature_req` by
    // value, so it must be captured before either runs.
    //
    // Limitation (documented, not confirmed to be a bug): because this clone
    // is taken *before* guards run and *before* routing populates path
    // params, `ExceptionContext::request` as seen by a filter's `catch()` is
    // a pre-guard/pre-routing snapshot. Guard-added request extensions and
    // resolved route/path params are therefore never visible to a filter --
    // only headers, method, path, and body as they arrived on the wire.
    let filter_ctx_request = state.filter_chain.as_ref().map(|_| armature_req.clone());

    // Evaluate guards before routing.
    //
    // Only guards whose scope prefix matches this request path are evaluated:
    // module guards are scoped to the base paths of the declaring module's own
    // controllers, while guards added via `Application::with_guard` use an empty
    // prefix and match every path. Guards always run before routing.
    if !state.guards.is_empty() {
        match evaluate_scoped_guards(&state.guards, &path, armature_req).await {
            Ok(req) => armature_req = req,
            Err(GuardRejection::Reject) => {
                warn!(method = %method, path = %path, "Request rejected by guard");
                let body = serde_json::json!({
                    "error": "Forbidden",
                    "status": 403,
                });
                let response = HttpResponse::new(403)
                    .with_json(&body)
                    .unwrap_or_else(|_| HttpResponse::new(403));
                return Ok(to_hyper_response(response, state.cors.as_deref()));
            }
            Err(GuardRejection::Error(err)) => {
                warn!(method = %method, path = %path, error = %err, "Guard returned an error");
                let response =
                    respond_to_error(err, filter_ctx_request, state.filter_chain.clone()).await;
                return Ok(to_hyper_response(response, state.cors.as_deref()));
            }
        }
    }

    // Route the request
    debug!(method = %method, path = %path, "Routing request");
    let response = match state.router.route(armature_req).await {
        Ok(resp) => {
            debug!(method = %method, path = %path, status = resp.status, "Request handled successfully");
            resp
        }
        Err(err) => {
            warn!(method = %method, path = %path, error = %err, "Request handling failed");
            respond_to_error(err, filter_ctx_request, state.filter_chain.clone()).await
        }
    };

    let duration = start.elapsed();
    debug!(
        method = %method,
        path = %path,
        status = response.status,
        duration_ms = duration.as_millis(),
        "Request completed"
    );

    Ok(to_hyper_response(response, state.cors.as_deref()))
}

/// Convert a handler error into a client-safe HTTP response.
///
/// Thin wrapper over [`Error::to_client_response`], the single canonical
/// error-to-response mapping shared by every server transport: 4xx errors keep
/// their message; 5xx messages are redacted to a generic body so internal
/// details never reach the client. The full error is logged at the call site.
fn error_response(err: &Error) -> HttpResponse {
    err.to_client_response()
}

/// Default upper bound on how long a single global exception filter chain
/// invocation is allowed to run before `respond_to_error` gives up on it and
/// falls back to [`error_response`]. Chosen to comfortably cover any
/// reasonable filter (a synchronous transform, at most a quick lookup) while
/// still bounding worst-case added latency per request; mirrors the 5s
/// safety-net convention already used for socket reads elsewhere in this
/// file's test harness (see `micro.rs`'s `send_raw_request`).
const DEFAULT_EXCEPTION_FILTER_TIMEOUT: Duration = Duration::from_secs(5);

/// Convert a handler/guard error into an `HttpResponse`, trying the
/// application's global exception filter chain first (see
/// [`Application::use_global_filter`]) before falling back to
/// [`error_response`].
///
/// Both `filter_chain` and `ctx_request` are `None` unless a filter has
/// actually been registered (see `handle_request`'s `filter_ctx_request`),
/// so the fallback path is exercised for every request when no filter is
/// configured -- identical to this framework's behavior before
/// `use_global_filter` existed.
///
/// Thin wrapper over [`respond_to_error_with_timeout`] using
/// [`DEFAULT_EXCEPTION_FILTER_TIMEOUT`]; see that function for the
/// panic/timeout isolation guarantees around the filter chain invocation.
async fn respond_to_error(
    err: Error,
    ctx_request: Option<HttpRequest>,
    filter_chain: Option<Arc<ExceptionFilterChain>>,
) -> HttpResponse {
    respond_to_error_with_timeout(
        err,
        ctx_request,
        filter_chain,
        DEFAULT_EXCEPTION_FILTER_TIMEOUT,
    )
    .await
}

/// Same as [`respond_to_error`], but with an explicit filter-chain timeout
/// (split out so tests can exercise the timeout path without an actual
/// multi-second wait).
///
/// A registered exception filter runs arbitrary, user-supplied code
/// (`ExceptionFilter::catch()`). Without isolation, a filter implementation
/// that panics would unwind the request task and one that hangs would stall
/// the connection forever -- either way taking down request handling for a
/// bug in third-party filter code, on the error path no less, which is
/// exactly when the server should be at its most robust. To prevent that,
/// the filter chain call runs on its own `tokio::spawn`ed task:
///
/// - A panic inside `catch()` unwinds only that spawned task; it surfaces
///   here as `Err(JoinError)` rather than propagating into the caller, and
///   is treated the same as a timeout.
/// - `tokio::time::timeout` bounds how long the task is waited on; if it
///   fires, the still-running task is aborted so it doesn't leak.
///
/// In both failure modes, `err`'s fallback response ([`error_response`],
/// identical to what would be returned with no filter chain configured at
/// all) is used -- the filter is treated as if it had declined to handle the
/// error (returned `None`), not as if the request itself had failed.
async fn respond_to_error_with_timeout(
    err: Error,
    ctx_request: Option<HttpRequest>,
    filter_chain: Option<Arc<ExceptionFilterChain>>,
    filter_timeout: Duration,
) -> HttpResponse {
    match (filter_chain, ctx_request) {
        (Some(chain), Some(request)) => {
            // Computed before `err` is moved into the isolated task below,
            // so it's available as the fallback on either a panic or a
            // timeout without requiring `Error` to be `Clone`.
            let fallback = error_response(&err);

            let task = tokio::spawn(async move { chain.handle(&err, &request).await });
            let abort_handle = task.abort_handle();

            match tokio::time::timeout(filter_timeout, task).await {
                Ok(Ok(response)) => response,
                Ok(Err(join_err)) => {
                    error!(
                        error = %join_err,
                        "Exception filter task panicked; falling back to the default error response"
                    );
                    fallback
                }
                Err(_elapsed) => {
                    // The task is still running (or about to start); stop it
                    // so a hanging filter doesn't keep burning resources
                    // forever in the background.
                    abort_handle.abort();
                    warn!(
                        timeout_secs = filter_timeout.as_secs_f64(),
                        "Exception filter chain timed out; falling back to the default error response"
                    );
                    fallback
                }
            }
        }
        _ => {
            // No filter chain configured, or (defensively) no context
            // request captured for it -- identical to the no-filter
            // fallback.
            error_response(&err)
        }
    }
}

/// Returns `true` if a request body of `len` bytes is within the configured
/// `max` limit.
///
/// Bodies exactly at the limit are accepted; anything larger is rejected with
/// `413 Payload Too Large`. Extracted as a pure function so the boundary is
/// unit-testable independent of the HTTP server.
fn body_within_limit(len: usize, max: usize) -> bool {
    len <= max
}

/// Build the `413 Payload Too Large` response with a JSON body matching the
/// canonical `{"error", "status"}` shape.
fn payload_too_large_response() -> HttpResponse {
    let body = serde_json::json!({
        "error": "Payload Too Large",
        "status": 413,
    });
    HttpResponse::new(413)
        .with_json(&body)
        .unwrap_or_else(|_| HttpResponse::new(413))
}

/// Reason a request was rejected while evaluating scoped guards.
enum GuardRejection {
    /// A guard rejected the request (`Ok(false)`) — respond with 403.
    Reject,
    /// A guard returned an error — respond with the error's client response.
    Error(Error),
}

/// Evaluate the guards whose scope prefix matches `path`, in order.
///
/// Module guards are scoped to the base paths of the declaring module's own
/// controllers (see [`Application::register_module`]); guards added via
/// [`Application::with_guard`] use an empty prefix and match every path. Guards
/// that do not match `path` are skipped entirely. Evaluation stops at the first
/// guard that rejects or errors.
///
/// On success returns the (possibly guard-mutated) request so routing can
/// continue; on rejection returns why the request was denied.
async fn evaluate_scoped_guards(
    guards: &[ScopedGuard],
    path: &str,
    request: HttpRequest,
) -> Result<HttpRequest, GuardRejection> {
    let matching: Vec<&ScopedGuard> = guards.iter().filter(|g| g.matches(path)).collect();
    if matching.is_empty() {
        return Ok(request);
    }
    let context = GuardContext::new(request);
    for scoped in matching {
        match scoped.guard.can_activate(&context).await {
            Ok(true) => {}
            Ok(false) => return Err(GuardRejection::Reject),
            Err(err) => return Err(GuardRejection::Error(err)),
        }
    }
    Ok(context.request)
}

/// Convert our HttpResponse to a hyper Response, applying CORS headers.
fn to_hyper_response(
    response: HttpResponse,
    cors: Option<&CorsConfig>,
) -> Response<Full<bytes::Bytes>> {
    let mut builder = Response::builder().status(response.status);

    for (key, value) in &response.headers {
        builder = builder.header(key, value);
    }
    for cookie_value in &response.cookies {
        builder = builder.header("Set-Cookie", cookie_value);
    }
    if let Some(cors) = cors {
        builder = builder.header("Access-Control-Allow-Origin", &cors.allow_origin);
        if cors.allow_credentials {
            builder = builder.header("Access-Control-Allow-Credentials", "true");
        }
    }

    // Zero-copy body passthrough to Hyper
    let body = Full::new(response.into_body_bytes());
    builder.body(body).unwrap_or_else(|_| {
        // A handler produced a header hyper rejects; fail closed with a 500.
        let mut fallback = Response::new(Full::new(bytes::Bytes::new()));
        *fallback.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_with_socket_tuning_stores_config() {
        let app = Application::new(Container::new(), Router::new())
            .with_socket_tuning(EpollConfig::low_latency());

        let config = app
            .epoll_config
            .as_ref()
            .expect("with_socket_tuning should store the config");
        assert_eq!(config.max_events, 256);
        assert!(config.tcp_nodelay);

        // Default is opt-out: no tuning unless requested.
        let plain = Application::new(Container::new(), Router::new());
        assert!(plain.epoll_config.is_none());
    }

    #[test]
    fn test_error_response_redacts_5xx_messages() {
        let err = Error::Internal("db password auth failed for user 'app'".to_string());
        let response = error_response(&err);
        assert_eq!(response.status, 500);
        let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap();
        assert!(!body.contains("db password"));
        assert!(body.contains("Internal Server Error"));
    }

    #[test]
    fn test_error_response_keeps_4xx_messages() {
        let err = Error::NotFound("User not found".to_string());
        let response = error_response(&err);
        assert_eq!(response.status, 404);
        let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap();
        assert!(body.contains("User not found"));
    }

    #[test]
    fn test_to_hyper_response_sets_headers_cookies_and_cors() {
        let response = HttpResponse::ok()
            .content_type("application/json")
            .cookie("session", "abc; HttpOnly")
            .with_body(b"{}".to_vec());
        let cors = CorsConfig::new("https://example.com").with_credentials();

        let hyper_resp = to_hyper_response(response, Some(&cors));
        assert_eq!(hyper_resp.status(), 200);
        assert_eq!(
            hyper_resp.headers().get("Content-Type").unwrap(),
            "application/json"
        );
        assert_eq!(
            hyper_resp.headers().get("Set-Cookie").unwrap(),
            "session=abc; HttpOnly"
        );
        assert_eq!(
            hyper_resp
                .headers()
                .get("Access-Control-Allow-Origin")
                .unwrap(),
            "https://example.com"
        );
        assert_eq!(
            hyper_resp
                .headers()
                .get("Access-Control-Allow-Credentials")
                .unwrap(),
            "true"
        );
    }

    // ---- Body-limit boundary (fix #4) --------------------------------------

    #[test]
    fn test_body_within_limit_boundary() {
        // Exactly at the limit is accepted; one byte over is rejected.
        assert!(body_within_limit(0, 10));
        assert!(body_within_limit(10, 10));
        assert!(!body_within_limit(11, 10));

        let max = DEFAULT_MAX_BODY_SIZE;
        assert!(body_within_limit(max, max));
        assert!(!body_within_limit(max + 1, max));
    }

    #[test]
    fn test_payload_too_large_response_path() {
        let resp = payload_too_large_response();
        assert_eq!(resp.status, 413);
        let body = String::from_utf8(resp.into_body_bytes().to_vec()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], 413);
        assert_eq!(parsed["error"], "Payload Too Large");
    }

    // ---- Scoped guards (fix #3) --------------------------------------------

    struct AllowGuard;
    #[async_trait::async_trait]
    impl Guard for AllowGuard {
        async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
            Ok(true)
        }
    }

    struct RecordingGuard {
        ran: Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl Guard for RecordingGuard {
        async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
            self.ran.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        }
    }

    #[test]
    fn test_scoped_guard_matches_is_segment_aware() {
        let g = ScopedGuard {
            prefix: "/admin".to_string(),
            guard: Arc::new(AllowGuard),
        };
        assert!(g.matches("/admin"));
        assert!(g.matches("/admin/users"));
        // Segment-aware: /administrators must NOT match /admin.
        assert!(!g.matches("/administrators"));
        assert!(!g.matches("/public"));

        // Empty prefix is a genuinely global guard.
        let global = ScopedGuard {
            prefix: String::new(),
            guard: Arc::new(AllowGuard),
        };
        assert!(global.matches("/anything"));
        assert!(global.matches("/"));

        // A "/" prefix is also global.
        let root = ScopedGuard {
            prefix: "/".to_string(),
            guard: Arc::new(AllowGuard),
        };
        assert!(root.matches("/anything"));
    }

    #[tokio::test]
    async fn test_scoped_guard_runs_only_for_its_controller_path() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guards = vec![ScopedGuard {
            prefix: "/admin".to_string(),
            guard: Arc::new(RecordingGuard { ran: ran.clone() }),
        }];

        // Matches /admin/x → guard runs.
        let req = HttpRequest::new("GET", "/admin/x".to_string());
        let decision = evaluate_scoped_guards(&guards, "/admin/x", req).await;
        assert!(decision.is_ok());
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));

        // Does NOT match /public/y → guard is not evaluated.
        ran.store(false, std::sync::atomic::Ordering::SeqCst);
        let req = HttpRequest::new("GET", "/public/y".to_string());
        let decision = evaluate_scoped_guards(&guards, "/public/y", req).await;
        assert!(decision.is_ok());
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));

        // /administrators must NOT match /admin → guard not evaluated.
        ran.store(false, std::sync::atomic::Ordering::SeqCst);
        let req = HttpRequest::new("GET", "/administrators".to_string());
        let _ = evaluate_scoped_guards(&guards, "/administrators", req).await;
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    fn admin_guard_registration() -> crate::module::GuardRegistration {
        crate::module::GuardRegistration {
            type_id: std::any::TypeId::of::<AllowGuard>(),
            type_name: "AllowGuard",
            factory: |_c| Ok(Arc::new(AllowGuard) as Arc<dyn Guard>),
        }
    }

    fn controller_registration(base_path: &'static str) -> crate::ControllerRegistration {
        crate::ControllerRegistration {
            type_id: std::any::TypeId::of::<()>(),
            type_name: "TestController",
            base_path,
            factory: |_c| Ok(Box::new(()) as Box<dyn std::any::Any + Send + Sync>),
            route_registrar: |_c, _r, _b| Ok(()),
        }
    }

    /// Module with a guard and a controller at `/admin`.
    struct AdminModule;
    impl Module for AdminModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![controller_registration("/admin")]
        }
        fn guards(&self) -> Vec<crate::module::GuardRegistration> {
            vec![admin_guard_registration()]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    /// Module that declares a guard but registers no controllers.
    struct GuardOnlyModule;
    impl Module for GuardOnlyModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn guards(&self) -> Vec<crate::module::GuardRegistration> {
            vec![admin_guard_registration()]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[test]
    fn test_register_module_scopes_guard_to_controller_base_path() {
        let container = Container::new();
        let mut router = Router::new();
        let mut guards: Vec<ScopedGuard> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        Application::register_module(
            &container,
            &mut router,
            &mut guards,
            &mut visited,
            &AdminModule,
        );

        assert_eq!(guards.len(), 1);
        assert_eq!(guards[0].prefix, "/admin");
        assert!(guards[0].matches("/admin/users"));
        assert!(!guards[0].matches("/public"));
    }

    #[test]
    fn test_register_module_guard_without_controllers_is_inert() {
        let container = Container::new();
        let mut router = Router::new();
        let mut guards: Vec<ScopedGuard> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        Application::register_module(
            &container,
            &mut router,
            &mut guards,
            &mut visited,
            &GuardOnlyModule,
        );

        // No controllers to scope to → guard registers nothing.
        assert!(guards.is_empty());
    }

    // ---- register_module dedups by concrete module type, not the erased
    // trait-object type (regression: T4b) ------------------------------
    //
    // `std::any::type_name_of_val(module: &dyn Module)` always evaluates to
    // the trait object's own type name ("dyn Module"), the same string for
    // every concrete module, because its type parameter is resolved from
    // the *static* type of the reference, not the concrete type behind the
    // vtable. A visited-set keyed on that string treats every module after
    // the first one touched as a duplicate and silently drops it.

    struct DistinctProviderA;
    struct DistinctProviderB;

    async fn distinct_handler_a(
        _req: crate::HttpRequest,
    ) -> Result<crate::HttpResponse, crate::Error> {
        Ok(crate::HttpResponse::ok())
    }

    async fn distinct_handler_b(
        _req: crate::HttpRequest,
    ) -> Result<crate::HttpResponse, crate::Error> {
        Ok(crate::HttpResponse::ok())
    }

    fn distinct_controller_registration_a() -> crate::ControllerRegistration {
        crate::ControllerRegistration {
            type_id: std::any::TypeId::of::<()>(),
            type_name: "DistinctControllerA",
            base_path: "/distinct-a",
            factory: |_c| Ok(Box::new(()) as Box<dyn std::any::Any + Send + Sync>),
            route_registrar: |_c, r, _b| {
                r.get("/distinct-a", distinct_handler_a);
                Ok(())
            },
        }
    }

    fn distinct_controller_registration_b() -> crate::ControllerRegistration {
        crate::ControllerRegistration {
            type_id: std::any::TypeId::of::<()>(),
            type_name: "DistinctControllerB",
            base_path: "/distinct-b",
            factory: |_c| Ok(Box::new(()) as Box<dyn std::any::Any + Send + Sync>),
            route_registrar: |_c, r, _b| {
                r.get("/distinct-b", distinct_handler_b);
                Ok(())
            },
        }
    }

    /// Imported module A: registers `DistinctProviderA` and a controller at
    /// `/distinct-a`.
    struct DistinctModuleA;
    impl Module for DistinctModuleA {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![crate::ProviderRegistration {
                type_id: std::any::TypeId::of::<DistinctProviderA>(),
                type_name: "DistinctProviderA",
                register_fn: |c| c.register(DistinctProviderA),
            }]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![distinct_controller_registration_a()]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    /// Imported module B: a concrete type distinct from `DistinctModuleA`;
    /// registers `DistinctProviderB` and a controller at `/distinct-b`.
    struct DistinctModuleB;
    impl Module for DistinctModuleB {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![crate::ProviderRegistration {
                type_id: std::any::TypeId::of::<DistinctProviderB>(),
                type_name: "DistinctProviderB",
                register_fn: |c| c.register(DistinctProviderB),
            }]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![distinct_controller_registration_b()]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    /// Root module with no providers/controllers of its own; everything
    /// observable comes from its two distinct imports.
    struct DistinctRootModule;
    impl Module for DistinctRootModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(DistinctModuleA), Box::new(DistinctModuleB)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[test]
    fn test_register_module_registers_all_distinct_imported_modules() {
        let container = Container::new();
        let mut router = Router::new();
        let mut guards: Vec<ScopedGuard> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        Application::register_module(
            &container,
            &mut router,
            &mut guards,
            &mut visited,
            &DistinctRootModule,
        );

        assert!(
            container.has::<DistinctProviderA>(),
            "first imported module's provider must be registered"
        );
        assert!(
            container.has::<DistinctProviderB>(),
            "second imported module's provider must be registered (must not \
             be dropped as a false-positive duplicate of the first)"
        );
        assert!(
            router.routes.iter().any(|r| r.path == "/distinct-a"),
            "first imported module's controller route must be registered"
        );
        assert!(
            router.routes.iter().any(|r| r.path == "/distinct-b"),
            "second imported module's controller route must be registered"
        );
    }

    // ---- true diamond import: the same concrete module reached via two
    // different parents must still register exactly once -----------------

    struct SharedDiamondProvider;

    static DIAMOND_PROVIDER_INIT_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    struct SharedDiamondModule;
    impl Module for SharedDiamondModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![crate::ProviderRegistration {
                type_id: std::any::TypeId::of::<SharedDiamondProvider>(),
                type_name: "SharedDiamondProvider",
                register_fn: |c| {
                    DIAMOND_PROVIDER_INIT_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    c.register(SharedDiamondProvider);
                },
            }]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    struct DiamondLeftModule;
    impl Module for DiamondLeftModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(SharedDiamondModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    struct DiamondRightModule;
    impl Module for DiamondRightModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(SharedDiamondModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    struct DiamondRootModule;
    impl Module for DiamondRootModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(DiamondLeftModule), Box::new(DiamondRightModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[test]
    fn test_register_module_diamond_import_registers_shared_module_once() {
        let container = Container::new();
        let mut router = Router::new();
        let mut guards: Vec<ScopedGuard> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        Application::register_module(
            &container,
            &mut router,
            &mut guards,
            &mut visited,
            &DiamondRootModule,
        );

        assert!(
            container.has::<SharedDiamondProvider>(),
            "shared module reachable via a diamond must still register"
        );
        assert_eq!(
            DIAMOND_PROVIDER_INIT_COUNT.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "diamond-imported module (reached via two different parents) \
             must register exactly once, not zero (dropped) or two \
             (duplicated)"
        );
    }

    // ---- Application::create() dedups diamond/cyclic imports end-to-end --
    //
    // `test_register_module_diamond_import_registers_shared_module_once`
    // above exercises `register_module` directly. These exercise the exact
    // same dedup logic through the full public `Application::create()`
    // entrypoint -- the actual bootstrap path real applications use -- and
    // additionally cover a genuinely cyclic import graph (X imports Y
    // imports X), which nothing above tests.

    static CREATE_DIAMOND_PROVIDER_INIT_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    struct CreateDiamondSharedProvider;

    async fn create_diamond_shared_handler(
        _req: crate::HttpRequest,
    ) -> Result<crate::HttpResponse, crate::Error> {
        Ok(crate::HttpResponse::ok())
    }

    fn create_diamond_shared_controller_registration() -> crate::ControllerRegistration {
        crate::ControllerRegistration {
            type_id: std::any::TypeId::of::<()>(),
            type_name: "CreateDiamondSharedController",
            base_path: "/create-diamond-shared",
            factory: |_c| Ok(Box::new(()) as Box<dyn std::any::Any + Send + Sync>),
            route_registrar: |_c, r, _b| {
                r.get("/create-diamond-shared", create_diamond_shared_handler);
                Ok(())
            },
        }
    }

    /// The shared module reached via both `CreateDiamondLeftModule` and
    /// `CreateDiamondRightModule` below (the "diamond").
    #[derive(Default)]
    struct CreateDiamondSharedModule;
    impl Module for CreateDiamondSharedModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![crate::ProviderRegistration {
                type_id: std::any::TypeId::of::<CreateDiamondSharedProvider>(),
                type_name: "CreateDiamondSharedProvider",
                register_fn: |c| {
                    CREATE_DIAMOND_PROVIDER_INIT_COUNT
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    c.register(CreateDiamondSharedProvider);
                },
            }]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![create_diamond_shared_controller_registration()]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[derive(Default)]
    struct CreateDiamondLeftModule;
    impl Module for CreateDiamondLeftModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(CreateDiamondSharedModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[derive(Default)]
    struct CreateDiamondRightModule;
    impl Module for CreateDiamondRightModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(CreateDiamondSharedModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[derive(Default)]
    struct CreateDiamondRootModule;
    impl Module for CreateDiamondRootModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![
                Box::new(CreateDiamondLeftModule),
                Box::new(CreateDiamondRightModule),
            ]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[tokio::test]
    async fn test_application_create_dedups_diamond_imported_module() {
        CREATE_DIAMOND_PROVIDER_INIT_COUNT.store(0, std::sync::atomic::Ordering::SeqCst);

        let app = Application::create::<CreateDiamondRootModule>().await;

        assert!(
            app.container.has::<CreateDiamondSharedProvider>(),
            "shared module reachable via a diamond (through two different \
             parent modules) must still register"
        );
        assert_eq!(
            CREATE_DIAMOND_PROVIDER_INIT_COUNT.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "diamond-imported module's provider must register exactly once \
             through Application::create, not zero (dropped) or two \
             (duplicated)"
        );

        let route_count = app
            .router
            .routes
            .iter()
            .filter(|r| r.path == "/create-diamond-shared")
            .count();
        assert_eq!(
            route_count, 1,
            "diamond-imported module's controller route must register \
             exactly once through Application::create"
        );
    }

    #[derive(Default)]
    struct CyclicImportXModule;
    impl Module for CyclicImportXModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![Box::new(CyclicImportYModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    struct CyclicImportYModule;
    impl Module for CyclicImportYModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            vec![]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            // Cycle: Y imports X, and X (above) imports Y. Each `imports()`
            // call fabricates a *fresh* instance of the other module type on
            // demand -- there's no literal infinitely-sized value here --
            // but `register_module`'s TypeId-keyed `visited` set must still
            // stop the recursion the second time either concrete type is
            // reached, or this would recurse forever and blow the stack.
            vec![Box::new(CyclicImportXModule)]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[tokio::test]
    async fn test_application_create_terminates_on_cyclic_imports() {
        // A generous bound: if the dedup guard in `register_module` ever
        // regresses to unconditional recursion, this fails fast with a
        // clear "timed out" failure instead of hanging the whole test
        // binary. (A true regression could also manifest as a stack
        // overflow, which no timeout can catch -- but a loud process abort
        // is at least as diagnosable as a silent hang.)
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Application::create::<CyclicImportXModule>(),
        )
        .await;

        assert!(
            result.is_ok(),
            "Application::create must terminate for a cyclic module import \
             graph, not hang"
        );
    }

    #[test]
    fn test_with_guard_registers_global_prefix() {
        let app =
            Application::new(Container::new(), Router::new()).with_guard(Arc::new(AllowGuard));
        assert_eq!(app.guards.len(), 1);
        assert!(app.guards[0].prefix.is_empty());
        assert!(app.guards[0].matches("/any/path"));
    }

    // ---- Application::create wires lifecycle hooks (Finding 1) ------------

    static LIFECYCLE_PROBE_INIT_CALLED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static LIFECYCLE_PROBE_BOOTSTRAP_CALLED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    /// Records the order hooks actually ran in, so the test below can assert
    /// the documented `OnModuleInit` -> `OnApplicationBootstrap` ordering
    /// contract, not just that both eventually fired.
    static LIFECYCLE_PROBE_ORDER: std::sync::Mutex<Vec<&'static str>> =
        std::sync::Mutex::new(Vec::new());

    #[derive(Clone, Default)]
    struct LifecycleProbeProvider;

    #[async_trait::async_trait]
    impl crate::lifecycle::OnModuleInit for LifecycleProbeProvider {
        async fn on_module_init(&self) -> crate::lifecycle::LifecycleResult {
            LIFECYCLE_PROBE_INIT_CALLED.store(true, std::sync::atomic::Ordering::SeqCst);
            LIFECYCLE_PROBE_ORDER.lock().unwrap().push("init");
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl crate::lifecycle::OnApplicationBootstrap for LifecycleProbeProvider {
        async fn on_application_bootstrap(&self) -> crate::lifecycle::LifecycleResult {
            LIFECYCLE_PROBE_BOOTSTRAP_CALLED.store(true, std::sync::atomic::Ordering::SeqCst);
            LIFECYCLE_PROBE_ORDER.lock().unwrap().push("bootstrap");
            Ok(())
        }
    }

    #[derive(Default)]
    struct LifecycleProbeModule;
    impl Module for LifecycleProbeModule {
        fn providers(&self) -> Vec<crate::ProviderRegistration> {
            // Exercises the real `provider_registration!` macro path (the
            // same one `armature_proc_macro`'s `#[module(...)]` codegen
            // mirrors), not a hand-rolled `ProviderRegistration`.
            vec![crate::provider_registration!(
                LifecycleProbeProvider,
                LifecycleProbeProvider
            )]
        }
        fn controllers(&self) -> Vec<crate::ControllerRegistration> {
            vec![]
        }
        fn imports(&self) -> Vec<Box<dyn Module>> {
            vec![]
        }
        fn exports(&self) -> Vec<std::any::TypeId> {
            vec![]
        }
    }

    #[tokio::test]
    async fn test_application_create_fires_on_module_init_and_bootstrap_hooks() {
        LIFECYCLE_PROBE_INIT_CALLED.store(false, std::sync::atomic::Ordering::SeqCst);
        LIFECYCLE_PROBE_BOOTSTRAP_CALLED.store(false, std::sync::atomic::Ordering::SeqCst);
        LIFECYCLE_PROBE_ORDER.lock().unwrap().clear();

        let app = Application::create::<LifecycleProbeModule>().await;

        assert!(
            LIFECYCLE_PROBE_INIT_CALLED.load(std::sync::atomic::Ordering::SeqCst),
            "OnModuleInit must fire automatically during Application::create"
        );
        assert!(
            LIFECYCLE_PROBE_BOOTSTRAP_CALLED.load(std::sync::atomic::Ordering::SeqCst),
            "OnApplicationBootstrap must fire automatically during Application::create"
        );
        assert!(app.container.has::<LifecycleProbeProvider>());

        // Documented ordering contract: OnModuleInit must run to completion
        // before OnApplicationBootstrap starts, not just "both eventually
        // fired in some order".
        let order = LIFECYCLE_PROBE_ORDER.lock().unwrap().clone();
        assert_eq!(
            order,
            vec!["init", "bootstrap"],
            "OnModuleInit must run before OnApplicationBootstrap"
        );
    }

    // ---- Application::use_global_filter wiring (Finding 3) ----------------

    struct AlwaysNotFoundGuard;
    #[async_trait::async_trait]
    impl Guard for AlwaysNotFoundGuard {
        async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
            Err(Error::NotFound("boom".to_string()))
        }
    }

    struct RecordingCatchAllFilter {
        called: Arc<std::sync::atomic::AtomicBool>,
    }
    #[async_trait::async_trait]
    impl crate::exception_filter::ExceptionFilter for RecordingCatchAllFilter {
        async fn catch(
            &self,
            error: &Error,
            _ctx: &crate::exception_filter::ExceptionContext,
        ) -> Option<HttpResponse> {
            if let Error::NotFound(_) = error {
                self.called.store(true, std::sync::atomic::Ordering::SeqCst);
                Some(
                    HttpResponse::new(599)
                        .with_json(&serde_json::json!({"caught_by": "RecordingCatchAllFilter"}))
                        .unwrap(),
                )
            } else {
                None
            }
        }
    }

    #[test]
    fn test_use_global_filter_populates_serve_state() {
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let app = Application::new(Container::new(), Router::new()).use_global_filter(
            RecordingCatchAllFilter {
                called: called.clone(),
            },
        );

        assert!(app.filter_chain.is_some());
        let state = app.serve_state(None);
        assert!(
            state.filter_chain.is_some(),
            "serve_state must carry the configured filter chain through to ServeState"
        );
    }

    #[test]
    fn test_no_filter_configured_leaves_serve_state_filter_chain_none() {
        let app = Application::new(Container::new(), Router::new());
        let state = app.serve_state(None);
        assert!(
            state.filter_chain.is_none(),
            "without use_global_filter, ServeState must carry no filter chain, \
             preserving the original error_response fallback behavior"
        );
    }

    /// Live end-to-end test: binds a real TCP listener, serves exactly one
    /// connection through the real `handle_request` function (the same one
    /// `Application::listen`/`listen_on` use), sends a raw HTTP request that
    /// triggers a guard error, and asserts the response actually returned
    /// over the wire is the one produced by the registered global filter --
    /// not `error_response`'s default `to_client_response()` output.
    #[tokio::test]
    async fn test_use_global_filter_transforms_error_in_live_handle_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let filter_chain = Arc::new(
            crate::exception_filter::ExceptionFilterChain::new().add_filter(
                RecordingCatchAllFilter {
                    called: called.clone(),
                },
            ),
        );

        let state = ServeState {
            router: Arc::new(OptimizedRouter::from_router(&Router::new())),
            cors: None,
            guards: vec![ScopedGuard {
                prefix: String::new(),
                guard: Arc::new(AlwaysNotFoundGuard),
            }]
            .into(),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            filter_chain: Some(filter_chain),
        };

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |req: Request<IncomingBody>| {
                let state = state.clone();
                async move { handle_request(req, state).await }
            });
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /anything HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        // Bounded the same way as micro.rs's `send_raw_request` test helper:
        // relies on `Connection: close` above to unblock `read_to_end` once
        // the server replies, with a safety timeout in case that path ever
        // regresses and the connection is left open.
        let mut raw_response = Vec::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut raw_response),
        )
        .await;
        let raw_response = String::from_utf8_lossy(&raw_response);

        assert!(
            raw_response.starts_with("HTTP/1.1 599"),
            "expected the filter's custom 599 status, got: {raw_response}"
        );
        assert!(
            raw_response.contains("RecordingCatchAllFilter"),
            "expected the filter's custom body, got: {raw_response}"
        );
        assert!(
            called.load(std::sync::atomic::Ordering::SeqCst),
            "the registered filter's catch() must actually have run"
        );
    }

    /// Handler that unconditionally returns an error, used to exercise the
    /// routing/handler-error branch of `respond_to_error` (as opposed to the
    /// guard-rejection branch `AlwaysNotFoundGuard` exercises above) end to
    /// end through a real socket.
    async fn always_erroring_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Err(Error::NotFound("handler boom".to_string()))
    }

    /// Live end-to-end test, sibling of
    /// `test_use_global_filter_transforms_error_in_live_handle_request`
    /// above: no guard is involved at all here. A real route is registered
    /// whose handler itself returns `Err(...)`, so this exercises the
    /// *routing/handler-error* branch of `respond_to_error` (the guard test
    /// above only ever exercises the guard-rejection branch, since its guard
    /// rejects every request before routing is ever reached).
    #[tokio::test]
    async fn test_use_global_filter_transforms_handler_error_in_live_handle_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let filter_chain = Arc::new(
            crate::exception_filter::ExceptionFilterChain::new().add_filter(
                RecordingCatchAllFilter {
                    called: called.clone(),
                },
            ),
        );

        let mut router = Router::new();
        router.get("/broken", always_erroring_handler);

        let state = ServeState {
            router: Arc::new(OptimizedRouter::from_router(&router)),
            cors: None,
            // No guards at all: this response must come from the router's
            // handler-error path, not guard rejection.
            guards: Vec::new().into(),
            max_body_size: DEFAULT_MAX_BODY_SIZE,
            filter_chain: Some(filter_chain),
        };

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = service_fn(move |req: Request<IncomingBody>| {
                let state = state.clone();
                async move { handle_request(req, state).await }
            });
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /broken HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();

        let mut raw_response = Vec::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut raw_response),
        )
        .await;
        let raw_response = String::from_utf8_lossy(&raw_response);

        assert!(
            raw_response.starts_with("HTTP/1.1 599"),
            "expected the filter's custom 599 status, got: {raw_response}"
        );
        assert!(
            raw_response.contains("RecordingCatchAllFilter"),
            "expected the filter's custom body, got: {raw_response}"
        );
        assert!(
            called.load(std::sync::atomic::Ordering::SeqCst),
            "the registered filter's catch() must actually have run for a \
             real handler error, not just a guard rejection"
        );
    }

    // ---- respond_to_error isolates panicking/hanging filters (Finding 2) --

    struct PanickingFilter;
    #[async_trait::async_trait]
    impl crate::exception_filter::ExceptionFilter for PanickingFilter {
        async fn catch(
            &self,
            _error: &Error,
            _ctx: &crate::exception_filter::ExceptionContext,
        ) -> Option<HttpResponse> {
            panic!("PanickingFilter deliberately panics for test coverage");
        }
    }

    struct HangingFilter;
    #[async_trait::async_trait]
    impl crate::exception_filter::ExceptionFilter for HangingFilter {
        async fn catch(
            &self,
            _error: &Error,
            _ctx: &crate::exception_filter::ExceptionContext,
        ) -> Option<HttpResponse> {
            // Deliberately sleeps far longer than the timeout used in the
            // test below, so it never actually completes -- exercising the
            // "hanging filter" isolation path.
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            None
        }
    }

    #[tokio::test]
    async fn test_respond_to_error_falls_back_when_filter_panics() {
        let chain = Arc::new(
            crate::exception_filter::ExceptionFilterChain::new().add_filter(PanickingFilter),
        );
        let req = HttpRequest::new("GET", "/panics".to_string());
        let err = Error::Internal("boom".to_string());

        // Must fall back to exactly what `error_response(&err)` (i.e. no
        // filter at all) would have produced: a panicking filter is treated
        // as though it declined to handle the error, not as a crashed
        // request/connection.
        let response = respond_to_error_with_timeout(
            err,
            Some(req),
            Some(chain),
            std::time::Duration::from_secs(5),
        )
        .await;

        assert_eq!(response.status, 500);
        let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap();
        assert!(
            body.contains("Internal Server Error"),
            "a panicking filter must fall back to the redacted default 5xx \
             body, got: {body}"
        );
    }

    #[tokio::test]
    async fn test_respond_to_error_falls_back_when_filter_hangs() {
        let chain = Arc::new(
            crate::exception_filter::ExceptionFilterChain::new().add_filter(HangingFilter),
        );
        let req = HttpRequest::new("GET", "/hangs".to_string());
        let err = Error::Internal("boom".to_string());

        // A short timeout (rather than the 5s production default) keeps this
        // test fast; what's under test is the fallback behavior on timeout,
        // not the exact default duration (that's `DEFAULT_EXCEPTION_FILTER_TIMEOUT`,
        // exercised indirectly via `respond_to_error`).
        let start = std::time::Instant::now();
        let response = respond_to_error_with_timeout(
            err,
            Some(req),
            Some(chain),
            std::time::Duration::from_millis(50),
        )
        .await;
        let elapsed = start.elapsed();

        assert_eq!(response.status, 500);
        let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap();
        assert!(
            body.contains("Internal Server Error"),
            "a hanging filter must fall back to the redacted default 5xx \
             body, got: {body}"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "a hanging filter must not block the caller past the configured \
             timeout, took {elapsed:?}"
        );
    }
}
