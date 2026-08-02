//! Route Matching Cache and Static Route Optimization
//!
//! This module provides optimizations for route matching:
//!
//! - **Route Cache**: LRU cache for recently matched routes
//! - **Static Fast Path**: O(1) HashMap lookup for static routes
//! - **Compiled Routes**: Pre-analyzed route patterns
//!
//! # Performance
//!
//! - Cache hit: ~5ns (vs ~50-150ns for pattern matching)
//! - Static lookup: ~10ns (vs ~50ns+ for trie traversal)
//! - Cache miss: Falls back to normal matching + caches result
//!
//! # Usage
//!
//! ```rust,ignore
//! use armature_core::route_cache::{CachedRouter, StaticRoutes};
//!
//! // Create cached router
//! let mut router = CachedRouter::new();
//! router.add_static("/api/health", handler);
//! router.add_pattern("/users/:id", handler);
//!
//! // Routing uses optimized paths automatically
//! let response = router.route(request).await?;
//! ```

use crate::handler::BoxedHandler;
use crate::route_constraint::RouteConstraints;
use crate::routing::Router;
use crate::{Error, HttpMethod, HttpRequest, HttpResponse};
use lru::LruCache;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Cache Key
// ============================================================================

/// A cache key combining method and path.
#[derive(Clone, Debug, Eq)]
pub struct RouteKey {
    /// HTTP method
    method: HttpMethod,
    /// Request path (without query string)
    path: String,
}

impl RouteKey {
    /// Create a new route key.
    #[inline]
    pub fn new(method: HttpMethod, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
        }
    }

    /// Create from request.
    ///
    /// Returns `None` if the request method is not a known HTTP method, so
    /// unknown methods are never silently treated as GET.
    #[inline]
    pub fn from_request(req: &HttpRequest) -> Option<Self> {
        let path = req
            .path
            .split_once('?')
            .map(|(p, _)| p)
            .unwrap_or(&req.path);

        Some(Self {
            method: HttpMethod::try_from(&req.method).ok()?,
            path: path.to_string(),
        })
    }
}

impl PartialEq for RouteKey {
    fn eq(&self, other: &Self) -> bool {
        self.method == other.method && self.path == other.path
    }
}

impl Hash for RouteKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.method.as_str().hash(state);
        self.path.hash(state);
    }
}

// ============================================================================
// Cache Entry
// ============================================================================

/// A cached route match result.
#[derive(Clone)]
pub struct CachedRoute {
    /// Index into the route table
    pub route_index: usize,
    /// Pre-extracted path parameters (name, segment_index)
    pub param_indices: Vec<(String, usize)>,
    /// Is this a static route (no params)?
    pub is_static: bool,
    /// Segment index of a catch-all parameter, if any.
    ///
    /// The catch-all parameter captures all remaining segments joined with
    /// `/`, mirroring `CompiledRoute::extract_params`.
    pub catch_all_index: Option<usize>,
}

impl CachedRoute {
    /// Create a cached entry for a static route.
    pub fn static_route(route_index: usize) -> Self {
        Self {
            route_index,
            param_indices: Vec::new(),
            is_static: true,
            catch_all_index: None,
        }
    }

    /// Create a cached entry for a parameterized route.
    pub fn with_params(route_index: usize, param_indices: Vec<(String, usize)>) -> Self {
        Self {
            route_index,
            param_indices,
            is_static: false,
            catch_all_index: None,
        }
    }

    /// Create a cached entry from a compiled route pattern.
    pub fn from_compiled(route_index: usize, compiled: &CompiledRoute) -> Self {
        Self {
            route_index,
            param_indices: compiled.param_indices.clone(),
            is_static: compiled.is_static,
            catch_all_index: compiled
                .has_catch_all
                .then(|| compiled.segments.len().saturating_sub(1)),
        }
    }

    /// Extract parameters from a path using cached indices.
    #[inline]
    pub fn extract_params(&self, path: &str) -> HashMap<String, String> {
        if self.is_static {
            return HashMap::new();
        }

        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut params = HashMap::with_capacity(self.param_indices.len());

        for (name, idx) in &self.param_indices {
            if self.catch_all_index == Some(*idx) {
                // Catch-all: join all remaining segments
                if let Some(rest) = segments.get(*idx..) {
                    params.insert(name.clone(), rest.join("/"));
                }
            } else if let Some(value) = segments.get(*idx) {
                params.insert(name.clone(), (*value).to_string());
            }
        }

        params
    }
}

// ============================================================================
// LRU Route Cache
// ============================================================================

