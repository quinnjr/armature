// Application bootstrapper and HTTP server

use crate::http2::{Http2Builder, Http2Config, Http2Stats};
use crate::http3::{Http3Config, Http3Stats};
use crate::logging::{debug, error, info, trace, warn};
use crate::pipeline::{PipelineConfig, PipelineStats, PipelinedHttp1Builder};
use crate::{
    Container, Error, HttpRequest, HttpResponse, HttpsConfig, LifecycleManager, Module, Router,
    TlsConfig,
};
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, body::Incoming as IncomingBody};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;
use std::sync::Arc;
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
        }
    }

    /// Configure CORS for the application. Handles preflight OPTIONS
    /// requests automatically and adds CORS headers to every response.
    pub fn with_cors(mut self, config: CorsConfig) -> Self {
        self.cors_config = Some(Arc::new(config));
        self
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

        // Initialize the root module
        let root_module = M::default();
        debug!("Root module instantiated");

        info!("Registering modules and dependencies");

        // Register all providers and controllers from the module tree
        Self::register_module(&container, &mut router, &root_module);

        info!("Executing lifecycle hooks");

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

    /// Register a module and its imports recursively
    fn register_module(container: &Container, router: &mut Router, module: &dyn Module) {
        let module_type = std::any::type_name_of_val(module);
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
                Self::register_module(container, router, imported_module.as_ref());
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
                Self::register_module(container, router, re_exported_module.as_ref());
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

        // Register all guards
        let guards = module.guards();
        if !guards.is_empty() {
            debug!(
                module_type = module_type,
                guard_count = guards.len(),
                "Registering guards"
            );
            for guard_reg in guards {
                match (guard_reg.factory)(container) {
                    Ok(_guard) => {
                        debug!(
                            module_type = module_type,
                            guard = guard_reg.type_name,
                            "Guard registered"
                        );
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
    pub async fn listen(self, port: u16) -> Result<(), Error> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));

        debug!(address = %addr, "Binding to address");
        let listener = TcpListener::bind(addr).await?;

        info!(
            address = %addr,
            pipeline_mode = ?self.pipeline_config.mode,
            pipeline_flush = self.pipeline_config.pipeline_flush,
            max_concurrent = self.pipeline_config.max_concurrent,
            "HTTP server listening with pipelining enabled"
        );

        let router = self.router.clone();
        let self_cors = self.cors_config.clone();
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

            let io = TokioIo::new(stream);
            let router = router.clone();
            let http_builder = pipeline_builder.configure_hyper_builder();
            let stats = Arc::clone(&pipeline_stats);

            // Track connection
            stats.connection_opened();

            let cors_for_spawn = self_cors.clone();
            tokio::spawn(async move {
                let stats_for_close = Arc::clone(&stats);
                let cors = cors_for_spawn;
                let service = service_fn(move |req: Request<IncomingBody>| {
                    let router = router.clone();
                    let stats = Arc::clone(&stats);
                    let cors = cors.clone();
                    async move {
                        stats.request_processed();
                        handle_request(req, router, cors).await
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

        info!(
            address = %addr,
            pipeline_mode = ?self.pipeline_config.mode,
            pipeline_flush = self.pipeline_config.pipeline_flush,
            "HTTPS server listening with pipelining enabled"
        );

        let acceptor = TlsAcceptor::from(tls_config.server_config);
        let router = self.router.clone();
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

            let acceptor = acceptor.clone();
            let router = router.clone();
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
                            let router = router.clone();
                            let stats = Arc::clone(&stats);
                            async move {
                                stats.request_processed();
                                handle_request(req, router, None).await
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
        let router = self.router.clone();

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

        println!("🔒 HTTPS Server listening on https://{}", https_addr);
        if config.http_redirect_addr.is_some() {
            println!("↪️  HTTP redirect server enabled");
        }

        let acceptor = TlsAcceptor::from(config.tls.server_config);

        loop {
            let (stream, _) = listener.accept().await?;
            let acceptor = acceptor.clone();
            let router = router.clone();

            tokio::spawn(async move {
                match acceptor.accept(stream).await {
                    Ok(tls_stream) => {
                        let io = TokioIo::new(tls_stream);

                        let service = service_fn(move |req: Request<IncomingBody>| {
                            let router = router.clone();
                            async move { handle_request(req, router, None).await }
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

        info!(
            address = %addr,
            max_concurrent_streams = self.http2_config.max_concurrent_streams,
            "HTTP/2 cleartext server listening (h2c)"
        );
        warn!("HTTP/2 cleartext (h2c) is not recommended for production. Use HTTPS.");

        let router = self.router.clone();
        let h2_builder =
            Http2Builder::with_stats(self.http2_config.clone(), Arc::clone(&self.http2_stats));
        let h2_stats = Arc::clone(&self.http2_stats);

        loop {
            let (stream, client_addr) = listener.accept().await?;
            trace!(client_address = %client_addr, "HTTP/2 connection accepted");

            let io = TokioIo::new(stream);
            let router = router.clone();
            let http_builder = h2_builder.configure_hyper_builder();
            let stats = Arc::clone(&h2_stats);

            // Track connection
            stats.connection_opened();

            tokio::spawn(async move {
                let stats_for_close = Arc::clone(&stats);
                let service = service_fn(move |req: Request<IncomingBody>| {
                    let router = router.clone();
                    let stats = Arc::clone(&stats);
                    async move {
                        stats.request_processed();
                        handle_request(req, router, None).await
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

        info!(
            address = %addr,
            max_concurrent_streams = self.http2_config.max_concurrent_streams,
            pipeline_mode = ?self.pipeline_config.mode,
            "HTTPS server listening with HTTP/2 and HTTP/1.1 (ALPN)"
        );

        let acceptor = TlsAcceptor::from(tls_config.server_config);
        let router = self.router.clone();
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

            let acceptor = acceptor.clone();
            let router = router.clone();
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
                                let router = router.clone();
                                let stats = Arc::clone(&stats);
                                async move {
                                    stats.request_processed();
                                    handle_request(req, router, None).await
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
                                let router = router.clone();
                                let stats = Arc::clone(&stats);
                                async move {
                                    stats.request_processed();
                                    handle_request(req, router, None).await
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

        let server = Http3Server::new(self.http3_config.clone(), self.router.clone());

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
        let router_h3 = Arc::clone(&self.router);
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
    router: Arc<Router>,
    cors: Option<Arc<CorsConfig>>,
) -> Result<Response<Full<bytes::Bytes>>, hyper::Error> {
    use std::time::Instant;

    let start = Instant::now();

    // Convert hyper request to our HttpRequest
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    trace!(method = %method, path = %path, "Incoming request");

    if method == "OPTIONS"
        && let Some(ref cors) = cors
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

    // Copy headers
    let header_count = req.headers().len();
    for (name, value) in req.headers() {
        if let Ok(value_str) = value.to_str() {
            armature_req
                .headers
                .insert(name.to_string(), value_str.to_string());
        }
    }
    trace!(header_count = header_count, "Headers parsed");

    // Read body into Bytes (zero-copy after this point)
    let body_bytes = req.collect().await?.to_bytes();
    let body_size = body_bytes.len();

    // Use zero-copy body storage
    if body_size > 0 {
        armature_req.set_body_bytes(body_bytes);
        trace!(body_size = body_size, "Request body received (zero-copy)");
    }

    // Route the request
    debug!(method = %method, path = %path, "Routing request");
    let response = match router.route(armature_req).await {
        Ok(resp) => {
            debug!(method = %method, path = %path, status = resp.status, "Request handled successfully");
            resp
        }
        Err(err) => {
            warn!(method = %method, path = %path, error = %err, "Request handling failed");
            // Convert error to response
            let status = err.status_code();
            let body = serde_json::json!({
                "error": err.to_string(),
                "status": status,
            });
            HttpResponse::new(status)
                .with_json(&body)
                .unwrap_or_else(|_| HttpResponse::internal_server_error())
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

    // Convert our HttpResponse to hyper Response
    let mut builder = Response::builder().status(response.status);

    for (key, value) in &response.headers {
        builder = builder.header(key, value);
    }
    for cookie_value in &response.cookies {
        builder = builder.header("Set-Cookie", cookie_value);
    }
    if let Some(ref cors) = cors {
        builder = builder.header("Access-Control-Allow-Origin", &cors.allow_origin);
        if cors.allow_credentials {
            builder = builder.header("Access-Control-Allow-Credentials", "true");
        }
    }

    // Zero-copy body passthrough to Hyper
    let body = Full::new(response.into_body_bytes());
    Ok(builder.body(body).unwrap())
}
