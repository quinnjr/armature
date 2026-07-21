//! gRPC configuration types.

use std::net::SocketAddr;
use std::time::Duration;

use crate::error::GrpcError;

/// TLS configuration for the gRPC server (rustls-backed, via tonic's
/// `tls-ring` feature — no OpenSSL/native-tls).
#[derive(Clone)]
pub struct GrpcServerTlsConfig {
    /// PEM-encoded server certificate chain.
    pub cert_pem: Vec<u8>,
    /// PEM-encoded server private key.
    pub key_pem: Vec<u8>,
    /// PEM-encoded CA certificate used to verify client certificates (mTLS).
    /// When `None`, client certificates are not requested.
    pub client_ca_cert_pem: Option<Vec<u8>>,
    /// Whether client certificate auth is optional when `client_ca_cert_pem`
    /// is set. Defaults to `false` (client cert required for mTLS).
    pub client_auth_optional: bool,
}

impl std::fmt::Debug for GrpcServerTlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcServerTlsConfig")
            .field("cert_pem", &format_args!("<{} bytes>", self.cert_pem.len()))
            .field("key_pem", &format_args!("<{} bytes>", self.key_pem.len()))
            .field("client_ca_cert_pem", &self.client_ca_cert_pem.is_some())
            .field("client_auth_optional", &self.client_auth_optional)
            .finish()
    }
}

impl GrpcServerTlsConfig {
    /// Create a new server TLS configuration from PEM-encoded certificate and key.
    ///
    /// ```
    /// use armature_grpc::GrpcServerTlsConfig;
    ///
    /// let tls = GrpcServerTlsConfig::new(b"cert-pem".to_vec(), b"key-pem".to_vec())
    ///     .client_ca(b"ca-pem".to_vec())
    ///     .client_auth_optional(true);
    /// assert!(tls.client_ca_cert_pem.is_some());
    /// assert!(tls.client_auth_optional);
    /// ```
    pub fn new(cert_pem: impl Into<Vec<u8>>, key_pem: impl Into<Vec<u8>>) -> Self {
        Self {
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            client_ca_cert_pem: None,
            client_auth_optional: false,
        }
    }

    /// Require (or accept) client certificates signed by the given CA for mTLS.
    pub fn client_ca(mut self, ca_pem: impl Into<Vec<u8>>) -> Self {
        self.client_ca_cert_pem = Some(ca_pem.into());
        self
    }

    /// Make client certificate auth optional (only meaningful with `client_ca`).
    pub fn client_auth_optional(mut self, optional: bool) -> Self {
        self.client_auth_optional = optional;
        self
    }
}

/// TLS configuration for the gRPC client (rustls-backed, via tonic's
/// `tls-ring` feature — no OpenSSL/native-tls).
#[derive(Clone, Default)]
pub struct GrpcClientTlsConfig {
    /// PEM-encoded CA certificate(s) used to verify the server's certificate.
    /// When `None`, the platform/webpki default roots configured on the
    /// `tonic` build are used (if any `tls-*-roots` feature is enabled).
    pub ca_cert_pem: Option<Vec<u8>>,
    /// Domain name to verify against the server's certificate.
    pub domain_name: Option<String>,
    /// PEM-encoded client certificate, for mutual TLS.
    pub client_cert_pem: Option<Vec<u8>>,
    /// PEM-encoded client private key, for mutual TLS.
    pub client_key_pem: Option<Vec<u8>>,
}

impl std::fmt::Debug for GrpcClientTlsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcClientTlsConfig")
            .field("ca_cert_pem", &self.ca_cert_pem.is_some())
            .field("domain_name", &self.domain_name)
            .field("client_cert_pem", &self.client_cert_pem.is_some())
            .field("client_key_pem", &self.client_key_pem.is_some())
            .finish()
    }
}

impl GrpcClientTlsConfig {
    /// Create a new, empty client TLS configuration.
    ///
    /// ```
    /// use armature_grpc::GrpcClientTlsConfig;
    ///
    /// let tls = GrpcClientTlsConfig::new()
    ///     .ca_certificate(b"ca-pem".to_vec())
    ///     .domain_name("example.com")
    ///     .identity(b"client-cert".to_vec(), b"client-key".to_vec());
    /// assert!(tls.ca_cert_pem.is_some());
    /// assert_eq!(tls.domain_name.as_deref(), Some("example.com"));
    /// assert!(tls.client_cert_pem.is_some());
    /// assert!(tls.client_key_pem.is_some());
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify the server's certificate against this PEM-encoded CA.
    pub fn ca_certificate(mut self, ca_pem: impl Into<Vec<u8>>) -> Self {
        self.ca_cert_pem = Some(ca_pem.into());
        self
    }