/// LRU cache for route matching results.
///
/// Backed by [`lru::LruCache`], which tracks true access-recency order via an
/// intrusive doubly-linked list: `get` promotes the accessed entry to
/// most-recently-used, and eviction on `insert` always removes the actual
/// least-recently-used entry — never an arbitrary one.
///
/// Thread-safe with interior mutability via a `Mutex`. A lookup mutates
/// recency order, so — unlike a plain read-through cache — there is no
/// benefit to a `RwLock`'s shared-read mode here; every operation needs
/// exclusive access.
pub struct RouteCache {
    /// Cached routes by key, in LRU order.
    cache: Mutex<LruCache<RouteKey, CachedRoute>>,
    /// Statistics
    stats: RouteCacheStats,
}

impl RouteCache {
    /// Create new cache with default size (1024 entries).
    pub fn new() -> Self {
        Self::with_capacity(1024)
    }

    /// Create cache with specific capacity.
    ///
    /// `lru::LruCache` requires a non-zero capacity; a caller-requested `0`
    /// is clamped to the smallest usable capacity of 1 rather than panicking.
    pub fn with_capacity(max_size: usize) -> Self {
        let capacity = NonZeroUsize::new(max_size).unwrap_or(NonZeroUsize::MIN);
        Self {
            cache: Mutex::new(LruCache::new(capacity)),
            stats: RouteCacheStats::default(),
        }
    }

    /// Get a cached route.
    ///
    /// A true LRU lookup: on a hit, the entry is promoted to
    /// most-recently-used, so it survives future evictions longer than
    /// entries that haven't been accessed recently.
    #[inline]
    pub fn get(&self, key: &RouteKey) -> Option<CachedRoute> {
        let mut cache = self.cache.lock();
        let result = cache.get(key).cloned();

        if result.is_some() {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
        }

        result
    }

    /// Insert a route into the cache.
    ///
    /// When the cache is already full and `key` is a new entry, the true
    /// least-recently-used entry (by access order) is evicted to make room.
    pub fn insert(&self, key: RouteKey, route: CachedRoute) {
        let mut cache = self.cache.lock();

        let len_before = cache.len();
        let was_present = cache.peek(&key).is_some();

        cache.put(key, route);

        if !was_present && len_before >= cache.cap().get() {
            self.stats.evictions.fetch_add(1, Ordering::Relaxed);
        }

        self.stats.insertions.fetch_add(1, Ordering::Relaxed);
    }

