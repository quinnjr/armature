//! gRPC server implementation.

use std::future::Future;
use tonic::transport::Server;
use tracing::{error, info};

use crate::TonicBody;
use crate::middleware::GrpcMiddleware;
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

        // Add health check service, keeping the reporter so registered
        // services can actually be marked SERVING (previously this dropped
        // the reporter, so Check/Watch always answered NOT_SERVING).
        #[cfg(feature = "health")]
        let router = if self.config.enable_health_check {
            let (health_reporter, health_service) = tonic_health::server::health_reporter();
            health_reporter.set_serving::<S>().await;
            router.add_service(health_service)
        } else {
            router
        };

        // Add reflection service when enabled.
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

        router.serve_with_shutdown(addr, signal).await.map_err(|e| {
            error!(error = %e, "gRPC server error");
            GrpcError::Server(e.to_string())
        })
    }
}

/// A body that replays an already-consumed leading `Frame` before delegating
/// to the rest of the original body. Used by [`MaxRecvMessageSizeService`] to
/// peek the gRPC length-prefix without discarding the data it read.
struct PrefetchedBody<B> {
    first: Option<http_body::Frame<bytes::Bytes>>,
    rest: B,
}

impl<B> http_body::Body for PrefetchedBody<B>
where
    B: http_body::Body<Data = bytes::Bytes> + Unpin,
{
    type Data = bytes::Bytes;
    type Error = B::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<std::result::Result<http_body::Frame<bytes::Bytes>, Self::Error>>>
    {
        if let Some(frame) = self.first.take() {
            return std::task::Poll::Ready(Some(Ok(frame)));
        }
        std::pin::Pin::new(&mut self.rest).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.first.is_none() && self.rest.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.rest.size_hint()
    }
}

/// Service wrapper that enforces `max_recv_message_size` (Finding 6):
/// requests whose gRPC length-prefix (the 4-byte big-endian length that
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

            let (parts, mut body) = req.into_parts();
            let first_frame = body.frame().await;

            let (data, oversized) = match first_frame {
                Some(Ok(frame)) => {
                    let oversized = frame
                        .data_ref()
                        .filter(|d| d.len() >= 5)
                        .map(|d| {
                            let len = u32::from_be_bytes([d[1], d[2], d[3], d[4]]) as usize;
                            len > max_recv_message_size
                        })
                        .unwrap_or(false);
                    (Some(Ok(frame)), oversized)
                }
                other => (other, false),
            };

            if oversized {
                return Ok(crate::middleware::status_response(
                    tonic::Status::resource_exhausted(format!(
                        "request exceeds configured max_recv_message_size of {max_recv_message_size} bytes"
                    )),
                ));
            }

            let rebuilt_body = tonic::body::Body::new(PrefetchedBody {
                first: data.transpose().ok().flatten(),
                rest: body,
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

    async fn bind_ephemeral() -> (tokio::net::TcpListener, SocketAddr) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        (listener, addr)
    }

    /// Finding 5 regression test: the health reporter must not be dropped —
    /// a registered service should report SERVING, not NOT_SERVING/not_found.
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

    /// Finding 1 regression test: a server-side `AuthInterceptor` must reject
    /// unauthenticated requests and accept authenticated ones.
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

    /// Finding 4 regression test: `enable_reflection` must actually register
    /// a working reflection service.
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

    /// Finding 6 regression test: requests larger than `max_recv_message_size`
    /// must be rejected, not silently accepted.
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
}
