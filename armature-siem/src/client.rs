//! SIEM client for sending events

use crate::config::{SiemConfig, Transport};
use crate::error::{SiemError, SiemResult};
use crate::event::SiemEvent;
use crate::format::{EventFormatter, get_formatter};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
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
    formatter: Arc<dyn EventFormatter>,
    transport: Arc<dyn SiemTransport>,
    batch: Arc<Mutex<Vec<SiemEvent>>>,
    /// Background time-based flush task, if batching + a non-zero
    /// `batch_flush_interval` are configured. Aborted on `Drop` so the task
    /// never outlives the client. This handle holds no `Arc` back to the
    /// client itself (only clones of `batch`/`formatter`/`transport` and an
    /// owned `SiemConfig`), so it cannot create a reference cycle that would
    /// prevent `Drop` from running.
    flush_task: Option<tokio::task::JoinHandle<()>>,
}

impl SiemClient {
    /// Create a new SIEM client
    pub fn new(config: SiemConfig) -> SiemResult<Self> {
        config.validate()?;

        let formatter: Arc<dyn EventFormatter> = Arc::from(get_formatter(config.format));
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

        let batch = Arc::new(Mutex::new(Vec::new()));

        let flush_task = if config.batching_enabled && !config.batch_flush_interval.is_zero() {
            Some(Self::spawn_flush_task(
                config.clone(),
                formatter.clone(),
                transport.clone(),
                batch.clone(),
            ))
        } else {
            None
        };

        Ok(Self {
            config,
            formatter,
            transport,
            batch,
            flush_task,
        })
    }

    /// Spawn the background task that periodically flushes the batch on
    /// `config.batch_flush_interval`. Only owns `Arc` clones of the pieces it
    /// needs (never the `SiemClient` itself), so it never blocks `Drop`.
    fn spawn_flush_task(
        config: SiemConfig,
        formatter: Arc<dyn EventFormatter>,
        transport: Arc<dyn SiemTransport>,
        batch: Arc<Mutex<Vec<SiemEvent>>>,
    ) -> tokio::task::JoinHandle<()> {
        let interval_dur = config.batch_flush_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval_dur);
            // The first tick fires immediately; consume it here so the first
            // *flush* only happens once the interval has actually elapsed.
            ticker.tick().await;

