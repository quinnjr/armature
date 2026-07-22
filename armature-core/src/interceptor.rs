// Interceptors for transforming requests and responses

use crate::{Error, HttpRequest, HttpResponse};
use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Execution context passed to interceptors
pub struct ExecutionContext {
    pub request: HttpRequest,
}

impl ExecutionContext {
    pub fn new(request: HttpRequest) -> Self {
        Self { request }
    }
}

/// Interceptor trait for request/response transformation
#[async_trait]
pub trait Interceptor: Send + Sync {
    /// Intercept the request before/after handler execution
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error>;
}

/// Logging interceptor
pub struct LoggingInterceptor;

#[async_trait]
impl Interceptor for LoggingInterceptor {
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let start = std::time::Instant::now();
        let method = context.request.method.clone();
        let path = context.request.path.clone();

        println!("→ {} {}", method, path);

        let result = next.await;

        let duration = start.elapsed();
        match &result {
            Ok(response) => {
                println!(
                    "← {} {} - {} ({:?})",
                    method, path, response.status, duration
                );
            }
            Err(e) => {
                println!("← {} {} - Error: {} ({:?})", method, path, e, duration);
            }
        }

        result
    }
}

/// Transform interceptor for modifying responses
pub struct TransformInterceptor<F>
where
    F: Fn(HttpResponse) -> HttpResponse + Send + Sync,
{
    transform: F,
}

impl<F> TransformInterceptor<F>
where
    F: Fn(HttpResponse) -> HttpResponse + Send + Sync,
{
    pub fn new(transform: F) -> Self {
        Self { transform }
    }
}

#[async_trait]
impl<F> Interceptor for TransformInterceptor<F>
where
    F: Fn(HttpResponse) -> HttpResponse + Send + Sync,
{
    async fn intercept(
        &self,
        _context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let response = next.await?;
        Ok((self.transform)(response))
    }
}

/// Response-cache interceptor.
///
/// Caches successful **GET/HEAD** responses keyed by `METHOD:path?sorted-query`
/// for `ttl_seconds`. Query parameters are folded into the key (sorted by name)
/// so that requests differing only in the query string — e.g. `?q=cats` vs
/// `?q=dogs` — never collide. On a fresh hit the cached response is returned
/// without invoking the downstream handler; otherwise the handler runs and its
/// response is stored. Expired entries are pruned lazily whenever a new response
/// is inserted.
///
/// The cache is process-local and shared across clones of the interceptor via
/// an `Arc`, so a single `CacheInterceptor` value serves every request routed
/// through it.
///
/// # Safety: do not use on per-user or authenticated routes
///
/// This is a **shared, unkeyed-by-identity** cache: it does not discriminate on
/// `Authorization`, `Cookie`, or any other per-user dimension. Placing it on a
/// route whose response depends on the caller's identity would serve one user's
/// body to another. To reduce that blast radius it will **not store** a response
/// that:
///
/// - was produced by a method other than `GET` or `HEAD`, or
/// - carries a `Set-Cookie` header (session/user state), or
/// - carries `Cache-Control: private` or `Cache-Control: no-store`.
///
/// These guards are defensive, not a substitute for correct placement: only
/// attach this interceptor to routes whose responses are identical for every
/// caller.
pub struct CacheInterceptor {
    /// Freshness window, in seconds. A cached entry is served only while its
    /// age is strictly less than this. `0` disables caching (nothing is ever
    /// fresh).
    pub ttl_seconds: u64,
    /// Backing store: `method:path` -> (stored-at instant, response).
    store: Arc<RwLock<HashMap<String, (Instant, HttpResponse)>>>,
}