    /// Clear the cache.
    pub fn clear(&self) {
        self.cache.lock().clear();
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &RouteCacheStats {
        &self.stats
    }

    /// Get current cache size.
    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for RouteCache {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Static Route Fast Path
// ============================================================================

/// Fast path for static routes using HashMap.
///
/// Static routes (no parameters) can be matched in O(1) time using
/// a direct HashMap lookup, bypassing pattern matching entirely.
pub struct StaticRoutes {
    /// Static routes by (method, path)
    routes: HashMap<RouteKey, usize>,
    /// Statistics
    stats: StaticRouteStats,
}

impl StaticRoutes {
    /// Create new static route store.
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            stats: StaticRouteStats::default(),
        }
    }

    /// Add a static route.
    pub fn add(&mut self, method: HttpMethod, path: impl Into<String>, route_index: usize) {
        let key = RouteKey::new(method, path);
        self.routes.insert(key, route_index);
    }

    /// Look up a static route.
    #[inline]
    pub fn get(&self, key: &RouteKey) -> Option<usize> {
        let result = self.routes.get(key).copied();

        if result.is_some() {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
        }

        result
    }

    /// Check if path is static (no parameter segments).
    pub fn is_static_path(path: &str) -> bool {
        !path.contains(':') && !path.contains('*')
    }

    /// Get statistics.
    pub fn stats(&self) -> &StaticRouteStats {
        &self.stats
    }

    /// Get number of static routes.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

impl Default for StaticRoutes {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Compiled Route Pattern
// ============================================================================

/// Pre-compiled route pattern for fast matching.
#[derive(Clone, Debug)]
pub struct CompiledRoute {
    /// Original pattern
    pub pattern: String,
    /// Parsed segments
    pub segments: Vec<RouteSegment>,
    /// Parameter indices (name, segment_index)
    pub param_indices: Vec<(String, usize)>,
    /// Is this a static route?
    pub is_static: bool,
    /// Has catch-all segment?
    pub has_catch_all: bool,
}

/// A segment in a route pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteSegment {
    /// Static segment (exact match)
    Static(String),
    /// Named parameter (:name)
    Param(String),
    /// Catch-all (*path)
    CatchAll(String),
}

impl CompiledRoute {
    /// Compile a route pattern.
    pub fn compile(pattern: &str) -> Self {
        let mut segments = Vec::new();
        let mut param_indices = Vec::new();
        let mut is_static = true;
        let mut has_catch_all = false;

        for (idx, part) in pattern.split('/').filter(|s| !s.is_empty()).enumerate() {
            if let Some(name) = part.strip_prefix(':') {
                segments.push(RouteSegment::Param(name.to_string()));
                param_indices.push((name.to_string(), idx));
                is_static = false;
            } else if let Some(name) = part.strip_prefix('*') {
                let name = if name.is_empty() { "*" } else { name };
                segments.push(RouteSegment::CatchAll(name.to_string()));
                param_indices.push((name.to_string(), idx));
                is_static = false;
                has_catch_all = true;
            } else {
                segments.push(RouteSegment::Static(part.to_string()));
            }
        }

        Self {
            pattern: pattern.to_string(),
            segments,
            param_indices,
            is_static,
            has_catch_all,
        }
    }

    /// Check if a path matches this pattern.
    #[inline]
    pub fn matches(&self, path: &str) -> bool {
        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if !self.has_catch_all && path_segments.len() != self.segments.len() {
            return false;
        }

        if self.has_catch_all && path_segments.len() < self.segments.len() - 1 {
            return false;
        }

        for (idx, segment) in self.segments.iter().enumerate() {
            match segment {
                RouteSegment::Static(s) => {
                    if path_segments.get(idx) != Some(&s.as_str()) {
                        return false;
                    }
                }
                RouteSegment::Param(_) => {
                    if idx >= path_segments.len() {
                        return false;
                    }
                }
                RouteSegment::CatchAll(_) => {
                    // Matches remaining segments
                    break;
                }
            }
        }

        true
    }

    /// Extract parameters from a matching path.
    pub fn extract_params(&self, path: &str) -> HashMap<String, String> {
        if self.is_static {
            return HashMap::new();
        }

        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut params = HashMap::with_capacity(self.param_indices.len());

        for (name, idx) in &self.param_indices {
            if let Some(segment) = self.segments.get(*idx) {
                match segment {
                    RouteSegment::Param(_) => {
                        if let Some(value) = path_segments.get(*idx) {
                            params.insert(name.clone(), (*value).to_string());
                        }
                    }
                    RouteSegment::CatchAll(_) => {
                        // Join remaining segments
                        let remaining: String = path_segments[*idx..].join("/");
                        params.insert(name.clone(), remaining);
                    }
                    _ => {}
                }
            }
        }

        params
    }
}

// ============================================================================
// Optimized Router
// ============================================================================

/// Router entry with compiled pattern.
pub struct OptimizedRoute {
    /// HTTP method
    pub method: HttpMethod,
    /// Compiled pattern
    pub compiled: CompiledRoute,
    /// Handler
    pub handler: BoxedHandler,
    /// Optional route constraints for parameter validation.
    ///
    /// Carried alongside the compiled pattern so the optimized dispatch can
    /// validate a matched route's parameters exactly as the linear
    /// [`Router`](crate::routing::Router) does, returning the same
    /// `Error::BadRequest` on failure.
    pub constraints: Option<RouteConstraints>,
}

/// Optimized router with caching and static fast path.
pub struct OptimizedRouter {
    /// All routes
    routes: Vec<OptimizedRoute>,
    /// Static route fast path
    static_routes: StaticRoutes,
    /// Route cache
    cache: RouteCache,
    /// Statistics
    stats: RouterStats,
}

impl OptimizedRouter {
    /// Create new optimized router.
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            static_routes: StaticRoutes::new(),
            cache: RouteCache::new(),
            stats: RouterStats::default(),
        }
    }

    /// Create with specific cache size.
    pub fn with_cache_size(cache_size: usize) -> Self {
        Self {
            routes: Vec::new(),
            static_routes: StaticRoutes::new(),
            cache: RouteCache::with_capacity(cache_size),
            stats: RouterStats::default(),
        }
    }

    /// Add a route.
    pub fn add_route(
        &mut self,
        method: HttpMethod,
        path: impl Into<String>,
        handler: BoxedHandler,
    ) {
        let path = path.into();
        let compiled = CompiledRoute::compile(&path);
        let route_index = self.routes.len();

        // Add to static routes if applicable
        if compiled.is_static {
            self.static_routes.add(method.clone(), &path, route_index);
        }

        self.routes.push(OptimizedRoute {
            method,
            compiled,
            handler,
            constraints: None,
        });
    }