            loop {
                ticker.tick().await;

                let events = {
                    let mut b = batch.lock().await;
                    std::mem::take(&mut *b)
                };

                if events.is_empty() {
                    continue;
                }

                match formatter.format_batch(&events, &config) {
                    Ok(formatted) => {
                        if let Err(err) = Self::send_via_transport(
                            &transport,
                            &config,
                            &formatted,
                            formatter.content_type(),
                        )
                        .await
                        {
                            tracing::warn!(
                                error = %err,
                                event_count = events.len(),
                                "background SIEM flush failed to send batch; events dropped"
                            );
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            event_count = events.len(),
                            "background SIEM flush failed to format batch; events dropped"
                        );
                    }
                }
            }
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
            self.send_with_retry(&formatted, self.formatter.content_type())
                .await
        }
    }

    /// Send an event immediately (bypassing batch)
    pub async fn send_immediate(&self, event: SiemEvent) -> SiemResult<()> {
        let formatted = self.formatter.format(&event, &self.config)?;
        self.send_with_retry(&formatted, self.formatter.content_type())
            .await
    }

    /// Send `data` via the underlying transport, retrying transient failures
    /// with exponential backoff.
    ///
    /// Total attempts made are `config.max_retries + 1` (the initial attempt
    /// plus up to `max_retries` retries). Backoff between attempts starts at
    /// `config.retry_delay` and doubles each subsequent attempt
    /// (`retry_delay * 2^attempt`), unless the transport reports
    /// [`SiemError::RateLimited`], in which case the server-provided
    /// retry-after duration is honored instead. Errors that are clearly
    /// permanent (bad configuration, auth failure, unsupported
    /// provider/format, malformed data) are not retried.
    async fn send_with_retry(&self, data: &str, content_type: &str) -> SiemResult<()> {
        Self::send_via_transport(&self.transport, &self.config, data, content_type).await
    }

    /// Free-standing version of `send_with_retry` that only needs a
    /// transport/config pair rather than a full `&self`. This is the seam
    /// the background flush task uses so it doesn't need to hold (or share)
    /// the `SiemClient` itself.
    async fn send_via_transport(
        transport: &Arc<dyn SiemTransport>,
        config: &SiemConfig,
        data: &str,
        content_type: &str,
    ) -> SiemResult<()> {
        let max_retries = config.max_retries;
        let mut attempt: u32 = 0;

        loop {
            match transport.send(data, content_type).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if attempt >= max_retries || !Self::is_retryable(&err) {
                        return Err(err);
                    }

                    let delay = match &err {
                        SiemError::RateLimited(retry_after_ms) => {
                            Duration::from_millis(*retry_after_ms)
                        }
                        _ => config
                            .retry_delay
                            .saturating_mul(2u32.saturating_pow(attempt)),
                    };

                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Whether an error from the transport represents a transient failure
    /// worth retrying, as opposed to a permanent/configuration error that
    /// will never succeed on retry.
    fn is_retryable(err: &SiemError) -> bool {
        !matches!(
            err,
            SiemError::Config(_)
                | SiemError::Auth(_)
                | SiemError::UnsupportedProvider(_)
                | SiemError::UnsupportedFormat(_)
                | SiemError::Serialization(_)
                | SiemError::UrlParse(_)
                | SiemError::BatchFull(_)
        )
    }

    /// Add event to batch, flushing if full
    ///
    /// The batch length is capped at `batch_size`. In the normal course the
    /// flush below drains it, so the cap is never reached; enforcing it makes
    /// the invariant checked rather than assumed, and gives
    /// [`SiemError::BatchFull`] a caller-visible meaning instead of being a
    /// variant nothing ever constructs.
    async fn add_to_batch(&self, event: SiemEvent) -> SiemResult<()> {
        let max_len = self.config.batch_size.max(1);
        let mut batch = self.batch.lock().await;

        if batch.len() >= max_len {
            return Err(SiemError::BatchFull(max_len));
        }

        batch.push(event);

        if batch.len() >= max_len {
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
        self.send_with_retry(&formatted, self.formatter.content_type())
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

impl Drop for SiemClient {
    /// Stop the background time-based flush task (if any) when the client is
    /// dropped, so it never keeps running against a client that no longer
    /// exists.
    fn drop(&mut self) {
        if let Some(handle) = self.flush_task.take() {
            handle.abort();
        }
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
    compression: bool,
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

        if let Some(ca_path) = &config.ca_cert_path {
            let pem = std::fs::read(ca_path).map_err(|e| {
                SiemError::Config(format!(
                    "Failed to read CA certificate '{}': {}",
                    ca_path, e
                ))
            })?;
            let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| {
                SiemError::Config(format!(
                    "Failed to parse CA certificate '{}': {}",
                    ca_path, e
                ))
            })?;
            builder = builder.add_root_certificate(cert);
        }

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
            compression: config.compression,
        })
    }

    /// Gzip-compress `data` for the `Content-Encoding: gzip` request body.
    fn gzip_compress(data: &str) -> SiemResult<Vec<u8>> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data.as_bytes())?;
        encoder.finish().map_err(SiemError::from)
    }

    /// Compute the Azure Sentinel `SharedKey` `Authorization` header value
    /// and the RFC 1123 `x-ms-date` string for `data` as of `date`.
    ///
    /// Canonical string per the Azure Monitor HTTP Data Collector API:
    /// `POST\n{content_length}\napplication/json\nx-ms-date:{rfc1123_date}\n/api/logs`
    /// HMAC-SHA256'd with the base64-decoded shared key, then base64-encoded.
    fn sentinel_signature(
        sentinel: &SentinelAuth,
        content_length: usize,
        date: chrono::DateTime<chrono::Utc>,
    ) -> SiemResult<(String, String)> {
        use base64::Engine as _;
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;

        let rfc1123_date = date.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
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

    /// Convert an HTTP `Retry-After` header value into the milliseconds
    /// used by `SiemError::RateLimited`.
    ///
    /// Per RFC 7231, `Retry-After` (when given as a delay-seconds value, as
    /// opposed to an HTTP-date) is in **seconds**, but `SiemError::RateLimited`
    /// documents its field as **milliseconds** (the retry-loop consumer in
    /// `send_via_transport` uses `Duration::from_millis`), so the seconds
    /// value must be converted here at the producer. A missing or
    /// unparseable header falls back to a conservative 1s (1000ms) default.
    fn retry_after_header_to_ms(header_value: Option<&str>) -> u64 {
        header_value
            .and_then(|v| v.parse::<u64>().ok())
            .map(|secs| secs.saturating_mul(1000))
            .unwrap_or(1000)
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
        let body: Vec<u8> = if self.compression {
            Self::gzip_compress(data)?
        } else {
            data.as_bytes().to_vec()
        };
        let content_length = body.len();

        let mut request = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", content_type)
            .body(body);

        if self.compression {
            request = request.header("Content-Encoding", "gzip");
        }

        if let Some(ref sentinel) = self.sentinel {
            let (auth, rfc1123_date) = Self::sentinel_signature(sentinel, content_length, date)?;
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
            // Rate limited.
            let retry_after_ms = Self::retry_after_header_to_ms(
                response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok()),
            );
            Err(SiemError::RateLimited(retry_after_ms))
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
    /// Pre-built TLS connector (only present when `tls` is true), built once
    /// in `new()` rather than per-`send` so that the CA file isn't re-read
    /// and the `rustls::ClientConfig` isn't rebuilt on every event.
    /// `tokio_rustls::TlsConnector` is a thin `Arc`-backed handle, so cloning
    /// it per-send is cheap.
    tls_connector: Option<tokio_rustls::TlsConnector>,
}

impl TcpTransport {
    /// Create a new TCP transport
    pub fn new(config: &SiemConfig) -> SiemResult<Self> {
        let tls = config.transport == Transport::Tls;
        let tls_verify = config.tls_verify;
        let ca_cert_path = config.ca_cert_path.clone();

        let tls_connector = if tls {
            Some(Self::build_tls_connector(tls_verify, &ca_cert_path)?)
        } else {
            None
        };

        Ok(Self {
            endpoint: config.endpoint.clone(),
            tls,
            tls_connector,
        })
    }

    /// Build a rustls-based TLS connector honoring `tls_verify` / `ca_cert_path`.
    ///
    /// Never falls back to a permissive default: verification is only
    /// disabled when `tls_verify` is explicitly `false`.
    ///
    /// Called once from `new()` (so a bad `ca_cert_path` surfaces as a
    /// construction-time error) rather than per-`send`.
    fn build_tls_connector(
        tls_verify: bool,
        ca_cert_path: &Option<String>,
    ) -> SiemResult<tokio_rustls::TlsConnector> {
        let provider = rustls::crypto::ring::default_provider();

        let client_config = if !tls_verify {
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

            if let Some(ca_path) = ca_cert_path {
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
            let connector = self.tls_connector.clone().ok_or_else(|| {
                SiemError::Transport(
                    "TLS transport has no connector initialized (this is a bug)".to_string(),
                )
            })?;

            let host = self
                .endpoint
                .rsplit_once(':')
                .map(|(host, _)| host)
                .unwrap_or(&self.endpoint);
            // A bracketed IPv6 literal (e.g. `[::1]:6514`) yields `[::1]`
            // from the split above; `ServerName::try_from` rejects the
            // brackets, so strip them before constructing the `ServerName`.
            let host = if let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']'))
            {
                inner
            } else {
                host
            };
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

            // Known-answer vector: pin the exact expected `Authorization`
            // header for this fixed shared key / workspace / date / body, so
            // a wrong canonical string layout can't self-confirm against the
            // mirror computation above (independently verified offline).
            assert_eq!(
                expected_auth,
                "SharedKey test-workspace-123:xJZwrcXTwTLznci6k1CzpQVgM4PIvPImd8Xlj6QEO1g="
            );

            let recorded = server.assert_received("POST", "/");
            assert_eq!(
                recorded.header("Authorization"),
                Some(expected_auth.as_str())
            );
            assert_eq!(recorded.header("x-ms-date"), Some(rfc1123_date.as_str()));
            assert!(recorded.header("Log-Type").is_some());
        }

        /// With compression enabled, the Sentinel `SharedKey` signature must
        /// be computed over the *compressed* body's length (matching what
        /// `send_at` actually puts on the wire), not the plaintext length —
        /// and the request must carry `Content-Encoding: gzip`.
        #[tokio::test]
        async fn signs_sentinel_requests_over_compressed_body_length() {
            let server = StubServer::start_single(StubResponse::new(200, "")).await;

            let workspace_id = "test-workspace-123";
            let shared_key_b64 =
                base64::engine::general_purpose::STANDARD.encode(b"super-secret-key-material");

            let config = SiemConfig::builder()
                .provider(SiemProvider::Sentinel)
                .endpoint(server.url())
                .token(shared_key_b64.clone())
                .workspace_id(workspace_id)
                .compression(true)
                .build()
                .expect("valid sentinel config");

            let transport = HttpTransport::new(&config).expect("build sentinel transport");

            let fixed_date = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();

            let body = r#"{"event":"test"}"#;
            transport
                .send_at(body, "application/json", fixed_date)
                .await
                .expect("send should succeed against stub server");

            let rfc1123_date = fixed_date.format("%a, %d %b %Y %H:%M:%S GMT").to_string();

            let recorded = server.assert_received("POST", "/");
            assert_eq!(recorded.header("Content-Encoding"), Some("gzip"));

            // The recorded body is the actual (gzipped) bytes sent on the
            // wire; the signature must be computed over its length, not the
            // plaintext body's length.
            let compressed_len = recorded.body.len();
            let expected_sig = expected_signature(&shared_key_b64, compressed_len, &rfc1123_date);
            let expected_auth = format!("SharedKey {}:{}", workspace_id, expected_sig);

            assert_eq!(
                recorded.header("Authorization"),
                Some(expected_auth.as_str())
            );
            assert_eq!(recorded.header("x-ms-date"), Some(rfc1123_date.as_str()));

            // Sanity: the plaintext length must differ from the compressed
            // length for this test to actually exercise the bug (otherwise
            // a wrong-length signature could accidentally match).
            assert_ne!(
                compressed_len,
                body.len(),
                "compressed and plaintext lengths must differ for this test to be meaningful"
            );
        }
    }

    /// `send`/`send_immediate` retry behavior against a server that fails a
    /// scripted number of times before succeeding (or never succeeds).
    ///
    /// `armature_testkit::StubServer` only scripts a single fixed response
    /// per route (see `armature-testkit/src/http_stub.rs`), so it cannot
    /// express "500, 500, then 200" for the same path. This module rolls a
    /// minimal, `AtomicUsize`-backed HTTP/1.1 stub that returns the next
    /// status off a scripted list (repeating the last entry once exhausted)
    /// and counts how many requests it received.
    #[cfg(feature = "http")]
    mod retry_with_backoff {
        use super::super::*;
        use crate::SiemProvider;
        use crate::config::SiemConfig;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        /// Spawn an HTTP/1.1 stub that responds to each accepted connection
        /// with the next status code from `statuses` (clamped to the last
        /// entry once exhausted), with an empty body. Returns the server's
        /// base URL and a shared counter of requests received.
        async fn spawn_scripted_stub(statuses: Vec<u16>) -> (String, Arc<AtomicUsize>) {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind scripted stub");
            let port = listener.local_addr().unwrap().port();
            let counter = Arc::new(AtomicUsize::new(0));
            let statuses = Arc::new(statuses);

            let counter_for_server = counter.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = match listener.accept().await {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let statuses = statuses.clone();
                    let counter = counter_for_server.clone();
                    tokio::spawn(async move {
                        let mut stream = stream;
                        let (reader_half, mut writer_half) = stream.split();
                        let mut reader = BufReader::new(reader_half);

                        // Read the request line + headers, tracking Content-Length.
                        let mut content_length: usize = 0;
                        loop {
                            let mut line = String::new();
                            let n = reader.read_line(&mut line).await.unwrap_or(0);
                            if n == 0 || line == "\r\n" {
                                break;
                            }
                            if let Some(rest) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                content_length = rest.trim().parse().unwrap_or(0);
                            }
                        }
                        if content_length > 0 {
                            let mut body = vec![0u8; content_length];
                            let _ = reader.read_exact(&mut body).await;
                        }

                        let idx = counter.fetch_add(1, Ordering::SeqCst);
                        let status = statuses
                            .get(idx)
                            .copied()
                            .unwrap_or(*statuses.last().unwrap());
                        let response = format!(
                            "HTTP/1.1 {status} status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = writer_half.write_all(response.as_bytes()).await;
                        let _ = writer_half.flush().await;
                    });
                }
            });

            (format!("http://127.0.0.1:{port}"), counter)
        }

        fn test_config(endpoint: String, max_retries: u32) -> SiemConfig {
            SiemConfig::builder()
                .provider(SiemProvider::Custom)
                .endpoint(endpoint)
                .token("test-token")
                .batching(false)
                .max_retries(max_retries)
                .retry_delay(Duration::from_millis(1))
                .build()
                .expect("valid config")
        }

        #[tokio::test]
        async fn retries_transient_failures_until_success() {
            // 500, 500, then 200: with max_retries >= 2 this must succeed
            // and the server must have been hit exactly 3 times.
            let (url, counter) = spawn_scripted_stub(vec![500, 500, 200]).await;
            let config = test_config(url, 3);
            let client = SiemClient::new(config).expect("build client");

            let result = client.send(SiemEvent::new("auth.failure")).await;

            assert!(
                result.is_ok(),
                "send should succeed once the server recovers: {:?}",
                result
            );
            assert_eq!(
                counter.load(Ordering::SeqCst),
                3,
                "server should have been hit exactly 3 times (2 failures + 1 success)"
            );
        }

        #[tokio::test]
        async fn exhausts_retries_and_returns_error() {
            // Server always returns 500: with max_retries = 2, the client
            // must make exactly 3 attempts total (1 initial + 2 retries)
            // and then surface the error instead of retrying forever.
            let (url, counter) = spawn_scripted_stub(vec![500, 500, 500, 500, 500]).await;
            let config = test_config(url, 2);
            let client = SiemClient::new(config).expect("build client");

            let result = client.send(SiemEvent::new("auth.failure")).await;

            assert!(
                result.is_err(),
                "send should return an error once retries are exhausted"
            );
            assert_eq!(
                counter.load(Ordering::SeqCst),
                3,
                "server should have been hit exactly 3 times (1 initial + 2 retries)"
            );
        }
    }

    /// Time-based background batch flush.
    #[cfg(feature = "http")]
    mod time_based_flush {
        use super::super::*;
        use crate::SiemProvider;
        use crate::config::SiemConfig;
        use armature_testkit::{StubResponse, StubServer};
        use std::time::Duration;

        // NOTE: ideally these would use a paused/virtual tokio clock
        // (`#[tokio::test(start_paused = true)]` + `tokio::time::advance`)
        // for determinism, but that requires the `tokio` `test-util`
        // feature, which isn't enabled for this crate (out of scope for a
        // `client.rs`-only change to add). Instead, the interval/wait
        // margins below are widened substantially (a 5x+ margin between the
        // flush interval and the assertion wait) to keep these robust
        // against CI scheduling jitter.

        #[tokio::test]
        async fn flushes_low_volume_batch_after_interval_elapses() {
            let server = StubServer::start_single(StubResponse::new(200, "")).await;

            let config = SiemConfig::builder()
                .provider(SiemProvider::Custom)
                .endpoint(server.url())
                .token("test-token")
                .batching(true)
                .batch_size(100) // well above the single event we send
                .batch_flush_interval(Duration::from_millis(50))
                .build()
                .expect("valid config");

            let client = SiemClient::new(config).expect("build client");

            client
                .send(SiemEvent::new("auth.success"))
                .await
                .expect("send should enqueue into the batch");

            // Below batch_size, so nothing should have been sent yet.
            assert!(server.requests().is_empty(), "flush must not be immediate");

            // Wait well past the flush interval for the background task to
            // fire (wide margin to absorb CI scheduling jitter).
            tokio::time::sleep(Duration::from_millis(500)).await;

            assert_eq!(
                server.requests().len(),
                1,
                "background task should have flushed the batch after the interval elapsed"
            );
        }

        #[tokio::test]
        async fn dropping_client_stops_background_flush_task() {
            let server = StubServer::start_single(StubResponse::new(200, "")).await;

            let config = SiemConfig::builder()
                .provider(SiemProvider::Custom)
                .endpoint(server.url())
                .token("test-token")
                .batching(true)
                .batch_size(100)
                .batch_flush_interval(Duration::from_millis(50))
                .build()
                .expect("valid config");

            let client = SiemClient::new(config).expect("build client");

            client
                .send(SiemEvent::new("auth.success"))
                .await
                .expect("send should enqueue into the batch");

            // Drop before the interval elapses: the background task must be
            // aborted and never get a chance to flush.
            drop(client);

            tokio::time::sleep(Duration::from_millis(500)).await;

            assert!(
                server.requests().is_empty(),
                "dropped client's background flush task must not still be running"
            );
        }
    }

    /// Gzip compression of the HTTP request body.
    #[cfg(feature = "http")]
    mod compression {
        use super::super::*;
        use crate::SiemProvider;
        use crate::config::SiemConfig;
        use armature_testkit::{StubResponse, StubServer};
        use std::io::Read;

        #[tokio::test]
        async fn gzips_body_and_sets_content_encoding_header() {
            let server = StubServer::start_single(StubResponse::new(200, "")).await;

            let config = SiemConfig::builder()
                .provider(SiemProvider::Custom)
                .endpoint(server.url())
                .token("test-token")
                .batching(false)
                .compression(true)
                .build()
                .expect("valid config");

            let transport = HttpTransport::new(&config).expect("build transport");
            let body = r#"{"event":"test"}"#;

            transport
                .send(body, "application/json")
                .await
                .expect("send should succeed against stub server");

            let recorded = server.assert_received("POST", "/");
            assert_eq!(recorded.header("Content-Encoding"), Some("gzip"));

            let mut decoder = flate2::read::GzDecoder::new(&recorded.body[..]);
            let mut decompressed = String::new();
            decoder
                .read_to_string(&mut decompressed)
                .expect("recorded body must be valid gzip");
            assert_eq!(decompressed, body);
        }
    }

    /// `retry_after_header_to_ms` seconds-to-milliseconds conversion.
    #[cfg(feature = "http")]
    mod retry_after_conversion {
        use super::super::*;

        #[test]
        fn converts_seconds_to_milliseconds() {
            assert_eq!(HttpTransport::retry_after_header_to_ms(Some("30")), 30_000);
        }

        #[test]
        fn missing_header_falls_back_to_one_second() {
            assert_eq!(HttpTransport::retry_after_header_to_ms(None), 1000);
        }

        #[test]
        fn non_numeric_http_date_falls_back_to_one_second() {
            assert_eq!(
                HttpTransport::retry_after_header_to_ms(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
                1000
            );
        }
    }

    /// Custom CA certificate loading for the reqwest-based HTTP transport.
    #[cfg(feature = "http")]
    mod ca_cert_path {
        use super::super::*;
        use crate::SiemProvider;
        use crate::config::SiemConfig;

        #[test]
        fn bogus_ca_cert_path_returns_clear_config_error() {
            let config = SiemConfig::builder()
                .provider(SiemProvider::Custom)
                .endpoint("https://example.invalid")
                .token("test-token")
                .ca_cert("/nonexistent/path/does-not-exist.pem")
                .build()
                .expect("valid config");

            match HttpTransport::new(&config) {
                Err(SiemError::Config(msg)) => {
                    assert!(
                        msg.contains("CA certificate"),
                        "error message should mention the CA certificate: {msg}"
                    );
                }
                Err(other) => panic!("expected SiemError::Config, got: {other:?}"),
                Ok(_) => panic!("a bogus ca_cert_path must produce an error, not succeed"),
            }
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
        let tls_connector = TcpTransport::build_tls_connector(tls_verify, &None)
            .expect("build tls connector for test transport");
        TcpTransport {
            endpoint,
            tls: true,
            tls_connector: Some(tls_connector),
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
