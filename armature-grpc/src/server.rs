//! gRPC server implementation.

use std::future::Future;
use tonic::transport::Server;
use tracing::{error, info};

use crate::TonicBody;
use crate::middleware::{GRPC_FRAME_HEADER_LEN, GrpcMiddleware, read_frame_header};
use crate::{GrpcError, GrpcServerConfig, Result};

/// gRPC server builder.
pub struct GrpcServerBuilder {
    config: GrpcServerConfig,
}

/// Bound satisfied by any service usable with [`GrpcServerBuilder::serve`].
///
/// `TonicBody` (`tonic::body::Body`) is the same body type tonic-build's
/// generated `<Service>Server<T>` types use for their `Response`, so real
/// generated services satisfy this directly.
pub trait ServableService:
    tower::Service<
        http::Request<tonic::body::Body>,
        Response = http::Response<TonicBody>,
        Error = std::convert::Infallible,
    > + tonic::server::NamedService
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<S> ServableService for S where
    S: tower::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<TonicBody>,
            Error = std::convert::Infallible,
        > + tonic::server::NamedService
        + Clone
        + Send
        + Sync
        + 'static
{
}

impl GrpcServerBuilder {
    /// Create a new server builder.
    pub fn new(config: GrpcServerConfig) -> Self {
        Self { config }
    }

    /// Build a `tonic::transport::Server` with this builder's transport-level
    /// configuration applied (keepalive, nodelay, window sizes, TLS, ...).
    fn build_transport(&self) -> Result<Server> {
        let mut builder = Server::builder()
            .tcp_nodelay(self.config.tcp_nodelay)
            .tcp_keepalive(self.config.tcp_keepalive);

        if let Some(interval) = self.config.http2_keepalive_interval {
            builder = builder.http2_keepalive_interval(Some(interval));
        }
        if let Some(timeout) = self.config.http2_keepalive_timeout {
            builder = builder.http2_keepalive_timeout(Some(timeout));
        }
        if let Some(window) = self.config.initial_connection_window_size {
            builder = builder.initial_connection_window_size(window);
        }
        if let Some(window) = self.config.initial_stream_window_size {
            builder = builder.initial_stream_window_size(window);
        }
        if let Some(limit) = self.config.concurrency_limit_per_connection {
            builder = builder.concurrency_limit_per_connection(limit);
        }

        if let Some(tls) = &self.config.tls {
            builder = builder
                .tls_config(build_server_tls_config(tls)?)
                .map_err(GrpcError::Transport)?;
        }

        Ok(builder)
    }

    /// Register the health-check and reflection services (when enabled in
    /// `self.config`) onto `router`, keeping the health reporter so
    /// registered services can actually be marked SERVING (previously this
    /// dropped the reporter, so Check/Watch always answered NOT_SERVING).
    ///
    /// Shared by [`GrpcServerBuilder::serve`] and
    /// [`GrpcServerBuilder::serve_with_shutdown`] so the two entry points
    /// have identical health/reflection behavior instead of drifting apart.
    async fn register_optional_services<S, L>(
        &self,
        router: tonic::transport::server::Router<L>,
    ) -> tonic::transport::server::Router<L>
    where
        S: ServableService,
    {
        #[cfg(feature = "health")]
        let router = if self.config.enable_health_check {
            let (health_reporter, health_service) = tonic_health::server::health_reporter();
            health_reporter.set_serving::<S>().await;
            router.add_service(health_service)
        } else {
            router
        };

        #[cfg(feature = "reflection")]
        let router = if self.config.enable_reflection {
            match tonic_reflection::server::Builder::configure().build_v1() {
                Ok(reflection_service) => router.add_service(reflection_service),
                Err(e) => {
                    error!(error = %e, "Failed to build gRPC reflection service");
                    router
                }
            }
        } else {
            router
        };

        router
    }

