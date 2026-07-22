//! Micro-framework API for Armature
//!
//! Provides a lightweight, Actix-style API for building web applications
//! without the full module/controller system.
//!
//! ## Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Micro-Framework Mode                          │
//! │                                                                  │
//! │  App::new()                                                     │
//! │    .data(State::new())          // Shared state                 │
//! │    .wrap(Logger::default())     // Middleware                   │
//! │    .route("/", get(index))      // Simple routes                │
//! │    .service(                    // Resource groups              │
//! │        scope("/api")                                            │
//! │            .route("/users", get(list_users))                    │
//! │            .route("/users/:id", get(get_user))                  │
//! │    )                                                            │
//! │    .run("0.0.0.0:8080")                                         │
//! │    .await?;                                                     │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_core::micro::*;
//!
//! async fn index() -> &'static str {
//!     "Hello, World!"
//! }
//!
//! async fn greet(path: Path<String>) -> String {
//!     format!("Hello, {}!", path.into_inner())
//! }
//!
//! #[tokio::main]
//! async fn main() -> std::io::Result<()> {
//!     App::new()
//!         .route("/", get(index))
//!         .route("/greet/:name", get(greet))
//!         .run("127.0.0.1:8080")
//!         .await
//! }
//! ```

use crate::handler::{BoxedHandler, IntoHandler};
use crate::route_cache::OptimizedRouter;
use crate::{Error, HttpMethod, HttpRequest, HttpResponse, Router};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::net::ToSocketAddrs;
use std::pin::Pin;
use std::sync::Arc;

// Re-export common types for convenience
pub use crate::{HttpRequest as Request, HttpResponse as Response};

/// Application state container
///
/// Wrap your application state in `Data<T>` to share it across handlers.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::micro::*;
///
/// struct AppState {
///     db: Pool,
///     config: Config,
/// }
///
/// async fn handler(state: Data<AppState>) -> &'static str {
///     // Access state.db, state.config, etc.
///     "OK"
/// }
///
/// App::new()
///     .data(AppState { db, config })
///     .route("/", get(handler))
/// ```
#[derive(Clone)]
pub struct Data<T: Clone + Send + Sync + 'static>(Arc<T>);

impl<T: Clone + Send + Sync + 'static> Data<T> {
    /// Create a new Data wrapper
    pub fn new(data: T) -> Self {
        Self(Arc::new(data))
    }

    /// Get a reference to the inner data
    pub fn get_ref(&self) -> &T {
        &self.0
    }

    /// Get the inner Arc
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T: Clone + Send + Sync + 'static> std::ops::Deref for Data<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Micro-framework application builder
///
/// Provides a fluent API for building web applications without
/// the full module/controller infrastructure.
pub struct App {
    router: Router,
    middleware: Vec<Arc<dyn Middleware>>,
    state: AppState,
    default_service: Option<BoxedHandler>,
}

/// Internal application state storage
#[derive(Default, Clone)]
struct AppState {
    data: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl AppState {
    fn insert<T: Clone + Send + Sync + 'static>(&mut self, data: T) {
        self.data.insert(TypeId::of::<T>(), Arc::new(data));
    }

    /// Get data by type
    #[allow(dead_code)]
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<Data<T>> {
        self.data
            .get(&TypeId::of::<T>())
            .and_then(|arc| arc.downcast_ref::<T>())
            .map(|t| Data(Arc::new(t.clone())))
    }
}

