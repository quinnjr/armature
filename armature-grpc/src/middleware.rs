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

use bytes::{BufMut, Bytes, BytesMut};

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
#[non_exhaustive]
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
    /// [`CompressionMiddleware::with_max_decompressed_size`] to override it
    /// (e.g. to match a `GrpcServerConfig::max_recv_message_size` configured
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

    /// Set the maximum number of bytes a gRPC body may expand to while being
    /// decompressed. Enforced *during* decompression (the decompressor is
    /// never allowed to produce more than this many bytes), not merely checked
    /// afterward — this is what actually bounds a decompression-bomb payload's
    /// memory usage — and applied both per-message and against the running
    /// total across the whole (possibly streaming) body. Requests/responses
    /// whose decompressed size would exceed this limit are rejected with
    /// `Status::resource_exhausted` rather than being allocated in full.
    pub fn with_max_decompressed_size(mut self, max_decompressed_size: usize) -> Self {
        self.max_decompressed_size = max_decompressed_size;
        self
    }

    /// Get the compression encoding.
    pub fn encoding(&self) -> CompressionEncoding {
        self.encoding
    }

    /// Get the configured maximum decompressed message size.
    pub fn max_decompressed_size(&self) -> usize {
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
    /// full and checking the result afterward: both codec branches delegate to
    /// [`read_bounded`], which wraps the decompressing reader in
    /// `Read::take(max_decompressed_size + 1)` before `read_to_end`, so a
    /// decompression-bomb payload (a small, highly compressible input that
    /// would otherwise expand to gigabytes) can never cause more than
    /// `max_decompressed_size + 1` bytes to be allocated — once that many bytes
    /// have been read, the `take` adapter stops handing the decoder anywhere
    /// further to write, `read_to_end` returns having filled `out` to just past
    /// the limit, and this function errors (with a [`DecompressedSizeExceeded`]
    /// marker) instead of returning the still-truncated result.
    fn decompress_bytes(
        self,
        data: &[u8],
        max_decompressed_size: usize,
    ) -> std::io::Result<Vec<u8>> {
        match self {
            CompressionEncoding::Gzip => {
                let decoder = flate2::read::GzDecoder::new(data);
                read_bounded(decoder, max_decompressed_size, "gzip")
            }
            CompressionEncoding::Zstd => {
                let mut decoder = zstd::stream::Decoder::new(data)?;
                // Bound the decoder's internal window allocation independently
                // of the output cap: a crafted zstd frame can request a very
                // large window (and thus a large up-front buffer) even before
                // producing output. 27 == 128 MiB, matching zstd's own
                // conservative default ceiling.
                decoder.window_log_max(27)?;
                read_bounded(decoder, max_decompressed_size, "zstd")
            }
            CompressionEncoding::None => Ok(data.to_vec()),
        }
    }
}

/// Marker error attached (via [`std::io::Error::new`]) to the specific failure
/// where a decompressed message/body exceeds its configured cap, so callers
/// (the streaming compression adapter) can distinguish a decompression-bomb /
/// over-limit rejection — which should surface as `Status::resource_exhausted`
/// — from a genuinely corrupt stream, which should surface as
/// `Status::internal`.
#[derive(Debug)]
struct DecompressedSizeExceeded {
    codec: &'static str,
    max: usize,
}

impl std::fmt::Display for DecompressedSizeExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "decompressed {} message exceeds max_decompressed_size of {} bytes",
            self.codec, self.max
        )
    }
}

impl std::error::Error for DecompressedSizeExceeded {}

/// Decompress everything from `reader` into a fresh `Vec`, refusing to allocate
/// more than `max` bytes. Shared by the gzip and zstd arms of
/// [`CompressionEncoding::decompress_bytes`] so the "take one past the cap,
/// then reject if we actually crossed it" logic lives in exactly one place.
/// Over-limit rejections carry a [`DecompressedSizeExceeded`] marker.
fn read_bounded(
    reader: impl std::io::Read,
    max: usize,
    codec: &'static str,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut limited = reader.take(max as u64 + 1);
    let mut out = Vec::new();
    limited.read_to_end(&mut out)?;
    if out.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            DecompressedSizeExceeded { codec, max },
        ));
    }
    Ok(out)
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

