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
#[derive(Clone)]
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

// ===========================================================================
// Compression
// ===========================================================================
//
// Real wire-level gRPC compression, implemented as a pair of generic Tower
// service wrappers (`CompressionService` for the server side,
// `CompressionClientService` for the client side) that operate directly on
// the gRPC length-prefixed message framing — the same technique
// `MaxRecvMessageSizeService` (see `server.rs`) uses to inspect/rewrite
// message frames generically.
//
// tonic's own `.accept_compressed()` / `.send_compressed()` methods (used by
// this comment's earlier revision as the intended integration point) are
// inherent methods on `tonic::server::Grpc<T>` / `tonic::client::Grpc<T>` —
// types that only exist *inside* tonic-build's generated
// `<Service>Server<T>` / `<Service>Client<T>` wrappers. Since this crate's
// `serve<S>` and client channel helpers are generic over any `S`/`Channel`
// and never see the generated wrapper type, there is no generic way to call
// those inherent methods (the same limitation documented on
// `max_recv_message_size`/`max_send_message_size` handling). So instead of a
// no-op, this module performs the compression itself: it decodes each
// length-prefixed gRPC message, gzip/zstd (de)compresses the payload, and
// rewrites the frame's compressed-flag byte and length — real bytes-on-the-
// wire compression, verified in this module's tests.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use http_body_util::{BodyExt, Full};

/// Default cap on how many bytes a single gRPC message may expand to during
/// decompression when no explicit limit has been configured via
/// [`CompressionMiddleware::max_decompressed_size`]. Chosen well above the
/// crate's default `max_recv_message_size` (4 MiB, see
/// `GrpcServerConfig::default`) to avoid surprising legitimate large
/// messages, while still bounding a decompression-bomb payload to a fixed,
/// finite amount of memory instead of the unbounded growth
/// `GzDecoder::read_to_end`/`zstd::stream::decode_all` previously allowed.
pub const DEFAULT_MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

/// Compression middleware configuration.
#[derive(Clone)]
pub struct CompressionMiddleware {
    encoding: CompressionEncoding,
    max_decompressed_size: usize,
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
    /// Create a new compression middleware. Decompressed message size is
    /// capped at [`DEFAULT_MAX_DECOMPRESSED_SIZE`]; use
    /// [`CompressionMiddleware::max_decompressed_size`] to override it (e.g.
    /// to match a `GrpcServerConfig::max_recv_message_size` configured
    /// elsewhere).
    pub fn new(encoding: CompressionEncoding) -> Self {
        Self {
            encoding,
            max_decompressed_size: DEFAULT_MAX_DECOMPRESSED_SIZE,
        }
    }

    /// Create a gzip compression middleware.
    pub fn gzip() -> Self {
        Self::new(CompressionEncoding::Gzip)
    }

    /// Create a zstd compression middleware.
    pub fn zstd() -> Self {
        Self::new(CompressionEncoding::Zstd)
    }

    /// Set the maximum number of bytes a single gRPC message may expand to
    /// while being decompressed. Enforced *during* decompression (the
    /// decompressor is never allowed to produce more than this many bytes),
    /// not merely checked afterward — this is what actually bounds a
    /// decompression-bomb payload's memory usage. Requests/responses whose
    /// decompressed size would exceed this limit are rejected with
    /// `Status::internal` (server side) or `Status::internal` (client side)
    /// rather than being allocated in full.
    pub fn max_decompressed_size(mut self, max_decompressed_size: usize) -> Self {
        self.max_decompressed_size = max_decompressed_size;
        self
    }

    /// Get the compression encoding.
    pub fn encoding(&self) -> CompressionEncoding {
        self.encoding
    }

    /// Get the configured maximum decompressed message size.
    pub fn max_decompressed_size_limit(&self) -> usize {
        self.max_decompressed_size
    }

    /// Wrap a server-side service so it actually decompresses incoming
    /// requests (when their `grpc-encoding` header matches this middleware's
    /// encoding) and compresses outgoing responses (when the request's
    /// `grpc-accept-encoding` header lists this encoding).
    pub fn wrap_server<S>(&self, service: S) -> CompressionService<S> {
        CompressionService {
            inner: service,
            encoding: self.encoding,
            max_decompressed_size: self.max_decompressed_size,
        }
    }

