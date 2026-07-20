//! gRPC client implementation.

use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, info};

use crate::{GrpcClientConfig, GrpcError, Result};

/// Apply this config's TLS settings (if any) to an `Endpoint`. TLS support is
/// always compiled in (`tonic`'s `tls-ring` feature is enabled unconditionally
/// in this crate's `Cargo.toml`), so this simply no-ops when `config.tls` is
/// `None`.
fn apply_tls(endpoint: Endpoint, config: &GrpcClientConfig) -> Result<Endpoint> {
    let Some(tls) = &config.tls else {
        return Ok(endpoint);
    };
    crate::crypto_provider::ensure_installed();

    let mut tls_config = tonic::transport::ClientTlsConfig::new();
    if let Some(ca) = &tls.ca_cert_pem {
        tls_config = tls_config.ca_certificate(tonic::transport::Certificate::from_pem(ca));
    }
    if let Some(domain) = &tls.domain_name {
        tls_config = tls_config.domain_name(domain.clone());
    }
    if let (Some(cert), Some(key)) = (&tls.client_cert_pem, &tls.client_key_pem) {
        tls_config = tls_config.identity(tonic::transport::Identity::from_pem(cert, key));
    }

    endpoint
        .tls_config(tls_config)
        .map_err(GrpcError::Transport)
}

/// gRPC channel wrapper with configuration.
#[derive(Clone)]
pub struct GrpcChannel {
    inner: Channel,
    config: GrpcClientConfig,
}

impl GrpcChannel {
    /// Get the inner tonic channel.
    pub fn inner(&self) -> &Channel {
        &self.inner
    }

    /// Get the configuration.
    pub fn config(&self) -> &GrpcClientConfig {
        &self.config
    }

    /// Execute a gRPC call with retry, honoring this channel's
    /// `retry_enabled` / `max_retry_attempts` configuration (Finding 8).
    ///
    /// `make_call` is invoked (and re-invoked on a retriable failure) to
    /// issue the RPC — typically a closure calling a generated client method
    /// against `self.inner().clone()`. Retries only on transport-level or
    /// retriable status codes (`Unavailable`, `ResourceExhausted`, `Aborted`,
    /// `DeadlineExceeded`); other errors are returned immediately.
    ///
    /// ```rust,ignore
    /// let channel = GrpcClient::connect(config).await?;
    /// let resp = channel
    ///     .call_with_retry(|| {
    ///         let mut client = GreeterClient::new(channel.inner().clone());
    ///         async move { client.say_hello(request.clone()).await }
    ///     })
    ///     .await?;
    /// ```
    pub async fn call_with_retry<F, Fut, T>(
        &self,
        mut make_call: F,
    ) -> std::result::Result<T, tonic::Status>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<T, tonic::Status>>,
    {
        if !self.config.retry_enabled {
            return make_call().await;
        }

        let retry = crate::middleware::RetryMiddleware::new(self.config.max_retry_attempts.max(1));
        retry.call_with_retry(make_call).await
    }
}

impl std::ops::Deref for GrpcChannel {
    type Target = Channel;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// gRPC client builder and connector.
pub struct GrpcClient;

impl GrpcClient {
    /// Connect to a gRPC server with the given configuration.
    pub async fn connect(config: GrpcClientConfig) -> Result<GrpcChannel> {
        info!(endpoint = %config.endpoint, "Connecting to gRPC server");

        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| GrpcError::Config(e.to_string()))?
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .tcp_nodelay(config.tcp_nodelay)
            .tcp_keepalive(config.tcp_keepalive);

        if let Some(interval) = config.http2_keepalive_interval {
            endpoint = endpoint.http2_keep_alive_interval(interval);
        }
        if let Some(timeout) = config.http2_keepalive_timeout {
            endpoint = endpoint.keep_alive_timeout(timeout);
        }
        if let Some(window) = config.initial_connection_window_size {
            endpoint = endpoint.initial_connection_window_size(window);
        }
        if let Some(window) = config.initial_stream_window_size {
            endpoint = endpoint.initial_stream_window_size(window);
        }
        if let Some(limit) = config.concurrency_limit {
            endpoint = endpoint.concurrency_limit(limit);
        }
        if let Some(rps) = config.rate_limit {
            endpoint = endpoint.rate_limit(rps, Duration::from_secs(1));
        }
        endpoint = apply_tls(endpoint, &config)?;