impl App {
    /// Create a new micro-framework application
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            middleware: Vec::new(),
            state: AppState::default(),
            default_service: None,
        }
    }

    /// Add shared application data
    ///
    /// Data can be accessed in handlers via `Data<T>` extractor.
    pub fn data<T: Clone + Send + Sync + 'static>(mut self, data: T) -> Self {
        self.state.insert(data);
        self
    }

    /// Add a middleware layer
    ///
    /// Middleware is executed in the order it is added (first added = outermost).
    pub fn wrap<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Add a route with a handler
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// App::new()
    ///     .route("/", get(index))
    ///     .route("/users", post(create_user))
    ///     .route("/users/:id", get(get_user).put(update_user).delete(delete_user))
    /// ```
    pub fn route(mut self, path: &str, route: RouteBuilder) -> Self {
        for (method, handler) in route.handlers {
            self.router.add_route(crate::routing::Route {
                method,
                path: path.to_string(),
                handler,
                constraints: None,
            });
        }
        self
    }

    /// Add a scoped resource group
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// App::new()
    ///     .service(
    ///         scope("/api/v1")
    ///             .route("/users", get(list_users))
    ///             .route("/users/:id", get(get_user))
    ///     )
    /// ```
    pub fn service(mut self, scope: Scope) -> Self {
        for route in scope.into_routes() {
            self.router.add_route(crate::routing::Route {
                method: route.method,
                path: route.path,
                handler: route.handler,
                constraints: route.constraints,
            });
        }
        self
    }

    /// Set a default handler for unmatched routes
    pub fn default_service<H, Args>(mut self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.default_service = Some(BoxedHandler::new(handler.into_handler()));
        self
    }

    /// Build and run the application
    ///
    /// This starts the HTTP server and blocks until shutdown.
    pub async fn run(self, addr: impl ToSocketAddrs) -> std::io::Result<()> {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid address")
        })?;

        let app = Arc::new(BuiltApp {
            router: Arc::new(OptimizedRouter::from_router(&self.router)),
            middleware: self.middleware,
            state: self.state,
            default_service: self.default_service,
        });

        run_server(app, addr).await
    }

    /// Build the application into an immutable form
    pub fn build(self) -> BuiltApp {
        BuiltApp {
            router: Arc::new(OptimizedRouter::from_router(&self.router)),
            middleware: self.middleware,
            state: self.state,
            default_service: self.default_service,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Built application ready to handle requests
///
/// Routing dispatches through the O(1) [`OptimizedRouter`], compiled once from
/// the builder's linear [`Router`] in [`App::build`]/[`App::run`].
pub struct BuiltApp {
    router: Arc<OptimizedRouter>,
    middleware: Vec<Arc<dyn Middleware>>,
    state: AppState,
    default_service: Option<BoxedHandler>,
}

impl BuiltApp {
    /// Handle an incoming request
    pub async fn handle(&self, mut request: HttpRequest) -> Result<HttpResponse, Error> {
        // Store state in request extensions for extractors
        request.extensions.insert(self.state.clone());

        // Build middleware chain - start with the innermost handler
        let router = self.router.clone();
        let default_service = self.default_service.clone();

        let handler: Next = Box::new(move |req| {
            let router = router.clone();
            let default_service = default_service.clone();
            Box::pin(async move {
                match router.route(req).await {
                    Ok(response) => Ok(response),
                    Err(Error::RouteNotFound(_)) if default_service.is_some() => {
                        let req = HttpRequest::new("GET".to_string(), "/404".to_string());
                        default_service.unwrap().call(req).await
                    }
                    Err(e) => Err(e),
                }
            })
        });

        // Apply middleware in reverse order (outermost first wraps innermost)
        let mut next = handler;
        for mw in self.middleware.iter().rev() {
            let mw = mw.clone();
            next = Box::new(move |req| mw.call(req, next));
        }

        next(request).await
    }
}

/// Route builder for specifying method handlers
pub struct RouteBuilder {
    handlers: Vec<(HttpMethod, BoxedHandler)>,
}

impl RouteBuilder {
    fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    fn with_method<H, Args>(mut self, method: HttpMethod, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.handlers
            .push((method, BoxedHandler::new(handler.into_handler())));
        self
    }

    /// Add a GET handler
    pub fn get<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::GET, handler)
    }

    /// Add a POST handler
    pub fn post<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::POST, handler)
    }

    /// Add a PUT handler
    pub fn put<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::PUT, handler)
    }

    /// Add a DELETE handler
    pub fn delete<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::DELETE, handler)
    }

    /// Add a PATCH handler
    pub fn patch<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::PATCH, handler)
    }

    /// Add a HEAD handler
    pub fn head<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::HEAD, handler)
    }

    /// Add an OPTIONS handler
    pub fn options<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::OPTIONS, handler)
    }

    /// Add a QUERY handler (safe query with a request body,
    /// draft-ietf-httpbis-safe-method-w-body)
    pub fn query<H, Args>(self, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        self.with_method(HttpMethod::QUERY, handler)
    }
}

/// Create a GET route
pub fn get<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().get(handler)
}

/// Create a POST route
pub fn post<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().post(handler)
}

/// Create a PUT route
pub fn put<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().put(handler)
}

/// Create a DELETE route
pub fn delete<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().delete(handler)
}

/// Create a PATCH route
pub fn patch<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().patch(handler)
}

/// Create a HEAD route
pub fn head<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().head(handler)
}

