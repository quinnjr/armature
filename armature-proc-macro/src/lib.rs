// Procedural macros for the Armature HTTP framework
// These macros provide Angular-style decorator syntax for Rust

use proc_macro::TokenStream;

mod body_limit_attr;
mod cache_attr;
mod catch_attr;
mod controller;
mod guard_attr;
mod injectable;
mod mcp;
mod middleware_attr;
mod module;
mod params;
mod route_validation;
mod routes;
mod routes_impl;
mod struct_factory;
mod timeout_attr;

/// Marks a struct as injectable, allowing it to be registered in the DI container
#[proc_macro_attribute]
pub fn injectable(attr: TokenStream, item: TokenStream) -> TokenStream {
    injectable::injectable_impl(attr, item)
}

/// Marks a struct as a controller with a base path
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    controller::controller_impl(attr, item)
}

/// Defines a module with providers, controllers, and imports
#[proc_macro_attribute]
pub fn module(attr: TokenStream, item: TokenStream) -> TokenStream {
    module::module_impl(attr, item)
}

/// HTTP GET route decorator
#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "GET")
}

/// HTTP POST route decorator
#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "POST")
}

/// HTTP PUT route decorator
#[proc_macro_attribute]
pub fn put(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "PUT")
}

/// HTTP DELETE route decorator
#[proc_macro_attribute]
pub fn delete(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "DELETE")
}

/// HTTP PATCH route decorator
#[proc_macro_attribute]
pub fn patch(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "PATCH")
}

/// HTTP OPTIONS route decorator
///
/// Used for handling CORS preflight requests or other OPTIONS method calls.
/// Note: For automatic CORS handling, consider using the CORS middleware instead.
///
/// # Usage
///
/// ```ignore
/// use armature::{controller, routes, options};
///
/// #[controller("/api")]
/// struct ApiController;
///
/// #[routes]
/// impl ApiController {
///     #[options("/resource")]
///     async fn resource_options() -> Result<HttpResponse, Error> {
///         Ok(HttpResponse::no_content()
///             .with_header("Allow", "GET, POST, OPTIONS"))
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn options(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "OPTIONS")
}

/// HTTP HEAD route decorator
///
/// HEAD requests are identical to GET requests but without the response body.
/// Useful for checking resource existence or metadata without transferring data.
///
/// # Usage
///
/// ```ignore
/// use armature::{controller, routes, head};
///
/// #[controller("/api")]
/// struct ApiController;
///
/// #[routes]
/// impl ApiController {
///     #[head("/resource/:id")]
///     async fn check_resource(req: HttpRequest) -> Result<HttpResponse, Error> {
///         let id = req.param("id")?;
///         // Check if resource exists
///         Ok(HttpResponse::ok()
///             .with_header("X-Resource-Exists", "true"))
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn head(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "HEAD")
}

/// HTTP QUERY route decorator
///
/// QUERY is a safe, idempotent method that carries the query in the request
/// body (draft-ietf-httpbis-safe-method-w-body). Use it for queries too large
/// or structured for a URL query string.
///
/// ```ignore
/// #[routes]
/// impl SearchController {
///     #[query("/search")]
///     async fn search(req: HttpRequest) -> Result<HttpResponse, Error> {
///         let filters: SearchFilters = req.json()?;
///         // Execute the query described by the request body
///         Ok(HttpResponse::json(&run_search(filters))?)
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn query(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::route_impl(attr, item, "QUERY")
}

/// Routes impl block decorator
///
/// This macro should be applied to the impl block of a controller to register
/// all route handlers with the framework. It works in conjunction with route
/// decorators (#[get], #[post], etc.) to enable proper route registration.
///
/// # Usage
///
/// ```ignore
/// use armature::{controller, routes, get, post};
///
/// #[controller("/api")]
/// #[derive(Default)]
/// struct ApiController;
///
/// #[routes]
/// impl ApiController {
///     #[get("/hello")]
///     async fn hello() -> Result<Json<Message>, Error> {
///         Ok(Json(Message { text: "Hello!".to_string() }))
///     }
///
///     #[post("/echo")]
///     async fn echo(req: HttpRequest) -> Result<Json<Message>, Error> {
///         let msg: Message = req.json()?;
///         Ok(Json(msg))
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn routes(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes_impl::routes_impl(attr, item)
}

/// Extracts and deserializes the request body
#[proc_macro_derive(Body)]
pub fn body_derive(input: TokenStream) -> TokenStream {
    params::body_derive_impl(input)
}

/// Extracts a path parameter
#[proc_macro_derive(Param)]
pub fn param_derive(input: TokenStream) -> TokenStream {
    params::param_derive_impl(input)
}

/// Extracts and deserializes query parameters
#[proc_macro_derive(Query)]
pub fn query_derive(input: TokenStream) -> TokenStream {
    params::query_derive_impl(input)
}