impl CacheInterceptor {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            ttl_seconds,
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Number of entries currently held (including any not-yet-pruned expired
    /// ones). Primarily useful for tests and diagnostics.
    pub fn len(&self) -> usize {
        self.store.read().len()
    }

    /// Whether the cache currently holds no entries.
    pub fn is_empty(&self) -> bool {
        self.store.read().is_empty()
    }
}

/// Produce an owned copy of a response. `HttpResponse` is deliberately not
/// `Clone` (it carries zero-copy `Bytes` internals), so we rebuild it from its
/// observable parts, preserving status, headers, cookies, and body.
fn clone_response(response: &HttpResponse) -> HttpResponse {
    let mut copy = HttpResponse::from_parts(
        response.status,
        response.headers.to_hashmap(),
        response.body_ref().to_vec(),
    );
    copy.cookies = response.cookies.clone();
    copy
}

/// Build the cache key for a request: `METHOD:path` with a canonicalized query
/// string appended, sorted by parameter name. Sorting means logically-identical
/// requests with different parameter orderings map to the same entry, while any
/// difference in a query value yields a distinct entry (so `?q=cats` and
/// `?q=dogs` never share a cached body).
///
/// Query params are already percent-decoded by the time they reach here, so a
/// decoded value may itself contain `&` or `=`. To keep the key injective we
/// length-prefix each key and value as `{len}:{bytes}` before joining with `=`
/// and `&` — the prefix lets a (hypothetical) parser skip exactly `len` bytes
/// regardless of what delimiter-like characters they contain, so two params
/// with different decoded content can never serialize to the same key. (E.g.
/// decoded `{"a": "1&b=2"}` and decoded `{"a": "1", "b": "2"}` would both join
/// to the literal string `a=1&b=2` under naive concatenation; length-prefixing
/// makes them `5:1&b=2` vs. `1:1` + `1:2` under distinct keys.)
fn cache_key(request: &HttpRequest) -> String {
    let mut key = format!("{}:{}", request.method, request.path);
    if request.query_params.is_empty() {
        return key;
    }
    let mut params: Vec<(&String, &String)> = request.query_params.iter().collect();
    params.sort_by(|a, b| a.0.cmp(b.0));
    key.push('?');
    for (k, v) in params {
        key.push_str(&k.len().to_string());
        key.push(':');
        key.push_str(k);
        key.push('=');
        key.push_str(&v.len().to_string());
        key.push(':');
        key.push_str(v);
        key.push('&');
    }
    key
}

/// Only safe, idempotent methods are cacheable here: `GET` and `HEAD`.
fn is_cacheable_method(method: &str) -> bool {
    method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD")
}

/// Whether a response must not be stored in this shared, identity-agnostic
/// cache: it carries a `Set-Cookie` (per-user session state) or a
/// `Cache-Control` directive that forbids shared storage (`private`/`no-store`).
///
/// `HttpResponse.headers` (`LazyHeaders`) is backed by a plain `HashMap` whose
/// `get`/`contains_key` are case-sensitive, but HTTP header *names* are not —
/// and nothing normalizes response header casing before it reaches here. A
/// handler emitting `cache-control: private` or `set-cookie: ...` (any casing
/// is legal) must be caught just like the canonically-cased form, so both
/// checks walk `headers.iter()` and compare names with `eq_ignore_ascii_case`.
fn must_not_cache(response: &HttpResponse) -> bool {
    if !response.cookies.is_empty() {
        return true;
    }
    let mut cache_control: Option<&str> = None;
    for (name, value) in response.headers.iter() {
        if name.eq_ignore_ascii_case("set-cookie") {
            return true;
        }
        if name.eq_ignore_ascii_case("cache-control") {
            cache_control = Some(value.as_str());
        }
    }
    if let Some(cc) = cache_control {
        return cc.split(',').any(|directive| {
            let directive = directive.trim();
            directive.eq_ignore_ascii_case("private") || directive.eq_ignore_ascii_case("no-store")
        });
    }
    false
}

#[async_trait]
impl Interceptor for CacheInterceptor {
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let ttl = Duration::from_secs(self.ttl_seconds);
        // Only safe methods (GET/HEAD) ever read from or write to the cache.
        let method_cacheable = is_cacheable_method(&context.request.method);
        let cache_key = cache_key(&context.request);

        // Lookup: serve a fresh hit without touching the handler.
        if method_cacheable {
            let store = self.store.read();
            if let Some((stored_at, cached)) = store.get(&cache_key)
                && stored_at.elapsed() < ttl
            {
                return Ok(clone_response(cached));
            }
        }

