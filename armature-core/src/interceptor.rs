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
/// Caches successful responses keyed by `method:path` for `ttl_seconds`. On a
/// fresh hit the cached response is returned without invoking the downstream
/// handler; otherwise the handler runs and its response is stored. Expired
/// entries are pruned lazily whenever a new response is inserted.
///
/// The cache is process-local and shared across clones of the interceptor via
/// an `Arc`, so a single `CacheInterceptor` value serves every request routed
/// through it.
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

#[async_trait]
impl Interceptor for CacheInterceptor {
    async fn intercept(
        &self,
        context: ExecutionContext,
        next: Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>,
    ) -> Result<HttpResponse, Error> {
        let cache_key = format!("{}:{}", context.request.method, context.request.path);
        let ttl = Duration::from_secs(self.ttl_seconds);

        // Lookup: serve a fresh hit without touching the handler.
        {
            let store = self.store.read();
            if let Some((stored_at, cached)) = store.get(&cache_key)
                && stored_at.elapsed() < ttl
            {
                return Ok(clone_response(cached));
            }
        }

        // Miss (or stale): run the handler.
        let response = next.await?;

        // Only cache success responses; errors and non-2xx must not be pinned.
        if (200..300).contains(&response.status) && self.ttl_seconds > 0 {
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
}
