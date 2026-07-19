//! SIEM client for sending events

use crate::config::{SiemConfig, Transport};
use crate::error::{SiemError, SiemResult};
use crate::event::SiemEvent;
use crate::format::{EventFormatter, get_formatter};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Trait for SIEM transports
#[async_trait]
pub trait SiemTransport: Send + Sync {
    /// Send formatted data to the SIEM
    async fn send(&self, data: &str, content_type: &str) -> SiemResult<()>;

    /// Close the transport connection
    async fn close(&self) -> SiemResult<()>;
}

/// SIEM client for sending security events
///
/// # Examples
///
/// ```no_run
/// use armature_siem::*;
///
/// # async fn example() -> Result<(), SiemError> {
/// let config = SiemConfig::builder()
///     .provider(SiemProvider::Splunk)
///     .endpoint("https://splunk.example.com:8088/services/collector")
///     .token("your-hec-token")
///     .build()?;
///
/// let client = SiemClient::new(config)?;
///
/// client.send(SiemEvent::new("user.login")
///     .src_user("alice")
///     .src_ip("192.168.1.100")
///     .action("login")
///     .outcome(EventOutcome::Success)).await?;
/// # Ok(())
/// # }
/// ```
pub struct SiemClient {
    config: SiemConfig,
    formatter: Box<dyn EventFormatter>,
    transport: Arc<dyn SiemTransport>,
    batch: Arc<Mutex<Vec<SiemEvent>>>,
}

impl SiemClient {
    /// Create a new SIEM client
    pub fn new(config: SiemConfig) -> SiemResult<Self> {
        config.validate()?;

        let formatter = get_formatter(config.format);
        let transport: Arc<dyn SiemTransport> = match config.transport {
            Transport::Https => {
                #[cfg(feature = "http")]
                {
                    Arc::new(HttpTransport::new(&config)?)
                }
                #[cfg(not(feature = "http"))]
                {
                    return Err(SiemError::Config(
                        "HTTP transport requires 'http' feature".to_string(),
                    ));
                }
            }
            Transport::Tcp | Transport::Tls => Arc::new(TcpTransport::new(&config)?),
            Transport::Udp => Arc::new(UdpTransport::new(&config)?),
        };

        Ok(Self {
            config,
            formatter,
            transport,
            batch: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Send a single event immediately
    pub async fn send(&self, event: SiemEvent) -> SiemResult<()> {
        if self.config.batching_enabled {
            self.add_to_batch(event).await
        } else {
            self.send_immediate(event).await
        }
    }

    /// Send multiple events
    pub async fn send_many(&self, events: Vec<SiemEvent>) -> SiemResult<()> {
        if self.config.batching_enabled {
            for event in events {
                self.add_to_batch(event).await?;
            }
            Ok(())
        } else {
            let formatted = self.formatter.format_batch(&events, &self.config)?;
            self.transport
                .send(&formatted, self.formatter.content_type())
                .await
        }
    }

    /// Send an event immediately (bypassing batch)
    pub async fn send_immediate(&self, event: SiemEvent) -> SiemResult<()> {
        let formatted = self.formatter.format(&event, &self.config)?;
        self.transport
            .send(&formatted, self.formatter.content_type())
            .await
    }

    /// Add event to batch, flushing if full
    async fn add_to_batch(&self, event: SiemEvent) -> SiemResult<()> {
        let mut batch = self.batch.lock().await;
        batch.push(event);

        if batch.len() >= self.config.batch_size {
            let events = std::mem::take(&mut *batch);
            drop(batch);
            self.flush_events(events).await?;
        }

        Ok(())
    }

    /// Flush the current batch
    pub async fn flush(&self) -> SiemResult<()> {
        let events = {
            let mut batch = self.batch.lock().await;
            std::mem::take(&mut *batch)
        };

        if !events.is_empty() {
            self.flush_events(events).await?;
        }

        Ok(())
    }

    /// Flush specific events
    async fn flush_events(&self, events: Vec<SiemEvent>) -> SiemResult<()> {
        let formatted = self.formatter.format_batch(&events, &self.config)?;
        self.transport
            .send(&formatted, self.formatter.content_type())
            .await
    }

    /// Close the client and flush remaining events
    pub async fn close(&self) -> SiemResult<()> {
        self.flush().await?;
        self.transport.close().await
    }

    /// Get the current batch size
    pub async fn batch_len(&self) -> usize {
        self.batch.lock().await.len()
    }
}

/// Azure Log Analytics HTTP Data Collector API path used in the SharedKey
/// signature's canonical string. This is fixed by the Azure API contract
/// and is unrelated to the actual request path/endpoint configured.
#[cfg(feature = "http")]
const SENTINEL_SIGNED_RESOURCE: &str = "/api/logs";

/// Per-request Azure Sentinel `SharedKey` signing material.
#[cfg(feature = "http")]
struct SentinelAuth {
    /// Log Analytics workspace ID (the `SharedKey {workspace_id}:{sig}` prefix)
    workspace_id: String,
    /// Base64-encoded shared key (primary or secondary workspace key)
    shared_key_b64: String,
    /// Value for the `Log-Type` header (the custom log table name)
    log_type: String,
}

/// HTTP transport (for Splunk HEC, Elastic, Sentinel, etc.)
#[cfg(feature = "http")]
pub struct HttpTransport {
    client: reqwest::Client,
    endpoint: String,
    auth_header: Option<String>,
    sentinel: Option<SentinelAuth>,
}

#[cfg(feature = "http")]
impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new(config: &SiemConfig) -> SiemResult<Self> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout);

        if !config.tls_verify {
            builder = builder.danger_accept_invalid_certs(true);
        }

        // Note: Compression is handled via headers, not client builder
        let _ = config.compression; // Used in request headers if needed

        let client = builder.build()?;

        let mut sentinel = None;

        // Build auth header based on provider
        let auth_header = match config.provider {
            crate::SiemProvider::Splunk => config.token.as_ref().map(|t| format!("Splunk {}", t)),
            crate::SiemProvider::Elastic | crate::SiemProvider::Datadog => {
                config.token.as_ref().map(|t| format!("Bearer {}", t))
            }
            crate::SiemProvider::Sentinel => {
                // Azure Sentinel uses SharedKey authentication, which must
                // be computed per-request (the signature covers the body's
                // content-length and the request's `x-ms-date`). No static
                // Authorization header is used for Sentinel.
                let workspace_id = config.workspace_id.clone().ok_or_else(|| {
                    SiemError::Config("Sentinel transport requires a workspace_id".to_string())
                })?;
                let shared_key_b64 = config.token.clone().ok_or_else(|| {
                    SiemError::Config("Sentinel transport requires a shared key token".to_string())
                })?;
                let log_type = config
                    .index
                    .clone()
                    .unwrap_or_else(|| "ArmatureEvent".to_string());
                sentinel = Some(SentinelAuth {
                    workspace_id,
                    shared_key_b64,
                    log_type,
                });
                None
            }
            crate::SiemProvider::SumoLogic => {
                // Sumo Logic uses the token in the URL or as header
                config.token.clone()
            }
            _ => {
                // Generic: check for basic auth or token
                if let (Some(user), Some(pass)) = (&config.username, &config.password) {
                    let encoded = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        format!("{}:{}", user, pass),
                    );
                    Some(format!("Basic {}", encoded))
                } else {
                    config.token.as_ref().map(|t| format!("Bearer {}", t))
                }
            }
        };