        let channel = endpoint.connect().await.map_err(GrpcError::Transport)?;

        debug!("gRPC client connected");

        Ok(GrpcChannel {
            inner: channel,
            config,
        })
    }

    /// Connect to a gRPC server with default configuration.
    pub async fn connect_default(endpoint: impl Into<String>) -> Result<GrpcChannel> {
        let config = GrpcClientConfig::builder().endpoint(endpoint).build();
        Self::connect(config).await
    }

    /// Create a lazy channel that connects on first use.
    pub fn lazy(config: GrpcClientConfig) -> Result<GrpcChannel> {
        info!(endpoint = %config.endpoint, "Creating lazy gRPC channel");

        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|e| GrpcError::Config(e.to_string()))?
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .tcp_nodelay(config.tcp_nodelay)
            .tcp_keepalive(config.tcp_keepalive);

        if let Some(interval) = config.http2_keepalive_interval {
            endpoint = endpoint.http2_keep_alive_interval(interval);
        }
        if let Some(timeout) = config.http2_keepalive_timeout {
            endpoint = endpoint.keep_alive_timeout(timeout);
        }
        if let Some(limit) = config.concurrency_limit {
            endpoint = endpoint.concurrency_limit(limit);
        }
        endpoint = apply_tls(endpoint, &config)?;

        let channel = endpoint.connect_lazy();

        Ok(GrpcChannel {
            inner: channel,
            config,
        })
    }

    /// Create a channel with load balancing across multiple endpoints.
    pub async fn connect_balanced(
        endpoints: Vec<String>,
        config: GrpcClientConfig,
    ) -> Result<GrpcChannel> {
        info!(
            endpoints = ?endpoints,
            "Creating load-balanced gRPC channel"
        );

        let endpoints: Vec<Endpoint> = endpoints
            .into_iter()
            .map(|ep| {
                Endpoint::from_shared(ep).map(|e| {
                    e.timeout(config.timeout)
                        .connect_timeout(config.connect_timeout)
                        .tcp_nodelay(config.tcp_nodelay)
                        .tcp_keepalive(config.tcp_keepalive)
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| GrpcError::Config(e.to_string()))?;
        let endpoints: Vec<Endpoint> = endpoints
            .into_iter()
            .map(|e| apply_tls(e, &config))
            .collect::<Result<Vec<_>>>()?;

        let channel = Channel::balance_list(endpoints.into_iter());

        Ok(GrpcChannel {
            inner: channel,
            config,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config() {
        let config = GrpcClientConfig::builder()
            .endpoint("http://localhost:50051")
            .timeout(Duration::from_secs(60))
            .build();

        assert_eq!(config.endpoint, "http://localhost:50051");
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    /// Finding 7 regression test: `tcp_keepalive` is a documented, builder-set
    /// config field previously read by neither client nor server. A pure
    /// unit assertion on socket-level keepalive isn't practical from a Tower
    /// `Endpoint` (tonic doesn't expose it for introspection), so this is a
    /// behavioral smoke test: a client built with `tcp_keepalive` configured
    /// must actually apply it via `Endpoint::tcp_keepalive` without erroring,
    /// and must still be able to connect and complete an RPC end-to-end.
    #[tokio::test]
    async fn client_with_tcp_keepalive_connects_and_completes_rpc() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (_reporter, health_service) = tonic_health::server::health_reporter();
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .tcp_keepalive(Some(Duration::from_secs(30)))
                .add_service(health_service)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let config = GrpcClientConfig::builder()
            .endpoint(format!("http://{addr}"))
            .tcp_keepalive(Duration::from_secs(30))
            .build();
        let channel = GrpcClient::connect(config).await.unwrap();

        let mut client =
            tonic_health::pb::health_client::HealthClient::new(channel.inner().clone());
        let resp = client
            .check(tonic_health::pb::HealthCheckRequest {
                service: String::new(),
            })
            .await;
        assert!(
            resp.is_ok(),
            "RPC over a tcp_keepalive-configured channel should succeed"
        );
    }
}
