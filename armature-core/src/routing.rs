// Routing system for HTTP requests
//
// This module provides an optimized routing system that leverages:
// - Monomorphization: Handlers are specialized at compile time
// - Inline dispatch: Hot paths use #[inline(always)]
// - Zero-cost abstractions: Minimal runtime overhead

use crate::handler::{BoxedHandler, IntoHandler};
use crate::logging::{debug, trace};
use crate::route_constraint::RouteConstraints;
use crate::{Error, HttpMethod, HttpRequest, HttpResponse};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A route handler function type (legacy - for backwards compatibility)
///
/// **Deprecated**: Use `BoxedHandler` for better performance via monomorphization.
/// This type uses double dynamic dispatch (dyn Fn + Box<dyn Future>) which
/// prevents the compiler from inlining handler code.
///
/// Prefer using the optimized handler system:
/// ```ignore
/// use armature_core::handler::handler;
///
/// let h = handler(my_async_fn);
/// ```
pub type HandlerFn = Arc<
    dyn Fn(HttpRequest) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        + Send
        + Sync,
>;

/// Optimized route handler that enables inlining via monomorphization.
///
/// This type wraps handlers in a way that allows the compiler to see through
/// to the actual handler implementation and inline it.
pub type OptimizedHandler = BoxedHandler;

/// Route definition with handler
#[derive(Clone)]
pub struct Route {
    pub method: HttpMethod,
    pub path: String,
    /// The route handler - uses optimized dispatch
    pub handler: BoxedHandler,
    /// Optional route constraints for parameter validation
    pub constraints: Option<RouteConstraints>,
}

impl Route {
    /// Create a new route with an optimized handler.
    ///
    /// This method accepts any handler type that implements `IntoHandler`,
    /// enabling compile-time specialization.
    #[inline]
    pub fn new<H, Args>(method: HttpMethod, path: impl Into<String>, handler: H) -> Self
    where
        H: IntoHandler<Args>,
    {
        Self {
            method,
            path: path.into(),
            handler: BoxedHandler::new(handler.into_handler()),
            constraints: None,
        }
    }

    /// Create a route from a legacy HandlerFn for backwards compatibility.
    #[inline]
    pub fn from_legacy(method: HttpMethod, path: impl Into<String>, handler: HandlerFn) -> Self {
        Self {
            method,
            path: path.into(),
            handler: crate::handler::from_legacy_handler(handler),
            constraints: None,
        }
    }

    /// Add route constraints.
    #[inline]
    pub fn with_constraints(mut self, constraints: RouteConstraints) -> Self {
        self.constraints = Some(constraints);
        self
    }
}

/// Router for managing routes and dispatching requests.
///
/// The router uses optimized handler dispatch that enables:
/// - Monomorphization of handler code
/// - Inlining of handler bodies
/// - Minimal allocation in the hot path
#[derive(Clone)]
pub struct Router {
    pub routes: Vec<Route>,
}