        Ok(Self {
            client,
            endpoint: config.endpoint.clone(),
            auth_header,
            sentinel,
        })
    }

    /// Compute the Azure Sentinel `SharedKey` `Authorization` header value
    /// and the RFC 1123 `x-ms-date` string for `data` as of `date`.
    ///
    /// Canonical string per the Azure Monitor HTTP Data Collector API:
    /// `POST\n{content_length}\napplication/json\nx-ms-date:{rfc1123_date}\n/api/logs`
    /// HMAC-SHA256'd with the base64-decoded shared key, then base64-encoded.
    fn sentinel_signature(
        sentinel: &SentinelAuth,
        data: &str,
        date: chrono::DateTime<chrono::Utc>,
    ) -> SiemResult<(String, String)> {
        use base64::Engine as _;
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let rfc1123_date = date.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
        let content_length = data.len();
        let canonical = format!(
            "POST\n{}\napplication/json\nx-ms-date:{}\n{}",
            content_length, rfc1123_date, SENTINEL_SIGNED_RESOURCE
        );

        let decoded_key = base64::engine::general_purpose::STANDARD
            .decode(&sentinel.shared_key_b64)
            .map_err(|e| SiemError::Auth(format!("invalid Sentinel shared key: {}", e)))?;

        let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_key)
            .map_err(|e| SiemError::Auth(format!("invalid Sentinel HMAC key: {}", e)))?;
        mac.update(canonical.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        let auth = format!("SharedKey {}:{}", sentinel.workspace_id, signature);
        Ok((auth, rfc1123_date))
    }

    /// Send with an explicit timestamp. This is the internal seam that
    /// makes Sentinel `SharedKey` signing deterministic and testable
    /// without threading a clock through the public `SiemTransport::send`
    /// signature: production code always calls this via `send` with
    /// `Utc::now()`, while tests can pin a fixed date to reproduce an
    /// exact expected signature.
    async fn send_at(
        &self,
        data: &str,
        content_type: &str,
        date: chrono::DateTime<chrono::Utc>,
    ) -> SiemResult<()> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", content_type)
            .body(data.to_string());

        if let Some(ref sentinel) = self.sentinel {
            let (auth, rfc1123_date) = Self::sentinel_signature(sentinel, data, date)?;
            request = request
                .header("Authorization", auth)
                .header("x-ms-date", rfc1123_date)
                .header("Log-Type", &sentinel.log_type)
                .header("time-generated-field", "timestamp");
        } else if let Some(ref auth) = self.auth_header {
            request = request.header("Authorization", auth);
        }

        let response = request.send().await?;
        let status = response.status();

        if status.is_success() {
            Ok(())
        } else if status.as_u16() == 429 {
            // Rate limited
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1000);
            Err(SiemError::RateLimited(retry_after))
        } else if status.as_u16() == 401 || status.as_u16() == 403 {
            Err(SiemError::Auth(format!(
                "Authentication failed: {}",
                status
            )))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(SiemError::Transport(format!("HTTP {} - {}", status, body)))
        }
    }
}