    /// Verify the server's certificate against this domain name.
    pub fn domain_name(mut self, domain: impl Into<String>) -> Self {
        self.domain_name = Some(domain.into());
        self
    }

    /// Present this PEM-encoded client identity for mutual TLS.
    pub fn identity(mut self, cert_pem: impl Into<Vec<u8>>, key_pem: impl Into<Vec<u8>>) -> Self {
        self.client_cert_pem = Some(cert_pem.into());
        self.client_key_pem = Some(key_pem.into());
        self
    }
}

/// gRPC server configuration.
#[derive(Debug, Clone)]
pub struct GrpcServerConfig {
    /// Address to bind to.
    pub bind_address: SocketAddr,
    /// Maximum message size for receiving.
    pub max_recv_message_size: usize,
    /// Maximum message size for sending.
    ///
    /// **Currently unenforced.** This crate's generic `serve<S>` has no
    /// access to `.max_encoding_message_size()` (or similar codec setters) —
    /// those are inherent methods on tonic-build's generated
    /// `<Service>Server<T>` wrapper types, not something a generic wrapper
    /// service can call on an arbitrary `S`. Setting this field currently
    /// has no effect; if you need outgoing size enforcement, call the
    /// generated service's own setter directly (e.g.
    /// `MyServiceServer::new(impl).max_encoding_message_size(n)`).
    pub max_send_message_size: usize,
    /// Enable HTTP/2 keepalive.
    pub http2_keepalive_interval: Option<Duration>,
    /// HTTP/2 keepalive timeout.
    pub http2_keepalive_timeout: Option<Duration>,
    /// TCP keepalive.
    pub tcp_keepalive: Option<Duration>,
    /// TCP nodelay.
    pub tcp_nodelay: bool,
    /// Enable gRPC health checking service.
    pub enable_health_check: bool,
    /// Enable server reflection.
    pub enable_reflection: bool,
    /// Concurrency limit per connection.
    pub concurrency_limit_per_connection: Option<usize>,
    /// Initial connection window size.
    pub initial_connection_window_size: Option<u32>,
    /// Initial stream window size.
    pub initial_stream_window_size: Option<u32>,
    /// TLS configuration. When `None`, the server accepts plaintext connections.
    pub tls: Option<GrpcServerTlsConfig>,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:50051".parse().unwrap(),
            max_recv_message_size: 4 * 1024 * 1024, // 4 MB
            max_send_message_size: 4 * 1024 * 1024, // 4 MB
            http2_keepalive_interval: Some(Duration::from_secs(60)),
            http2_keepalive_timeout: Some(Duration::from_secs(20)),
            tcp_keepalive: Some(Duration::from_secs(60)),
            tcp_nodelay: true,
            enable_health_check: true,
            enable_reflection: false,
            concurrency_limit_per_connection: None,
            initial_connection_window_size: None,
            initial_stream_window_size: None,
            tls: None,
        }
    }
}

impl GrpcServerConfig {
    /// Create a new builder.
    pub fn builder() -> GrpcServerConfigBuilder {
        GrpcServerConfigBuilder::default()
    }
}

/// Builder for [`GrpcServerConfig`].
///
/// Construct one with [`GrpcServerConfig::builder`], chain the setter methods,
/// then call [`GrpcServerConfigBuilder::build`] (which validates the bind
/// address).
///
/// ```
/// use armature_grpc::GrpcServerConfig;
///
/// let config = GrpcServerConfig::builder()
///     .bind_address("0.0.0.0:50051")
///     .max_recv_message_size(8 * 1024 * 1024)
///     .build()
///     .expect("valid bind address");
/// ```
#[derive(Debug, Default)]
pub struct GrpcServerConfigBuilder {
    config: GrpcServerConfig,
    bind_address_error: Option<String>,
}