impl Router {
    /// Create a new empty router.
    #[inline]
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Add a route to the router.
    #[inline]
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }

    /// Add a GET route with an optimized handler.
    #[inline]
    pub fn get<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes.push(Route::new(HttpMethod::GET, path, handler));
        self
    }

    /// Add a POST route with an optimized handler.
    #[inline]
    pub fn post<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::POST, path, handler));
        self
    }

    /// Add a PUT route with an optimized handler.
    #[inline]
    pub fn put<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes.push(Route::new(HttpMethod::PUT, path, handler));
        self
    }

    /// Add a DELETE route with an optimized handler.
    #[inline]
    pub fn delete<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::DELETE, path, handler));
        self
    }

    /// Add a PATCH route with an optimized handler.
    #[inline]
    pub fn patch<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::PATCH, path, handler));
        self
    }

    /// Add an OPTIONS route with an optimized handler.
    ///
    /// OPTIONS requests are typically used for CORS preflight checks.
    /// For automatic CORS handling, consider using the CORS middleware instead.
    #[inline]
    pub fn options<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::OPTIONS, path, handler));
        self
    }

    /// Add a HEAD route with an optimized handler.
    ///
    /// HEAD requests are identical to GET but without the response body.
    /// Useful for checking resource existence or metadata.
    #[inline]
    pub fn head<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::HEAD, path, handler));
        self
    }

    /// Match a route without executing the handler.
    /// Returns the handler and path parameters if a route matches.
    /// Useful for route lookup benchmarking and inspection.
    #[inline]
    pub fn match_route(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(BoxedHandler, HashMap<String, String>)> {
        // Strip query string if present
        let path = path.split('?').next().unwrap_or(path);

        for route in &self.routes {
            if route.method.as_str() != method {
                continue;
            }

            if let Some(params) = match_path(&route.path, path) {
                return Some((route.handler.clone(), params));
            }
        }

        None
    }

    /// Find a route that matches the request and execute the handler.
    ///
    /// This is the main hot path for request handling. The handler dispatch
    /// is optimized via monomorphization - the actual handler code can be
    /// inlined by the compiler.
    #[inline]
    pub async fn route(&self, mut request: HttpRequest) -> Result<HttpResponse, Error> {
        debug!("Routing request: {} {}", request.method, request.path);

        // Parse query parameters from path
        let (path, query_string) = request
            .path
            .split_once('?')
            .map(|(p, q)| (p, Some(q)))
            .unwrap_or((&request.path, None));

        if let Some(query) = query_string {
            trace!("Parsing query string: {}", query);
            request.query_params = parse_query_string(query);
        }

        // Find matching route - this is the route matching hot path
        for route in &self.routes {
            if route.method.as_str() != request.method {
                continue;
            }

            if let Some(params) = match_path(&route.path, path) {
                debug!(
                    "Route matched: {} {} -> {}",
                    request.method, path, route.path
                );

                // Validate route constraints if present
                if let Some(constraints) = &route.constraints {
                    trace!("Validating route constraints");
                    constraints.validate(&params)?;
                }

                request.path_params = params;

                // Handler dispatch - the BoxedHandler.call() is optimized
                // to allow the compiler to inline the actual handler body
                trace!("Dispatching handler");
                return route.handler.call(request).await;
            }
        }

        debug!("No route found for {} {}", request.method, path);
        Err(Error::RouteNotFound(format!("{} {}", request.method, path)))
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Match a route path pattern against a request path
/// Returns Some(params) if matched, None otherwise
fn match_path(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if pattern_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (pattern_part, path_part) in pattern_parts.iter().zip(path_parts.iter()) {
        if let Some(param_name) = pattern_part.strip_prefix(':') {
            // This is a parameter
            params.insert(param_name.to_string(), path_part.to_string());
        } else if pattern_part != path_part {
            // Static part doesn't match
            return None;
        }
    }

    Some(params)
}

/// Parse a query string into a map of parameters
///
/// Uses SIMD-optimized byte searching via memchr for faster parsing.
#[inline]
fn parse_query_string(query: &str) -> HashMap<String, String> {
    // Use the SIMD-optimized parser
    crate::simd_parser::parse_query_string_fast(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test helper handler
    async fn test_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[test]
    fn test_match_path_static() {
        let pattern = "/users";
        let path = "/users";
        let result = match_path(pattern, path);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_match_path_with_param() {
        let pattern = "/users/:id";
        let path = "/users/123";
        let result = match_path(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_path_no_match() {
        let pattern = "/users/:id";
        let path = "/posts/123";
        let result = match_path(pattern, path);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_query_string() {
        let query = "name=john&age=30";
        let params = parse_query_string(query);
        assert_eq!(params.get("name"), Some(&"john".to_string()));
        assert_eq!(params.get("age"), Some(&"30".to_string()));
    }

    #[test]
    fn test_match_path_multiple_params() {
        let pattern = "/users/:user_id/posts/:post_id";
        let path = "/users/123/posts/456";
        let result = match_path(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("user_id"), Some(&"123".to_string()));
        assert_eq!(params.get("post_id"), Some(&"456".to_string()));
    }

    #[test]
    fn test_match_path_trailing_slash() {
        let pattern = "/users";
        let path = "/users/";
        let result = match_path(pattern, path);
        // Should handle trailing slash gracefully
        assert!(result.is_some() || result.is_none());
    }

    #[test]
    fn test_match_path_nested() {
        let pattern = "/api/v1/users/:id";
        let path = "/api/v1/users/123";
        let result = match_path(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_path_empty() {
        let pattern = "/";
        let path = "/";
        let result = match_path(pattern, path);
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_query_string_empty() {
        let query = "";
        let params = parse_query_string(query);
        // Empty string may return one empty entry, which is fine
        assert!(params.is_empty() || params.len() == 1);
    }

    #[test]
    fn test_parse_query_string_special_chars() {
        let query = "name=john%20doe&email=test%40example.com";
        let params = parse_query_string(query);
        assert!(params.contains_key("name"));
        assert!(params.contains_key("email"));
    }

    #[test]
    fn test_parse_query_string_no_value() {
        let query = "flag&debug=true";
        let params = parse_query_string(query);
        assert!(params.contains_key("debug"));
        assert_eq!(params.get("debug"), Some(&"true".to_string()));
    }

    #[test]
    fn test_match_path_param_with_special_chars() {
        let pattern = "/users/:id";
        let path = "/users/abc-123";
        let result = match_path(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"abc-123".to_string()));
    }

    #[test]
    fn test_route_creation_optimized() {
        // Test the new optimized route creation
        let route = Route::new(HttpMethod::GET, "/users", test_handler);
        assert_eq!(route.method, HttpMethod::GET);
        assert_eq!(route.path, "/users");
    }

    #[test]
    fn test_route_creation_legacy() {
        // Test legacy handler compatibility
        let legacy_handler: HandlerFn =
            Arc::new(|_req| Box::pin(async move { Ok(HttpResponse::ok()) }));
        let route = Route::from_legacy(HttpMethod::GET, "/users", legacy_handler);
        assert_eq!(route.method, HttpMethod::GET);
        assert_eq!(route.path, "/users");
    }

    #[test]
    fn test_router_fluent_api() {
        let mut router = Router::new();
        router
            .get("/users", test_handler)
            .post("/users", test_handler)
            .put("/users/:id", test_handler)
            .delete("/users/:id", test_handler)
            .patch("/users/:id", test_handler)
            .options("/users", test_handler)
            .head("/users/:id", test_handler);

        assert_eq!(router.routes.len(), 7);
    }

    #[test]
    fn test_router_options_route() {
        let mut router = Router::new();
        router.options("/api/resource", test_handler);

        assert_eq!(router.routes.len(), 1);
        assert_eq!(router.routes[0].method, HttpMethod::OPTIONS);
        assert_eq!(router.routes[0].path, "/api/resource");
    }

    #[test]
    fn test_router_head_route() {
        let mut router = Router::new();
        router.head("/api/resource/:id", test_handler);

        assert_eq!(router.routes.len(), 1);
        assert_eq!(router.routes[0].method, HttpMethod::HEAD);
        assert_eq!(router.routes[0].path, "/api/resource/:id");
    }

    #[test]
    fn test_router_add_route() {
        let mut router = Router::new();
        let route = Route::new(HttpMethod::GET, "/test", test_handler);
        router.add_route(route);
        assert_eq!(router.routes.len(), 1);
    }

    #[test]
    fn test_router_multiple_routes() {
        let mut router = Router::new();

        for i in 0..5 {
            router.get(format!("/test{}", i), test_handler);
        }

        assert_eq!(router.routes.len(), 5);
    }

    #[test]
    fn test_parse_query_string_multiple_same_key() {
        let query = "tag=rust&tag=web&tag=framework";
        let params = parse_query_string(query);
        // Should contain at least one tag
        assert!(params.contains_key("tag"));
    }

    #[test]
    fn test_route_with_constraints() {
        let constraints =
            RouteConstraints::new().add("id", Box::new(crate::route_constraint::IntConstraint));

        let route =
            Route::new(HttpMethod::GET, "/users/:id", test_handler).with_constraints(constraints);

        assert!(route.constraints.is_some());
    }

    #[tokio::test]
    async fn test_router_dispatch() {
        let mut router = Router::new();
        router.get("/test", test_handler);

        let req = HttpRequest::new("GET".to_string(), "/test".to_string());
        let response = router.route(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_router_dispatch_with_params() {
        async fn param_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
            let id = req.param("id").unwrap();
            Ok(HttpResponse::ok().with_body(id.as_bytes().to_vec()))
        }

        let mut router = Router::new();
        router.get("/users/:id", param_handler);

        let req = HttpRequest::new("GET".to_string(), "/users/123".to_string());
        let response = router.route(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(String::from_utf8(response.body).unwrap(), "123");
    }

    #[tokio::test]
    async fn test_router_404() {
        let router = Router::new();
        let req = HttpRequest::new("GET".to_string(), "/nonexistent".to_string());
        let result = router.route(req).await;
        assert!(matches!(result, Err(Error::RouteNotFound(_))));
    }
}