#[cfg(feature = "http")]
#[async_trait]
impl SiemTransport for HttpTransport {
    async fn send(&self, data: &str, content_type: &str) -> SiemResult<()> {
        self.send_at(data, content_type, chrono::Utc::now()).await
    }

    async fn close(&self) -> SiemResult<()> {
        Ok(())
    }
}

/// Certificate verifier that disables server certificate validation.
///
/// Only used when `SiemConfig::tls_verify` is explicitly set to `false`.
/// SIEM/syslog endpoints are frequently internal, self-signed collectors,
/// so this is an intentional, opt-in escape hatch — it must never be the
/// default and must never be selected implicitly.
#[derive(Debug)]
struct NoServerCertVerification(rustls::crypto::CryptoProvider);

impl NoServerCertVerification {
    fn new(provider: rustls::crypto::CryptoProvider) -> Arc<Self> {
        Arc::new(Self(provider))
    }
}

impl rustls::client::danger::ServerCertVerifier for NoServerCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// TCP transport (for Syslog, QRadar, ArcSight)
pub struct TcpTransport {
    endpoint: String,
    tls: bool,
    tls_verify: bool,
    ca_cert_path: Option<String>,
}

impl TcpTransport {
    /// Create a new TCP transport
    pub fn new(config: &SiemConfig) -> SiemResult<Self> {
        Ok(Self {
            endpoint: config.endpoint.clone(),
            tls: config.transport == Transport::Tls,
            tls_verify: config.tls_verify,
            ca_cert_path: config.ca_cert_path.clone(),
        })
    }

    /// Build a rustls-based TLS connector honoring `tls_verify` / `ca_cert_path`.
    ///
    /// Never falls back to a permissive default: verification is only
    /// disabled when `tls_verify` is explicitly `false`.
    fn build_tls_connector(&self) -> SiemResult<tokio_rustls::TlsConnector> {
        let provider = rustls::crypto::ring::default_provider();

        let client_config = if !self.tls_verify {
            rustls::ClientConfig::builder_with_provider(Arc::new(provider.clone()))
                .with_safe_default_protocol_versions()
                .map_err(|e| {
                    SiemError::Transport(format!("Failed to configure TLS protocol: {}", e))
                })?
                .dangerous()
                .with_custom_certificate_verifier(NoServerCertVerification::new(provider))
                .with_no_client_auth()
        } else {
            let mut roots = rustls::RootCertStore::empty();

            if let Some(ca_path) = &self.ca_cert_path {
                let file = std::fs::File::open(ca_path).map_err(|e| {
                    SiemError::Transport(format!(
                        "Failed to open CA certificate '{}': {}",
                        ca_path, e
                    ))
                })?;
                let mut reader = std::io::BufReader::new(file);
                for cert in rustls_pemfile::certs(&mut reader) {
                    let cert = cert.map_err(|e| {
                        SiemError::Transport(format!("Failed to parse CA certificate: {}", e))
                    })?;
                    roots.add(cert).map_err(|e| {
                        SiemError::Transport(format!(
                            "Failed to add CA certificate to root store: {}",
                            e
                        ))
                    })?;
                }
            } else {
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }

            rustls::ClientConfig::builder_with_provider(Arc::new(provider))
                .with_safe_default_protocol_versions()
                .map_err(|e| {
                    SiemError::Transport(format!("Failed to configure TLS protocol: {}", e))
                })?
                .with_root_certificates(roots)
                .with_no_client_auth()
        };

        Ok(tokio_rustls::TlsConnector::from(Arc::new(client_config)))
    }
}