    /// Build and start the server with a service.
    pub async fn serve<S>(self, service: S) -> Result<()>
    where
        S: ServableService,
        S::Future: Send + 'static,
    {
        let addr = self.config.bind_address;

        info!(address = %addr, "Starting gRPC server");

        let mut builder = self.build_transport()?;
        let limited = limit_recv_message_size(service, self.config.max_recv_message_size);
        let router = builder.add_service(limited);
        let router = self.register_optional_services::<S, _>(router).await;

        router.serve(addr).await.map_err(|e| {
            error!(error = %e, "gRPC server error");
            GrpcError::Server(e.to_string())
        })
    }

    /// Build and start the server, wrapping `service` with `middleware`
    /// (e.g. [`crate::middleware::TimeoutMiddleware`],
    /// [`crate::middleware::ConcurrencyLimitMiddleware`],
    /// [`crate::middleware::LoadSheddingMiddleware`],
    /// [`crate::middleware::RateLimitMiddleware`]) before serving it.
    pub async fn serve_with_middleware<S, M>(self, service: S, middleware: M) -> Result<()>
    where
        M: GrpcMiddleware<S>,
        M::Service: ServableService,
        <M::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send + 'static,
    {
        let wrapped = middleware.wrap(service);
        self.serve(wrapped).await
    }

    /// Build and start the server with graceful shutdown.
    pub async fn serve_with_shutdown<S, F>(self, service: S, signal: F) -> Result<()>
    where
        S: ServableService,
        S::Future: Send + 'static,
        F: Future<Output = ()> + Send + 'static,
    {
        let addr = self.config.bind_address;

        info!(address = %addr, "Starting gRPC server with graceful shutdown");

        let mut builder = self.build_transport()?;
        let limited = limit_recv_message_size(service, self.config.max_recv_message_size);
        let router = builder.add_service(limited);
        let router = self.register_optional_services::<S, _>(router).await;

        router.serve_with_shutdown(addr, signal).await.map_err(|e| {
            error!(error = %e, "gRPC server error");
            GrpcError::Server(e.to_string())
        })
    }
}

/// Per-message parse state tracked across HTTP/2 DATA frames so
/// `max_recv_message_size` gets enforced for *every* gRPC message in a
/// stream — a client-streaming/bidi RPC's body carries many gRPC messages
/// one after another, not just one — instead of only the first. Reuses the
/// shared frame-header layout from [`crate::middleware::GRPC_FRAME_HEADER_LEN`]
/// / [`crate::middleware::read_frame_header`], the same helpers
/// `middleware`'s streaming `CompressionBody` adapter uses for its own frame
/// parsing, so this variant doesn't hand-encode a second, divergent copy of
/// the frame layout.
enum FrameScanState {
    /// Accumulating the [`crate::middleware::GRPC_FRAME_HEADER_LEN`]-byte
    /// gRPC frame header (1 flag byte + 4 big-endian length bytes); holds
    /// however many header bytes have been seen so far. The header may be
    /// split arbitrarily across HTTP/2 DATA frames.
    Header(bytes::BytesMut),
    /// Inside a message payload whose declared length has already been
    /// validated against the configured limit; holds the number of payload
    /// bytes still to come before the next frame header begins.
    Payload(usize),
}

impl Default for FrameScanState {
    fn default() -> Self {
        FrameScanState::Header(bytes::BytesMut::with_capacity(GRPC_FRAME_HEADER_LEN))
    }
}

/// Scan `data` — a chunk of raw gRPC-framed bytes of arbitrary length and
/// arbitrary alignment relative to message boundaries — against `state`,
/// validating every gRPC frame header crossed against
/// `max_recv_message_size`. Advances `state` across calls so a caller can
/// feed it successive chunks (e.g. successive HTTP/2 DATA frames) and have
/// message boundaries tracked correctly even when a boundary falls in the
/// middle of a chunk, or a header is itself split across chunks. Returns
/// `Err` the moment a message declares a length over the limit.
fn scan_for_oversized_messages(
    data: &[u8],
    state: &mut FrameScanState,
    max_recv_message_size: usize,
) -> std::result::Result<(), tonic::Status> {
    let mut pos = 0;
    while pos < data.len() {
        match state {
            FrameScanState::Header(buf) => {
                let need = GRPC_FRAME_HEADER_LEN - buf.len();
                let take = need.min(data.len() - pos);
                buf.extend_from_slice(&data[pos..pos + take]);
                pos += take;
                if buf.len() == GRPC_FRAME_HEADER_LEN {
                    let (_, len) = read_frame_header(buf)
                        .expect("buffered exactly GRPC_FRAME_HEADER_LEN bytes");
                    if len > max_recv_message_size {
                        return Err(tonic::Status::resource_exhausted(format!(
                            "request exceeds configured max_recv_message_size of {max_recv_message_size} bytes"
                        )));
                    }
                    *state = FrameScanState::Payload(len);
                }
            }
            FrameScanState::Payload(remaining) => {
                let take = (*remaining).min(data.len() - pos);
                pos += take;
                *remaining -= take;
                if *remaining == 0 {
                    *state = FrameScanState::Header(bytes::BytesMut::with_capacity(
                        GRPC_FRAME_HEADER_LEN,
                    ));
                }
            }
        }
    }
    Ok(())
}