/// Create an OPTIONS route
pub fn options<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().options(handler)
}

/// Create a QUERY route (safe query with a request body,
/// draft-ietf-httpbis-safe-method-w-body)
pub fn query<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args>,
{
    RouteBuilder::new().query(handler)
}

/// Create a route that matches any HTTP method
pub fn any<H, Args>(handler: H) -> RouteBuilder
where
    H: IntoHandler<Args> + Clone,
{
    RouteBuilder::new()
        .get(handler.clone())
        .post(handler.clone())
        .put(handler.clone())
        .delete(handler.clone())
        .patch(handler.clone())
        .head(handler.clone())
        .options(handler)
}

/// Resource scope for grouping routes
pub struct Scope {
    prefix: String,
    routes: Vec<ScopeRoute>,
    middleware: Vec<Arc<dyn Middleware>>,
}

struct ScopeRoute {
    method: HttpMethod,
    path: String,
    handler: BoxedHandler,
    constraints: Option<crate::route_constraint::RouteConstraints>,
}

impl Scope {
    fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            routes: Vec::new(),
            middleware: Vec::new(),
        }
    }

    /// Add a route to this scope
    pub fn route(mut self, path: &str, route: RouteBuilder) -> Self {
        for (method, handler) in route.handlers {
            self.routes.push(ScopeRoute {
                method,
                path: path.to_string(),
                handler,
                constraints: None,
            });
        }
        self
    }

    /// Add middleware to this scope
    pub fn wrap<M: Middleware + 'static>(mut self, middleware: M) -> Self {
        self.middleware.push(Arc::new(middleware));
        self
    }

    /// Nest another scope
    ///
    /// The inner scope's middleware is applied to its routes immediately;
    /// this scope's middleware wraps them (and all other routes) when the
    /// scope itself is registered, so nested scopes inherit parent middleware.
    pub fn service(mut self, inner: Scope) -> Self {
        self.routes.extend(inner.into_routes());
        self
    }

    /// Consume the scope, prefixing route paths and applying the scope's
    /// middleware to each route handler (first added = outermost).
    fn into_routes(self) -> Vec<ScopeRoute> {
        let Self {
            prefix,
            routes,
            middleware,
        } = self;

        routes
            .into_iter()
            .map(|route| ScopeRoute {
                method: route.method,
                path: format!("{}{}", prefix, route.path),
                handler: wrap_handler(route.handler, &middleware),
                constraints: route.constraints,
            })
            .collect()
    }
}

/// Wrap a handler with a middleware stack (first added = outermost)
fn wrap_handler(handler: BoxedHandler, middleware: &[Arc<dyn Middleware>]) -> BoxedHandler {
    let mut handler = handler;
    for mw in middleware.iter().rev() {
        let mw = Arc::clone(mw);
        let inner = handler;
        handler = BoxedHandler::new(
            (move |req: HttpRequest| {
                let mw = Arc::clone(&mw);
                let inner = inner.clone();
                async move {
                    let next: Next = Box::new(move |req| inner.call(req));
                    mw.call(req, next).await
                }
            })
            .into_handler(),
        );
    }
    handler
}

/// Create a new scope with the given prefix
pub fn scope(prefix: impl Into<String>) -> Scope {
    Scope::new(prefix)
}

/// Type alias for the next handler in middleware chain
pub type Next = Box<
    dyn FnOnce(HttpRequest) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        + Send,
>;

/// Middleware trait for the micro-framework
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::micro::*;
///
/// struct Logger;
///
/// impl Middleware for Logger {
///     fn call(
///         &self,
///         req: Request,
///         next: Next,
///     ) -> Pin<Box<dyn Future<Output = Result<Response, Error>> + Send>> {
///         Box::pin(async move {
///             println!("Request: {} {}", req.method, req.path);
///             let start = std::time::Instant::now();
///             let response = next(req).await;
///             println!("Response in {:?}", start.elapsed());
///             response
///         })
///     }
/// }
/// ```
pub trait Middleware: Send + Sync {
    /// Process a request and optionally call the next handler
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;
}

/// Simple logging middleware
pub struct Logger {
    #[allow(dead_code)]
    format: LogFormat,
}

/// Log format options
#[derive(Clone, Copy, Default)]
pub enum LogFormat {
    #[default]
    Default,
    Combined,
    Short,
}

impl Default for Logger {
    fn default() -> Self {
        Self {
            format: LogFormat::Default,
        }
    }
}