/// Which way a [`CompressionBody`] transforms the gRPC message frames flowing
/// through it.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// Compress: match frames whose compressed-flag byte is `0` (uncompressed),
    /// compress their payload, and flip the flag to `1`.
    Compress,
    /// Decompress: match frames whose compressed-flag byte is `1` (compressed),
    /// decompress their payload, and flip the flag to `0`.
    Decompress,
}

impl Direction {
    /// The compressed-flag byte value a frame must carry for this direction to
    /// transform it (frames carrying the other flag are forwarded unchanged).
    fn matching_flag(self) -> u8 {
        match self {
            Direction::Compress => 0,
            Direction::Decompress => 1,
        }
    }
}

/// Serialize one gRPC length-prefixed frame: 1 compressed-flag byte + 4
/// big-endian length bytes + payload. Rejects payloads longer than `u32::MAX`
/// rather than silently truncating the length prefix (which would desync the
/// gRPC framing on the wire).
fn reframe(flag: u8, payload: &[u8]) -> Result<Bytes, tonic::Status> {
    let len = u32::try_from(payload.len()).map_err(|_| {
        tonic::Status::internal("transformed gRPC message exceeds u32 frame length")
    })?;
    let mut out = BytesMut::with_capacity(GRPC_FRAME_HEADER_LEN + payload.len());
    out.put_u8(flag);
    out.put_u32(len);
    out.put_slice(payload);
    Ok(out.freeze())
}

/// A streaming [`http_body::Body`] adapter that (de)compresses gRPC
/// length-prefixed message frames incrementally as they flow through, instead
/// of buffering the whole body, transforming it, and re-emitting it as a
/// trailer-less `Full`. This is what makes the compression middleware correct
/// on three fronts the old buffer-and-`Full` approach got wrong:
///
/// * **Trailers survive.** tonic carries `grpc-status`/`grpc-message` in the
///   HTTP/2 trailers frame of every non-trailers-only response; collecting a
///   body to `Bytes` and rebuilding it as `Full` dropped that frame, so a
///   compressed response reached a real gRPC client with no status and the RPC
///   failed ("stream closed without grpc-status"). Here, a
///   [`http_body::Frame::trailers`] frame is forwarded untouched.
/// * **Aggregate memory is bounded.** A per-message cap alone doesn't bound a
///   body of *many* small compressed frames each expanding to the cap. This
///   adapter enforces `max_decompressed_size` against the running total across
///   the whole body (not just per message), rejecting with
///   `Status::resource_exhausted` once cumulative decompressed output would
///   exceed it.
/// * **Streaming flows incrementally.** Each message is emitted as soon as it
///   is complete, so a server-streaming / `Watch` / subscribe RPC delivers
///   messages as they arrive instead of stalling until the stream ends (an
///   infinite stream previously deadlocked the call).
///
/// Because the middleware is generic over the server-builder path and cannot
/// observe the server's `max_recv_message_size`, the decompressed-size check is
/// enforced here — per message *and* in aggregate — rather than by defaulting
/// the cap to `max_recv_message_size` at compose time.
struct CompressionBody<B> {
    inner: B,
    encoding: CompressionEncoding,
    direction: Direction,
    max_decompressed_size: usize,
    /// Running total of bytes produced by decompression so far, used to bound
    /// the whole body (not just any single message). Unused for
    /// [`Direction::Compress`], which only ever shrinks its input.
    decompressed_total: usize,
    /// Undelivered input bytes accumulated across inner frames until a full
    /// gRPC message can be cut out and transformed.
    buf: BytesMut,
    /// Whether the inner body has yielded `None` (no more data frames coming).
    inner_done: bool,
}

impl<B> CompressionBody<B> {
    fn new(
        inner: B,
        encoding: CompressionEncoding,
        direction: Direction,
        max_decompressed_size: usize,
    ) -> Self {
        Self {
            inner,
            encoding,
            direction,
            max_decompressed_size,
            decompressed_total: 0,
            buf: BytesMut::new(),
            inner_done: false,
        }
    }