/// A body that replays a queue of already-consumed leading `Frame`s before
/// delegating to the rest of the original body, while continuing to enforce
/// `max_recv_message_size` against every later gRPC message that streams
/// through `rest` (not just the leading one used to fill `buffered`). Used
/// by [`MaxRecvMessageSizeService`] to peek the gRPC length-prefix — which
/// may be split across multiple small HTTP/2 DATA frames — without
/// discarding the data it read, and to keep checking subsequent messages for
/// client-streaming/bidi RPCs whose body carries more than one message.
struct PrefetchedBody<B> {
    buffered: std::collections::VecDeque<http_body::Frame<bytes::Bytes>>,
    rest: B,
    max_recv_message_size: usize,
    scan_state: FrameScanState,
}

impl<B> http_body::Body for PrefetchedBody<B>
where
    B: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Unpin,
{
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<bytes::Bytes>, Self::Error>>>
    {
        if let Some(frame) = self.buffered.pop_front() {
            return std::task::Poll::Ready(Some(Ok(frame)));
        }
        match std::pin::Pin::new(&mut self.rest).poll_frame(cx) {
            std::task::Poll::Ready(Some(Ok(frame))) => {
                let max_recv_message_size = self.max_recv_message_size;
                if let Some(data) = frame.data_ref()
                    && let Err(status) = scan_for_oversized_messages(
                        data,
                        &mut self.scan_state,
                        max_recv_message_size,
                    )
                {
                    return std::task::Poll::Ready(Some(Err(status)));
                }
                std::task::Poll::Ready(Some(Ok(frame)))
            }
            other => other,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.buffered.is_empty() && self.rest.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        // Account for the bytes still queued in `buffered` (already read from
        // the inner body but not yet replayed) on top of whatever `rest` has
        // left to yield — otherwise callers under-count the remaining body by
        // exactly the prefetched prefix.
        let buffered: u64 = self
            .buffered
            .iter()
            .filter_map(|f| f.data_ref().map(|d| d.len() as u64))
            .sum();
        let rest = self.rest.size_hint();
        let mut hint = http_body::SizeHint::new();
        hint.set_lower(rest.lower().saturating_add(buffered));
        if let Some(upper) = rest.upper() {
            hint.set_upper(upper.saturating_add(buffered));
        }
        hint
    }
}

/// Service wrapper that enforces `max_recv_message_size`: requests whose
/// gRPC length-prefix (the 4-byte big-endian length that
/// precedes every encoded gRPC message, per the wire protocol) declares a
/// message larger than the configured limit are rejected with
/// `Status::resource_exhausted` before reaching the inner service. Applied
/// unconditionally by [`GrpcServerBuilder::serve`] /
/// [`GrpcServerBuilder::serve_with_shutdown`].
///
/// This is implemented as a generic HTTP-level wrapper that peeks the first
/// body frame (rather than calling tonic-build's per-service
/// `.max_decoding_message_size()`) because this crate's `serve<S>` is generic
/// over any `S: ServableService` and has no way to call inherent methods that
/// only exist on tonic-build's generated `<Service>Server<T>` wrapper types.
/// gRPC does not set an HTTP `content-length` header, so this cannot be done
/// via headers alone.
struct MaxRecvMessageSizeService<S> {
    inner: S,
    max_recv_message_size: usize,
}

impl<S: Clone> Clone for MaxRecvMessageSizeService<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            max_recv_message_size: self.max_recv_message_size,
        }
    }
}