#[async_trait]
impl SiemTransport for TcpTransport {
    async fn send(&self, data: &str, _content_type: &str) -> SiemResult<()> {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;

        let stream = TcpStream::connect(&self.endpoint).await?;

        if self.tls {
            let connector = self.build_tls_connector()?;

            let host = self
                .endpoint
                .rsplit_once(':')
                .map(|(host, _)| host)
                .unwrap_or(&self.endpoint);
            let server_name =
                rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| {
                    SiemError::Transport(format!("Invalid TLS server name '{}': {}", host, e))
                })?;

            // On any TLS setup/handshake failure this returns Err and never
            // falls back to writing the event over the plaintext `stream`.
            let mut tls_stream = connector
                .connect(server_name, stream)
                .await
                .map_err(|e| SiemError::Transport(format!("TLS handshake failed: {}", e)))?;

            tls_stream.write_all(data.as_bytes()).await?;
            tls_stream.write_all(b"\n").await?;
            tls_stream.flush().await?;
        } else {
            let mut stream = stream;
            stream.write_all(data.as_bytes()).await?;
            stream.write_all(b"\n").await?;
            stream.flush().await?;
        }

        Ok(())
    }

    async fn close(&self) -> SiemResult<()> {
        Ok(())
    }
}

/// UDP transport (for Syslog)
pub struct UdpTransport {
    endpoint: String,
}

impl UdpTransport {
    /// Create a new UDP transport
    pub fn new(config: &SiemConfig) -> SiemResult<Self> {
        Ok(Self {
            endpoint: config.endpoint.clone(),
        })
    }
}

#[async_trait]
impl SiemTransport for UdpTransport {
    async fn send(&self, data: &str, _content_type: &str) -> SiemResult<()> {
        use tokio::net::UdpSocket;

        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(&self.endpoint).await?;
        socket.send(data.as_bytes()).await?;

        Ok(())
    }

    async fn close(&self) -> SiemResult<()> {
        Ok(())
    }
}

/// Memory transport for testing
pub struct MemoryTransport {
    messages: Arc<Mutex<Vec<String>>>,
}

impl MemoryTransport {
    /// Create a new memory transport
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get all sent messages
    pub async fn get_messages(&self) -> Vec<String> {
        self.messages.lock().await.clone()
    }

    /// Clear messages
    pub async fn clear(&self) {
        self.messages.lock().await.clear();
    }
}

impl Default for MemoryTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SiemTransport for MemoryTransport {
    async fn send(&self, data: &str, _content_type: &str) -> SiemResult<()> {
        self.messages.lock().await.push(data.to_string());
        Ok(())
    }

    async fn close(&self) -> SiemResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    mod sentinel_signing {
        use super::super::*;
        use crate::SiemProvider;
        use crate::config::SiemConfig;
        use armature_testkit::{StubResponse, StubServer};
        use base64::Engine as _;
        use chrono::{TimeZone, Utc};
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        /// A known Azure Sentinel SharedKey signing vector, computed
        /// independently of `HttpTransport`'s implementation so the test
        /// pins the exact canonicalization Azure expects:
        ///
        /// canonical string = "POST\n{content_length}\napplication/json\nx-ms-date:{rfc1123_date}\n/api/logs"
        /// signature = base64(HMAC-SHA256(base64_decode(shared_key), canonical_string))
        fn expected_signature(
            shared_key_b64: &str,
            content_length: usize,
            rfc1123_date: &str,
        ) -> String {
            let canonical = format!(
                "POST\n{}\napplication/json\nx-ms-date:{}\n/api/logs",
                content_length, rfc1123_date
            );
            let decoded_key = base64::engine::general_purpose::STANDARD
                .decode(shared_key_b64)
                .expect("test shared key must be valid base64");
            let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_key)
                .expect("HMAC can take a key of any length");
            mac.update(canonical.as_bytes());
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
        }