    /// Wrap a client-side channel (e.g. `channel.inner().clone()`, or any
    /// other `tower::Service<http::Request<TonicBody>>`) so outgoing requests
    /// are actually compressed with this encoding (advertised via
    /// `grpc-encoding`/`grpc-accept-encoding`) and incoming responses are
    /// decompressed when the server compressed them.
    ///
    /// The result satisfies tonic's generated-client bound
    /// (`tonic::client::GrpcService<tonic::body::Body>`) and can be passed
    /// directly to a generated `<Service>Client::new`:
    ///
    /// ```rust,ignore
    /// let compressed = CompressionMiddleware::gzip().wrap_channel(channel.inner().clone());
    /// let mut client = GreeterClient::new(compressed);
    /// ```
    pub fn wrap_channel<S>(&self, channel: S) -> CompressionClientService<S> {
        CompressionClientService {
            inner: channel,
            encoding: self.encoding,
            max_decompressed_size: self.max_decompressed_size,
        }
    }
}

impl CompressionEncoding {
    /// The `grpc-encoding` / `grpc-accept-encoding` wire value for this
    /// encoding, or `None` for [`CompressionEncoding::None`] (which never
    /// compresses).
    fn wire_name(self) -> Option<&'static str> {
        match self {
            CompressionEncoding::Gzip => Some("gzip"),
            CompressionEncoding::Zstd => Some("zstd"),
            CompressionEncoding::None => None,
        }
    }

    fn compress_bytes(self, data: &[u8]) -> std::io::Result<Vec<u8>> {
        match self {
            CompressionEncoding::Gzip => {
                use flate2::Compression;
                use flate2::write::GzEncoder;
                use std::io::Write;
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data)?;
                encoder.finish()
            }
            CompressionEncoding::Zstd => zstd::stream::encode_all(data, 0),
            CompressionEncoding::None => Ok(data.to_vec()),
        }
    }

    /// Decompress `data`, refusing to let the decompressor produce more than
    /// `max_decompressed_size` bytes.
    ///
    /// The cap is enforced *during* decompression, not by decompressing in
    /// full and checking the result afterward: both branches wrap the
    /// decompressing reader in `Read::take(max_decompressed_size + 1)` before
    /// `read_to_end`, so a decompression-bomb payload (a small, highly
    /// compressible input that would otherwise expand to gigabytes) can never
    /// cause more than `max_decompressed_size + 1` bytes to be allocated —
    /// once that many bytes have been read, the `take` adapter stops handing
    /// the decoder anywhere further to write, `read_to_end` returns having
    /// filled `out` to just past the limit, and this function errors instead
    /// of returning the (still-truncated) result.
    fn decompress_bytes(
        self,
        data: &[u8],
        max_decompressed_size: usize,
    ) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        match self {
            CompressionEncoding::Gzip => {
                use flate2::read::GzDecoder;
                let decoder = GzDecoder::new(data);
                let mut limited = decoder.take(max_decompressed_size as u64 + 1);
                let mut out = Vec::new();
                limited.read_to_end(&mut out)?;
                if out.len() > max_decompressed_size {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "decompressed gzip message exceeds max_decompressed_size of {max_decompressed_size} bytes"
                        ),
                    ));
                }
                Ok(out)
            }
            CompressionEncoding::Zstd => {
                let decoder = zstd::stream::Decoder::new(data)?;
                let mut limited = decoder.take(max_decompressed_size as u64 + 1);
                let mut out = Vec::new();
                limited.read_to_end(&mut out)?;
                if out.len() > max_decompressed_size {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "decompressed zstd message exceeds max_decompressed_size of {max_decompressed_size} bytes"
                        ),
                    ));
                }
                Ok(out)
            }
            CompressionEncoding::None => Ok(data.to_vec()),
        }
    }
}

/// Length, in bytes, of a gRPC length-prefixed message frame header: 1
/// compressed-flag byte followed by 4 big-endian payload-length bytes. Shared
/// by every place in this crate that parses gRPC frame headers by hand (this
/// module's [`transform_grpc_frames`] and `server.rs`'s
/// `MaxRecvMessageSizeService`) so the "1 flag byte + 4 length bytes" layout
/// is defined once instead of being re-encoded as the magic number `5` (and
/// raw `[1..5]` slices) in multiple independent parsers.
pub(crate) const GRPC_FRAME_HEADER_LEN: usize = 5;