/// Request timeout decorator
///
/// Applies a timeout to the decorated route handler. If the handler doesn't
/// complete within the specified duration, a 408 Request Timeout error is returned.
///
/// # Usage
///
/// ```ignore
/// use armature::{get, timeout};
///
/// // Timeout in seconds (default unit)
/// #[timeout(5)]
/// #[get("/quick")]
/// async fn quick_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Timeout with explicit unit
/// #[timeout(seconds = 30)]
/// #[get("/slow")]
/// async fn slow_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Timeout in milliseconds
/// #[timeout(ms = 500)]
/// #[get("/fast")]
/// async fn fast_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Timeout in minutes
/// #[timeout(minutes = 2)]
/// #[get("/long-running")]
/// async fn long_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
/// ```
#[proc_macro_attribute]
pub fn timeout(attr: TokenStream, item: TokenStream) -> TokenStream {
    timeout_attr::timeout_impl(attr, item)
}

/// Request body size limit decorator
///
/// Applies a body size limit to the decorated route handler. If the request body
/// exceeds the specified size, a 413 Payload Too Large error is returned.
///
/// # Usage
///
/// ```ignore
/// use armature::{post, body_limit};
///
/// // Limit in bytes
/// #[body_limit(1024)]
/// #[post("/tiny")]
/// async fn tiny_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Limit with unit suffix (as string)
/// #[body_limit("10mb")]
/// #[post("/upload")]
/// async fn upload_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Limit with named parameter
/// #[body_limit(mb = 5)]
/// #[post("/medium")]
/// async fn medium_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Various formats supported:
/// #[body_limit(512kb)]      // 512 kilobytes (identifier style)
/// #[body_limit(kb = 512)]   // 512 kilobytes (named parameter)
/// #[body_limit("1.5mb")]    // 1.5 megabytes (string with float)
/// #[body_limit(1gb)]        // 1 gigabyte
/// #[body_limit(bytes = 2048)] // 2048 bytes
/// ```
#[proc_macro_attribute]
pub fn body_limit(attr: TokenStream, item: TokenStream) -> TokenStream {
    body_limit_attr::body_limit_impl(attr, item)
}

/// Cache method decorator
///
/// Automatically caches the result of a method. The cache key is generated from
/// the function name and arguments, and successful results are stored with a TTL.
///
/// # Usage
///
/// ```ignore
/// use armature::cache;
///
/// // Basic caching with default TTL (1 hour)
/// #[cache]
/// async fn get_user(id: i64) -> Result<User, Error> {
///     // Expensive operation
/// }
///
/// // Custom TTL (in seconds)
/// #[cache(ttl = 300)]
/// async fn get_posts(user_id: i64) -> Result<Vec<Post>, Error> {
///     // Cached for 5 minutes
/// }
///
/// // Custom cache key template
/// #[cache(key = "user:profile:{}", ttl = 600)]
/// async fn get_profile(user_id: i64) -> Result<Profile, Error> {
///     // Cached with specific key format
/// }
///
/// // With tags for invalidation
/// #[cache(ttl = 3600, tag = "users")]
/// async fn get_all_users() -> Result<Vec<User>, Error> {
///     // Can be invalidated by tag
/// }
/// ```
///
/// # Requirements
///
/// - The function must be async
/// - The return type must be `Result<T, E>` where `T: Serialize + DeserializeOwned`
/// - Requires a `__cache` or `__tagged_cache` variable in scope
///
/// # Notes
///
/// - Only successful results (`Ok` variants) are cached
/// - Cache keys are generated from the function name and arguments
/// - Default TTL is 3600 seconds (1 hour)
#[proc_macro_attribute]
pub fn cache(attr: TokenStream, item: TokenStream) -> TokenStream {
    cache_attr::cache_impl(attr, item)
}

/// MCP tool decorator
///
/// Marks an async function as an MCP (Model Context Protocol) tool that can be
/// discovered and invoked by AI clients like Cursor and Claude.
///
/// # Usage
///
/// ```ignore
/// use armature::mcp;
/// use armature_mcp::ToolCallResult;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct WeatherInput {
///     location: String,
/// }
///
/// #[mcp(name = "get_weather", description = "Get current weather for a location")]
/// async fn get_weather(input: WeatherInput) -> ToolCallResult {
///     let weather = fetch_weather(&input.location).await;
///     ToolCallResult::text(format!("Weather in {}: {}", input.location, weather))
/// }
/// ```
///
/// # Attributes
///
/// - `name` - Tool name (defaults to function name)
/// - `description` - Human-readable description
/// - `owner` - Owner type for grouping (auto-generated if not specified)
///
/// # Requirements
///
/// - Function must be async
/// - Function must return `ToolCallResult`
/// - Input parameter should implement `Deserialize`
#[proc_macro_attribute]
pub fn mcp(attr: TokenStream, item: TokenStream) -> TokenStream {
    mcp::mcp_impl(attr, item)
}