    /// Build an `OptimizedRouter` from a fully-populated linear [`Router`].
    ///
    /// Every [`Route`](crate::routing::Route) — method, path, handler, and
    /// optional constraints — is ingested into the optimized structures so the
    /// serve path can dispatch in O(1) for the common case while preserving the
    /// linear router's exact semantics:
    ///
    /// * Routes keep their registration order in `routes`, and the pattern-match
    ///   fallback returns the **first** matching route in that order, matching
    ///   [`Router::route`](crate::routing::Router::route).
    /// * A static route is only added to the O(1) static fast path if no
    ///   **earlier** route (of the same method) also matches that concrete path.
    ///   This preserves first-registered-wins precedence: an earlier `:param` or
    ///   `*catch_all` route that would shadow a later static path is still
    ///   selected by the fallback scan, never bypassed by the HashMap. Duplicate
    ///   static registrations are likewise resolved to the first one.
    /// * Constraints travel with each route and are validated after the match.
    pub fn from_router(router: &Router) -> Self {
        let mut opt = Self::new();

        for route in &router.routes {
            let compiled = CompiledRoute::compile(&route.path);
            let route_index = opt.routes.len();

            // Only take the O(1) static fast path when no earlier route shadows
            // this concrete path. `matches` handles static, `:param`, and
            // `*catch_all` earlier routes, so precedence is identical to the
            // linear first-match scan.
            if compiled.is_static {
                let shadowed_by_earlier = opt.routes.iter().any(|earlier| {
                    earlier.method == route.method && earlier.compiled.matches(&route.path)
                });
                if !shadowed_by_earlier {
                    opt.static_routes
                        .add(route.method.clone(), &route.path, route_index);
                }
            }

            opt.routes.push(OptimizedRoute {
                method: route.method.clone(),
                compiled,
                handler: route.handler.clone(),
                constraints: route.constraints.clone(),
            });
        }

        opt
    }

    /// Route a request with optimized matching.
    pub async fn route(&self, mut request: HttpRequest) -> Result<HttpResponse, Error> {
        self.stats.requests.fetch_add(1, Ordering::Relaxed);

        // Match on the path alone; the query is parsed on demand by
        // `HttpRequest::query`, and only if a handler asks for it.
        let path = request.path_only();

        // Unknown HTTP methods must not fall back to GET handlers.
        let Ok(method) = HttpMethod::try_from(&request.method) else {
            return Err(Error::RouteNotFound(format!("{} {}", request.method, path)));
        };

        let key = RouteKey::new(method.clone(), path);

        // 1. Try static route fast path (O(1)). Static routes carry no path
        //    params; any constraints validate against the empty map (matching
        //    the linear router, which only checks params that are present).
        if let Some(route_index) = self.static_routes.get(&key) {
            self.stats.static_hits.fetch_add(1, Ordering::Relaxed);
            let route = &self.routes[route_index];
            if let Some(constraints) = &route.constraints {
                constraints.validate(&request.path_params)?;
            }
            return route.handler.call(request).await;
        }

        // 2. Try cache (O(1)).
        if let Some(cached) = self.cache.get(&key) {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            let route = &self.routes[cached.route_index];
            let params = cached.extract_params(path);
            if let Some(constraints) = &route.constraints {
                constraints.validate(&params)?;
            }
            request.path_params = params;
            return route.handler.call(request).await;
        }

        // 3. Fall back to pattern matching. Returns the first matching route in
        //    registration order, mirroring the linear `Router::route`.
        self.stats.pattern_matches.fetch_add(1, Ordering::Relaxed);

        for (route_index, route) in self.routes.iter().enumerate() {
            if route.method != method {
                continue;
            }

            if route.compiled.matches(path) {
                // Cache the match for future requests
                let cached = CachedRoute::from_compiled(route_index, &route.compiled);
                self.cache.insert(key, cached);

                // Extract params, validate constraints, then call handler.
                let params = route.compiled.extract_params(path);
                if let Some(constraints) = &route.constraints {
                    constraints.validate(&params)?;
                }
                request.path_params = params;
                return route.handler.call(request).await;
            }
        }

        Err(Error::RouteNotFound(format!("{} {}", request.method, path)))
    }

    /// Get statistics.
    pub fn stats(&self) -> &RouterStats {
        &self.stats
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> &RouteCacheStats {
        self.cache.stats()
    }

    /// Get static route statistics.
    pub fn static_stats(&self) -> &StaticRouteStats {
        self.static_routes.stats()
    }

    /// Clear the route cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get number of routes.
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Check if router is empty.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

impl Default for OptimizedRouter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Route cache statistics.
#[derive(Debug, Default)]
pub struct RouteCacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    insertions: AtomicU64,
    evictions: AtomicU64,
}

impl RouteCacheStats {
    /// Get cache hits.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get cache misses.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get insertions.
    pub fn insertions(&self) -> u64 {
        self.insertions.load(Ordering::Relaxed)
    }

