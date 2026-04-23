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

/// HTTP transport (for Splunk HEC, Elastic, Sentinel, etc.)
#[cfg(feature = "http")]
pub struct HttpTransport {
    client: reqwest::Client,
    endpoint: String,
    auth_header: Option<String>,
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

        // Build auth header based on provider
        let auth_header = match config.provider {
            crate::SiemProvider::Splunk => config.token.as_ref().map(|t| format!("Splunk {}", t)),
            crate::SiemProvider::Elastic | crate::SiemProvider::Datadog => {
                config.token.as_ref().map(|t| format!("Bearer {}", t))
            }
            crate::SiemProvider::Sentinel => {
                // Azure Sentinel uses SharedKey authentication
                config.token.clone()
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
        })
    }
}

#[cfg(feature = "http")]
#[async_trait]
impl SiemTransport for HttpTransport {
    async fn send(&self, data: &str, content_type: &str) -> SiemResult<()> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", content_type)
            .body(data.to_string());

        if let Some(ref auth) = self.auth_header {
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

    async fn close(&self) -> SiemResult<()> {
        Ok(())
    }
}

/// TCP transport (for Syslog, QRadar, ArcSight)
pub struct TcpTransport {
    endpoint: String,
    tls: bool,
}

impl TcpTransport {
    /// Create a new TCP transport
    pub fn new(config: &SiemConfig) -> SiemResult<Self> {
        Ok(Self {
            endpoint: config.endpoint.clone(),
            tls: config.transport == Transport::Tls,
        })
    }
}

#[async_trait]
impl SiemTransport for TcpTransport {
    async fn send(&self, data: &str, _content_type: &str) -> SiemResult<()> {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpStream;

        let mut stream = TcpStream::connect(&self.endpoint).await?;

        // For TLS, we would wrap with TLS here
        // For now, plain TCP
        if self.tls {
            tracing::warn!("TLS transport requested but not fully implemented, using plain TCP");
        }

        stream.write_all(data.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

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
}