    /// Transform a single complete gRPC frame (`flag` + `payload`) into its
    /// output framing. Frames not matching this direction's flag are re-emitted
    /// unchanged; matching frames are (de)compressed and their flag flipped.
    fn transform_one(&mut self, flag: u8, payload: &[u8]) -> Result<Bytes, tonic::Status> {
        let matching = self.direction.matching_flag();
        if flag != matching {
            // Not our concern (e.g. an already-uncompressed frame in a
            // decompress stream) — forward it verbatim.
            return reframe(flag, payload);
        }
        let transformed = match self.direction {
            Direction::Compress => self.encoding.compress_bytes(payload).map_err(|e| {
                tonic::Status::internal(format!("failed to compress gRPC message: {e}"))
            })?,
            Direction::Decompress => {
                if self.decompressed_total >= self.max_decompressed_size {
                    return Err(tonic::Status::resource_exhausted(format!(
                        "decompressed gRPC body exceeds max_decompressed_size of {} bytes",
                        self.max_decompressed_size
                    )));
                }
                // Cap this message at the remaining aggregate budget so neither
                // a single bomb nor many messages summing past the cap can
                // allocate more than `max_decompressed_size` total.
                let remaining = self.max_decompressed_size - self.decompressed_total;
                let decompressed = self
                    .encoding
                    .decompress_bytes(payload, remaining)
                    .map_err(|e| decompress_error_to_status(&e))?;
                self.decompressed_total += decompressed.len();
                decompressed
            }
        };
        reframe(1 - matching, &transformed)
    }
}

/// Classify a decompression `io::Error`: an over-limit rejection (carrying a
/// [`DecompressedSizeExceeded`] marker) becomes `resource_exhausted`; anything
/// else (a genuinely corrupt/truncated stream) becomes `internal`.
fn decompress_error_to_status(e: &std::io::Error) -> tonic::Status {
    if e.get_ref()
        .is_some_and(|inner| inner.is::<DecompressedSizeExceeded>())
    {
        tonic::Status::resource_exhausted(e.to_string())
    } else {
        tonic::Status::internal(format!("failed to decompress gRPC message: {e}"))
    }
}

impl<B> http_body::Body for CompressionBody<B>
where
    B: http_body::Body<Data = Bytes, Error = tonic::Status> + Unpin,
{
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            // Emit one complete transformed message if the buffer holds one.
            if let Some((flag, len)) = read_frame_header(&this.buf) {
                let total = GRPC_FRAME_HEADER_LEN + len;
                if this.buf.len() >= total {
                    let frame = this.buf.split_to(total);
                    let payload = &frame[GRPC_FRAME_HEADER_LEN..];
                    return match this.transform_one(flag, payload) {
                        Ok(out) => Poll::Ready(Some(Ok(http_body::Frame::data(out)))),
                        Err(status) => Poll::Ready(Some(Err(status))),
                    };
                }
            }

            // Not enough buffered for a full message; pull more from the inner
            // body (or finish).
            if this.inner_done {
                if this.buf.is_empty() {
                    return Poll::Ready(None);
                }
                return Poll::Ready(Some(Err(tonic::Status::internal(
                    "truncated gRPC message: body ended mid-frame while (de)compressing",
                ))));
            }

            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Ready(Some(Ok(frame))) => {
                    if let Some(data) = frame.data_ref() {
                        this.buf.extend_from_slice(data);
                        continue;
                    }
                    // A non-data frame — trailers (grpc-status/grpc-message) or
                    // any other metadata — is forwarded untouched. Any bytes
                    // still buffered here are a partial frame with no following
                    // data, i.e. a truncated body.
                    if !this.buf.is_empty() {
                        return Poll::Ready(Some(Err(tonic::Status::internal(
                            "truncated gRPC message before trailers while (de)compressing",
                        ))));
                    }
                    return Poll::Ready(Some(Ok(frame)));
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    this.inner_done = true;
                    continue;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner_done && self.buf.is_empty()
    }
}

