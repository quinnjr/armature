//! gRPC middleware using Tower.
//!
//! Each middleware type in this module implements [`GrpcMiddleware`] and wraps
//! a tonic-compatible `tower::Service` with real, observable behavior — not
//! just a config holder. They are designed to compose: every server-side
//! wrapper (`Timeout`, `ConcurrencyLimit`, `LoadShedding`, `RateLimit`) accepts
//! and produces a service with the same signature
//! (`Service<http::Request<ReqBody>, Response = http::Response<TonicBody>, Error = Infallible>`)
//! used by [`crate::server::GrpcServerBuilder::serve`], so they can be stacked
//! and then handed to `serve`/`serve_with_middleware`.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};
use tonic::Status;
use tower::Layer;

use crate::TonicBody;

/// Build an `http::Response<TonicBody>` that carries the given gRPC `Status`,
/// mirroring how tonic's own `InterceptedService` turns a rejected request
/// into a response (see `tonic::service::interceptor::ResponseFuture`).
pub(crate) fn status_response(status: Status) -> http::Response<TonicBody> {
    let (parts, ()) = status.into_http::<()>().into_parts();
    http::Response::from_parts(parts, TonicBody::empty())
}

/// gRPC middleware trait.
pub trait GrpcMiddleware<S> {
    /// The wrapped service type.
    type Service;

    /// Wrap the service with this middleware.
    fn wrap(self, service: S) -> Self::Service;
}

/// Middleware layer for Tower compatibility.
#[derive(Clone)]
pub struct MiddlewareLayer<M> {
    middleware: M,
}

impl<M> MiddlewareLayer<M> {
    /// Create a new middleware layer.
    pub fn new(middleware: M) -> Self {
        Self { middleware }
    }
}

impl<S, M> Layer<S> for MiddlewareLayer<M>
where
    M: GrpcMiddleware<S> + Clone,
{
    type Service = M::Service;

    fn layer(&self, service: S) -> Self::Service {
        self.middleware.clone().wrap(service)
    }
}

/// Bound shared by every server-side middleware wrapper in this module: a
/// cloneable tonic-compatible service whose future is `Send`.
trait GrpcService<ReqBody>:
    tower::Service<http::Request<ReqBody>, Response = http::Response<TonicBody>, Error = Infallible>
    + Clone
    + Send
    + 'static
{
}

impl<S, ReqBody> GrpcService<ReqBody> for S where
    S: tower::Service<
            http::Request<ReqBody>,
            Response = http::Response<TonicBody>,
            Error = Infallible,
        > + Clone
        + Send
        + 'static
{
}

// ===========================================================================
// Timeout
// ===========================================================================

/// Timeout middleware for gRPC. Bounds the wrapped service's request duration
/// and returns `Status::deadline_exceeded` once it elapses.
#[derive(Clone)]
pub struct TimeoutMiddleware {
    timeout: Duration,
}