    /// Get evictions.
    pub fn evictions(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Get hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let hits = self.hits() as f64;
        let total = hits + self.misses() as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }
}

/// Static route statistics.
#[derive(Debug, Default)]
pub struct StaticRouteStats {
    hits: AtomicU64,
    misses: AtomicU64,
}

impl StaticRouteStats {
    /// Get hits.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get misses.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get hit ratio.
    pub fn hit_ratio(&self) -> f64 {
        let hits = self.hits() as f64;
        let total = hits + self.misses() as f64;
        if total > 0.0 { hits / total } else { 0.0 }
    }
}

/// Router statistics.
#[derive(Debug, Default)]
pub struct RouterStats {
    requests: AtomicU64,
    static_hits: AtomicU64,
    cache_hits: AtomicU64,
    pattern_matches: AtomicU64,
}

impl RouterStats {
    /// Get total requests.
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Get static route hits.
    pub fn static_hits(&self) -> u64 {
        self.static_hits.load(Ordering::Relaxed)
    }

    /// Get cache hits.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Get pattern match fallbacks.
    pub fn pattern_matches(&self) -> u64 {
        self.pattern_matches.load(Ordering::Relaxed)
    }

    /// Get optimization efficiency (static + cache hits / total).
    pub fn optimization_ratio(&self) -> f64 {
        let optimized = self.static_hits() + self.cache_hits();
        let total = self.requests();
        if total > 0 {
            optimized as f64 / total as f64
        } else {
            0.0
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_route_key_equality() {
        let key1 = RouteKey::new(HttpMethod::GET, "/users");
        let key2 = RouteKey::new(HttpMethod::GET, "/users");
        let key3 = RouteKey::new(HttpMethod::POST, "/users");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cached_route_static() {
        let cached = CachedRoute::static_route(0);
        assert!(cached.is_static);

        let params = cached.extract_params("/users");
        assert!(params.is_empty());
    }

    #[test]
    fn test_cached_route_with_params() {
        let cached = CachedRoute::with_params(0, vec![("id".to_string(), 1)]);
        assert!(!cached.is_static);

        let params = cached.extract_params("/users/123");
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_route_key_from_request_unknown_method() {
        let req = HttpRequest::new("PROPFIND", "/health".to_string());
        assert!(RouteKey::from_request(&req).is_none());

        let req = HttpRequest::new("GET", "/health?x=1".to_string());
        let key = RouteKey::from_request(&req).unwrap();
        assert_eq!(key, RouteKey::new(HttpMethod::GET, "/health"));
    }

    #[test]
    fn test_cached_route_catch_all_extracts_remaining_segments() {
        let compiled = CompiledRoute::compile("/files/*path");
        let cached = CachedRoute::from_compiled(0, &compiled);
        assert_eq!(cached.catch_all_index, Some(1));

        let params = cached.extract_params("/files/docs/readme.md");
        assert_eq!(params.get("path"), Some(&"docs/readme.md".to_string()));

        let params = cached.extract_params("/files/docs");
        assert_eq!(params.get("path"), Some(&"docs".to_string()));
    }

    #[tokio::test]
    async fn test_router_unknown_method_not_routed_to_get() {
        let mut router = OptimizedRouter::new();
        router.add_route(
            HttpMethod::GET,
            "/health",
            crate::handler::handler(|_req: HttpRequest| async {
                Ok::<_, Error>(HttpResponse::ok())
            }),
        );

        // Known method hits the static fast path.
        let ok = router
            .route(HttpRequest::new("GET", "/health".to_string()))
            .await;
        assert!(ok.is_ok());

        // Unknown method must not fall back to the GET handler.
        let err = router
            .route(HttpRequest::new("PROPFIND", "/health".to_string()))
            .await;
        assert!(matches!(err, Err(Error::RouteNotFound(_))));
    }

    #[tokio::test]
    async fn test_router_cached_catch_all_params() {
        let mut router = OptimizedRouter::new();
        router.add_route(
            HttpMethod::GET,
            "/files/*path",
            crate::handler::handler(|req: HttpRequest| async move {
                let path = req.path_params.get("path").cloned().unwrap_or_default();
                let mut response = HttpResponse::ok();
                response.body = Bytes::from(path.into_bytes());
                Ok::<_, Error>(response)
            }),
        );

        // First request: pattern match populates the cache.
        let first = router
            .route(HttpRequest::new("GET", "/files/docs/readme.md".to_string()))
            .await
            .unwrap();
        assert_eq!(first.body, Bytes::from_static(b"docs/readme.md"));

        // Second request: served from the cache, must yield the same params.
        let second = router
            .route(HttpRequest::new("GET", "/files/docs/readme.md".to_string()))
            .await
            .unwrap();
        assert_eq!(second.body, Bytes::from_static(b"docs/readme.md"));
        assert!(router.stats().cache_hits() > 0);
    }

    #[tokio::test]
    async fn test_router_decodes_query_params() {
        let mut router = OptimizedRouter::new();
        router.add_route(
            HttpMethod::GET,
            "/search",
            crate::handler::handler(|req: HttpRequest| async move {
                let q = req.query_param("q").unwrap_or_default().to_owned();
                let mut response = HttpResponse::ok();
                response.body = Bytes::from(q.into_bytes());
                Ok::<_, Error>(response)
            }),
        );

        let response = router
            .route(HttpRequest::new(
                "GET",
                "/search?q=hello%20world".to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(response.body, Bytes::from_static(b"hello world"));
    }

    #[test]
    fn test_route_cache() {
        let cache = RouteCache::new();

        let key = RouteKey::new(HttpMethod::GET, "/users");
        let route = CachedRoute::static_route(0);

        // Miss
        assert!(cache.get(&key).is_none());
        assert_eq!(cache.stats().misses(), 1);

        // Insert
        cache.insert(key.clone(), route);

        // Hit
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.stats().hits(), 1);
    }

    #[test]
    fn test_static_routes() {
        let mut static_routes = StaticRoutes::new();

        static_routes.add(HttpMethod::GET, "/api/health", 0);
        static_routes.add(HttpMethod::GET, "/api/users", 1);

        let key = RouteKey::new(HttpMethod::GET, "/api/health");
        assert_eq!(static_routes.get(&key), Some(0));

        let key = RouteKey::new(HttpMethod::GET, "/api/users");
        assert_eq!(static_routes.get(&key), Some(1));

        let key = RouteKey::new(HttpMethod::GET, "/api/missing");
        assert_eq!(static_routes.get(&key), None);
    }

    #[test]
    fn test_is_static_path() {
        assert!(StaticRoutes::is_static_path("/api/health"));
        assert!(StaticRoutes::is_static_path("/users"));
        assert!(!StaticRoutes::is_static_path("/users/:id"));
        assert!(!StaticRoutes::is_static_path("/files/*path"));
    }

    #[test]
    fn test_compiled_route_static() {
        let compiled = CompiledRoute::compile("/api/health");
        assert!(compiled.is_static);
        assert!(compiled.param_indices.is_empty());
        assert!(compiled.matches("/api/health"));
        assert!(!compiled.matches("/api/users"));
    }

    #[test]
    fn test_compiled_route_with_param() {
        let compiled = CompiledRoute::compile("/users/:id");
        assert!(!compiled.is_static);
        assert_eq!(compiled.param_indices.len(), 1);

        assert!(compiled.matches("/users/123"));
        assert!(compiled.matches("/users/abc"));
        assert!(!compiled.matches("/users"));
        assert!(!compiled.matches("/users/123/extra"));

        let params = compiled.extract_params("/users/123");
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_compiled_route_multiple_params() {
        let compiled = CompiledRoute::compile("/users/:user_id/posts/:post_id");
        assert!(!compiled.is_static);
        assert_eq!(compiled.param_indices.len(), 2);

        assert!(compiled.matches("/users/123/posts/456"));

        let params = compiled.extract_params("/users/123/posts/456");
        assert_eq!(params.get("user_id"), Some(&"123".to_string()));
        assert_eq!(params.get("post_id"), Some(&"456".to_string()));
    }

    #[test]
    fn test_compiled_route_catch_all() {
        let compiled = CompiledRoute::compile("/files/*path");
        assert!(!compiled.is_static);
        assert!(compiled.has_catch_all);

        assert!(compiled.matches("/files/docs"));
        assert!(compiled.matches("/files/docs/readme.md"));

        let params = compiled.extract_params("/files/docs/readme.md");
        assert_eq!(params.get("path"), Some(&"docs/readme.md".to_string()));
    }

    #[test]
    fn test_router_stats() {
        let stats = RouterStats::default();

        stats.requests.fetch_add(100, Ordering::Relaxed);
        stats.static_hits.fetch_add(50, Ordering::Relaxed);
        stats.cache_hits.fetch_add(30, Ordering::Relaxed);
        stats.pattern_matches.fetch_add(20, Ordering::Relaxed);

        assert_eq!(stats.requests(), 100);
        assert_eq!(stats.static_hits(), 50);
        assert_eq!(stats.cache_hits(), 30);
        assert_eq!(stats.pattern_matches(), 20);
        assert!((stats.optimization_ratio() - 0.8).abs() < 0.001);
    }

    // ------------------------------------------------------------------
    // `from_router`: the compiled serve-path router must dispatch with the
    // exact semantics of the linear `Router::route`.
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_from_router_dispatches_static_and_param() {
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/health",
            |_req: HttpRequest| async { Ok::<_, Error>(HttpResponse::ok()) },
        ));
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/users/:id",
            |req: HttpRequest| async move {
                let id = req.path_params.get("id").cloned().unwrap_or_default();
                let mut r = HttpResponse::ok();
                r.body = Bytes::from(id.into_bytes());
                Ok::<_, Error>(r)
            },
        ));

        let opt = OptimizedRouter::from_router(&router);

        // Static route → O(1) static fast path.
        let resp = opt.route(HttpRequest::new("GET", "/health")).await.unwrap();
        assert_eq!(resp.status, 200);
        assert!(opt.stats().static_hits() >= 1);

        // Param route → pattern match, params extracted onto the request.
        let resp = opt
            .route(HttpRequest::new("GET", "/users/42"))
            .await
            .unwrap();
        assert_eq!(resp.body, Bytes::from_static(b"42"));
    }

    #[tokio::test]
    async fn test_from_router_catch_all() {
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/files/*path",
            |req: HttpRequest| async move {
                let p = req.path_params.get("path").cloned().unwrap_or_default();
                let mut r = HttpResponse::ok();
                r.body = Bytes::from(p.into_bytes());
                Ok::<_, Error>(r)
            },
        ));

        let opt = OptimizedRouter::from_router(&router);

        // Catch-all returns the full joined remainder, not a single segment.
        let resp = opt
            .route(HttpRequest::new("GET", "/files/docs/readme.md"))
            .await
            .unwrap();
        assert_eq!(resp.body, Bytes::from_static(b"docs/readme.md"));
    }

    #[tokio::test]
    async fn test_from_router_validates_constraints() {
        let constraints =
            RouteConstraints::new().add("id", Box::new(crate::route_constraint::UIntConstraint));
        let mut router = crate::routing::Router::new();
        router.add_route(
            crate::routing::Route::new(HttpMethod::GET, "/users/:id", |_req: HttpRequest| async {
                Ok::<_, Error>(HttpResponse::ok())
            })
            .with_constraints(constraints),
        );

        let opt = OptimizedRouter::from_router(&router);

        // Valid param passes.
        let ok = opt.route(HttpRequest::new("GET", "/users/123")).await;
        assert!(ok.is_ok());

        // Invalid param → same BadRequest as the linear router.
        let err = opt.route(HttpRequest::new("GET", "/users/abc")).await;
        assert!(matches!(err, Err(Error::BadRequest(_))));

        // Cached path must re-validate constraints identically.
        let err_again = opt.route(HttpRequest::new("GET", "/users/abc")).await;
        assert!(matches!(err_again, Err(Error::BadRequest(_))));
    }

    #[tokio::test]
    async fn test_from_router_unknown_method_not_get() {
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/health",
            |_req: HttpRequest| async { Ok::<_, Error>(HttpResponse::ok()) },
        ));
        let opt = OptimizedRouter::from_router(&router);

        let err = opt.route(HttpRequest::new("PROPFIND", "/health")).await;
        assert!(matches!(err, Err(Error::RouteNotFound(_))));
    }

    #[tokio::test]
    async fn test_from_router_query_method_routing() {
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::QUERY,
            "/search",
            |req: HttpRequest| async move {
                Ok::<_, Error>(HttpResponse::ok().with_bytes_body(req.body))
            },
        ));
        let opt = OptimizedRouter::from_router(&router);

        // QUERY carries its query in the body; routing matches on method+path.
        let mut req = HttpRequest::new("QUERY", "/search");
        req.body = Bytes::from_static(b"name=john");
        let resp = opt.route(req).await.unwrap();
        assert_eq!(resp.into_body_bytes().as_ref(), b"name=john");

        // GET to the same path must NOT reach the QUERY handler.
        let err = opt.route(HttpRequest::new("GET", "/search")).await;
        assert!(matches!(err, Err(Error::RouteNotFound(_))));
    }

    #[tokio::test]
    async fn test_from_router_preserves_registration_order_precedence() {
        // A `:param` route registered BEFORE a static route that it shadows.
        // The linear router returns the first registered match; the optimized
        // router must not let the static HashMap override that.
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/users/:id",
            |req: HttpRequest| async move {
                let id = req.path_params.get("id").cloned().unwrap_or_default();
                Ok::<_, Error>(HttpResponse::ok().with_body(format!("param:{id}").into_bytes()))
            },
        ));
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/users/me",
            |_req: HttpRequest| async {
                Ok::<_, Error>(HttpResponse::ok().with_body(b"static".to_vec()))
            },
        ));

        // Reference: what the linear router does for /users/me.
        let linear = router
            .clone()
            .route(HttpRequest::new("GET", "/users/me"))
            .await
            .unwrap();

        let opt = OptimizedRouter::from_router(&router);
        let optimized = opt
            .route(HttpRequest::new("GET", "/users/me"))
            .await
            .unwrap();

        // Identical: both select the earlier-registered :id route.
        assert_eq!(optimized.body, linear.body);
        assert_eq!(optimized.body, Bytes::from_static(b"param:me"));
    }

    #[tokio::test]
    async fn test_from_router_decodes_query_params() {
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/search",
            |req: HttpRequest| async move {
                let q = req.query_param("q").unwrap_or_default().to_owned();
                Ok::<_, Error>(HttpResponse::ok().with_body(q.into_bytes()))
            },
        ));
        let opt = OptimizedRouter::from_router(&router);

        let resp = opt
            .route(HttpRequest::new("GET", "/search?q=hello%20world"))
            .await
            .unwrap();
        assert_eq!(resp.body, Bytes::from_static(b"hello world"));
    }

    #[test]
    fn test_from_router_skips_shadowed_static_fast_path() {
        // `/users/:id` (index 0) shadows the later static `/users/me` (index 1),
        // so `/users/me` must NOT be registered in the static fast path.
        let mut router = crate::routing::Router::new();
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/users/:id",
            |_req: HttpRequest| async { Ok::<_, Error>(HttpResponse::ok()) },
        ));
        router.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/users/me",
            |_req: HttpRequest| async { Ok::<_, Error>(HttpResponse::ok()) },
        ));
        let opt = OptimizedRouter::from_router(&router);
        // The shadowed static route stays out of the O(1) map.
        assert!(
            opt.static_routes
                .get(&RouteKey::new(HttpMethod::GET, "/users/me"))
                .is_none()
        );

        // A non-shadowed static route DOES get the fast path.
        let mut router2 = crate::routing::Router::new();
        router2.add_route(crate::routing::Route::new(
            HttpMethod::GET,
            "/health",
            |_req: HttpRequest| async { Ok::<_, Error>(HttpResponse::ok()) },
        ));
        let opt2 = OptimizedRouter::from_router(&router2);
        assert!(
            opt2.static_routes
                .get(&RouteKey::new(HttpMethod::GET, "/health"))
                .is_some()
        );
    }

    #[test]
    fn test_route_cache_eviction() {
        let cache = RouteCache::with_capacity(10);

        // Fill cache
        for i in 0..15 {
            let key = RouteKey::new(HttpMethod::GET, format!("/route/{}", i));
            cache.insert(key, CachedRoute::static_route(i));
        }

        // Should have evicted some entries
        assert!(cache.len() <= 10);
        assert!(cache.stats().evictions() > 0);
    }

    /// Regression: eviction must follow true access recency, not
    /// `HashMap`'s unspecified iteration order. A cache with random
    /// eviction would only pass this by chance (roughly 1-in-N per key
    /// where N is the capacity); a real LRU passes it deterministically
    /// every time.
    #[test]
    fn test_route_cache_lru_eviction_respects_recency() {
        let cache = RouteCache::with_capacity(3);

        let key0 = RouteKey::new(HttpMethod::GET, "/route/0");
        let key1 = RouteKey::new(HttpMethod::GET, "/route/1");
        let key2 = RouteKey::new(HttpMethod::GET, "/route/2");
        let key3 = RouteKey::new(HttpMethod::GET, "/route/3");

        // Fill the cache to capacity, oldest to newest: key0, key1, key2.
        cache.insert(key0.clone(), CachedRoute::static_route(0));
        cache.insert(key1.clone(), CachedRoute::static_route(1));
        cache.insert(key2.clone(), CachedRoute::static_route(2));

        // Refresh key0 (the oldest entry): a real LRU promotes it to
        // most-recently-used, so it must survive the next eviction even
        // though key1 and key2 were inserted after it.
        assert!(cache.get(&key0).is_some());

        // Cache is full; inserting a new key must evict the true LRU entry
        // — key1, the oldest entry that was never re-accessed — not an
        // arbitrary one.
        cache.insert(key3.clone(), CachedRoute::static_route(3));

        assert!(
            cache.get(&key0).is_some(),
            "recently-accessed entry must survive eviction"
        );
        assert!(
            cache.get(&key1).is_none(),
            "genuinely-unaccessed entry must be evicted"
        );
        assert!(cache.get(&key2).is_some());
        assert!(cache.get(&key3).is_some());
        assert_eq!(cache.len(), 3);
    }
}