impl Logger {
    /// Create a new logger with the specified format
    pub fn new(format: LogFormat) -> Self {
        Self { format }
    }
}

impl Middleware for Logger {
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        let method = req.method.clone();
        let path = req.path.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let result = next(req).await;
            let elapsed = start.elapsed();

            match &result {
                Ok(response) => {
                    tracing::info!(
                        method = %method,
                        path = %path,
                        status = response.status,
                        duration_ms = elapsed.as_millis() as u64,
                        "Request completed"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        method = %method,
                        path = %path,
                        error = %e,
                        duration_ms = elapsed.as_millis() as u64,
                        "Request failed"
                    );
                }
            }

            result
        })
    }
}

/// CORS middleware
pub struct Cors {
    allowed_origins: Vec<String>,
    allowed_methods: Vec<String>,
    allowed_headers: Vec<String>,
    allow_credentials: bool,
    max_age: u32,
}

impl Default for Cors {
    fn default() -> Self {
        Self {
            allowed_origins: vec!["*".to_string()],
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            allowed_headers: vec!["*".to_string()],
            allow_credentials: false,
            max_age: 86400,
        }
    }
}

impl Cors {
    /// Create a permissive CORS configuration
    pub fn permissive() -> Self {
        Self::default()
    }

    /// Set allowed origins
    pub fn allowed_origins(mut self, origins: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowed_origins = origins.into_iter().map(Into::into).collect();
        self
    }

    /// Set allowed methods
    pub fn allowed_methods(mut self, methods: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowed_methods = methods.into_iter().map(Into::into).collect();
        self
    }

    /// Set allowed headers
    pub fn allowed_headers(mut self, headers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.allowed_headers = headers.into_iter().map(Into::into).collect();
        self
    }

    /// Allow credentials
    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.allow_credentials = allow;
        self
    }

    /// Set max age for preflight cache
    pub fn max_age(mut self, seconds: u32) -> Self {
        self.max_age = seconds;
        self
    }
}

impl Middleware for Cors {
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        let is_preflight = req.method == "OPTIONS";
        let allowed_origins = self.allowed_origins.clone();
        let allowed_methods = self.allowed_methods.join(", ");
        let allowed_headers = self.allowed_headers.join(", ");
        let allow_credentials = self.allow_credentials;
        let max_age = self.max_age;

        Box::pin(async move {
            if is_preflight {
                let mut response = HttpResponse::no_content();
                response.headers.insert(
                    "Access-Control-Allow-Origin".to_string(),
                    allowed_origins.first().cloned().unwrap_or_default(),
                );
                response
                    .headers
                    .insert("Access-Control-Allow-Methods".to_string(), allowed_methods);
                response
                    .headers
                    .insert("Access-Control-Allow-Headers".to_string(), allowed_headers);
                response
                    .headers
                    .insert("Access-Control-Max-Age".to_string(), max_age.to_string());
                if allow_credentials {
                    response.headers.insert(
                        "Access-Control-Allow-Credentials".to_string(),
                        "true".to_string(),
                    );
                }
                return Ok(response);
            }

            let mut response = next(req).await?;
            response.headers.insert(
                "Access-Control-Allow-Origin".to_string(),
                allowed_origins.first().cloned().unwrap_or_default(),
            );
            if allow_credentials {
                response.headers.insert(
                    "Access-Control-Allow-Credentials".to_string(),
                    "true".to_string(),
                );
            }

            Ok(response)
        })
    }
}

/// Gzip-compression middleware.
///
/// When the client's `Accept-Encoding` permits gzip, the response body is
/// compressed with the configured [`CompressionLevel`], `Content-Encoding:
/// gzip` is set, and `Accept-Encoding` is merged into `Vary` so downstream
/// caches key on the negotiated encoding — merged rather than overwritten, so
/// a pre-existing `Vary` (e.g. `Vary: Origin` from CORS middleware upstream)
/// survives. Responses that already carry a `Content-Encoding` or have an
/// empty body are passed through untouched (but still gain `Vary`).
pub struct Compress {
    level: CompressionLevel,
}

/// Compression level
#[derive(Clone, Copy, Default)]
pub enum CompressionLevel {
    Fast,
    #[default]
    Default,
    Best,
}

impl CompressionLevel {
    /// Map to the corresponding `flate2` compression level.
    fn to_flate2(self) -> flate2::Compression {
        match self {
            CompressionLevel::Fast => flate2::Compression::fast(),
            CompressionLevel::Default => flate2::Compression::default(),
            CompressionLevel::Best => flate2::Compression::best(),
        }
    }
}