/// Parse a gRPC frame header from the start of `bytes`: returns
/// `(compressed_flag, payload_len)` if at least [`GRPC_FRAME_HEADER_LEN`]
/// bytes are available, or `None` if `bytes` is too short to contain a full
/// header yet (e.g. a length-prefix fragmented across multiple HTTP/2 DATA
/// frames).
pub(crate) fn read_frame_header(bytes: &[u8]) -> Option<(u8, usize)> {
    if bytes.len() < GRPC_FRAME_HEADER_LEN {
        return None;
    }
    let flag = bytes[0];
    let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    Some((flag, len))
}

/// Rewrite every gRPC length-prefixed message frame in `bytes` whose
/// compressed-flag byte equals `matching_flag`, applying `transform` to its
/// payload and flipping the flag to `1 - matching_flag`. Frames that don't
/// match `matching_flag` are copied through unchanged. Returns `Err` if the
/// buffer isn't a clean sequence of complete gRPC frames (truncated length
/// prefix, truncated payload, or trailing bytes) or if `transform` fails —
/// callers must not forward `bytes` unmodified in that case, since it may be
/// a genuinely malformed/corrupted body.
fn transform_grpc_frames(
    mut bytes: Bytes,
    matching_flag: u8,
    transform: impl Fn(&[u8]) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<Bytes> {
    let mut out = BytesMut::with_capacity(bytes.len());
    while !bytes.is_empty() {
        let Some((flag, len)) = read_frame_header(&bytes) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated gRPC message length-prefix",
            ));
        };
        if bytes.len() < GRPC_FRAME_HEADER_LEN + len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated gRPC message payload",
            ));
        }
        let payload = &bytes[GRPC_FRAME_HEADER_LEN..GRPC_FRAME_HEADER_LEN + len];
        if flag == matching_flag {
            let transformed = transform(payload)?;
            out.put_u8(1 - matching_flag);
            out.put_u32(transformed.len() as u32);
            out.put_slice(&transformed);
        } else {
            out.put_u8(flag);
            out.put_u32(len as u32);
            out.put_slice(payload);
        }
        bytes.advance(GRPC_FRAME_HEADER_LEN + len);
    }
    Ok(out.freeze())
}

async fn buffer_body(body: TonicBody) -> Result<Bytes, tonic::Status> {
    Ok(body.collect().await?.to_bytes())
}

fn accept_encoding_contains(headers: &http::HeaderMap, name: &str) -> bool {
    headers
        .get("grpc-accept-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').any(|part| part.trim() == name))
        .unwrap_or(false)
}

/// Server-side service produced by [`CompressionMiddleware::wrap_server`].
/// See the module-level comment above [`CompressionMiddleware`] for why this
/// is a generic frame-rewriting wrapper rather than a call to tonic's
/// `accept_compressed`/`send_compressed`.
pub struct CompressionService<S> {
    inner: S,
    encoding: CompressionEncoding,
    max_decompressed_size: usize,
}

impl<S: Clone> Clone for CompressionService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            encoding: self.encoding,
            max_decompressed_size: self.max_decompressed_size,
        }
    }
}