impl GrpcServerConfigBuilder {
    /// Set the bind address.
    ///
    /// A malformed address is not reported until [`GrpcServerConfigBuilder::build`]
    /// is called (returning `Err(GrpcError::Config(..))`) rather than panicking.
    pub fn bind_address(mut self, addr: impl Into<String>) -> Self {
        match addr.into().parse() {
            Ok(parsed) => {
                self.config.bind_address = parsed;
                // Clear any error from a prior malformed call: the final,
                // valid address wins.
                self.bind_address_error = None;
            }
            Err(e) => self.bind_address_error = Some(e.to_string()),
        }
        self
    }

    /// Set the bind address from a SocketAddr.
    pub fn bind_socket_addr(mut self, addr: SocketAddr) -> Self {
        self.config.bind_address = addr;
        // A `SocketAddr` is always valid, so this clears any error left by a
        // prior malformed `bind_address` call.
        self.bind_address_error = None;
        self
    }

    /// Set the maximum receive message size.
    pub fn max_recv_message_size(mut self, size: usize) -> Self {
        self.config.max_recv_message_size = size;
        self
    }

    /// Set the maximum send message size.
    ///
    /// **Currently unenforced** — see the doc comment on
    /// [`GrpcServerConfig::max_send_message_size`] for why: this crate's
    /// generic `serve<S>` has no way to call the codegen-only setter that
    /// would actually apply it.
    pub fn max_send_message_size(mut self, size: usize) -> Self {
        self.config.max_send_message_size = size;
        self
    }

    /// Enable HTTP/2 keepalive.
    pub fn http2_keepalive(mut self, interval: Duration, timeout: Duration) -> Self {
        self.config.http2_keepalive_interval = Some(interval);
        self.config.http2_keepalive_timeout = Some(timeout);
        self
    }

    /// Set TCP keepalive.
    pub fn tcp_keepalive(mut self, duration: Duration) -> Self {
        self.config.tcp_keepalive = Some(duration);
        self
    }

    /// Enable TCP nodelay.
    pub fn tcp_nodelay(mut self, enable: bool) -> Self {
        self.config.tcp_nodelay = enable;
        self
    }

    /// Enable gRPC health checking.
    pub fn enable_health_check(mut self) -> Self {
        self.config.enable_health_check = true;
        self
    }

    /// Enable server reflection.
    pub fn enable_reflection(mut self) -> Self {
        self.config.enable_reflection = true;
        self
    }

    /// Set concurrency limit per connection.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        self.config.concurrency_limit_per_connection = Some(limit);
        self
    }

    /// Configure TLS for the server. When set, the server serves over TLS
    /// exclusively (plaintext connections are refused).
    pub fn tls(mut self, tls: GrpcServerTlsConfig) -> Self {
        self.config.tls = Some(tls);
        self
    }

    /// Build the configuration.
    ///
    /// Returns `Err(GrpcError::Config(..))` if [`GrpcServerConfigBuilder::bind_address`]
    /// was given a malformed address.
    pub fn build(self) -> Result<GrpcServerConfig, GrpcError> {
        if let Some(err) = self.bind_address_error {
            return Err(GrpcError::Config(format!("invalid bind address: {err}")));
        }
        Ok(self.config)
    }
}

/// gRPC client configuration.
#[derive(Debug, Clone)]
pub struct GrpcClientConfig {
    /// Server endpoint URL.
    pub endpoint: String,
    /// Request timeout.
    pub timeout: Duration,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Enable HTTP/2 keepalive.
    pub http2_keepalive_interval: Option<Duration>,
    /// HTTP/2 keepalive timeout.
    pub http2_keepalive_timeout: Option<Duration>,
    /// TCP keepalive.
    pub tcp_keepalive: Option<Duration>,
    /// TCP nodelay.
    pub tcp_nodelay: bool,
    /// Concurrency limit.
    pub concurrency_limit: Option<usize>,
    /// Rate limit (requests per second).
    pub rate_limit: Option<u64>,
    /// Initial connection window size.
    pub initial_connection_window_size: Option<u32>,
    /// Initial stream window size.
    pub initial_stream_window_size: Option<u32>,
    /// Enable retry.
    pub retry_enabled: bool,
    /// Maximum retry attempts.
    pub max_retry_attempts: u32,
    /// TLS configuration. When `None`, the client connects over plaintext.
    pub tls: Option<GrpcClientTlsConfig>,
}