/// Whether a request's `Accept-Encoding` header permits gzip.
///
/// Recognises an explicit `gzip` token (or the `*` wildcard) and honours an
/// explicit `q=0`, which disables the encoding.
pub(crate) fn accepts_gzip(req: &HttpRequest) -> bool {
    let Some(value) = req.headers.get("accept-encoding") else {
        return false;
    };
    // Compare tokens case-insensitively in place instead of allocating a
    // lowercased copy of the whole header value on every request.
    value.split(',').any(|part| {
        let mut segs = part.split(';').map(str::trim);
        let coding = segs.next().unwrap_or("");
        if !coding.eq_ignore_ascii_case("gzip") && coding != "*" {
            return false;
        }
        // Reject `q=0`(.0…); any other q (or none) permits the encoding. The
        // `q` parameter name is matched case-insensitively (e.g. `Q=0`).
        !segs.any(|p| {
            p.split_once('=')
                .filter(|(k, _)| k.eq_ignore_ascii_case("q"))
                .and_then(|(_, v)| v.parse::<f32>().ok())
                .map(|q| q == 0.0)
                .unwrap_or(false)
        })
    })
}

/// Gzip-encode `data` at the given level, returning `None` if encoding fails.
pub(crate) fn gzip_encode(data: &[u8], level: CompressionLevel) -> Option<Vec<u8>> {
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::with_capacity(data.len() / 2 + 32), level.to_flate2());
    encoder.write_all(data).ok()?;
    encoder.finish().ok()
}

/// Bodies at least this large are gzip-encoded on a blocking thread (via
/// `spawn_blocking`) instead of inline on the async worker. gzip is CPU-bound,
/// so a multi-megabyte body at `Best` can stall a tokio worker for milliseconds;
/// small bodies compress fast enough that the spawn round-trip would cost more
/// than it saves, so they stay inline.
pub(crate) const GZIP_OFFLOAD_THRESHOLD: usize = 32 * 1024;

/// Gzip-encode `data`, offloading to a blocking thread once the body reaches
/// [`GZIP_OFFLOAD_THRESHOLD`] so the CPU-bound encode does not block the async
/// worker. Behaviour-preserving: the bytes returned are identical to
/// [`gzip_encode`]; only *where* the work runs changes. Below the threshold it
/// encodes inline to avoid the spawn overhead. Returns `None` if encoding fails
/// or the blocking task is cancelled/panics (caller then passes the body
/// through uncompressed).
pub(crate) async fn gzip_encode_offloaded(data: &[u8], level: CompressionLevel) -> Option<Vec<u8>> {
    if data.len() < GZIP_OFFLOAD_THRESHOLD {
        return gzip_encode(data, level);
    }
    // Own the bytes so the blocking task is `'static`; no lock is held here, so
    // this never pins a guard across the `.await`.
    let owned = data.to_vec();
    tokio::task::spawn_blocking(move || gzip_encode(&owned, level))
        .await
        .ok()
        .flatten()
}

/// Add `Accept-Encoding` to the response's `Vary` header, *merging* with any
/// existing value instead of clobbering it — e.g. `Vary: Origin` set upstream
/// by CORS middleware must survive compression adding its own dimension.
///
/// Leaves an existing `*` wildcard alone (it already covers every header) and
/// is a no-op if `Accept-Encoding` is already present as a token
/// (case-insensitive), so repeated calls don't pile up duplicates.
pub(crate) fn add_vary_accept_encoding(response: &mut HttpResponse) {
    const TOKEN: &str = "Accept-Encoding";

    let merged = match response.headers.get("Vary") {
        None => TOKEN.to_string(),
        Some(existing) => {
            let already_present = existing.trim() == "*"
                || existing
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(TOKEN));
            if already_present {
                return;
            }
            format!("{}, {}", existing, TOKEN)
        }
    };

    response.headers.insert("Vary".to_string(), merged);
}

impl Default for Compress {
    fn default() -> Self {
        Self {
            level: CompressionLevel::Default,
        }
    }
}

impl Compress {
    /// Create with specific compression level
    pub fn new(level: CompressionLevel) -> Self {
        Self { level }
    }
}

