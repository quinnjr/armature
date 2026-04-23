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
        for route in scope.routes {
            let full_path = format!("{}{}", scope.prefix, route.path);
            self.router.add_route(crate::routing::Route {
                method: route.method,
                path: full_path,
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
            router: self.router,
            middleware: self.middleware,
            state: self.state,
            default_service: self.default_service,
        });

        run_server(app, addr).await
    }

    /// Build the application into an immutable form
    pub fn build(self) -> BuiltApp {
        BuiltApp {
            router: self.router,
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
pub struct BuiltApp {
    router: Router,
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
    pub fn service(mut self, inner: Scope) -> Self {
        for route in inner.routes {
            let full_path = format!("{}{}", inner.prefix, route.path);
            self.routes.push(ScopeRoute {
                method: route.method,
                path: full_path,
                handler: route.handler,
                constraints: route.constraints,
            });
        }
        self
    }
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

/// Compression middleware
pub struct Compress {
    #[allow(dead_code)]
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
        Box::pin(async move {
            let mut response = next(req).await?;
            // Add Vary header for proper caching
            response
                .headers
                .insert("Vary".to_string(), "Accept-Encoding".to_string());
            // Note: Actual compression would be done at the transport layer
            Ok(response)
        })
    }
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

                            Ok::<_, std::convert::Infallible>(
                                builder
                                    .body(http_body_util::Full::new(bytes::Bytes::from(resp.body)))
                                    .unwrap(),
                            )
                        }
                        Err(e) => {
                            let status = match &e {
                                Error::RouteNotFound(_) => 404,
                                Error::Validation(_) => 400,
                                Error::Unauthorized(_) => 401,
                                Error::Forbidden(_) => 403,
                                _ => 500,
                            };

                            Ok(hyper::Response::builder()
                                .status(status)
                                .header("Content-Type", "application/json")
                                .body(http_body_util::Full::new(bytes::Bytes::from(format!(
                                    r#"{{"error":"{}"}}"#,
                                    e
                                ))))
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

        assert_eq!(app.router.routes.len(), 3);
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
}