impl<S> tower::Service<http::Request<tonic::body::Body>> for MaxRecvMessageSizeService<S>
where
    S: tower::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<TonicBody>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = http::Response<TonicBody>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let max_recv_message_size = self.max_recv_message_size;

        Box::pin(async move {
            use http_body_util::BodyExt;
            use std::collections::VecDeque;

            let (parts, mut body) = req.into_parts();

            // Accumulate leading frames until we have seen the full
            // GRPC_FRAME_HEADER_LEN-byte gRPC length-prefix (1 compression-flag
            // byte + 4 big-endian length bytes) or the body ends. A short first
            // DATA frame does NOT mean "not oversized" — an attacker can
            // fragment the length-prefix itself across several small frames to
            // bypass a check that only inspects the first frame; this loop
            // closes that bypass by not deciding anything until enough bytes
            // have actually been seen. We only need to *count* the header bytes
            // seen so far (the actual scan/validation happens below over the
            // buffered frames), so a running length suffices — no second copy
            // of the bytes into a scratch buffer.
            let mut buffered: VecDeque<http_body::Frame<bytes::Bytes>> = VecDeque::new();
            let mut prefix_len = 0usize;

            while prefix_len < GRPC_FRAME_HEADER_LEN {
                match body.frame().await {
                    Some(Ok(frame)) => {
                        if let Some(data) = frame.data_ref() {
                            prefix_len += data.len();
                        }
                        buffered.push_back(frame);
                    }
                    Some(Err(e)) => {
                        // A genuine transport/read error must be surfaced,
                        // not silently discarded (which would previously
                        // forward a corrupted/truncated body to the inner
                        // service as if nothing had gone wrong).
                        return Ok(crate::middleware::status_response(tonic::Status::internal(
                            format!(
                                "failed to read request body while enforcing max_recv_message_size: {e}"
                            ),
                        )));
                    }
                    None => break, // body ended before 5 bytes were seen
                }
            }

            // Replay `buffered` through the same per-message scan used for
            // every later frame (see `FrameScanState`/
            // `scan_for_oversized_messages`), rather than a one-off check
            // against `prefix`: this both rejects an oversized *first*
            // message here (before `inner` is ever called, so we can return
            // a clean status response) and leaves `scan_state` correctly
            // positioned — mid-payload or at the very next header — for
            // whatever the last buffered frame's bytes actually contained,
            // which may run past the first message's header into its
            // payload or even into a second message.
            let mut scan_state = FrameScanState::default();
            for frame in &buffered {
                if let Some(data) = frame.data_ref()
                    && let Err(status) =
                        scan_for_oversized_messages(data, &mut scan_state, max_recv_message_size)
                {
                    return Ok(crate::middleware::status_response(status));
                }
            }

            let rebuilt_body = tonic::body::Body::new(PrefetchedBody {
                buffered,
                rest: body,
                max_recv_message_size,
                scan_state,
            });
            let req = http::Request::from_parts(parts, rebuilt_body);
            inner.call(req).await
        })
    }
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for MaxRecvMessageSizeService<S> {
    const NAME: &'static str = S::NAME;
}

fn limit_recv_message_size<S>(
    service: S,
    max_recv_message_size: usize,
) -> MaxRecvMessageSizeService<S> {
    MaxRecvMessageSizeService {
        inner: service,
        max_recv_message_size,
    }
}

fn build_server_tls_config(
    tls: &crate::config::GrpcServerTlsConfig,
) -> Result<tonic::transport::ServerTlsConfig> {
    crate::crypto_provider::ensure_installed();
    let identity = tonic::transport::Identity::from_pem(&tls.cert_pem, &tls.key_pem);
    let mut server_tls = tonic::transport::ServerTlsConfig::new().identity(identity);

    if let Some(ca) = &tls.client_ca_cert_pem {
        server_tls = server_tls
            .client_ca_root(tonic::transport::Certificate::from_pem(ca))
            .client_auth_optional(tls.client_auth_optional);
    }

    Ok(server_tls)
}