impl Middleware for Compress {
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        let level = self.level;
        // Negotiate before `req` is consumed by the handler.
        let client_accepts_gzip = accepts_gzip(&req);
        Box::pin(async move {
            let mut response = next(req).await?;

            // Advertise that the representation varies by Accept-Encoding, even
            // when we ultimately do not compress (so shared caches stay correct).
            // Merges with any pre-existing `Vary` (e.g. `Vary: Origin` from CORS
            // middleware upstream) instead of clobbering it.
            add_vary_accept_encoding(&mut response);

            // Only compress when the client accepts gzip, the body is non-empty,
            // and it is not already content-encoded.
            let already_encoded = response.headers.contains_key("Content-Encoding");
            if client_accepts_gzip
                && !already_encoded
                && !response.body_ref().is_empty()
                && let Some(compressed) = gzip_encode_offloaded(response.body_ref(), level).await
            {
                let had_content_length = response.headers.contains_key("Content-Length");
                response = response.with_body(compressed);
                response
                    .headers
                    .insert("Content-Encoding".to_string(), "gzip".to_string());
                if had_content_length {
                    response.headers.insert(
                        "Content-Length".to_string(),
                        response.body_len().to_string(),
                    );
                }
            }

            Ok(response)
        })
    }
}

/// Map a handler error to an HTTP status and JSON body.
///
/// The client body comes from the canonical [`Error::to_client_response`], so
/// the micro server emits the exact same `{"error", "status"}` shape as the
/// HTTP/1, HTTP/2, and HTTP/3 servers. For 5xx errors the real internal message
/// is logged here (via `tracing::error`) but never echoed to the client — the
/// redacted helper replaces it with a generic message. 4xx errors keep their
/// message. JSON escaping is handled by `serde_json`.
fn error_response_parts(e: &Error) -> (u16, String) {
    let status = e.status_code();
    if status >= 500 {
        tracing::error!(error = %e, status, "Request handler failed");
    }
    let response = e.to_client_response();
    let body = String::from_utf8(response.into_body_bytes().to_vec()).unwrap_or_default();
    (status, body)
}