impl<S> tower::Service<http::Request<TonicBody>> for CompressionService<S>
where
    S: tower::Service<http::Request<TonicBody>, Response = http::Response<TonicBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let encoding = self.encoding;
        let max_decompressed_size = self.max_decompressed_size;

        Box::pin(async move {
            // `CompressionEncoding::None` never compresses or decompresses
            // anything — short-circuit entirely rather than falling into the
            // buffer/transform path below. Without this, `wire_name()` is
            // `None`, and `incoming_encoding.as_deref() == encoding.wire_name()`
            // (`None == None`) is true whenever the client sent no
            // `grpc-encoding` header at all — the common uncompressed case —
            // which would needlessly buffer the body and run it through the
            // identity "decompress" path, mislabeling the frame's compression
            // flag along the way.
            let Some(our_name) = encoding.wire_name() else {
                return inner.call(req).await;
            };

            let (mut parts, body) = req.into_parts();

            let incoming_encoding = parts
                .headers
                .get("grpc-encoding")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string());
            let client_accepts_our_encoding = accept_encoding_contains(&parts.headers, our_name);

            // Decompress the request body only when the client told us it's
            // compressed with the encoding we understand; otherwise pass the
            // body through unmodified (preserves streaming for the common
            // uncompressed case).
            let req_body = if incoming_encoding.as_deref() == Some(our_name) {
                let bytes = match buffer_body(body).await {
                    Ok(b) => b,
                    Err(status) => return Ok(status_response(status)),
                };
                match transform_grpc_frames(bytes, 1, |p| {
                    encoding.decompress_bytes(p, max_decompressed_size)
                }) {
                    Ok(decompressed) => {
                        parts.headers.remove("grpc-encoding");
                        TonicBody::new(Full::new(decompressed))
                    }
                    Err(e) => {
                        return Ok(status_response(Status::internal(format!(
                            "failed to decompress gRPC request message: {e}"
                        ))));
                    }
                }
            } else {
                body
            };

            let resp = inner
                .call(http::Request::from_parts(parts, req_body))
                .await?;

            // Compress the response only if the client advertised support
            // for our encoding.
            if !client_accepts_our_encoding {
                return Ok(resp);
            }

            let (mut rparts, rbody) = resp.into_parts();
            let bytes = match buffer_body(rbody).await {
                Ok(b) => b,
                Err(status) => return Ok(status_response(status)),
            };
            match transform_grpc_frames(bytes.clone(), 0, |p| encoding.compress_bytes(p)) {
                Ok(compressed) => {
                    rparts
                        .headers
                        .insert("grpc-encoding", http::HeaderValue::from_static(our_name));
                    Ok(http::Response::from_parts(
                        rparts,
                        TonicBody::new(Full::new(compressed)),
                    ))
                }
                Err(e) => {
                    // Fall back to the original, uncompressed bytes rather
                    // than dropping the response, but don't silently discard
                    // the underlying error.
                    tracing::warn!(
                        error = %e,
                        "gRPC compression failed; sending response uncompressed"
                    );
                    Ok(http::Response::from_parts(
                        rparts,
                        TonicBody::new(Full::new(bytes)),
                    ))
                }
            }
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for CompressionService<S> {
    const NAME: &'static str = S::NAME;
}

impl<S> GrpcMiddleware<S> for CompressionMiddleware
where
    S: tower::Service<http::Request<TonicBody>, Response = http::Response<TonicBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Service = CompressionService<S>;

    fn wrap(self, service: S) -> Self::Service {
        self.wrap_server(service)
    }
}

/// Client-side service produced by [`CompressionMiddleware::wrap_channel`].
pub struct CompressionClientService<S> {
    inner: S,
    encoding: CompressionEncoding,
    max_decompressed_size: usize,
}

impl<S: Clone> Clone for CompressionClientService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            encoding: self.encoding,
            max_decompressed_size: self.max_decompressed_size,
        }
    }
}