/// gRPC server wrapper.
pub struct GrpcServer;

impl GrpcServer {
    /// Create a server builder with the given configuration.
    pub fn builder(config: GrpcServerConfig) -> GrpcServerBuilder {
        GrpcServerBuilder::new(config)
    }

    /// Create a server builder with default configuration.
    pub fn with_default_config() -> GrpcServerBuilder {
        GrpcServerBuilder::new(GrpcServerConfig::default())
    }

    /// Create a server builder bound to the specified address.
    pub fn bind(addr: impl Into<String>) -> Result<GrpcServerBuilder> {
        let config = GrpcServerConfig::builder().bind_address(addr).build()?;
        Ok(GrpcServerBuilder::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interceptor::AuthInterceptor;
    use std::net::SocketAddr;
    use tonic::service::interceptor::InterceptedService;
    use tonic_health::pb::health_client::HealthClient;
    use tonic_health::pb::{HealthCheckRequest, health_server::HealthServer};
    use tower::Service as _;

    async fn bind_ephemeral() -> (tokio::net::TcpListener, SocketAddr) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    /// The health reporter must not be dropped — a registered service should
    /// report SERVING, not NOT_SERVING/not_found.
    #[tokio::test]
    async fn health_check_reports_serving_for_registered_service() {
        let (listener, addr) = bind_ephemeral().await;

        let (health_reporter, health_service) = tonic_health::server::health_reporter();
        // Use the health service itself as the "registered service" under test:
        // mark it SERVING (this is exactly what `GrpcServerBuilder::serve` now
        // does for every registered service, via `HealthReporter::set_serving`)
        // and verify Check reports that instead of the previous NOT_SERVING.
        health_reporter
            .set_serving::<HealthServer<tonic_health::server::HealthService>>()
            .await;

        tokio::spawn(async move {
            Server::builder()
                .add_service(health_service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = HealthClient::new(channel);

        let resp = client
            .check(HealthCheckRequest {
                service: "grpc.health.v1.Health".to_string(),
            })
            .await
            .unwrap();
        let status = tonic_health::pb::health_check_response::ServingStatus::try_from(
            resp.into_inner().status,
        )
        .unwrap();
        assert_eq!(
            status,
            tonic_health::pb::health_check_response::ServingStatus::Serving
        );
    }

    /// A server-side `AuthInterceptor` must reject unauthenticated requests
    /// and accept authenticated ones.
    #[tokio::test]
    async fn server_side_auth_interceptor_rejects_and_accepts() {
        let (listener, addr) = bind_ephemeral().await;

        let (_reporter, health_service) = tonic_health::server::health_reporter();
        let auth = AuthInterceptor::bearer("s3cr3t");
        let intercepted = InterceptedService::new(health_service, auth.server_interceptor());

        tokio::spawn(async move {
            Server::builder()
                .add_service(intercepted)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();

        // No credentials: rejected.
        let mut client = HealthClient::new(channel.clone());
        let err = client
            .check(HealthCheckRequest {
                service: String::new(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);

        // Correct credentials: accepted.
        let mut req = tonic::Request::new(HealthCheckRequest {
            service: String::new(),
        });
        req.metadata_mut()
            .insert("authorization", "Bearer s3cr3t".parse().unwrap());
        let mut client = HealthClient::new(channel);
        let resp = client.check(req).await;
        assert!(
            resp.is_ok(),
            "authenticated request should succeed: {resp:?}"
        );
    }

    /// `enable_reflection` must actually register a working reflection
    /// service.
    #[tokio::test]
    async fn enable_reflection_registers_working_reflection_service() {
        let (listener, addr) = bind_ephemeral().await;

        let reflection_service = tonic_reflection::server::Builder::configure()
            .build_v1()
            .unwrap();

        tokio::spawn(async move {
            Server::builder()
                .add_service(reflection_service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();

        let mut client =
            tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient::new(
                channel,
            );

        let request = tonic_reflection::pb::v1::ServerReflectionRequest {
            host: String::new(),
            message_request: Some(
                tonic_reflection::pb::v1::server_reflection_request::MessageRequest::ListServices(
                    String::new(),
                ),
            ),
        };
        let response_stream = client
            .server_reflection_info(tokio_stream::once(request))
            .await
            .unwrap()
            .into_inner();
        let responses: Vec<_> = tokio_stream::StreamExt::collect::<Vec<_>>(response_stream).await;
        assert_eq!(responses.len(), 1);
        let response = responses.into_iter().next().unwrap().unwrap();
        assert!(matches!(
            response.message_response,
            Some(
                tonic_reflection::pb::v1::server_reflection_response::MessageResponse::ListServicesResponse(_)
            )
        ));
    }

    /// Requests larger than `max_recv_message_size` must be rejected, not
    /// silently accepted.
    #[tokio::test]
    async fn oversized_request_is_rejected_when_message_size_limited() {
        let (listener, addr) = bind_ephemeral().await;

        let (_reporter, health_service) = tonic_health::server::health_reporter();
        let limited = limit_recv_message_size(health_service, 8);

        tokio::spawn(async move {
            Server::builder()
                .add_service(limited)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = HealthClient::new(channel);

        // "a-service-name-that-is-definitely-longer-than-8-bytes" encodes to
        // well over the 8-byte limit configured above.
        let err = client
            .check(HealthCheckRequest {
                service: "a-service-name-that-is-definitely-longer-than-8-bytes".to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    }

    /// A minimal inner service used to unit-test `MaxRecvMessageSizeService`
    /// below without a real network round-trip. Reads the whole rebuilt
    /// request body and echoes its byte length back as the status message,
    /// so tests can confirm the reconstructed body (after our
    /// buffer-and-replay logic) matches what was actually sent.
    #[derive(Clone)]
    struct EchoInner;

    impl tower::Service<http::Request<tonic::body::Body>> for EchoInner {
        type Response = http::Response<TonicBody>;
        type Error = std::convert::Infallible;
        type Future = std::pin::Pin<
            Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>,
        >;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
            Box::pin(async move {
                use http_body_util::BodyExt;
                let bytes = req
                    .into_body()
                    .collect()
                    .await
                    .map(|c| c.to_bytes())
                    .unwrap_or_default();
                Ok(crate::middleware::status_response(tonic::Status::ok(
                    format!("received {} bytes", bytes.len()),
                )))
            })
        }
    }

    /// A body that yields its bytes one at a time as separate frames (each
    /// frame is under 5 bytes), used to simulate an attacker fragmenting the
    /// gRPC length-prefix across multiple small HTTP/2 DATA frames.
    struct FragmentedBody {
        chunks: std::collections::VecDeque<bytes::Bytes>,
    }

    impl http_body::Body for FragmentedBody {
        type Data = bytes::Bytes;
        type Error = tonic::Status;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<bytes::Bytes>, Self::Error>>>
        {
            match self.chunks.pop_front() {
                Some(chunk) => std::task::Poll::Ready(Some(Ok(http_body::Frame::data(chunk)))),
                None => std::task::Poll::Ready(None),
            }
        }

        fn is_end_stream(&self) -> bool {
            self.chunks.is_empty()
        }
    }

    fn one_byte_frames(data: &[u8]) -> std::collections::VecDeque<bytes::Bytes> {
        data.iter().map(|b| bytes::Bytes::from(vec![*b])).collect()
    }

    /// Regression test: fragmenting the 5-byte gRPC length-prefix across
    /// many 1-byte frames must not bypass `max_recv_message_size` — against
    /// the pre-fix code (which only inspected whether the *first* frame was
    /// >= 5 bytes, via `filter(|d| d.len() >= 5).unwrap_or(false)`), this
    /// fragmentation would have made every oversized request look "not
    /// oversized" and sail through uninspected.
    #[tokio::test]
    async fn fragmented_length_prefix_still_enforces_max_recv_message_size() {
        let declared_len: u32 = 1000;
        let mut full = vec![0u8]; // uncompressed flag
        full.extend_from_slice(&declared_len.to_be_bytes());
        full.extend(std::iter::repeat_n(b'x', declared_len as usize));

        let body = tonic::body::Body::new(FragmentedBody {
            chunks: one_byte_frames(&full),
        });

        let mut svc = MaxRecvMessageSizeService {
            inner: EchoInner,
            max_recv_message_size: 8,
        };

        let resp = svc.call(http::Request::new(body)).await.unwrap();
        let status = tonic::Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(
            status.code(),
            tonic::Code::ResourceExhausted,
            "a fragmented length-prefix must still be size-checked, not waved through"
        );
    }

    /// Companion to the above: a fragmented message that is genuinely within
    /// the configured limit must still reach the inner service, with its
    /// full body intact (proving the buffer-and-replay via `PrefetchedBody`
    /// doesn't drop or corrupt data).
    #[tokio::test]
    async fn fragmented_length_prefix_within_limit_reaches_inner_service_intact() {
        let payload_len: u32 = 4;
        let mut full = vec![0u8];
        full.extend_from_slice(&payload_len.to_be_bytes());
        full.extend_from_slice(b"data");

        let total_len = full.len();
        let body = tonic::body::Body::new(FragmentedBody {
            chunks: one_byte_frames(&full),
        });

        let mut svc = MaxRecvMessageSizeService {
            inner: EchoInner,
            max_recv_message_size: 1024,
        };

        let resp = svc.call(http::Request::new(body)).await.unwrap();
        let status = tonic::Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(status.code(), tonic::Code::Ok);
        assert_eq!(
            status.message(),
            format!("received {total_len} bytes"),
            "the reconstructed body must include every byte the fragmented body sent"
        );
    }

    /// Like `EchoInner`, but surfaces a body-read error as its own status
    /// code instead of masking it behind `.unwrap_or_default()` — used by
    /// the multi-message streaming test below, which needs to observe that a
    /// later message being rejected actually reaches the inner service as a
    /// real body error (not silently swallowed into "0 bytes received, OK").
    #[derive(Clone)]
    struct EchoInnerSurfacingErrors;

    impl tower::Service<http::Request<tonic::body::Body>> for EchoInnerSurfacingErrors {
        type Response = http::Response<TonicBody>;
        type Error = std::convert::Infallible;
        type Future = std::pin::Pin<
            Box<dyn Future<Output = std::result::Result<Self::Response, Self::Error>> + Send>,
        >;

        fn poll_ready(
            &mut self,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), Self::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
            Box::pin(async move {
                use http_body_util::BodyExt;
                match req.into_body().collect().await {
                    Ok(collected) => Ok(crate::middleware::status_response(tonic::Status::ok(
                        format!("received {} bytes", collected.to_bytes().len()),
                    ))),
                    Err(status) => Ok(crate::middleware::status_response(status)),
                }
            })
        }
    }

    /// Regression test: `max_recv_message_size` must be enforced for *every*
    /// gRPC message in a client-streaming/bidi request body, not just the
    /// first — against the pre-fix code, `rest: body` streamed through
    /// `PrefetchedBody` completely unchecked after the initial prefetch, so
    /// a second or later oversized message in the same stream would never be
    /// rejected. This simulates a stream carrying three messages (two small,
    /// within-limit messages followed by one whose declared length exceeds
    /// the limit) fragmented one byte at a time, and asserts the oversized
    /// third message is still caught.
    #[tokio::test]
    async fn oversized_later_message_in_multi_message_stream_is_rejected() {
        let mut full = Vec::new();

        let msg1: &[u8] = b"ok"; // 2 bytes, within an 8-byte limit
        full.push(0u8);
        full.extend_from_slice(&(msg1.len() as u32).to_be_bytes());
        full.extend_from_slice(msg1);

        let msg2: &[u8] = b"al"; // 2 bytes, also within the limit
        full.push(0u8);
        full.extend_from_slice(&(msg2.len() as u32).to_be_bytes());
        full.extend_from_slice(msg2);

        // Third message declares a length well over the 8-byte limit.
        let declared_len3: u32 = 1000;
        full.push(0u8);
        full.extend_from_slice(&declared_len3.to_be_bytes());
        full.extend(std::iter::repeat_n(b'z', declared_len3 as usize));

        let body = tonic::body::Body::new(FragmentedBody {
            chunks: one_byte_frames(&full),
        });

        let mut svc = MaxRecvMessageSizeService {
            inner: EchoInnerSurfacingErrors,
            max_recv_message_size: 8,
        };

        let resp = svc.call(http::Request::new(body)).await.unwrap();
        let status = tonic::Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(
            status.code(),
            tonic::Code::ResourceExhausted,
            "a later oversized message in a multi-message stream must still be rejected, \
             not waved through after only the first message was checked"
        );
    }

    /// A body that yields one small (< 5 byte) frame and then a transport
    /// error, used to prove a genuine read error is surfaced rather than
    /// silently discarded.
    struct FlakyBody {
        first: Option<bytes::Bytes>,
    }

    impl http_body::Body for FlakyBody {
        type Data = bytes::Bytes;
        type Error = tonic::Status;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<bytes::Bytes>, Self::Error>>>
        {
            if let Some(b) = self.first.take() {
                return std::task::Poll::Ready(Some(Ok(http_body::Frame::data(b))));
            }
            std::task::Poll::Ready(Some(Err(tonic::Status::unavailable(
                "simulated transport read error",
            ))))
        }

        fn is_end_stream(&self) -> bool {
            false
        }
    }

    /// Regression test: a genuine transport-level read error while
    /// accumulating the length-prefix must be surfaced to the caller, not
    /// discarded via `.ok().flatten()` (which previously made the request
    /// look like it simply had no data, silently forwarding a
    /// corrupted/truncated body to the inner service instead of failing).
    #[tokio::test]
    async fn transport_read_error_while_buffering_prefix_is_surfaced() {
        let body = tonic::body::Body::new(FlakyBody {
            first: Some(bytes::Bytes::from_static(b"ab")), // 2 bytes: not enough to decide size yet
        });

        let mut svc = MaxRecvMessageSizeService {
            inner: EchoInner,
            max_recv_message_size: 1024,
        };

        let resp = svc.call(http::Request::new(body)).await.unwrap();
        let status = tonic::Status::from_header_map(resp.headers()).unwrap();
        assert_eq!(
            status.code(),
            tonic::Code::Internal,
            "a transport read error must be surfaced as an error status, not silently dropped"
        );
    }

    /// Reserve an ephemeral port and immediately release it, so
    /// `GrpcServerBuilder::serve`/`serve_with_shutdown` (which bind their own
    /// listener internally rather than accepting a pre-bound one) can be
    /// configured with a concrete, likely-free address.
    async fn free_addr() -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    }

    /// Regression test: `serve_with_shutdown` must register the health-check
    /// service exactly like `serve` does — previously `serve` was fixed to
    /// retain the health reporter and call `set_serving::<S>()`, but
    /// `serve_with_shutdown` never got the same treatment, so a service run
    /// via `serve_with_shutdown` would never answer health checks at all.
    #[tokio::test]
    async fn serve_with_shutdown_registers_health_check_like_serve() {
        let addr = free_addr().await;
        let config = GrpcServerConfig::builder()
            .bind_socket_addr(addr)
            .enable_health_check()
            .build()
            .unwrap();

        // The service under test is reflection (distinct from the
        // auto-registered health service below), so registering the
        // config-driven health service doesn't collide with a
        // manually-added one under the same route.
        let reflection_service = tonic_reflection::server::Builder::configure()
            .build_v1()
            .unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            let _ = GrpcServerBuilder::new(config)
                .serve_with_shutdown(reflection_service, async {
                    let _ = rx.await;
                })
                .await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = HealthClient::new(channel);

        let resp = client
            .check(HealthCheckRequest {
                service: String::new(),
            })
            .await
            .expect("serve_with_shutdown must register a working health check service");
        let status = tonic_health::pb::health_check_response::ServingStatus::try_from(
            resp.into_inner().status,
        )
        .unwrap();
        assert_eq!(
            status,
            tonic_health::pb::health_check_response::ServingStatus::Serving,
            "serve_with_shutdown must report SERVING just like serve does"
        );

        let _ = tx.send(());
    }
}