        #[tokio::test]
        async fn signs_sentinel_requests_with_shared_key_hmac() {
            let server = StubServer::start_single(StubResponse::new(200, "")).await;

            let workspace_id = "test-workspace-123";
            // Base64-encoded shared key material (Azure shared keys are
            // always base64-encoded on the wire).
            let shared_key_b64 =
                base64::engine::general_purpose::STANDARD.encode(b"super-secret-key-material");

            let config = SiemConfig::builder()
                .provider(SiemProvider::Sentinel)
                .endpoint(server.url())
                .token(shared_key_b64.clone())
                .workspace_id(workspace_id)
                .build()
                .expect("valid sentinel config");

            let transport = HttpTransport::new(&config).expect("build sentinel transport");

            // Fixed date, injected via the internal seam, so the signature
            // is fully reproducible.
            let fixed_date = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

            let body = r#"{"event":"test"}"#;
            transport
                .send_at(body, "application/json", fixed_date)
                .await
                .expect("send should succeed against stub server");

            let rfc1123_date = fixed_date.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
            let expected_sig = expected_signature(&shared_key_b64, body.len(), &rfc1123_date);
            let expected_auth = format!("SharedKey {}:{}", workspace_id, expected_sig);

            let recorded = server.assert_received("POST", "/");
            assert_eq!(
                recorded.header("Authorization"),
                Some(expected_auth.as_str())
            );
            assert_eq!(recorded.header("x-ms-date"), Some(rfc1123_date.as_str()));
            assert!(recorded.header("Log-Type").is_some());
        }
    }

    #[tokio::test]
    async fn test_memory_transport() {
        let transport = MemoryTransport::new();

        transport.send("test message", "text/plain").await.unwrap();
        transport
            .send("second message", "text/plain")
            .await
            .unwrap();

        let messages = transport.get_messages().await;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0], "test message");
    }

    /// Build a self-signed rustls `ServerConfig` for `localhost` using rcgen,
    /// for standing up an in-test TLS listener.
    fn self_signed_server_config() -> rustls::ServerConfig {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_der = cert.cert.der().clone();
        let key_der =
            rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());

        // Pin the ring provider explicitly: under `--features full` both the
        // `ring` and `aws-lc-rs` rustls backends can be present, which makes the
        // process-default provider ambiguous. The production connector pins ring
        // the same way (see `build_tls_connector`).
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .unwrap()
    }

    fn tcp_tls_transport(endpoint: String, tls_verify: bool) -> TcpTransport {
        TcpTransport {
            endpoint,
            tls: true,
            tls_verify,
            ca_cert_path: None,
        }
    }

    /// A `tls=true` transport must complete a real TLS handshake and deliver
    /// the event over the encrypted stream, not plaintext TCP.
    #[tokio::test]
    async fn test_tls_transport_delivers_over_encrypted_channel() {
        use tokio::net::TcpListener;

        let server_config = Arc::new(self_signed_server_config());
        let acceptor = tokio_rustls::TlsAcceptor::from(server_config);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut tls_stream = acceptor.accept(stream).await.unwrap();

            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(&mut tls_stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            line
        });

        // tls_verify=false: the listener uses a self-signed cert not in any
        // root store, so verification must be explicitly disabled for this
        // test to exercise the encrypted round trip.
        let transport = tcp_tls_transport(format!("localhost:{}", port), false);

        transport
            .send("encrypted security event", "text/plain")
            .await
            .expect("TLS send should succeed and complete a handshake");

        let received = server.await.unwrap();
        assert_eq!(received.trim_end(), "encrypted security event");
    }

    /// A `tls=true` transport pointed at a plaintext listener must NOT
    /// deliver the event in cleartext: the listener should observe a TLS
    /// ClientHello (not the raw event bytes), and `send` must return an
    /// error rather than silently downgrading to plaintext.
    #[tokio::test]
    async fn test_tls_transport_never_falls_back_to_plaintext() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 512];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            buf[..n].to_vec()
        });

        let transport = tcp_tls_transport(format!("127.0.0.1:{}", port), false);

        let result = transport.send("top secret event", "text/plain").await;
        assert!(
            result.is_err(),
            "TLS transport against a non-TLS peer must error, never fall back to plaintext"
        );

        let received = server.await.unwrap();
        let plaintext_needle = b"top secret event";
        assert!(
            !received
                .windows(plaintext_needle.len())
                .any(|w| w == plaintext_needle),
            "plaintext event bytes must never appear on the wire when tls=true"
        );
        // A real TLS ClientHello starts with record type 0x16 (handshake).
        assert_eq!(
            received.first(),
            Some(&0x16u8),
            "expected a TLS ClientHello on the wire, got: {:?}",
            received
        );
    }
}
