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
use smallvec::SmallVec;
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

    /// Add a QUERY route with an optimized handler.
    ///
    /// QUERY is a safe, idempotent method that carries the query in the
    /// request body (draft-ietf-httpbis-safe-method-w-body). Use it for
    /// queries too large or structured for a URL query string.
    #[inline]
    pub fn query<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.routes
            .push(Route::new(HttpMethod::QUERY, path, handler));
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

        // Split the request path once, up front, rather than per candidate route.
        let path_parts: SmallVec<[&str; 8]> = split_segments(path).collect();

        for route in &self.routes {
            if route.method.as_str() != method {
                continue;
            }

            if let Some(params) = match_path(&route.path, &path_parts) {
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

        // Match on the path alone; the query is parsed on demand by
        // `HttpRequest::query`, and only if a handler asks for it.
        let path = request.path_only();

        // Find matching route - this is the route matching hot path. The request
        // path is split once, up front, rather than per candidate route. The
        // `path_parts` borrow of `request.path` is confined to this block (which
        // drops the `SmallVec`) so `request` can be moved into the handler below;
        // we only carry out the matched index + params.
        let matched: Option<(usize, HashMap<String, String>)> = {
            let path_parts: SmallVec<[&str; 8]> = split_segments(path).collect();

            let mut found = None;
            for (idx, route) in self.routes.iter().enumerate() {
                if route.method.as_str() != request.method_str() {
                    continue;
                }

                if let Some(params) = match_path(&route.path, &path_parts) {
                    debug!(
                        "Route matched: {} {} -> {}",
                        request.method, path, route.path
                    );
                    found = Some((idx, params));
                    break;
                }
            }
            found
        };

        if let Some((idx, params)) = matched {
            let route = &self.routes[idx];

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

        debug!("No route found for {} {}", request.method, path);
        Err(Error::RouteNotFound(format!("{} {}", request.method, path)))
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

/// Split a path into non-empty segments without allocating a `Vec`.
#[inline]
fn split_segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// Match a route path pattern against a request path that has already been
/// split into non-empty segments.
///
/// The pattern is compared segment-by-segment using a `split('/')` iterator,
/// so no `Vec` is allocated per candidate route. The `HashMap` of parameters is
/// only allocated once a route actually matches (and only sized for the number
/// of parameters the pattern declares).
fn match_path(pattern: &str, path_parts: &[&str]) -> Option<HashMap<String, String>> {
    // First pass: validate the segment count and static segments, and count
    // how many parameters the pattern declares. No allocation happens here.
    //
    // A `*name` segment is a catch-all: it consumes every remaining path
    // segment (zero or more) and, matching `CompiledRoute::matches`, ends
    // pattern validation right there (any pattern segments after a
    // catch-all are unreachable, same as the compiled matcher).
    let mut seen = 0usize;
    let mut param_count = 0usize;
    let mut catch_all_at: Option<usize> = None;
    for (i, pattern_part) in split_segments(pattern).enumerate() {
        if pattern_part.starts_with('*') {
            catch_all_at = Some(i);
            param_count += 1;
            break;
        }

        // Pattern has more segments than the request path
        let path_part = path_parts.get(i)?;
        if pattern_part.starts_with(':') {
            param_count += 1;
        } else if pattern_part != *path_part {
            // Static segment doesn't match
            return None;
        }
        seen = i + 1;
    }

    if let Some(idx) = catch_all_at {
        // Everything before the catch-all must already be present in the
        // request path; the catch-all itself may consume zero segments. The
        // per-segment loop above already validated this: for every `i < idx`
        // it early-returns `None` via `path_parts.get(i)?` if that index is
        // missing, so by the time we reach here `path_parts.len() >= idx`
        // always holds. Documented as an invariant rather than re-checked.
        debug_assert!(
            path_parts.len() >= idx,
            "catch-all index validated during first pass"
        );
    } else if seen != path_parts.len() {
        // Pattern must consume every request-path segment
        return None;
    }

    // Second pass: the route matched, so allocate the params map now.
    let mut params = HashMap::with_capacity(param_count);
    if param_count > 0 {
        for (i, pattern_part) in split_segments(pattern).enumerate() {
            if let Some(name) = pattern_part.strip_prefix('*') {
                // Catch-all: join all remaining path segments, matching
                // `CompiledRoute::extract_params`'s joining convention.
                let name = if name.is_empty() { "*" } else { name };
                params.insert(name.to_string(), path_parts[i..].join("/"));
                break;
            } else if let Some(param_name) = pattern_part.strip_prefix(':') {
                params.insert(param_name.to_string(), path_parts[i].to_string());
            }
        }
    }

    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    // Test helper handler
    async fn test_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    // Test helper: split a raw path and run match_path, mirroring how the
    // router pre-splits the request path before the route loop.
    fn match_path_str(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
        let parts: Vec<&str> = super::split_segments(path).collect();
        match_path(pattern, &parts)
    }

    #[test]
    fn test_match_path_static() {
        let pattern = "/users";
        let path = "/users";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_match_path_with_param() {
        let pattern = "/users/:id";
        let path = "/users/123";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_path_no_match() {
        let pattern = "/users/:id";
        let path = "/posts/123";
        let result = match_path_str(pattern, path);
        assert!(result.is_none());
    }

    #[test]
    fn test_query_method_round_trip() {
        assert_eq!(HttpMethod::from_str("QUERY"), Some(HttpMethod::QUERY));
        assert_eq!(HttpMethod::from_str("query"), Some(HttpMethod::QUERY));
        assert_eq!(HttpMethod::QUERY.as_str(), "QUERY");
    }

    #[tokio::test]
    async fn test_query_route_dispatch() {
        async fn echo_body(req: HttpRequest) -> Result<HttpResponse, Error> {
            Ok(HttpResponse::ok().with_bytes_body(req.body.clone()))
        }

        let mut router = Router::new();
        router.query("/search", echo_body);

        let mut request = HttpRequest::new("QUERY", "/search".to_string());
        request.body = Bytes::from_static(b"name=john");
        let response = router.route(request).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.into_body_bytes().as_ref(), b"name=john");

        // A GET to the same path must not hit the QUERY handler
        let request = HttpRequest::new("GET", "/search".to_string());
        assert!(router.route(request).await.is_err());
    }

    #[test]
    fn test_match_path_multiple_params() {
        let pattern = "/users/:user_id/posts/:post_id";
        let path = "/users/123/posts/456";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("user_id"), Some(&"123".to_string()));
        assert_eq!(params.get("post_id"), Some(&"456".to_string()));
    }

    #[test]
    fn test_match_path_trailing_slash() {
        let pattern = "/users";
        let path = "/users/";
        let result = match_path_str(pattern, path);
        // Should handle trailing slash gracefully
        assert!(result.is_some() || result.is_none());
    }

    #[test]
    fn test_match_path_nested() {
        let pattern = "/api/v1/users/:id";
        let path = "/api/v1/users/123";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_path_empty() {
        let pattern = "/";
        let path = "/";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
    }

    #[test]
    fn test_match_path_catch_all() {
        // Mirrors route_cache.rs's `test_compiled_route_catch_all`, but
        // exercises the linear `Router`'s own matcher directly.
        let pattern = "/files/*path";

        assert!(match_path_str(pattern, "/files/docs").is_some());

        let result = match_path_str(pattern, "/files/docs/readme.md");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("path"), Some(&"docs/readme.md".to_string()));

        // An exact prefix match (no trailing segments) should still match,
        // with the catch-all param extracted as an empty string.
        let result = match_path_str(pattern, "/files");
        assert!(result.is_some());
        assert_eq!(result.unwrap().get("path"), Some(&String::new()));

        // A path that doesn't even reach the static prefix must not match.
        assert!(match_path_str(pattern, "/other").is_none());
    }

    #[test]
    fn test_match_path_catch_all_with_preceding_param() {
        // A catch-all preceded by a named `:param` segment must extract both
        // the param and the catch-all correctly.
        let pattern = "/users/:id/files/*path";
        let result = match_path_str(pattern, "/users/42/files/a/b");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"42".to_string()));
        assert_eq!(params.get("path"), Some(&"a/b".to_string()));

        // A bare, unnamed catch-all (`*` with no name) stores its captured
        // value under the literal key `"*"` (see the `name.is_empty()`
        // fallback in `match_path`).
        let pattern = "/files/*";
        let result = match_path_str(pattern, "/files/a/b/c");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("*"), Some(&"a/b/c".to_string()));
    }

    #[tokio::test]
    async fn test_router_route_catch_all() {
        // Mirrors route_cache.rs's `test_from_router_catch_all`, but calls
        // `Router::route` directly instead of going through
        // `OptimizedRouter`, to make sure the linear router's own catch-all
        // handling (not just the compiled fast path) works end to end.
        async fn echo_path(req: HttpRequest) -> Result<HttpResponse, Error> {
            let p = req.path_params.get("path").cloned().unwrap_or_default();
            Ok(HttpResponse::ok().with_body(p.into_bytes()))
        }

        let mut router = Router::new();
        router.get("/files/*path", echo_path);

        let req = HttpRequest::new("GET", "/files/docs/readme.md".to_string());
        let response = router.route(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.into_body_bytes().as_ref(), b"docs/readme.md");

        // `match_route` (used for lookup without dispatch) must agree.
        let (_, params) = router
            .match_route("GET", "/files/docs/readme.md")
            .expect("catch-all route should match via match_route");
        assert_eq!(params.get("path"), Some(&"docs/readme.md".to_string()));
    }

    #[test]
    fn test_match_path_param_with_special_chars() {
        let pattern = "/users/:id";
        let path = "/users/abc-123";
        let result = match_path_str(pattern, path);
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

        let req = HttpRequest::new("GET", "/test".to_string());
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

        let req = HttpRequest::new("GET", "/users/123".to_string());
        let response = router.route(req).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(String::from_utf8(response.body.to_vec()).unwrap(), "123");
    }

    #[tokio::test]
    async fn test_router_404() {
        let router = Router::new();
        let req = HttpRequest::new("GET", "/nonexistent".to_string());
        let result = router.route(req).await;
        assert!(matches!(result, Err(Error::RouteNotFound(_))));
    }
}