/// MCP resource decorator
///
/// Marks an async function as an MCP resource that exposes data to AI clients.
///
/// # Usage
///
/// ```ignore
/// use armature::mcp_resource;
/// use armature_mcp::ResourceContent;
///
/// #[mcp_resource(
///     uri = "config://app/settings",
///     name = "App Settings",
///     description = "Application configuration",
///     mime_type = "application/json"
/// )]
/// async fn get_settings() -> ResourceContent {
///     let settings = load_settings().await;
///     ResourceContent::json("config://app/settings", serde_json::to_string(&settings).unwrap())
/// }
/// ```
///
/// # Attributes
///
/// - `uri` - Resource URI (required)
/// - `name` - Human-readable name (defaults to function name)
/// - `description` - Optional description
/// - `mime_type` - MIME type of the resource
/// - `owner` - Owner type for grouping
#[proc_macro_attribute]
pub fn mcp_resource(attr: TokenStream, item: TokenStream) -> TokenStream {
    mcp::mcp_resource_impl(attr, item)
}

/// Exception filter decorator.
///
/// Turns an `async fn(error: &Error, ctx: &ExceptionContext) -> HttpResponse`
/// into an [`ExceptionFilter`](armature_core::exception_filter::ExceptionFilter)
/// implementation plus a factory function of the same name.
///
/// # Usage
///
/// ```ignore
/// use armature_proc_macro::catch;
///
/// // Catch all errors
/// #[catch]
/// async fn handle_all(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
///     HttpResponse::internal_server_error()
/// }
///
/// // Catch specific error variants
/// #[catch(NotFound, RouteNotFound)]
/// async fn handle_not_found(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
///     HttpResponse::not_found()
/// }
///
/// // With priority and an explicit name
/// #[catch(Validation, priority = 100, name = "validation")]
/// async fn handle_validation(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
///     HttpResponse::new(422)
/// }
/// ```
#[proc_macro_attribute]
pub fn catch(attr: TokenStream, item: TokenStream) -> TokenStream {
    catch_attr::catch_impl(attr, item)
}

/// Applies one or more guard *types* to a route handler.
///
/// Each guard type must implement [`Guard`](armature_core::guard::Guard) and
/// `Default`. Before the handler runs, every guard's `can_activate` is checked;
/// a `false` result short-circuits with `Error::Forbidden` and an `Err` is
/// propagated.
///
/// ```ignore
/// use armature_proc_macro::{get, use_guard};
///
/// #[use_guard(AuthenticationGuard)]
/// #[get("/protected")]
/// async fn protected(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
/// ```
#[proc_macro_attribute]
pub fn use_guard(attr: TokenStream, item: TokenStream) -> TokenStream {
    guard_attr::use_guard_impl(attr, item)
}

/// Applies one or more guard *instances* to a route handler.
///
/// On a controller **struct** it attaches guard metadata that the [`module`]
/// route registrar enforces: before dispatching any route declared on that
/// controller, every guard's `can_activate` runs, and the first guard that does
/// not activate (or errors) short-circuits the request with `403 Forbidden`.
///
/// ```ignore
/// use armature_proc_macro::{controller, get, guard, routes};
///
/// // Handler form — guard instance on a single route.
/// #[guard(RolesGuard::new(vec!["admin".to_string()]))]
/// #[get("/admin")]
/// async fn admin(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Controller-struct form — guards every route on the controller.
/// #[controller("/admin")]
/// #[guard(AuthGuard)]
/// struct AdminController;
/// ```
#[proc_macro_attribute]
pub fn guard(attr: TokenStream, item: TokenStream) -> TokenStream {
    guard_attr::guard_impl(attr, item)
}

/// Applies one or more middleware *instances* to a route handler.
///
/// The handler is wrapped in a [`MiddlewareChain`](armature_core::middleware::MiddlewareChain)
/// built from the supplied middleware expressions.
///
/// ```ignore
/// use armature_proc_macro::{get, use_middleware};
///
/// #[use_middleware(LoggerMiddleware::new(), CorsMiddleware::new())]
/// #[get("/users")]
/// async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
/// ```
#[proc_macro_attribute]
pub fn use_middleware(attr: TokenStream, item: TokenStream) -> TokenStream {
    middleware_attr::use_middleware_impl(attr, item)
}

/// Applies middleware to a route handler function.
///
/// On a controller **struct** it attaches middleware metadata that the
/// [`module`] route registrar enforces: every route declared on that controller
/// is wrapped in a middleware chain built from the supplied expressions (the
/// first-listed middleware runs first).
///
/// ```ignore
/// use armature_proc_macro::{controller, get, middleware};
///
/// // Handler form — middleware on a single route.
/// #[middleware(LoggerMiddleware::new())]
/// #[get("/users")]
/// async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // Controller-struct form — wraps every route on the controller.
/// #[controller("/users")]
/// #[middleware(LoggerMiddleware::new())]
/// struct UsersController;
/// ```
#[proc_macro_attribute]
pub fn middleware(attr: TokenStream, item: TokenStream) -> TokenStream {
    middleware_attr::middleware_impl(attr, item)
}