impl<S> tower::Service<http::Request<TonicBody>> for CompressionClientService<S>
where
    S: tower::Service<http::Request<TonicBody>, Response = http::Response<TonicBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let encoding = self.encoding;
        let max_decompressed_size = self.max_decompressed_size;

        Box::pin(async move {
            // As in `CompressionService::call`: `CompressionEncoding::None`
            // never compresses/decompresses anything, so short-circuit
            // before the buffer/transform path rather than relying on
            // `wire_name()` being `None` on both sides of a comparison.
            let Some(our_name) = encoding.wire_name() else {
                return inner.call(req).await;
            };

            let (mut parts, body) = req.into_parts();

            let req_body = match buffer_body(body).await {
                Ok(bytes) => {
                    match transform_grpc_frames(bytes.clone(), 0, |p| encoding.compress_bytes(p)) {
                        Ok(compressed) => {
                            parts
                                .headers
                                .insert("grpc-encoding", http::HeaderValue::from_static(our_name));
                            parts.headers.insert(
                                "grpc-accept-encoding",
                                http::HeaderValue::from_static(our_name),
                            );
                            TonicBody::new(Full::new(compressed))
                        }
                        Err(e) => {
                            // Compression failed; send the original bytes
                            // uncompressed rather than dropping the request, but
                            // don't silently discard the underlying error.
                            tracing::warn!(
                                error = %e,
                                "gRPC compression failed; sending request uncompressed"
                            );
                            TonicBody::new(Full::new(bytes))
                        }
                    }
                }
                Err(status) => return Ok(status_response(status)),
            };

            let resp = inner
                .call(http::Request::from_parts(parts, req_body))
                .await?;

            let (mut rparts, rbody) = resp.into_parts();
            let response_encoding = rparts
                .headers
                .get("grpc-encoding")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string());

            if response_encoding.as_deref() != Some(our_name) {
                return Ok(http::Response::from_parts(rparts, rbody));
            }

            let bytes = match buffer_body(rbody).await {
                Ok(b) => b,
                Err(status) => return Ok(status_response(status)),
            };
            match transform_grpc_frames(bytes, 1, |p| {
                encoding.decompress_bytes(p, max_decompressed_size)
            }) {
                Ok(decompressed) => {
                    rparts.headers.remove("grpc-encoding");
                    Ok(http::Response::from_parts(
                        rparts,
                        TonicBody::new(Full::new(decompressed)),
                    ))
                }
                Err(e) => Ok(status_response(Status::internal(format!(
                    "failed to decompress gRPC response message: {e}"
                )))),
            }
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for CompressionClientService<S> {
    const NAME: &'static str = S::NAME;
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

    // =======================================================================
    // Compression
    // =======================================================================

    /// Build a single gRPC length-prefixed message frame: 1 compressed-flag
    /// byte + 4 big-endian length bytes + payload.
    fn frame(flag: u8, payload: &[u8]) -> Bytes {
        let mut out = BytesMut::with_capacity(5 + payload.len());
        out.put_u8(flag);
        out.put_u32(payload.len() as u32);
        out.put_slice(payload);
        out.freeze()
    }

    #[derive(Clone)]
    struct CaptureService {
        received: Arc<Mutex<Option<Bytes>>>,
        response_payload: Bytes,
    }

    impl tower::Service<http::Request<TonicBody>> for CaptureService {
        type Response = http::Response<TonicBody>;
        type Error = Infallible;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
            let received = self.received.clone();
            let response_payload = self.response_payload.clone();
            Box::pin(async move {
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                *received.lock().await = Some(bytes);
                let body = TonicBody::new(Full::new(frame(0, &response_payload)));
                Ok(http::Response::new(body))
            })
        }
    }

    /// Regression test: `CompressionMiddleware` must actually transform
    /// bytes on the wire — decompressing a compressed incoming request
    /// before the inner service sees it, and compressing the response when
    /// the caller advertised support — not merely store the configured
    /// encoding as inert config.
    #[tokio::test]
    async fn compression_service_decompresses_request_and_compresses_response() {
        let original_request_text =
            b"hello, this is the original uncompressed request payload text";
        let compressed_request = CompressionEncoding::Gzip
            .compress_bytes(original_request_text)
            .unwrap();
        let request_frame = frame(1, &compressed_request); // flag=1: compressed

        let mut req = http::Request::new(TonicBody::new(Full::new(request_frame)));
        req.headers_mut()
            .insert("grpc-encoding", http::HeaderValue::from_static("gzip"));
        req.headers_mut().insert(
            "grpc-accept-encoding",
            http::HeaderValue::from_static("gzip"),
        );

        // A large, highly repetitive response payload — compresses well, so
        // we can assert the wire bytes actually shrank.
        let response_payload: Bytes = Bytes::from(vec![b'a'; 4096]);

        let captured = Arc::new(Mutex::new(None));
        let inner = CaptureService {
            received: captured.clone(),
            response_payload: response_payload.clone(),
        };

        let mut svc = CompressionMiddleware::gzip().wrap_server(inner);
        let resp = svc.call(req).await.unwrap();

        // The inner service must have received the *decompressed* original
        // text with the frame's flag rewritten to 0.
        let received = captured.lock().await.clone().unwrap();
        assert_eq!(received[0], 0, "decompressed frame must carry flag=0");
        assert_eq!(&received[5..], original_request_text);

        // The response must have been compressed: grpc-encoding set, flag=1,
        // and the wire payload strictly smaller than the uncompressed original.
        assert_eq!(resp.headers().get("grpc-encoding").unwrap(), "gzip");
        let resp_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            resp_bytes[0], 1,
            "response frame must carry flag=1 (compressed)"
        );
        let resp_len =
            u32::from_be_bytes([resp_bytes[1], resp_bytes[2], resp_bytes[3], resp_bytes[4]])
                as usize;
        let resp_payload = &resp_bytes[5..5 + resp_len];
        assert!(
            resp_payload.len() < response_payload.len(),
            "compressed payload ({} bytes) must be smaller than the original ({} bytes) — \
             proves real compression happened, not a no-op",
            resp_payload.len(),
            response_payload.len()
        );
        let decompressed = CompressionEncoding::Gzip
            .decompress_bytes(resp_payload, DEFAULT_MAX_DECOMPRESSED_SIZE)
            .unwrap();
        assert_eq!(decompressed, response_payload.to_vec());
    }

    /// Regression test: `CompressionMiddleware::wrap_channel` must actually
    /// compress outgoing requests (setting `grpc-encoding`) and decompress
    /// compressed incoming responses, mirroring the server-side behavior
    /// above for the client path.
    #[tokio::test]
    async fn compression_client_service_compresses_request_and_decompresses_response() {
        #[derive(Clone)]
        struct ServerLikeService;

        impl tower::Service<http::Request<TonicBody>> for ServerLikeService {
            type Response = http::Response<TonicBody>;
            type Error = Infallible;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                Poll::Ready(Ok(()))
            }

            fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
                Box::pin(async move {
                    assert_eq!(
                        req.headers().get("grpc-encoding").unwrap(),
                        "gzip",
                        "the client must advertise the encoding it used"
                    );
                    let bytes = req.into_body().collect().await.unwrap().to_bytes();
                    assert_eq!(
                        bytes[0], 1,
                        "the outgoing request must actually be compressed (flag=1)"
                    );
                    let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
                    let decompressed = CompressionEncoding::Gzip
                        .decompress_bytes(&bytes[5..5 + len], DEFAULT_MAX_DECOMPRESSED_SIZE)
                        .unwrap();
                    assert_eq!(
                        decompressed,
                        b"the client's original outgoing message, sent uncompressed"
                    );

                    let response_payload = vec![b'z'; 2048];
                    let compressed = CompressionEncoding::Gzip
                        .compress_bytes(&response_payload)
                        .unwrap();
                    let mut resp =
                        http::Response::new(TonicBody::new(Full::new(frame(1, &compressed))));
                    resp.headers_mut()
                        .insert("grpc-encoding", http::HeaderValue::from_static("gzip"));
                    Ok(resp)
                })
            }
        }

        let mut svc = CompressionMiddleware::gzip().wrap_channel(ServerLikeService);
        let req = http::Request::new(TonicBody::new(Full::new(frame(
            0,
            b"the client's original outgoing message, sent uncompressed",
        ))));
        let resp = svc.call(req).await.unwrap();

        assert!(
            resp.headers().get("grpc-encoding").is_none(),
            "a fully-decompressed response should have grpc-encoding stripped"
        );
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes[0], 0, "decompressed response frame must carry flag=0");
        let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        assert_eq!(&bytes[5..5 + len], vec![b'z'; 2048].as_slice());
    }

    /// Regression test: a gzip decompression bomb — a small, highly
    /// compressible payload that would expand to far more than the
    /// configured limit — must be rejected *during* decompression rather
    /// than fully decompressed into memory first and checked afterward.
    /// Against the pre-fix `GzDecoder::read_to_end` with no cap, this would
    /// have unconditionally allocated the full decompressed size.
    #[tokio::test]
    async fn gzip_decompression_bomb_is_rejected_during_decompression() {
        // 8 MiB of zeros compresses to a tiny gzip payload.
        let bomb_source = vec![0u8; 8 * 1024 * 1024];
        let compressed = CompressionEncoding::Gzip
            .compress_bytes(&bomb_source)
            .unwrap();
        assert!(
            compressed.len() < 8192,
            "the source data must actually compress well for this to be a meaningful bomb test"
        );

        // A moderate test-sized limit, far below the bomb's real
        // decompressed size, keeps the test fast while still proving the
        // bound is enforced during decompression rather than after.
        let limit = 1024;
        let result = CompressionEncoding::Gzip.decompress_bytes(&compressed, limit);
        assert!(
            result.is_err(),
            "a decompression bomb exceeding the configured limit must be rejected, not fully \
             decompressed into memory"
        );
    }

    /// Same as above, but for zstd — proving `decode_all` (unbounded) was
    /// replaced with a bounded, incremental decode via `zstd::stream::Decoder`.
    #[tokio::test]
    async fn zstd_decompression_bomb_is_rejected_during_decompression() {
        let bomb_source = vec![0u8; 8 * 1024 * 1024];
        let compressed = CompressionEncoding::Zstd
            .compress_bytes(&bomb_source)
            .unwrap();
        assert!(
            compressed.len() < 8192,
            "the source data must actually compress well for this to be a meaningful bomb test"
        );

        let limit = 1024;
        let result = CompressionEncoding::Zstd.decompress_bytes(&compressed, limit);
        assert!(
            result.is_err(),
            "a decompression bomb exceeding the configured limit must be rejected, not fully \
             decompressed into memory"
        );
    }

    /// Companion sanity check: a payload that decompresses to *within* the
    /// limit must still succeed, proving the bound doesn't just reject
    /// everything.
    #[tokio::test]
    async fn decompression_within_limit_still_succeeds() {
        let source = b"a small, well within any reasonable limit payload".to_vec();
        let compressed = CompressionEncoding::Gzip.compress_bytes(&source).unwrap();
        let result = CompressionEncoding::Gzip.decompress_bytes(&compressed, source.len() + 16);
        assert_eq!(result.unwrap(), source);
    }

    /// Regression test: `CompressionMiddleware::new(CompressionEncoding::None)`
    /// must pass request and response bodies through completely untouched —
    /// against the pre-fix code, `incoming_encoding.as_deref() ==
    /// encoding.wire_name()` compared `None == None`, which is true whenever
    /// the client sent no `grpc-encoding` header at all (the common
    /// uncompressed case), sending the frame through the buffer/"decompress"
    /// path and potentially mislabeling its compression flag.
    #[tokio::test]
    async fn none_encoding_server_passes_body_through_byte_for_byte() {
        let original_payload = b"an uncompressed request with no grpc-encoding header at all";
        let request_frame = frame(0, original_payload); // flag=0: not compressed

        // No `grpc-encoding` / `grpc-accept-encoding` headers set at all —
        // the common case for a plain, uncompressed gRPC call.
        let req = http::Request::new(TonicBody::new(Full::new(request_frame.clone())));

        let captured = Arc::new(Mutex::new(None));
        let inner = CaptureService {
            received: captured.clone(),
            response_payload: Bytes::from_static(b"an uncompressed response payload"),
        };

        let mut svc = CompressionMiddleware::new(CompressionEncoding::None).wrap_server(inner);
        let resp = svc.call(req).await.unwrap();

        let received = captured.lock().await.clone().unwrap();
        assert_eq!(
            received, request_frame,
            "a None-encoding middleware must forward the request frame byte-for-byte unmodified"
        );

        let resp_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            resp_bytes,
            frame(0, b"an uncompressed response payload"),
            "a None-encoding middleware must forward the response frame byte-for-byte unmodified"
        );
    }

    /// Client-side counterpart of the test above: `wrap_channel` with
    /// `CompressionEncoding::None` must not compress outgoing requests or
    /// attempt to decompress incoming responses.
    #[tokio::test]
    async fn none_encoding_client_passes_body_through_byte_for_byte() {
        #[derive(Clone)]
        struct ServerLikeService {
            received: Arc<Mutex<Option<Bytes>>>,
        }

        impl tower::Service<http::Request<TonicBody>> for ServerLikeService {
            type Response = http::Response<TonicBody>;
            type Error = Infallible;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

            fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
                Poll::Ready(Ok(()))
            }

            fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
                assert!(
                    req.headers().get("grpc-encoding").is_none(),
                    "a None-encoding client must not set grpc-encoding"
                );
                let received = self.received.clone();
                Box::pin(async move {
                    let bytes = req.into_body().collect().await.unwrap().to_bytes();
                    *received.lock().await = Some(bytes);
                    Ok(http::Response::new(TonicBody::new(Full::new(frame(
                        0,
                        b"server response, also uncompressed",
                    )))))
                })
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut svc =
            CompressionMiddleware::new(CompressionEncoding::None).wrap_channel(ServerLikeService {
                received: captured.clone(),
            });

        let request_frame = frame(0, b"client request, sent uncompressed");
        let req = http::Request::new(TonicBody::new(Full::new(request_frame.clone())));
        let resp = svc.call(req).await.unwrap();

        let received = captured.lock().await.clone().unwrap();
        assert_eq!(
            received, request_frame,
            "a None-encoding client must forward the request frame byte-for-byte unmodified"
        );
        let resp_bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            resp_bytes,
            frame(0, b"server response, also uncompressed"),
            "a None-encoding client must forward the response frame byte-for-byte unmodified"
        );
    }
}