impl TimeoutMiddleware {
    /// Create a new timeout middleware.
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Get the timeout duration.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl<S> GrpcMiddleware<S> for TimeoutMiddleware {
    type Service = TimeoutService<S>;

    fn wrap(self, service: S) -> Self::Service {
        TimeoutService {
            inner: service,
            timeout: self.timeout,
        }
    }
}

/// Service produced by [`TimeoutMiddleware`].
#[derive(Clone)]
pub struct TimeoutService<S> {
    inner: S,
    timeout: Duration,
}

impl<S, ReqBody> tower::Service<http::Request<ReqBody>> for TimeoutService<S>
where
    S: GrpcService<ReqBody>,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let timeout = self.timeout;
        Box::pin(async move {
            match tokio::time::timeout(timeout, inner.call(req)).await {
                Ok(result) => result,
                Err(_elapsed) => Ok(status_response(Status::deadline_exceeded(format!(
                    "request exceeded timeout of {timeout:?}"
                )))),
            }
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for TimeoutService<S> {
    const NAME: &'static str = S::NAME;
}

// ===========================================================================
// Concurrency limit
// ===========================================================================

/// Concurrency limit middleware. Caps the number of in-flight requests via a
/// semaphore; excess requests wait (serializing) rather than being dropped.
#[derive(Clone)]
pub struct ConcurrencyLimitMiddleware {
    max_concurrent: usize,
}

impl ConcurrencyLimitMiddleware {
    /// Create a new concurrency limit middleware.
    pub fn new(max_concurrent: usize) -> Self {
        Self { max_concurrent }
    }

    /// Get the concurrency limit.
    pub fn limit(&self) -> usize {
        self.max_concurrent
    }
}

impl<S> GrpcMiddleware<S> for ConcurrencyLimitMiddleware {
    type Service = ConcurrencyLimitService<S>;

    fn wrap(self, service: S) -> Self::Service {
        ConcurrencyLimitService {
            inner: service,
            semaphore: Arc::new(Semaphore::new(self.max_concurrent.max(1))),
        }
    }
}

/// Service produced by [`ConcurrencyLimitMiddleware`].
#[derive(Clone)]
pub struct ConcurrencyLimitService<S> {
    inner: S,
    semaphore: Arc<Semaphore>,
}

impl<S, ReqBody> tower::Service<http::Request<ReqBody>> for ConcurrencyLimitService<S>
where
    S: GrpcService<ReqBody>,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let semaphore = self.semaphore.clone();
        Box::pin(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("concurrency limit semaphore should never be closed");
            inner.call(req).await
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for ConcurrencyLimitService<S> {
    const NAME: &'static str = S::NAME;
}

// ===========================================================================
// Load shedding
// ===========================================================================

/// Load shedding middleware that rejects new requests once the number of
/// in-flight requests exceeds `max_concurrent`, instead of queueing them
/// (unlike [`ConcurrencyLimitMiddleware`], which blocks).
#[derive(Clone)]
pub struct LoadSheddingMiddleware {
    enabled: bool,
    max_concurrent: usize,
}

impl LoadSheddingMiddleware {
    /// Create a new load shedding middleware with a default overload
    /// threshold of 64 concurrent requests.
    pub fn new() -> Self {
        Self {
            enabled: true,
            max_concurrent: 64,
        }
    }

    /// Enable or disable load shedding.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Set the number of concurrent in-flight requests above which new
    /// requests are rejected.
    pub fn max_concurrent(mut self, max_concurrent: usize) -> Self {
        self.max_concurrent = max_concurrent;
        self
    }
}

impl Default for LoadSheddingMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> GrpcMiddleware<S> for LoadSheddingMiddleware {
    type Service = LoadSheddingService<S>;

    fn wrap(self, service: S) -> Self::Service {
        LoadSheddingService {
            inner: service,
            semaphore: Arc::new(Semaphore::new(self.max_concurrent.max(1))),
            enabled: self.enabled,
        }
    }
}

/// Service produced by [`LoadSheddingMiddleware`].
#[derive(Clone)]
pub struct LoadSheddingService<S> {
    inner: S,
    semaphore: Arc<Semaphore>,
    enabled: bool,
}

impl<S, ReqBody> tower::Service<http::Request<ReqBody>> for LoadSheddingService<S>
where
    S: GrpcService<ReqBody>,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        if !self.enabled {
            let clone = self.inner.clone();
            let mut inner = std::mem::replace(&mut self.inner, clone);
            return Box::pin(async move { inner.call(req).await });
        }

        match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => {
                let clone = self.inner.clone();
                let mut inner = std::mem::replace(&mut self.inner, clone);
                Box::pin(async move {
                    let result = inner.call(req).await;
                    drop(permit);
                    result
                })
            }
            Err(_) => Box::pin(async move {
                Ok(status_response(Status::unavailable(
                    "server overloaded: load shedding is rejecting new requests",
                )))
            }),
        }
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for LoadSheddingService<S> {
    const NAME: &'static str = S::NAME;
}

// ===========================================================================
// Rate limit
// ===========================================================================

/// Rate limiting middleware for gRPC. Rejects requests over the configured
/// per-second rate with `Status::resource_exhausted` using a fixed 1-second
/// sliding window.
pub struct RateLimitMiddleware {
    requests_per_second: u64,
}

impl RateLimitMiddleware {
    /// Create a new rate limit middleware.
    pub fn new(requests_per_second: u64) -> Self {
        Self {
            requests_per_second,
        }
    }

    /// Get the rate limit.
    pub fn rps(&self) -> u64 {
        self.requests_per_second
    }
}

impl Clone for RateLimitMiddleware {
    fn clone(&self) -> Self {
        Self {
            requests_per_second: self.requests_per_second,
        }
    }
}

impl<S> GrpcMiddleware<S> for RateLimitMiddleware {
    type Service = RateLimitService<S>;

    fn wrap(self, service: S) -> Self::Service {
        RateLimitService {
            inner: service,
            requests_per_second: self.requests_per_second,
            window: Arc::new(Mutex::new(RateWindow {
                start: Instant::now(),
                count: 0,
            })),
        }
    }
}

struct RateWindow {
    start: Instant,
    count: u64,
}

/// Service produced by [`RateLimitMiddleware`].
#[derive(Clone)]
pub struct RateLimitService<S> {
    inner: S,
    requests_per_second: u64,
    window: Arc<Mutex<RateWindow>>,
}

impl<S, ReqBody> tower::Service<http::Request<ReqBody>> for RateLimitService<S>
where
    S: GrpcService<ReqBody>,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let window = self.window.clone();
        let rps = self.requests_per_second;
        Box::pin(async move {
            let allowed = {
                let mut guard = window.lock().await;
                if guard.start.elapsed() >= Duration::from_secs(1) {
                    guard.start = Instant::now();
                    guard.count = 0;
                }
                guard.count += 1;
                guard.count <= rps.max(1)
            };

            if allowed {
                inner.call(req).await
            } else {
                Ok(status_response(Status::resource_exhausted(
                    "rate limit exceeded",
                )))
            }
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for RateLimitService<S> {
    const NAME: &'static str = S::NAME;
}

// ===========================================================================
// Retry
// ===========================================================================

/// Retry middleware for gRPC. Primarily intended for **client-side** use
/// (retrying application-level RPC calls, since generic HTTP request bodies
/// can't be safely replayed): see [`RetryMiddleware::call_with_retry`], which
/// is what [`crate::client::GrpcChannel::call_with_retry`] and
/// `GrpcClientConfig::retry_enabled` / `max_retry_attempts` are wired to.
///
/// A [`GrpcMiddleware`] impl is also provided for services whose request body
/// is `Clone` (e.g. buffered/decoded requests), for composability with the
/// rest of this module.
#[derive(Clone)]
pub struct RetryMiddleware {
    max_attempts: u32,
    retry_codes: Vec<tonic::Code>,
}

impl RetryMiddleware {
    /// Create a new retry middleware. `max_attempts` is the total number of
    /// attempts (including the first), so `1` means "no retries".
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            retry_codes: vec![
                tonic::Code::Unavailable,
                tonic::Code::ResourceExhausted,
                tonic::Code::Aborted,
                tonic::Code::DeadlineExceeded,
            ],
        }
    }

    /// Set the codes that should trigger a retry.
    pub fn with_retry_codes(mut self, codes: Vec<tonic::Code>) -> Self {
        self.retry_codes = codes;
        self
    }

    /// Get the maximum number of attempts.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Check if a code should trigger a retry.
    pub fn should_retry(&self, code: tonic::Code) -> bool {
        self.retry_codes.contains(&code)
    }

    /// Execute an async gRPC call with retry, honoring `max_attempts` and
    /// `retry_codes`. Retries only on statuses in `retry_codes` (transport
    /// failures should be surfaced by the caller as one of those codes, e.g.
    /// `Unavailable`); application-level errors otherwise are returned
    /// immediately without retrying.
    pub async fn call_with_retry<F, Fut, T>(&self, mut make_call: F) -> Result<T, Status>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, Status>>,
    {
        let max_attempts = self.max_attempts.max(1);
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match make_call().await {
                Ok(value) => return Ok(value),
                Err(status) if attempt < max_attempts && self.should_retry(status.code()) => {
                    continue;
                }
                Err(status) => return Err(status),
            }
        }
    }
}

impl<S> GrpcMiddleware<S> for RetryMiddleware {
    type Service = RetryService<S>;

    fn wrap(self, service: S) -> Self::Service {
        RetryService {
            inner: service,
            config: self,
        }
    }
}

/// Service produced by [`RetryMiddleware`]. Requires a `Clone` request body
/// so a failed attempt's request can be replayed.
#[derive(Clone)]
pub struct RetryService<S> {
    inner: S,
    config: RetryMiddleware,
}

impl<S, ReqBody> tower::Service<http::Request<ReqBody>> for RetryService<S>
where
    S: tower::Service<http::Request<ReqBody>, Error = Status> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    ReqBody: Clone + Send + Sync + 'static,
{
    type Response = S::Response;
    type Error = Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Status>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let config = self.config.clone();
        Box::pin(async move { config.call_with_retry(|| inner.call(req.clone())).await })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for RetryService<S> {
    const NAME: &'static str = S::NAME;
}

// ===========================================================================
// Compression (config holder — actual (de)compression is enabled via tonic's
// own `gzip`/`zstd` service builder methods, this type just carries the
// preference for callers to apply)
// ===========================================================================

/// Compression middleware configuration.
pub struct CompressionMiddleware {
    encoding: CompressionEncoding,
}

/// Compression encoding types.
#[derive(Debug, Clone, Copy)]
pub enum CompressionEncoding {
    /// Gzip compression.
    Gzip,
    /// Zstd compression.
    Zstd,
    /// No compression.
    None,
}

impl CompressionMiddleware {
    /// Create a new compression middleware.
    pub fn new(encoding: CompressionEncoding) -> Self {
        Self { encoding }
    }

    /// Create a gzip compression middleware.
    pub fn gzip() -> Self {
        Self::new(CompressionEncoding::Gzip)
    }

    /// Create a zstd compression middleware.
    pub fn zstd() -> Self {
        Self::new(CompressionEncoding::Zstd)
    }

    /// Get the compression encoding.
    pub fn encoding(&self) -> CompressionEncoding {
        self.encoding
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
    use tokio::sync::Barrier;
    use tower::Service as _;

    /// A minimal cloneable tonic-shaped service for unit-testing the
    /// middleware wrappers without needing a full tonic server.
    #[derive(Clone)]
    struct EchoService {
        delay: Option<Duration>,
        in_flight: Arc<AtomicUsize>,
        max_observed_in_flight: Arc<AtomicUsize>,
    }

    impl tower::Service<http::Request<()>> for EchoService {
        type Response = http::Response<TonicBody>;
        type Error = Infallible;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _req: http::Request<()>) -> Self::Future {
            let delay = self.delay;
            let in_flight = self.in_flight.clone();
            let max_observed = self.max_observed_in_flight.clone();
            Box::pin(async move {
                let current = in_flight.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                max_observed.fetch_max(current, AtomicOrdering::SeqCst);
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                in_flight.fetch_sub(1, AtomicOrdering::SeqCst);
                Ok(status_response(Status::ok("done")))
            })
        }
    }

    fn req() -> http::Request<()> {
        http::Request::new(())
    }

    #[tokio::test]
    async fn timeout_middleware_times_out_slow_calls() {
        let echo = EchoService {
            delay: Some(Duration::from_millis(200)),
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_observed_in_flight: Arc::new(AtomicUsize::new(0)),
        };
        let mut svc = TimeoutMiddleware::new(Duration::from_millis(20)).wrap(echo);

        let resp = svc.call(req()).await.unwrap();
        let status = Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[tokio::test]
    async fn timeout_middleware_lets_fast_calls_through() {
        let echo = EchoService {
            delay: None,
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_observed_in_flight: Arc::new(AtomicUsize::new(0)),
        };
        let mut svc = TimeoutMiddleware::new(Duration::from_secs(5)).wrap(echo);

        let resp = svc.call(req()).await.unwrap();
        let status = Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(status.code(), tonic::Code::Ok);
    }

    #[tokio::test]
    async fn concurrency_limit_serializes_calls() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_observed = Arc::new(AtomicUsize::new(0));
        let echo = EchoService {
            delay: Some(Duration::from_millis(50)),
            in_flight: in_flight.clone(),
            max_observed_in_flight: max_observed.clone(),
        };
        let svc = ConcurrencyLimitMiddleware::new(1).wrap(echo);

        let mut a = svc.clone();
        let mut b = svc.clone();
        let (r1, r2) = tokio::join!(a.call(req()), b.call(req()));
        r1.unwrap();
        r2.unwrap();

        // With a concurrency limit of 1, the two overlapping calls must have
        // been serialized: at most 1 was ever in flight simultaneously.
        assert_eq!(max_observed.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn load_shedding_rejects_over_threshold() {
        let barrier = Arc::new(Barrier::new(2));
        #[derive(Clone)]
        struct BlockingService {
            barrier: Arc<Barrier>,
        }
        impl tower::Service<http::Request<()>> for BlockingService {
            type Response = http::Response<TonicBody>;
            type Error = Infallible;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;
            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                Poll::Ready(Ok(()))
            }
            fn call(&mut self, _req: http::Request<()>) -> Self::Future {
                let barrier = self.barrier.clone();
                Box::pin(async move {
                    barrier.wait().await;
                    Ok(status_response(Status::ok("done")))
                })
            }
        }

        let svc = LoadSheddingMiddleware::new()
            .max_concurrent(1)
            .wrap(BlockingService {
                barrier: barrier.clone(),
            });

        let mut held = svc.clone();
        let held_call = tokio::spawn(async move { held.call(req()).await.unwrap() });

        // Give the first call a moment to acquire the permit.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut rejected = svc.clone();
        let rejected_resp = rejected.call(req()).await.unwrap();
        let rejected_status = Status::from_header_map(rejected_resp.headers()).unwrap();
        assert_eq!(rejected_status.code(), tonic::Code::Unavailable);

        // Release the first call so the test can finish cleanly.
        barrier.wait().await;
        held_call.await.unwrap();
    }

    #[tokio::test]
    async fn load_shedding_disabled_does_not_reject() {
        let echo = EchoService {
            delay: None,
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_observed_in_flight: Arc::new(AtomicUsize::new(0)),
        };
        let mut svc = LoadSheddingMiddleware::new()
            .enabled(false)
            .max_concurrent(1)
            .wrap(echo);

        let resp = svc.call(req()).await.unwrap();
        let status = Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(status.code(), tonic::Code::Ok);
    }

    #[tokio::test]
    async fn rate_limit_rejects_burst_over_configured_rps() {
        let echo = EchoService {
            delay: None,
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_observed_in_flight: Arc::new(AtomicUsize::new(0)),
        };
        let mut svc = RateLimitMiddleware::new(1).wrap(echo);

        let first = svc.call(req()).await.unwrap();
        let first_status = Status::from_header_map(first.headers()).unwrap();
        assert_eq!(first_status.code(), tonic::Code::Ok);

        let second = svc.call(req()).await.unwrap();
        let second_status = Status::from_header_map(second.headers()).unwrap();
        assert_eq!(second_status.code(), tonic::Code::ResourceExhausted);
    }

    #[tokio::test]
    async fn retry_middleware_eventually_succeeds() {
        let attempts = Arc::new(AtomicU64::new(0));
        let retry = RetryMiddleware::new(5);

        let result: Result<&'static str, Status> = retry
            .call_with_retry(|| {
                let attempts = attempts.clone();
                async move {
                    let n = attempts.fetch_add(1, AtomicOrdering::SeqCst);
                    if n < 2 {
                        Err(Status::unavailable("temporary"))
                    } else {
                        Ok("ok")
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), "ok");
        assert_eq!(attempts.load(AtomicOrdering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_middleware_gives_up_after_max_attempts() {
        let attempts = Arc::new(AtomicU64::new(0));
        let retry = RetryMiddleware::new(2);

        let result: Result<&'static str, Status> = retry
            .call_with_retry(|| {
                let attempts = attempts.clone();
                async move {
                    attempts.fetch_add(1, AtomicOrdering::SeqCst);
                    Err(Status::unavailable("still down"))
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(AtomicOrdering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_middleware_does_not_retry_non_retriable_status() {
        let attempts = Arc::new(AtomicU64::new(0));
        let retry = RetryMiddleware::new(5);

        let result: Result<&'static str, Status> = retry
            .call_with_retry(|| {
                let attempts = attempts.clone();
                async move {
                    attempts.fetch_add(1, AtomicOrdering::SeqCst);
                    Err(Status::invalid_argument("bad request"))
                }
            })
            .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(AtomicOrdering::SeqCst), 1);
    }
}