        // Miss (or stale): run the handler.
        let response = next.await?;

        // Store only when: the method is safe, caching is enabled, the response
        // is a success (2xx), and it is safe to share — i.e. it does not carry a
        // Set-Cookie or a private/no-store Cache-Control. This keeps per-user
        // bodies and session cookies from bleeding across callers.
        if method_cacheable
            && self.ttl_seconds > 0
            && (200..300).contains(&response.status)
            && !must_not_cache(&response)
        {
            let mut store = self.store.write();
            // Prune expired entries so a full-of-stale cache cannot grow without
            // bound on the insert path.
            store.retain(|_, (stored_at, _)| stored_at.elapsed() < ttl);
            store.insert(cache_key, (Instant::now(), clone_response(&response)));
        }

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logging_interceptor_creation() {
        let _interceptor = LoggingInterceptor;
    }

    #[test]
    fn test_cache_interceptor_creation() {
        let interceptor = CacheInterceptor::new(60);
        assert_eq!(interceptor.ttl_seconds, 60);
    }

    #[test]
    fn test_cache_interceptor_different_ttls() {
        let i1 = CacheInterceptor::new(30);
        let i2 = CacheInterceptor::new(120);
        let i3 = CacheInterceptor::new(3600);

        assert_eq!(i1.ttl_seconds, 30);
        assert_eq!(i2.ttl_seconds, 120);
        assert_eq!(i3.ttl_seconds, 3600);
    }

    #[test]
    fn test_transform_interceptor_creation() {
        let _interceptor = TransformInterceptor::new(|res| res);
    }

    #[test]
    fn test_execution_context_creation() {
        let request = crate::HttpRequest::new("GET".to_string(), "/test".to_string());

        let context = ExecutionContext::new(request.clone());
        assert_eq!(context.request.method, "GET");
        assert_eq!(context.request.path, "/test");
    }

    #[test]
    fn test_execution_context_with_metadata() {
        let mut request = crate::HttpRequest::new("POST".to_string(), "/api/users".to_string());
        request.body = vec![1, 2, 3];

        let context = ExecutionContext::new(request.clone());
        assert_eq!(context.request.body.len(), 3);
    }

    #[test]
    fn test_cache_interceptor_zero_ttl() {
        let interceptor = CacheInterceptor::new(0);
        assert_eq!(interceptor.ttl_seconds, 0);
    }

    #[test]
    fn test_cache_interceptor_long_ttl() {
        let one_day = 86400;
        let interceptor = CacheInterceptor::new(one_day);
        assert_eq!(interceptor.ttl_seconds, one_day);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counting_next(
        calls: Arc<AtomicUsize>,
        status: u16,
        body: &'static [u8],
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = HttpResponse::new(status);
            resp.body = body.to_vec();
            Ok(resp)
        })
    }

    /// Regression: a fresh hit must be served from the store without invoking
    /// the downstream handler a second time. The old passthrough implementation
    /// called `next` on every request, so this asserted call-count of 1 failed.
    #[tokio::test]
    async fn test_cache_interceptor_caches_within_ttl() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/cached".into()));
        let first = interceptor
            .intercept(ctx, counting_next(calls.clone(), 200, b"payload"))
            .await
            .unwrap();
        assert_eq!(first.body_ref(), b"payload");

        let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/cached".into()));
        let second = interceptor
            .intercept(ctx, counting_next(calls.clone(), 200, b"payload"))
            .await
            .unwrap();
        assert_eq!(second.body_ref(), b"payload");
        assert_eq!(second.status, 200);