/// Run the HTTP server
async fn run_server(app: Arc<BuiltApp>, addr: std::net::SocketAddr) -> std::io::Result<()> {
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Micro-framework server listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let app = app.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let app = app.clone();
                async move {
                    // Convert hyper request to our HttpRequest
                    let method = req.method().to_string();
                    let path = req
                        .uri()
                        .path_and_query()
                        .map(|pq| pq.to_string())
                        .unwrap_or_else(|| "/".to_string());

                    let mut http_req = HttpRequest::new(method, path);

                    // Copy headers
                    for (name, value) in req.headers() {
                        if let Ok(v) = value.to_str() {
                            http_req.headers.insert(name.to_string(), v.to_string());
                        }
                    }

                    // Read body
                    use http_body_util::BodyExt;
                    let body_bytes = req
                        .collect()
                        .await
                        .map(|b| b.to_bytes().to_vec())
                        .unwrap_or_default();
                    http_req.body = body_bytes;

                    // Handle request
                    let response = app.handle(http_req).await;

                    // Convert to hyper response
                    match response {
                        Ok(resp) => {
                            let mut builder = hyper::Response::builder().status(resp.status);

                            for (name, value) in &resp.headers {
                                builder = builder.header(name.as_str(), value.as_str());
                            }
                            for cookie in &resp.cookies {
                                builder = builder.header("Set-Cookie", cookie.as_str());
                            }

                            Ok::<_, std::convert::Infallible>(
                                builder
                                    .body(http_body_util::Full::new(resp.into_body_bytes()))
                                    .unwrap(),
                            )
                        }
                        Err(e) => {
                            let (status, body) = error_response_parts(&e);

                            Ok(hyper::Response::builder()
                                .status(status)
                                .header("Content-Type", "application/json")
                                .body(http_body_util::Full::new(bytes::Bytes::from(body)))
                                .unwrap())
                        }
                    }
                }
            });

            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                tracing::error!("Connection error: {}", err);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[test]
    fn test_app_builder() {
        let app = App::new()
            .route("/", get(test_handler))
            .route("/users", get(test_handler).post(test_handler))
            .build();

        assert_eq!(app.router.len(), 3);
    }

    #[test]
    fn test_scope() {
        let scope = scope("/api")
            .route("/users", get(test_handler))
            .route("/posts", get(test_handler).post(test_handler));

        assert_eq!(scope.routes.len(), 3);
    }

    #[test]
    fn test_data() {
        let data = Data::new(42i32);
        assert_eq!(*data, 42);
    }

    #[tokio::test]
    async fn test_built_app_handle() {
        let app = App::new().route("/test", get(test_handler)).build();

        let req = HttpRequest::new("GET".to_string(), "/test".to_string());
        let response = app.handle(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    struct CountingMiddleware {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Middleware for CountingMiddleware {
        fn call(
            &self,
            req: HttpRequest,
            next: Next,
        ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next(req)
        }
    }

    #[tokio::test]
    async fn test_scope_middleware_runs() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let app = App::new()
            .service(
                scope("/admin")
                    .wrap(CountingMiddleware {
                        calls: calls.clone(),
                    })
                    .route("/users", get(test_handler)),
            )
            .route("/public", get(test_handler))
            .build();

        // Scoped route triggers the scope middleware
        let req = HttpRequest::new("GET".to_string(), "/admin/users".to_string());
        let response = app.handle(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Routes outside the scope do not
        let req = HttpRequest::new("GET".to_string(), "/public".to_string());
        let response = app.handle(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_nested_scope_inherits_parent_middleware() {
        let parent_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inner_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let app = App::new()
            .service(
                scope("/api")
                    .wrap(CountingMiddleware {
                        calls: parent_calls.clone(),
                    })
                    .service(
                        scope("/v1")
                            .wrap(CountingMiddleware {
                                calls: inner_calls.clone(),
                            })
                            .route("/users", get(test_handler)),
                    ),
            )
            .build();

        let req = HttpRequest::new("GET".to_string(), "/api/v1/users".to_string());
        let response = app.handle(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(parent_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(inner_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_built_app_dispatches_catch_all() {
        async fn files(req: HttpRequest) -> Result<HttpResponse, Error> {
            let p = req.path_params.get("path").cloned().unwrap_or_default();
            Ok(HttpResponse::ok().with_body(p.into_bytes()))
        }

        // The optimized serve-path router resolves catch-all params.
        let app = App::new().route("/files/*path", get(files)).build();
        let req = HttpRequest::new("GET".to_string(), "/files/docs/readme.md".to_string());
        let resp = app.handle(req).await.unwrap();
        assert_eq!(resp.body, b"docs/readme.md");
    }

    #[tokio::test]
    async fn test_built_app_query_method_and_unknown_method() {
        async fn echo(req: HttpRequest) -> Result<HttpResponse, Error> {
            Ok(HttpResponse::ok().with_body(req.body.clone()))
        }

        let app = App::new().route("/search", query(echo)).build();

        // QUERY carries its query in the body and routes on method+path.
        let mut req = HttpRequest::new("QUERY".to_string(), "/search".to_string());
        req.body = b"name=john".to_vec();
        let resp = app.handle(req).await.unwrap();
        assert_eq!(resp.into_body_bytes().as_ref(), b"name=john");

        // Unknown method must not fall through to a GET handler.
        let app2 = App::new().route("/search", get(echo)).build();
        let req = HttpRequest::new("PROPFIND".to_string(), "/search".to_string());
        let err = app2.handle(req).await;
        assert!(matches!(err, Err(Error::RouteNotFound(_))));
    }

    #[test]
    fn test_error_response_parts_uses_status_code() {
        assert_eq!(error_response_parts(&Error::Conflict("dup".into())).0, 409);
        assert_eq!(
            error_response_parts(&Error::TooManyRequests("slow down".into())).0,
            429
        );
        assert_eq!(error_response_parts(&Error::NotFound("gone".into())).0, 404);
        assert_eq!(
            error_response_parts(&Error::ServiceUnavailable("down".into())).0,
            503
        );
    }

    #[test]
    fn test_error_response_parts_escapes_json() {
        let e = Error::Validation(r#"bad "quoted" input"#.to_string());
        let (status, body) = error_response_parts(&e);
        assert_eq!(status, 400);

        // Body must be valid JSON despite quotes in the message
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            parsed["error"]
                .as_str()
                .unwrap()
                .contains(r#"bad "quoted" input"#)
        );
    }

    #[test]
    fn test_error_response_parts_hides_internal_message() {
        let e = Error::Internal("secret database password".to_string());
        let (status, body) = error_response_parts(&e);
        assert_eq!(status, 500);
        assert!(!body.contains("secret database password"));

        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "Internal Server Error");
    }

    fn body_next(body: Vec<u8>) -> Next {
        Box::new(move |_req: HttpRequest| {
            Box::pin(async move {
                let mut resp = HttpResponse::ok();
                resp.body = body;
                Ok(resp)
            })
        })
    }

    fn gunzip(data: &[u8]) -> Vec<u8> {
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(data);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).expect("valid gzip stream");
        out
    }

    /// Regression: `Compress` must actually gzip the body (and advertise it via
    /// `Content-Encoding: gzip`) when the client accepts gzip. The old stub
    /// returned the body untouched and never set `Content-Encoding`.
    #[tokio::test]
    async fn test_compress_gzips_when_accepted() {
        let mw = Compress::new(CompressionLevel::Best);
        let mut req = HttpRequest::new("GET".into(), "/".into());
        req.headers.insert("accept-encoding", "gzip, deflate, br");

        let original = vec![b'a'; 8192];
        let resp = mw.call(req, body_next(original.clone())).await.unwrap();

        assert_eq!(
            resp.headers.get("Content-Encoding").map(String::as_str),
            Some("gzip")
        );
        assert_eq!(
            resp.headers.get("Vary").map(String::as_str),
            Some("Accept-Encoding")
        );
        // Highly compressible payload must actually shrink.
        assert!(resp.body_ref().len() < original.len());
        assert_eq!(gunzip(resp.body_ref()), original);
    }

    /// Without `Accept-Encoding: gzip` the body must pass through uncompressed,
    /// but `Vary: Accept-Encoding` is still set for correct downstream caching.
    #[tokio::test]
    async fn test_compress_skips_without_accept_encoding() {
        let mw = Compress::default();
        let req = HttpRequest::new("GET".into(), "/".into());

        let original = vec![b'b'; 8192];
        let resp = mw.call(req, body_next(original.clone())).await.unwrap();

        assert!(resp.headers.get("Content-Encoding").is_none());
        assert_eq!(resp.body_ref(), original.as_slice());
        assert_eq!(
            resp.headers.get("Vary").map(String::as_str),
            Some("Accept-Encoding")
        );
    }

    /// Regression: a pre-existing `Vary` header (e.g. `Vary: Origin` set by
    /// CORS middleware upstream) must be preserved, not clobbered, when
    /// `Compress` adds its own `Accept-Encoding` dimension.
    #[tokio::test]
    async fn test_compress_merges_vary_with_existing_value() {
        let mw = Compress::new(CompressionLevel::Best);
        let mut req = HttpRequest::new("GET".into(), "/".into());
        req.headers.insert("accept-encoding", "gzip");

        let original = vec![b'c'; 8192];
        let next: Next = Box::new(move |_req: HttpRequest| {
            Box::pin(async move {
                let mut resp = HttpResponse::ok();
                resp.body = original;
                resp.headers
                    .insert("Vary".to_string(), "Origin".to_string());
                Ok(resp)
            })
        });

        let resp = mw.call(req, next).await.unwrap();

        let vary = resp.headers.get("Vary").cloned().unwrap_or_default();
        let tokens: Vec<&str> = vary.split(',').map(str::trim).collect();
        assert!(
            tokens.contains(&"Origin"),
            "Vary lost pre-existing Origin token: {vary}"
        );
        assert!(
            tokens
                .iter()
                .any(|t| t.eq_ignore_ascii_case("Accept-Encoding")),
            "Vary missing Accept-Encoding token: {vary}"
        );
    }

    /// A body larger than [`GZIP_OFFLOAD_THRESHOLD`] takes the `spawn_blocking`
    /// offload path. The output must be byte-identical to inline encoding:
    /// gzip-decoding it yields the original bytes exactly.
    #[tokio::test]
    async fn test_compress_offloads_large_body_and_round_trips() {
        let mw = Compress::new(CompressionLevel::Best);
        let mut req = HttpRequest::new("GET".into(), "/".into());
        req.headers.insert("accept-encoding", "gzip");

        // Well above the offload threshold; non-repeating so it does not trivially
        // collapse to nothing.
        let original: Vec<u8> = (0..(GZIP_OFFLOAD_THRESHOLD * 4))
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        assert!(original.len() > GZIP_OFFLOAD_THRESHOLD);

        let resp = mw.call(req, body_next(original.clone())).await.unwrap();

        assert_eq!(
            resp.headers.get("Content-Encoding").map(String::as_str),
            Some("gzip")
        );
        assert_eq!(gunzip(resp.body_ref()), original);
    }
}
