// Routing system for HTTP requests
//
// This module provides an optimized routing system that leverages:
// - Monomorphization: Handlers are specialized at compile time
// - Inline dispatch: Hot paths use #[inline(always)]
// - Zero-cost abstractions: Minimal runtime overhead

use crate::handler::{BoxedHandler, IntoHandler};
use crate::logging::{debug, trace};
use crate::param_intern;
use crate::route_constraint::RouteConstraints;
use crate::{Error, HttpMethod, HttpRequest, HttpResponse, Method, RouteParams};
use bytes::Bytes;
use smallvec::SmallVec;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

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
pub struct Router {
    pub routes: Vec<Route>,
    /// Built on first dispatch, invalidated by [`Router::add_route`].
    ///
    /// Lazy rather than built per insertion because routes are registered in
    /// bulk at startup, and rebuilding the trees on every `add_route` would be
    /// quadratic for no benefit.
    index: OnceLock<MethodIndex>,
}

impl Clone for Router {
    /// The clone starts with an unbuilt index.
    ///
    /// `OnceLock` is not `Clone`, and the index is a pure function of `routes`,
    /// so rebuilding it on first dispatch costs nothing that carrying it would
    /// have saved.
    fn clone(&self) -> Self {
        Self {
            routes: self.routes.clone(),
            index: OnceLock::new(),
        }
    }
}

impl Router {
    /// Create a new empty router.
    #[inline]
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            index: OnceLock::new(),
        }
    }

    /// Add a route to the router.
    #[inline]
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
        // `&mut self` is what makes this sound: no dispatch can be reading the
        // index while it is replaced.
        self.index = OnceLock::new();
    }

    /// The method-indexed trees, built on first use.
    #[inline]
    fn index(&self) -> &MethodIndex {
        self.index.get_or_init(|| MethodIndex::build(&self.routes))
    }

    /// Add a GET route with an optimized handler.
    #[inline]
    pub fn get<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.add_route(Route::new(HttpMethod::GET, path, handler));
        self
    }

    /// Add a POST route with an optimized handler.
    #[inline]
    pub fn post<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.add_route(Route::new(HttpMethod::POST, path, handler));
        self
    }

    /// Add a PUT route with an optimized handler.
    #[inline]
    pub fn put<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.add_route(Route::new(HttpMethod::PUT, path, handler));
        self
    }

    /// Add a DELETE route with an optimized handler.
    #[inline]
    pub fn delete<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.add_route(Route::new(HttpMethod::DELETE, path, handler));
        self
    }

    /// Add a PATCH route with an optimized handler.
    #[inline]
    pub fn patch<H, Args>(&mut self, path: impl Into<String>, handler: H) -> &mut Self
    where
        H: IntoHandler<Args>,
    {
        self.add_route(Route::new(HttpMethod::PATCH, path, handler));
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
        self.add_route(Route::new(HttpMethod::OPTIONS, path, handler));
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
        self.add_route(Route::new(HttpMethod::HEAD, path, handler));
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
        self.add_route(Route::new(HttpMethod::QUERY, path, handler));
        self
    }

    /// Match a route without executing the handler.
    /// Returns the handler and path parameters if a route matches.
    /// Useful for route lookup benchmarking and inspection.
    ///
    /// **Route constraints are not evaluated here.** A route whose parameters
    /// would fail its [`RouteConstraints`] still matches, so this is not a
    /// dispatch entry point — use [`Router::route`], which validates the
    /// constraints before calling the handler.
    #[inline]
    pub fn match_route(&self, method: &str, path: &str) -> Option<(BoxedHandler, RouteParams)> {
        // Strip query string if present
        let path = path.split('?').next().unwrap_or(path);
        let method = Method::from(method);
        self.index()
            .find(&self.routes, &method, path)
            .map(|(idx, params)| (self.routes[idx].handler.clone(), params))
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

        // The borrow of `request.path` is confined to this statement so the
        // request can be moved into the handler below; only the matched index
        // and the captured params are carried out.
        let matched = self.index().find(&self.routes, &request.method, path);

        if let Some((idx, params)) = matched {
            let route = &self.routes[idx];

            // Validate route constraints if present
            if let Some(constraints) = &route.constraints {
                trace!("Validating route constraints");
                constraints.validate(&params)?;
            }

            request.set_params(params);

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

/// Rewrite an armature route pattern into `matchit` syntax.
///
/// How many parameters one route may carry before `matchit` refuses it.
///
/// It rewrites each non-catch-all parameter to a single letter while
/// normalizing a route and starts at `a`, so the 26th exhausts the alphabet —
/// and it announces that by panicking rather than returning an error, which no
/// `is_err` arm can turn into a fallback. 25 is therefore the most a route may
/// carry; anything above is kept out of the tree and answered by the linear
/// scan instead.
///
/// The `{`-byte count this is compared against is deliberately an upper bound,
/// not the exact figure `matchit` uses: a trailing catch-all is exempt from the
/// rewrite and consumes no letter, and an escaped `{{` is collapsed before the
/// wildcards are found. Over-counting costs one route a linear scan it did not
/// strictly need; under-counting would leave the panic reachable, so the bound
/// is kept on the safe side rather than tightened to match.
const MATCHIT_MAX_PARAMS: usize = 25;

/// `:id` becomes `{id}` and `*rest` becomes `{*rest}`. User-facing pattern
/// syntax does not change — this is the seam that keeps it from having to.
/// Segments that are already braced pass through untouched.
pub(crate) fn translate_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    for (i, segment) in pattern.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        match segment.as_bytes().first() {
            Some(b':') => {
                out.push('{');
                out.push_str(&segment[1..]);
                out.push('}');
            }
            Some(b'*') => {
                out.push_str("{*");
                out.push_str(&segment[1..]);
                out.push('}');
            }
            _ => out.push_str(segment),
        }
    }
    out
}