        // Handler ran exactly once; the second response came from cache.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(interceptor.len(), 1);
    }

    /// A zero TTL means nothing is ever fresh, so every request must hit the
    /// handler (the freshness gate, exercised in the "miss" direction).
    #[tokio::test]
    async fn test_cache_interceptor_zero_ttl_never_caches() {
        let interceptor = CacheInterceptor::new(0);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..3 {
            let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/x".into()));
            interceptor
                .intercept(ctx, counting_next(calls.clone(), 200, b"body"))
                .await
                .unwrap();
        }

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(interceptor.is_empty());
    }

    /// Distinct method/path pairs are cached independently and do not collide.
    #[tokio::test]
    async fn test_cache_interceptor_distinct_keys() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/a".into()));
        interceptor
            .intercept(ctx, counting_next(calls.clone(), 200, b"a"))
            .await
            .unwrap();
        let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/b".into()));
        interceptor
            .intercept(ctx, counting_next(calls.clone(), 200, b"b"))
            .await
            .unwrap();

        // Two distinct keys => two handler invocations, two cached entries.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(interceptor.len(), 2);
    }

    /// Non-success responses must not be cached (errors should not be pinned).
    #[tokio::test]
    async fn test_cache_interceptor_skips_non_success() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/err".into()));
            interceptor
                .intercept(ctx, counting_next(calls.clone(), 500, b"boom"))
                .await
                .unwrap();
        }

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(interceptor.is_empty());
    }

    /// Requests that differ only in a query parameter must not collide: the
    /// second must receive its own body, not a replay of the first. The old
    /// `method:path` key ignored the query string entirely, so `?q=cats` and
    /// `?q=dogs` shared one entry.
    #[tokio::test]
    async fn test_cache_interceptor_query_params_distinct() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        let mut req_cats = HttpRequest::new("GET".into(), "/search".into());
        req_cats.query_params.insert("q".into(), "cats".into());
        let first = interceptor
            .intercept(
                ExecutionContext::new(req_cats),
                counting_next(calls.clone(), 200, b"cats-result"),
            )
            .await
            .unwrap();
        assert_eq!(first.body_ref(), b"cats-result");

        let mut req_dogs = HttpRequest::new("GET".into(), "/search".into());
        req_dogs.query_params.insert("q".into(), "dogs".into());
        let second = interceptor
            .intercept(
                ExecutionContext::new(req_dogs),
                counting_next(calls.clone(), 200, b"dogs-result"),
            )
            .await
            .unwrap();
        // Must be the dogs handler's body, not a replay of the cats entry.
        assert_eq!(second.body_ref(), b"dogs-result");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    fn cookie_next(
        calls: Arc<AtomicUsize>,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        Box::pin(async move {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = HttpResponse::ok();
            resp.body = format!("user-{n}").into_bytes();
            resp.cookies.push(format!("session=secret-{n}; HttpOnly"));
            Ok(resp)
        })
    }

    /// A response carrying a `Set-Cookie` holds per-user session state and must
    /// never be cached: the handler must be re-invoked on the next request and
    /// user A's cookie must never be replayed to user B.
    #[tokio::test]
    async fn test_cache_interceptor_refuses_set_cookie() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        let first = interceptor
            .intercept(
                ExecutionContext::new(HttpRequest::new("GET".into(), "/me".into())),
                cookie_next(calls.clone()),
            )
            .await
            .unwrap();
        assert_eq!(first.body_ref(), b"user-0");
        assert_eq!(first.cookies, vec!["session=secret-0; HttpOnly".to_string()]);

        // Second request must NOT be served user-0's body or cookie from cache.
        let second = interceptor
            .intercept(
                ExecutionContext::new(HttpRequest::new("GET".into(), "/me".into())),
                cookie_next(calls.clone()),
            )
            .await
            .unwrap();
        assert_eq!(second.body_ref(), b"user-1");
        assert_eq!(second.cookies, vec!["session=secret-1; HttpOnly".to_string()]);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a Set-Cookie response must be re-fetched, never cached"
        );
        assert!(interceptor.is_empty());
    }

    /// Only safe methods (GET/HEAD) are cached. A cacheable-status POST must run
    /// its handler on every request and must not be stored.
    #[tokio::test]
    async fn test_cache_interceptor_skips_unsafe_methods() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let ctx = ExecutionContext::new(HttpRequest::new("POST".into(), "/submit".into()));
            interceptor
                .intercept(ctx, counting_next(calls.clone(), 200, b"ok"))
                .await
                .unwrap();
        }

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(interceptor.is_empty(), "POST responses must not be cached");
    }

    // ---- Round 2 regression tests -----------------------------------------

    fn header_next(
        calls: Arc<AtomicUsize>,
        header_name: &'static str,
        header_value: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        Box::pin(async move {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let mut resp = HttpResponse::ok();
            resp.body = format!("body-{n}").into_bytes();
            resp.headers
                .insert(header_name.to_string(), header_value.to_string());
            Ok(resp)
        })
    }

    /// FIX A (RED->GREEN): HTTP header names are case-insensitive and nothing
    /// normalizes response header casing before it reaches the cache. A
    /// response carrying a lowercased `cache-control: private` must be refused
    /// exactly like the canonically-cased form.
    #[tokio::test]
    async fn test_cache_interceptor_refuses_lowercase_cache_control_private() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/private".into()));
            interceptor
                .intercept(ctx, header_next(calls.clone(), "cache-control", "private"))
                .await
                .unwrap();
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a lowercased Cache-Control: private response must never be cached"
        );
        assert!(interceptor.is_empty());
    }

    /// FIX A (RED->GREEN): same as above for the `no-store` directive, and for
    /// a value with mixed case and trailing directives (`No-Store, max-age=0`).
    #[tokio::test]
    async fn test_cache_interceptor_refuses_lowercase_cache_control_no_store() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/no-store".into()));
            interceptor
                .intercept(
                    ctx,
                    header_next(calls.clone(), "cache-control", "No-Store, max-age=0"),
                )
                .await
                .unwrap();
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a lowercased Cache-Control: no-store response must never be cached"
        );
        assert!(interceptor.is_empty());
    }

    /// FIX A (RED->GREEN): the `Set-Cookie` *header* check (independent of the
    /// `response.cookies` backstop) must also be case-insensitive.
    #[tokio::test]
    async fn test_cache_interceptor_refuses_lowercase_set_cookie_header() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        for _ in 0..2 {
            let ctx = ExecutionContext::new(HttpRequest::new("GET".into(), "/lc-cookie".into()));
            interceptor
                .intercept(
                    ctx,
                    header_next(calls.clone(), "set-cookie", "session=abc; HttpOnly"),
                )
                .await
                .unwrap();
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a lowercased set-cookie header must never be cached"
        );
        assert!(interceptor.is_empty());
    }

    /// FIX B (RED->GREEN): two requests whose DECODED query params differ —
    /// `{"a": "1&b=2"}` (what `?a=1%26b%3D2` decodes to) vs.
    /// `{"a": "1", "b": "2"}` (what `?a=1&b=2` decodes to) — must not collide
    /// on the same cache key. The old joiner concatenated raw decoded values
    /// with literal unescaped `&`/`=`, so both produced the literal string
    /// `a=1&b=2` and shared one entry.
    #[tokio::test]
    async fn test_cache_interceptor_query_delimiter_injection_distinct() {
        let interceptor = CacheInterceptor::new(60);
        let calls = Arc::new(AtomicUsize::new(0));

        let mut req_encoded = HttpRequest::new("GET".into(), "/inject".into());
        req_encoded.query_params.insert("a".into(), "1&b=2".into());
        let first = interceptor
            .intercept(
                ExecutionContext::new(req_encoded),
                counting_next(calls.clone(), 200, b"encoded-result"),
            )
            .await
            .unwrap();
        assert_eq!(first.body_ref(), b"encoded-result");

        let mut req_plain = HttpRequest::new("GET".into(), "/inject".into());
        req_plain.query_params.insert("a".into(), "1".into());
        req_plain.query_params.insert("b".into(), "2".into());
        let second = interceptor
            .intercept(
                ExecutionContext::new(req_plain),
                counting_next(calls.clone(), 200, b"plain-result"),
            )
            .await
            .unwrap();

        // Must be the plain handler's own body, not a replay of the encoded entry.
        assert_eq!(second.body_ref(), b"plain-result");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "delimiter-colliding decoded query params must not share a cache entry"
        );
    }
}