/// Wrap `body` in a [`CompressionBody`] and box it back into a [`TonicBody`].
fn compression_body(
    body: TonicBody,
    encoding: CompressionEncoding,
    direction: Direction,
    max_decompressed_size: usize,
) -> TonicBody {
    TonicBody::new(CompressionBody::new(
        body,
        encoding,
        direction,
        max_decompressed_size,
    ))
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
            // transform path below. Without this, `wire_name()` is `None`, and
            // `incoming_encoding.as_deref() == encoding.wire_name()`
            // (`None == None`) is true whenever the client sent no
            // `grpc-encoding` header at all — the common uncompressed case —
            // which would needlessly run the body through the identity
            // "decompress" path, mislabeling the frame's compression flag along
            // the way.
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
            // body through unmodified. Either way the body streams frame by
            // frame through the inner service — nothing is aggregated.
            let req_body = if incoming_encoding.as_deref() == Some(our_name) {
                parts.headers.remove("grpc-encoding");
                compression_body(body, encoding, Direction::Decompress, max_decompressed_size)
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
            rparts
                .headers
                .insert("grpc-encoding", http::HeaderValue::from_static(our_name));
            let compressed_body =
                compression_body(rbody, encoding, Direction::Compress, max_decompressed_size);
            Ok(http::Response::from_parts(rparts, compressed_body))
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
            // before the transform path rather than relying on `wire_name()`
            // being `None` on both sides of a comparison.
            let Some(our_name) = encoding.wire_name() else {
                return inner.call(req).await;
            };

            let (mut parts, body) = req.into_parts();

            // Compress the outgoing request, advertising the encoding used, and
            // let the compressed frames stream to the server.
            parts
                .headers
                .insert("grpc-encoding", http::HeaderValue::from_static(our_name));
            parts.headers.insert(
                "grpc-accept-encoding",
                http::HeaderValue::from_static(our_name),
            );
            let req_body =
                compression_body(body, encoding, Direction::Compress, max_decompressed_size);

            let resp = inner
                .call(http::Request::from_parts(parts, req_body))
                .await?;

            let (mut rparts, rbody) = resp.into_parts();
            let response_encoding = rparts
                .headers
                .get("grpc-encoding")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string());

            // Only decompress when the server actually used our encoding;
            // otherwise stream the response through untouched.
            if response_encoding.as_deref() != Some(our_name) {
                return Ok(http::Response::from_parts(rparts, rbody));
            }

            rparts.headers.remove("grpc-encoding");
            let decompressed_body = compression_body(
                rbody,
                encoding,
                Direction::Decompress,
                max_decompressed_size,
            );
            Ok(http::Response::from_parts(rparts, decompressed_body))
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for CompressionClientService<S> {
    const NAME: &'static str = S::NAME;
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::{BodyExt, Full};
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

    /// Regression test: a genuinely catastrophic gzip decompression bomb —
    /// 4 GiB of plaintext that compresses to only a few MiB — must be rejected
    /// *during* decompression, never fully decompressed into memory first. A
    /// naive decompress-all-then-check implementation would OOM (or take
    /// forever) trying to materialize the full 4 GiB here; the bounded
    /// `Read::take` path caps allocation at the (tiny) limit and errors out.
    #[tokio::test]
    async fn gzip_decompression_bomb_is_rejected_during_decompression() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..4096 {
            enc.write_all(&chunk).unwrap(); // 4 GiB plaintext, only ~MiBs compressed
        }
        let compressed = enc.finish().unwrap();

        let result = CompressionEncoding::Gzip.decompress_bytes(&compressed, 1024);
        assert!(
            result.is_err(),
            "a 4 GiB gzip bomb must be rejected during decompression, never fully materialized"
        );
    }

    /// Same as above, but for zstd — proving the bounded, incremental decode
    /// (`zstd::stream::Decoder` + `Read::take`, with a capped window) never
    /// materializes the full 4 GiB.
    #[tokio::test]
    async fn zstd_decompression_bomb_is_rejected_during_decompression() {
        use std::io::Write;
        let mut enc = zstd::stream::Encoder::new(Vec::new(), 0).unwrap();
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..4096 {
            enc.write_all(&chunk).unwrap(); // 4 GiB plaintext
        }
        let compressed = enc.finish().unwrap();

        let result = CompressionEncoding::Zstd.decompress_bytes(&compressed, 1024);
        assert!(
            result.is_err(),
            "a 4 GiB zstd bomb must be rejected during decompression, never fully materialized"
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

    /// Exact-boundary check for gzip: a cap of exactly the decompressed length
    /// must succeed, and one byte below it must fail — proving the bound is
    /// `<= max`, off by neither direction.
    #[tokio::test]
    async fn gzip_decompression_exact_boundary() {
        let source = vec![7u8; 5000];
        let compressed = CompressionEncoding::Gzip.compress_bytes(&source).unwrap();
        assert_eq!(
            CompressionEncoding::Gzip
                .decompress_bytes(&compressed, source.len())
                .unwrap(),
            source,
            "a cap of exactly the decompressed size must succeed"
        );
        assert!(
            CompressionEncoding::Gzip
                .decompress_bytes(&compressed, source.len() - 1)
                .is_err(),
            "a cap one byte below the decompressed size must fail"
        );
    }

    /// Exact-boundary check for zstd (there was previously no zstd happy-path
    /// test at all): a cap of exactly the decompressed length must succeed, and
    /// one byte below it must fail.
    #[tokio::test]
    async fn zstd_decompression_exact_boundary() {
        let source = vec![9u8; 5000];
        let compressed = CompressionEncoding::Zstd.compress_bytes(&source).unwrap();
        assert_eq!(
            CompressionEncoding::Zstd
                .decompress_bytes(&compressed, source.len())
                .unwrap(),
            source,
            "a cap of exactly the decompressed size must succeed"
        );
        assert!(
            CompressionEncoding::Zstd
                .decompress_bytes(&compressed, source.len() - 1)
                .is_err(),
            "a cap one byte below the decompressed size must fail"
        );
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
        // flag=1 deliberately: the mislabeling bug the None short-circuit
        // guards against only fires on a flag=1 frame (the identity
        // "decompress" path would flip the flag to 0). A flag=0 frame would
        // pass this test even against the pre-fix code, so it must be flag=1.
        let request_frame = frame(1, original_payload);

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

    /// A minimal body that yields a fixed queue of `http_body::Frame`s, used to
    /// drive the streaming `CompressionBody` adapter directly (including a
    /// trailers frame, which the old buffer-and-`Full` path silently dropped).
    struct QueuedBody {
        frames: std::collections::VecDeque<http_body::Frame<Bytes>>,
    }

    impl http_body::Body for QueuedBody {
        type Data = Bytes;
        type Error = tonic::Status;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Bytes>, Self::Error>>> {
            Poll::Ready(self.frames.pop_front().map(Ok))
        }

        fn is_end_stream(&self) -> bool {
            self.frames.is_empty()
        }
    }

    /// Primary regression test (bug 1, trailer loss): the streaming
    /// `CompressionBody` adapter must forward the HTTP/2 trailers frame
    /// untouched. tonic carries `grpc-status`/`grpc-message` in trailers, so
    /// the old `Collected::to_bytes()` + `Full::new(...)` path — which drops
    /// trailers entirely — made every compressed response fail against a real
    /// gRPC client ("stream closed without grpc-status"). Here we push a data
    /// frame followed by a trailers frame through the adapter and assert both
    /// the compressed data *and* the trailers come out the other side.
    #[tokio::test]
    async fn compression_body_forwards_trailers_frame() {
        let payload = vec![b'q'; 512];
        let mut trailers = http::HeaderMap::new();
        trailers.insert("grpc-status", http::HeaderValue::from_static("0"));
        trailers.insert("grpc-message", http::HeaderValue::from_static("ok"));

        let inner = QueuedBody {
            frames: [
                http_body::Frame::data(frame(0, &payload)),
                http_body::Frame::trailers(trailers.clone()),
            ]
            .into_iter()
            .collect(),
        };

        let body = CompressionBody::new(
            inner,
            CompressionEncoding::Gzip,
            Direction::Compress,
            DEFAULT_MAX_DECOMPRESSED_SIZE,
        );
        let collected = body.collect().await.unwrap();

        let got_trailers = collected
            .trailers()
            .expect("the trailers frame must survive (de)compression");
        assert_eq!(
            got_trailers.get("grpc-status").unwrap(),
            "0",
            "grpc-status must be preserved through compression"
        );
        assert_eq!(got_trailers.get("grpc-message").unwrap(), "ok");

        let data = collected.to_bytes();
        assert_eq!(
            data[0], 1,
            "the data frame must have been compressed (flag=1)"
        );
    }

    /// Primary regression test (bugs 2 & 3): the adapter emits each message as
    /// soon as it is complete (so streaming RPCs flow incrementally rather than
    /// stalling until end-of-stream), and it enforces `max_decompressed_size`
    /// against the running total across the whole body (not just per message),
    /// rejecting with `resource_exhausted` once cumulative output would exceed
    /// the cap. Three 1000-byte messages under a 2500-byte aggregate cap: the
    /// first two decode and are delivered incrementally, the third pushes the
    /// running total past the cap and is rejected.
    #[tokio::test]
    async fn compression_body_streams_incrementally_and_bounds_aggregate_size() {
        let payload = vec![0u8; 1000];
        let one = CompressionEncoding::Gzip.compress_bytes(&payload).unwrap();

        // Three compressed messages back-to-back in a single inner data frame.
        let mut all = BytesMut::new();
        for _ in 0..3 {
            all.extend_from_slice(&frame(1, &one));
        }

        let inner = QueuedBody {
            frames: [http_body::Frame::data(all.freeze())].into_iter().collect(),
        };

        // Aggregate cap admits two 1000-byte messages (2000) but not a third.
        let mut body = CompressionBody::new(
            inner,
            CompressionEncoding::Gzip,
            Direction::Decompress,
            2500,
        );

        for i in 0..2 {
            let f = body
                .frame()
                .await
                .unwrap_or_else(|| panic!("message {i} should be delivered incrementally"))
                .unwrap();
            let Ok(d) = f.into_data() else {
                panic!("expected a data frame");
            };
            assert_eq!(d[0], 0, "decompressed frame must carry flag=0");
            assert_eq!(&d[GRPC_FRAME_HEADER_LEN..], &payload[..]);
        }

        // The third message pushes the cumulative decompressed total over the
        // cap and must be rejected — proving the bound is aggregate, not
        // per-message.
        let err = body
            .frame()
            .await
            .expect("a third frame poll is expected")
            .expect_err("the aggregate cap must reject the third message");
        assert_eq!(
            err.code(),
            tonic::Code::ResourceExhausted,
            "exceeding the aggregate decompressed-size cap must yield resource_exhausted"
        );
    }

    /// Primary end-to-end regression test: wrap a *real* tonic service (the
    /// health service) with `CompressionMiddleware::gzip().wrap_server(...)`,
    /// serve it, and call it through `wrap_channel(...)` gzip on the client
    /// side. This exercises the full path — request compressed by the client,
    /// decompressed by the server, response compressed by the server,
    /// decompressed by the client — and, crucially, the response trailers
    /// carrying `grpc-status`. Before the streaming rewrite this failed with
    /// "stream closed without grpc-status" because the buffer-and-`Full` path
    /// dropped the trailers frame; it must now succeed.
    #[tokio::test]
    async fn compression_middleware_round_trip_with_real_tonic_service_preserves_trailers() {
        use tonic::transport::Server;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (reporter, health_service) = tonic_health::server::health_reporter();
        reporter
            .set_serving::<tonic_health::pb::health_server::HealthServer<
                tonic_health::server::HealthService,
            >>()
            .await;

        let wrapped = CompressionMiddleware::gzip().wrap_server(health_service);

        tokio::spawn(async move {
            Server::builder()
                .add_service(wrapped)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let compressed = CompressionMiddleware::gzip().wrap_channel(channel);
        let mut client = tonic_health::pb::health_client::HealthClient::new(compressed);

        let resp = client
            .check(tonic_health::pb::HealthCheckRequest {
                service: "grpc.health.v1.Health".to_string(),
            })
            .await
            .expect(
                "a gzip-compressed RPC against a real tonic service must succeed end-to-end; \
                 failure here means trailers (grpc-status) were lost",
            );

        let status = tonic_health::pb::health_check_response::ServingStatus::try_from(
            resp.into_inner().status,
        )
        .unwrap();
        assert_eq!(
            status,
            tonic_health::pb::health_check_response::ServingStatus::Serving
        );
    }
}