/// Strip one trailing `/` so a target reaches `matchit` in the shape the
/// segment matcher would have seen.
///
/// `matchit` treats a trailing slash as significant; `split_segments` — and
/// therefore `match_path_spans` and `CompiledRoute::matches` — drops empty
/// segments entirely. Normalizing keeps `/users/` matching `/users` on the
/// tree's fast path instead of only via the fallback scan. Stranger shapes
/// (`//users`, `/a//b`) are left to that scan.
#[inline]
fn normalize_target(path: &str) -> &str {
    match path.strip_suffix('/') {
        Some("") | None => path,
        Some(stripped) => stripped,
    }
}

/// The interned parameter names a pattern declares, in pattern order.
type ParamNames = SmallVec<[&'static str; 4]>;

/// Intern a pattern's parameter names once, at index-build time.
///
/// Patterns already written in `matchit` brace syntax are not recognized here;
/// `MethodIndex::find` interns those on demand instead.
fn param_names(pattern: &str) -> ParamNames {
    split_segments(pattern)
        .filter_map(|segment| {
            segment.strip_prefix(':').or_else(|| {
                segment
                    .strip_prefix('*')
                    .map(|name| if name.is_empty() { "*" } else { name })
            })
        })
        .map(param_intern::intern)
        .collect()
}

/// The number of method-indexed trees: the routable methods of [`HttpMethod`].
const METHOD_SLOTS: usize = 8;

/// Index of `method` into the tree array.
#[inline]
fn method_slot(method: &Method) -> Option<usize> {
    Some(match method {
        Method::Get => 0,
        Method::Post => 1,
        Method::Put => 2,
        Method::Delete => 3,
        Method::Patch => 4,
        Method::Head => 5,
        Method::Options => 6,
        Method::Query => 7,
        // CONNECT, TRACE, and unknown tokens are not routable; the caller
        // answers them the same way it answers a path that does not match.
        _ => return None,
    })
}

/// Whether two patterns can both match the same path.
///
/// Deliberately over-approximate: when the answer is not obviously "no" it
/// says yes. A false yes costs one route a place in the tree and nothing else,
/// while a false no would put two overlapping routes in the same tree and let
/// `matchit`'s specificity order override registration order.
fn patterns_overlap(a: &str, b: &str) -> bool {
    fn is_param(segment: &str) -> bool {
        segment.starts_with(':') || segment.starts_with('*') || segment.starts_with('{')
    }

    let pa: SmallVec<[&str; 8]> = split_segments(a).collect();
    let pb: SmallVec<[&str; 8]> = split_segments(b).collect();

    let a_catch = pa.last().is_some_and(|s| s.starts_with('*'));
    let b_catch = pb.last().is_some_and(|s| s.starts_with('*'));

    // A catch-all absorbs any number of trailing segments; without one on
    // either side the two patterns describe paths of exactly one length.
    if !a_catch && !b_catch && pa.len() != pb.len() {
        return false;
    }

    let fixed_a = pa.len() - usize::from(a_catch);
    let fixed_b = pb.len() - usize::from(b_catch);

    // A catch-all still has to get past its own fixed prefix, so a pattern
    // shorter than that prefix cannot reach it.
    if a_catch && !b_catch && pb.len() < fixed_a {
        return false;
    }
    if b_catch && !a_catch && pa.len() < fixed_b {
        return false;
    }

    // Two static segments in the same position that differ are the only thing
    // that can rule an overlap out; a parameter on either side accepts
    // whatever the other names.
    (0..fixed_a.min(fixed_b)).all(|i| is_param(pa[i]) || is_param(pb[i]) || pa[i] == pb[i])
}

/// One `matchit` tree per routable method, plus a linear fallback.
///
/// The fallback exists because `matchit` rejects conflicting patterns while the
/// linear scan this replaces accepted them and took the first match. Silently
/// dropping the loser would change the behavior of applications that register
/// overlapping routes, so a rejected insert lands here and is scanned in
/// registration order.
struct MethodIndex {
    trees: [Option<matchit::Router<(usize, ParamNames)>>; METHOD_SLOTS],
    fallback: Vec<usize>,
}

impl MethodIndex {
    fn build(routes: &[Route]) -> Self {
        let mut trees: [Option<matchit::Router<(usize, ParamNames)>>; METHOD_SLOTS] =
            Default::default();
        let mut fallback = Vec::new();

        // What an earlier same-method route could shadow a later static path
        // with, split so the check stays linear in practice: an exact duplicate
        // is a set probe, and only genuinely parameterized patterns need a scan.
        let mut static_seen: [HashSet<String>; METHOD_SLOTS] = Default::default();
        let mut patterns: [Vec<&str>; METHOD_SLOTS] = Default::default();

        for (idx, route) in routes.iter().enumerate() {
            let method = Method::from(route.method.clone());
            let Some(slot) = method_slot(&method) else {
                fallback.push(idx);
                continue;
            };

            if route.path.contains(':') || route.path.contains('*') {
                // A parameterized pattern that overlaps an earlier one must not
                // share the tree with it. Inside a single `matchit` tree the
                // winner is decided by specificity — static beats parameter
                // beats catch-all — which is the opposite rule to this
                // framework's first-registered-wins, and the fallback rescan
                // below cannot help because it only looks at routes the tree
                // does not hold. Registering `/:x/:y` and then `/a/:z` used to
                // hand `/a/q` to the *later* route for exactly that reason.
                //
                // Sending the later one to the fallback list restores the
                // order: a path both describe hits the earlier route in the
                // tree and stops there, while a path only the later one
                // describes misses the tree and reaches it through the full
                // scan. Unlike the static case below it cannot simply be
                // dropped — it is still the only route for everything the
                // earlier pattern does not cover.
                let overlapped = patterns[slot]
                    .iter()
                    .copied()
                    .any(|earlier| patterns_overlap(earlier, &route.path));
                patterns[slot].push(&route.path);
                if overlapped {
                    fallback.push(idx);
                    continue;
                }
            } else {
                // A static pattern names exactly one concrete path, so an
                // earlier route matching that path makes this one unreachable
                // under first-registered-wins. Inserting it anyway would hand
                // it the match: `matchit` prefers a static segment over a
                // parameter, where this framework prefers the earlier
                // registration. Dropping it keeps the two agreeing.
                let parts: SmallVec<[&str; 8]> = split_segments(&route.path).collect();
                let duplicate = !static_seen[slot].insert(parts.join("/"));
                let shadowed = duplicate
                    || patterns[slot]
                        .iter()
                        .copied()
                        .any(|earlier| match_path_spans(earlier, &parts).is_some());
                if shadowed {
                    continue;
                }
            }

            // `matchit` renames parameters to single letters starting at `a`
            // while normalizing, and *panics* — it does not return an error —
            // once a route needs one past `z`. A panic is not something the
            // `is_err` arm below can turn into a fallback, so the count is
            // checked first. Such a route is pathological, but registering one
            // should cost a linear scan, not abort the process.
            //
            // Counted on the *translated* pattern rather than on
            // `param_names`, which sees only this crate's `:name` and `*name`
            // spellings. A pattern written with braces directly is passed
            // through untouched, so it is a parameter to `matchit` while being
            // invisible here — and 26 of those panic just the same.
            let translated = translate_pattern(&route.path);
            let tree = trees[slot].get_or_insert_with(matchit::Router::new);
            if translated.bytes().filter(|&b| b == b'{').count() > MATCHIT_MAX_PARAMS {
                fallback.push(idx);
                continue;
            }

            let value = (idx, param_names(&route.path));
            if tree.insert(translated, value).is_err() {
                // A conflict, an unsupported pattern, or a duplicate. Preserve
                // first-registered-wins by scanning instead.
                fallback.push(idx);
            }
        }

        Self { trees, fallback }
    }

    /// The lowest-registration-index route matching `method` and `path`.
    fn find(&self, routes: &[Route], method: &Method, path: &str) -> Option<(usize, RouteParams)> {
        let mut best: Option<(usize, RouteParams)> = None;

        if let Some(slot) = method_slot(method)
            && let Some(tree) = self.trees[slot].as_ref()
            && let Ok(m) = tree.at(normalize_target(path))
        {
            let (idx, names) = m.value;
            let mut params = RouteParams::new();
            for (name, value) in m.params.iter() {
                // The names were interned when this very pattern was indexed,
                // so the lookup hits and no process-global lock is taken on the
                // request path. Brace-syntax patterns are the exception
                // `param_names` does not cover; they intern here instead.
                let interned = names
                    .iter()
                    .copied()
                    .find(|candidate| *candidate == name)
                    .unwrap_or_else(|| param_intern::intern(name));
                params.push((interned, Bytes::copy_from_slice(value.as_bytes())));
            }
            best = Some((*idx, params));
        }

        let parts: SmallVec<[&str; 8]> = split_segments(path).collect();
        match best.as_ref().map(|(idx, _)| *idx) {
            // A fallback route registered earlier than the tree match has to
            // win, or adding a conflicting route later would silently change
            // which handler serves an existing path.
            Some(idx) => {
                if let Some(earlier) = Self::scan(
                    routes,
                    method,
                    &parts,
                    self.fallback.iter().copied(),
                    Some(idx),
                ) {
                    best = Some(earlier);
                }
            }
            // A tree miss is not a 404 on its own: `matchit` requires a
            // catch-all to consume at least one segment, and it does not see
            // targets whose empty segments only `split_segments` removes. The
            // segment matcher — and `OptimizedRouter`, which must agree with it
            // — accepts both, so widen the scan to every route of this method
            // before giving up.
            None => best = Self::scan(routes, method, &parts, 0..routes.len(), None),
        }

        best
    }

    /// The first of `candidates` (in registration order) matching `method` and
    /// the pre-split `parts`, stopping once `limit` is passed.
    fn scan(
        routes: &[Route],
        method: &Method,
        parts: &[&str],
        candidates: impl Iterator<Item = usize>,
        limit: Option<usize>,
    ) -> Option<(usize, RouteParams)> {
        for idx in candidates {
            if limit.is_some_and(|best| best < idx) {
                break;
            }
            let route = &routes[idx];
            if Method::from(route.method.clone()) != *method {
                continue;
            }
            if let Some(params) = match_path_spans(&route.path, parts) {
                return Some((idx, params));
            }
        }
        None
    }
}

/// Match `pattern` against pre-split `path_parts`, capturing spans.
///
/// The fallback matcher, used only for routes `matchit` would not accept. The
/// tree path does not come through here.
fn match_path_spans(pattern: &str, path_parts: &[&str]) -> Option<RouteParams> {
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

    // Second pass: the route matched, so collect the captures now.
    //
    // Names are interned from the pattern, which is a fixed set decided at
    // route-registration time, so the table cannot grow with traffic. It does
    // mean a lock per captured parameter, which is why this stays off the tree
    // fast path: both `MethodIndex` and `CompiledRoute` intern at build time.
    let mut params = RouteParams::new();
    if param_count > 0 {
        for (i, pattern_part) in split_segments(pattern).enumerate() {
            if let Some(name) = pattern_part.strip_prefix('*') {
                // Catch-all: join all remaining path segments, matching
                // `CompiledRoute::extract_params`'s joining convention.
                let name = if name.is_empty() { "*" } else { name };
                params.push((
                    param_intern::intern(name),
                    Bytes::from(path_parts[i..].join("/")),
                ));
                break;
            } else if let Some(param_name) = pattern_part.strip_prefix(':') {
                params.push((
                    param_intern::intern(param_name),
                    Bytes::copy_from_slice(path_parts[i].as_bytes()),
                ));
            }
        }
    }

    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RouteParamsExt;
    use bytes::Bytes;

    // Test helper handler
    async fn test_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    // Test helper: split a raw path and run match_path, mirroring how the
    // router pre-splits the request path before the route loop.
    fn match_path_str(pattern: &str, path: &str) -> Option<RouteParams> {
        let parts: Vec<&str> = super::split_segments(path).collect();
        match_path_spans(pattern, &parts)
    }

    #[test]
    fn translates_armature_patterns_to_matchit_syntax() {
        assert_eq!(translate_pattern("/users"), "/users");
        assert_eq!(translate_pattern("/users/:id"), "/users/{id}");
        assert_eq!(
            translate_pattern("/users/:user_id/posts/:post_id"),
            "/users/{user_id}/posts/{post_id}"
        );
        assert_eq!(translate_pattern("/files/*path"), "/files/{*path}");
        assert_eq!(
            translate_pattern("/users/:id/files/*path"),
            "/users/{id}/files/{*path}"
        );
        // Already-braced patterns pass through, so a user who wrote matchit
        // syntax directly is not broken by the translation.
        assert_eq!(translate_pattern("/users/{id}"), "/users/{id}");
    }

    #[tokio::test]
    async fn dispatches_by_method_without_comparing_strings() {
        let mut router = Router::new();
        router.get("/r", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        router.post("/r", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(201))
        });

        let get = router.route(HttpRequest::new("GET", "/r")).await.unwrap();
        let post = router.route(HttpRequest::new("POST", "/r")).await.unwrap();
        assert_eq!(get.status, 200);
        assert_eq!(post.status, 201);
    }

    #[tokio::test]
    async fn captures_params_as_spans_of_the_target() {
        let mut router = Router::new();
        router.get("/users/:id/posts/:post", |req: HttpRequest| async move {
            let mut r = HttpResponse::new(200);
            r.body = Bytes::from(format!(
                "{}:{}",
                req.param("id").unwrap_or("-"),
                req.param("post").unwrap_or("-")
            ));
            Ok::<_, Error>(r)
        });

        let resp = router
            .route(HttpRequest::new("GET", "/users/42/posts/7"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"42:7");
    }

    #[tokio::test]
    async fn catch_all_still_matches() {
        let mut router = Router::new();
        router.get("/files/*path", |req: HttpRequest| async move {
            let mut r = HttpResponse::new(200);
            r.body = Bytes::from(req.param("path").unwrap_or("-").to_owned());
            Ok::<_, Error>(r)
        });
        let resp = router
            .route(HttpRequest::new("GET", "/files/a/b/c.txt"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"a/b/c.txt");
    }

    #[tokio::test]
    async fn a_query_string_does_not_participate_in_matching() {
        let mut router = Router::new();
        router.get("/s", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        let resp = router
            .route(HttpRequest::new("GET", "/s?a=1&b=2"))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn duplicate_routes_keep_first_registered_wins() {
        // matchit rejects conflicting inserts; the old linear scan accepted them
        // and took the first. Existing applications rely on that, so a conflict
        // must fall back to a scan rather than panicking or reordering.
        let mut router = Router::new();
        router.get("/dup", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        router.get("/dup", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(500))
        });
        let resp = router.route(HttpRequest::new("GET", "/dup")).await.unwrap();
        assert_eq!(resp.status, 200, "the first registration must win");
    }

    #[tokio::test]
    async fn routes_added_after_the_first_dispatch_are_visible() {
        let mut router = Router::new();
        router.get("/a", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        assert_eq!(
            router
                .route(HttpRequest::new("GET", "/a"))
                .await
                .unwrap()
                .status,
            200
        );

        // The index is built lazily; adding a route has to invalidate it.
        router.get("/b", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(201))
        });
        assert_eq!(
            router
                .route(HttpRequest::new("GET", "/b"))
                .await
                .unwrap()
                .status,
            201
        );
    }

    /// Registration order beats `matchit`'s static-over-parameter preference,
    /// matching both the scan this replaced and `OptimizedRouter`.
    #[tokio::test]
    async fn an_earlier_param_route_shadows_a_later_static_one() {
        let mut router = Router::new();
        router.get("/users/:id", |req: HttpRequest| async move {
            let id = req.param("id").unwrap_or("-").to_owned();
            Ok::<_, Error>(HttpResponse::ok().with_body(format!("param:{id}").into_bytes()))
        });
        router.get("/users/me", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::ok().with_body(b"static".to_vec()))
        });

        let resp = router
            .route(HttpRequest::new("GET", "/users/me"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"param:me");
    }

    /// The other order is unambiguous: the static route is both earlier and the
    /// one `matchit` would pick.
    #[tokio::test]
    async fn an_earlier_static_route_wins_over_a_later_param_one() {
        let mut router = Router::new();
        router.get("/users/me", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::ok().with_body(b"static".to_vec()))
        });
        router.get("/users/:id", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::ok().with_body(b"param".to_vec()))
        });

        let resp = router
            .route(HttpRequest::new("GET", "/users/me"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"static");
    }

    #[tokio::test]
    async fn an_unroutable_method_is_not_a_panic() {
        let mut router = Router::new();
        router.get("/a", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        // An `Other` method has no tree; it must reach the no-match branch
        // rather than unwrapping a missing one.
        let resp = router.route(HttpRequest::new("PURGE", "/a")).await;
        assert!(matches!(resp, Err(Error::RouteNotFound(_))));
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
        assert_eq!(params.get_str("id"), Some("123"));
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
        assert_eq!(params.get_str("user_id"), Some("123"));
        assert_eq!(params.get_str("post_id"), Some("456"));
    }

    #[test]
    fn test_match_path_trailing_slash() {
        // Empty segments are dropped, so a trailing slash is not significant.
        let result = match_path_str("/users", "/users/");
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 0);

        let result = match_path_str("/users/:id", "/users/42/");
        assert_eq!(result.unwrap().get_str("id"), Some("42"));
    }

    /// `matchit` treats a trailing slash as significant; the segment matcher
    /// this index sits in front of does not, and `OptimizedRouter` does not
    /// either. Dispatch has to follow the segment matcher.
    #[tokio::test]
    async fn a_trailing_slash_does_not_change_the_match() {
        let mut router = Router::new();
        router.get("/users", |_req: HttpRequest| async {
            Ok::<_, Error>(HttpResponse::new(200))
        });
        router.get("/users/:id", |req: HttpRequest| async move {
            let id = req.param("id").unwrap_or("-").to_owned();
            Ok::<_, Error>(HttpResponse::ok().with_body(id.into_bytes()))
        });

        assert_eq!(
            router
                .route(HttpRequest::new("GET", "/users/"))
                .await
                .unwrap()
                .status,
            200
        );
        let resp = router
            .route(HttpRequest::new("GET", "/users/42/"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"42");
    }

    /// `matchit` requires a catch-all to consume at least one segment, but the
    /// documented contract — and `CompiledRoute::matches` — is zero or more.
    #[tokio::test]
    async fn a_catch_all_matches_zero_remaining_segments() {
        let mut router = Router::new();
        router.get("/files/*path", |req: HttpRequest| async move {
            let mut r = HttpResponse::new(200);
            r.body = Bytes::from(format!("[{}]", req.param("path").unwrap_or("-")));
            Ok::<_, Error>(r)
        });

        for target in ["/files", "/files/"] {
            let resp = router
                .route(HttpRequest::new("GET", target))
                .await
                .unwrap_or_else(|_| panic!("{target} should match /files/*path"));
            assert_eq!(resp.body_slice(), b"[]", "{target}");
        }

        // A path that never reaches the static prefix still 404s.
        assert!(
            router
                .route(HttpRequest::new("GET", "/other"))
                .await
                .is_err()
        );
    }

    #[test]
    fn test_match_path_nested() {
        let pattern = "/api/v1/users/:id";
        let path = "/api/v1/users/123";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get_str("id"), Some("123"));
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
        assert_eq!(params.get_str("path"), Some("docs/readme.md"));

        // An exact prefix match (no trailing segments) should still match,
        // with the catch-all param extracted as an empty string.
        let result = match_path_str(pattern, "/files");
        assert!(result.is_some());
        assert_eq!(result.unwrap().get_str("path"), Some(""));

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
        assert_eq!(params.get_str("id"), Some("42"));
        assert_eq!(params.get_str("path"), Some("a/b"));

        // A bare, unnamed catch-all (`*` with no name) stores its captured
        // value under the literal key `"*"` (see the `name.is_empty()`
        // fallback in `match_path`).
        let pattern = "/files/*";
        let result = match_path_str(pattern, "/files/a/b/c");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get_str("*"), Some("a/b/c"));
    }

    #[tokio::test]
    async fn test_router_route_catch_all() {
        // Mirrors route_cache.rs's `test_from_router_catch_all`, but calls
        // `Router::route` directly instead of going through
        // `OptimizedRouter`, to make sure the linear router's own catch-all
        // handling (not just the compiled fast path) works end to end.
        async fn echo_path(req: HttpRequest) -> Result<HttpResponse, Error> {
            let p = req.param("path").map(str::to_owned).unwrap_or_default();
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
        assert_eq!(params.get_str("path"), Some("docs/readme.md"));
    }

    #[test]
    fn test_match_path_param_with_special_chars() {
        let pattern = "/users/:id";
        let path = "/users/abc-123";
        let result = match_path_str(pattern, path);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get_str("id"), Some("abc-123"));
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

    /// Registration order decides, even when the later route is the one
    /// `matchit` would prefer.
    ///
    /// Inside a single tree the winner is chosen by specificity — static beats
    /// parameter beats catch-all — which is the opposite of this framework's
    /// rule. `/a/q` is described by both patterns here, and `matchit` picks the
    /// second because its first segment is static, so keeping both in the tree
    /// silently hands the path to the later route.
    #[tokio::test]
    async fn an_earlier_pattern_beats_a_later_more_specific_one() {
        async fn first(_req: HttpRequest) -> Result<HttpResponse, Error> {
            Ok(HttpResponse::ok().with_body(b"first".to_vec()))
        }
        async fn second(_req: HttpRequest) -> Result<HttpResponse, Error> {
            Ok(HttpResponse::ok().with_body(b"second".to_vec()))
        }

        let mut router = Router::new();
        router.add_route(Route::new(HttpMethod::GET, "/:x/:y", first));
        router.add_route(Route::new(HttpMethod::GET, "/a/:z", second));

        let response = router
            .route(HttpRequest::new("GET", "/a/q".to_string()))
            .await
            .expect("a path both patterns describe must route");
        assert_eq!(
            response.body.as_ref(),
            b"first",
            "the later, more specific route answered a path the earlier one already described"
        );

        // The later route is not merely shadowed away: it still owns everything
        // the earlier pattern does not describe.
        let response = router
            .route(HttpRequest::new("GET", "/a/b/c".to_string()))
            .await;
        assert!(
            response.is_err(),
            "neither pattern describes a three-segment path"
        );
    }

    #[test]
    fn overlap_detection_only_rules_out_what_it_can_prove_disjoint() {
        // Same shape, a parameter facing anything: overlapping.
        assert!(patterns_overlap("/:x/:y", "/a/:z"));
        assert!(patterns_overlap("/a/:z", "/:x/:y"));
        assert!(patterns_overlap("/:x", "/a"));

        // Two static segments that differ in the same position: disjoint, and
        // this is the only thing that proves it.
        assert!(!patterns_overlap("/a/:z", "/b/:z"));

        // Different lengths with no catch-all to absorb the difference.
        assert!(!patterns_overlap("/:x/:y", "/:x"));

        // A catch-all reaches any depth at or past its fixed prefix.
        assert!(patterns_overlap("/a/*rest", "/a/:x/:y"));
        assert!(!patterns_overlap("/a/b/*rest", "/a"));
    }

    /// `matchit` panics rather than erroring once a route needs a 27th
    /// parameter letter, and a panic during index construction takes down
    /// whatever was registering the route. Such a route has to fall back to the
    /// linear scan, and — the part worth asserting — still match.
    #[tokio::test]
    async fn a_route_past_matchits_parameter_ceiling_still_matches() {
        async fn last_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
            let v = req.param("p29").unwrap_or_default().to_owned();
            Ok(HttpResponse::ok().with_body(v.into_bytes()))
        }

        // 30 parameters: comfortably past the 25 `matchit` can normalize.
        let pattern: String = (0..30).map(|i| format!("/:p{i}")).collect();
        let target: String = (0..30).map(|i| format!("/v{i}")).collect();

        let mut router = Router::new();
        router.get(&pattern, last_handler);

        let response = router
            .route(HttpRequest::new("GET", target))
            .await
            .expect("an over-parameterized route must route, not panic");
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body.as_ref(),
            b"v29",
            "the fallback scan must still bind parameters"
        );
    }

    /// The ceiling itself, rather than a value comfortably past it. The other
    /// tests use 30 parameters, which would still pass if the constant were off
    /// by a few; only routes at exactly 25 and exactly 26 pin it down. Both must
    /// match — 25 through the tree, 26 through the linear scan — and neither may
    /// panic.
    #[tokio::test]
    async fn routes_on_either_side_of_the_parameter_ceiling_both_match() {
        async fn last_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
            // The last parameter differs between the two patterns, and the
            // 26-parameter one carries both names — so the longer route's
            // parameter has to be checked first.
            let v = req
                .param("p25")
                .or_else(|| req.param("p24"))
                .unwrap_or_default()
                .to_owned();
            Ok(HttpResponse::ok().with_body(v.into_bytes()))
        }

        for count in [MATCHIT_MAX_PARAMS, MATCHIT_MAX_PARAMS + 1] {
            let pattern: String = (0..count).map(|i| format!("/:p{i}")).collect();
            let target: String = (0..count).map(|i| format!("/v{i}")).collect();

            let mut router = Router::new();
            router.get(&pattern, last_handler);

            let response = router
                .route(HttpRequest::new("GET", target))
                .await
                .unwrap_or_else(|_| panic!("a {count}-parameter route must match"));
            assert_eq!(response.status, 200);
            assert_eq!(
                response.body.as_ref(),
                format!("v{}", count - 1).as_bytes(),
                "a {count}-parameter route must still bind its last parameter"
            );
        }
    }

    /// The same ceiling, reached through brace syntax instead of `:name`.
    /// `translate_pattern` passes an already-braced segment through untouched,
    /// so those parameters never appear in `param_names` — counting there
    /// rather than on the translated pattern leaves the panic reachable.
    ///
    /// Only the absence of a panic is asserted. Such a route is answered by the
    /// linear scan, and `match_path_spans` understands `:name` and `*name` but
    /// not braces, so it does not match — brace spelling has only ever worked
    /// by reaching `matchit`. Turning a process-killing panic into a route that
    /// does not match is the fix; making brace syntax work in the fallback is a
    /// separate question about whether it is a supported spelling at all.
    #[tokio::test]
    async fn brace_spelled_parameters_do_not_panic_past_the_ceiling() {
        let pattern: String = (0..30).map(|i| format!("/{{p{i}}}")).collect();
        let target: String = (0..30).map(|i| format!("/v{i}")).collect();

        let mut router = Router::new();
        router.get(&pattern, test_handler);

        // `route` returning `Err(RouteNotFound)` is a result; a panic is not.
        let _ = router.route(HttpRequest::new("GET", target)).await;
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