impl Default for GrpcClientConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:50051".to_string(),
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            http2_keepalive_interval: Some(Duration::from_secs(60)),
            http2_keepalive_timeout: Some(Duration::from_secs(20)),
            tcp_keepalive: Some(Duration::from_secs(60)),
            tcp_nodelay: true,
            concurrency_limit: None,
            rate_limit: None,
            initial_connection_window_size: None,
            initial_stream_window_size: None,
            retry_enabled: true,
            max_retry_attempts: 3,
            tls: None,
        }
    }
}

impl GrpcClientConfig {
    /// Create a new builder.
    pub fn builder() -> GrpcClientConfigBuilder {
        GrpcClientConfigBuilder::default()
    }
}

/// Builder for [`GrpcClientConfig`].
///
/// Construct one with [`GrpcClientConfig::builder`], chain the setter methods,
/// then call [`GrpcClientConfigBuilder::build`]. Note that `retry`/
/// `max_retry_attempts` are opt-in and are only honored when calls are wrapped
/// in `GrpcChannel::call_with_retry` (see the crate docs).
///
/// ```
/// use armature_grpc::GrpcClientConfig;
///
/// let config = GrpcClientConfig::builder()
///     .endpoint("http://localhost:50051")
///     .build();
/// ```
#[derive(Debug, Default)]
pub struct GrpcClientConfigBuilder {
    config: GrpcClientConfig,
}

impl GrpcClientConfigBuilder {
    /// Set the endpoint URL.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the connect timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    /// Enable HTTP/2 keepalive.
    pub fn http2_keepalive(mut self, interval: Duration, timeout: Duration) -> Self {
        self.config.http2_keepalive_interval = Some(interval);
        self.config.http2_keepalive_timeout = Some(timeout);
        self
    }

    /// Set TCP keepalive.
    pub fn tcp_keepalive(mut self, duration: Duration) -> Self {
        self.config.tcp_keepalive = Some(duration);
        self
    }

    /// Enable TCP nodelay.
    pub fn tcp_nodelay(mut self, enable: bool) -> Self {
        self.config.tcp_nodelay = enable;
        self
    }

    /// Set concurrency limit.
    pub fn concurrency_limit(mut self, limit: usize) -> Self {
        self.config.concurrency_limit = Some(limit);
        self
    }

    /// Set rate limit (requests per second).
    pub fn rate_limit(mut self, rps: u64) -> Self {
        self.config.rate_limit = Some(rps);
        self
    }

    /// Enable or disable retry.
    pub fn retry(mut self, enabled: bool) -> Self {
        self.config.retry_enabled = enabled;
        self
    }

    /// Set maximum retry attempts.
    pub fn max_retry_attempts(mut self, attempts: u32) -> Self {
        self.config.max_retry_attempts = attempts;
        self
    }

    /// Configure TLS for the client connection.
    pub fn tls(mut self, tls: GrpcClientTlsConfig) -> Self {
        self.config.tls = Some(tls);
        self
    }

    /// Build the configuration.
    pub fn build(self) -> GrpcClientConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_bind_address_returns_config_error_instead_of_panicking() {
        let result = GrpcServerConfig::builder()
            .bind_address("not an address")
            .build();
        assert!(matches!(result, Err(GrpcError::Config(_))));
    }

    #[test]
    fn valid_bind_address_builds_successfully() {
        let config = GrpcServerConfig::builder()
            .bind_address("127.0.0.1:12345")
            .build()
            .expect("valid address should build");
        assert_eq!(config.bind_address, "127.0.0.1:12345".parse().unwrap());
    }

    #[test]
    fn later_valid_bind_address_clears_earlier_parse_error() {
        let config = GrpcServerConfig::builder()
            .bind_address("not an address")
            .bind_address("127.0.0.1:1")
            .build()
            .expect("a later valid address should clear the earlier error");
        assert_eq!(config.bind_address, "127.0.0.1:1".parse().unwrap());
    }

    #[test]
    fn bind_socket_addr_clears_earlier_parse_error() {
        let addr: SocketAddr = "127.0.0.1:2".parse().unwrap();
        let config = GrpcServerConfig::builder()
            .bind_address("not an address")
            .bind_socket_addr(addr)
            .build()
            .expect("bind_socket_addr should clear the earlier error");
        assert_eq!(config.bind_address, addr);
    }
}
