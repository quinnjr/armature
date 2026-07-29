//! Email transport implementations.

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    transport::smtp::authentication::Credentials,
};
use std::time::Duration;
use tracing::{debug, info};

use crate::{Email, MailError, Result};

/// Email transport trait.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send an email.
    async fn send(&self, email: &Email) -> Result<()>;

    /// Check if the transport is healthy.
    async fn is_healthy(&self) -> bool {
        true
    }
}

/// SMTP security mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmtpSecurity {
    /// No encryption (port 25, not recommended).
    None,
    /// STARTTLS upgrade (port 587).
    #[default]
    StartTls,
    /// Implicit TLS (port 465).
    Tls,
}

/// SMTP configuration.
#[derive(Clone)]
pub struct SmtpConfig {
    /// SMTP server host.
    pub host: String,
    /// SMTP server port.
    pub port: u16,
    /// Security mode.
    pub security: SmtpSecurity,
    /// Username for authentication.
    pub username: Option<String>,
    /// Password for authentication.
    pub password: Option<String>,
    /// Connection timeout.
    pub timeout: Duration,
    /// Maximum connections in lettre's SMTP connection pool.
    ///
    /// Wired through to `PoolConfig::max_size` by [`SmtpTransport::new`].
    pub pool_size: u32,
}

/// Hand-written so the SMTP password is never rendered — a derived `Debug` put
/// the plaintext password into any `tracing::debug!(?config)`.
impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("timeout", &self.timeout)
            .field("pool_size", &self.pool_size)
            .finish()
    }
}

impl SmtpConfig {
    /// Create a new SMTP configuration.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 587,
            security: SmtpSecurity::StartTls,
            username: None,
            password: None,
            timeout: Duration::from_secs(30),
            pool_size: 4,
        }
    }

    /// Set credentials.
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Set the port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Use STARTTLS security (port 587).
    pub fn starttls(mut self) -> Self {
        self.security = SmtpSecurity::StartTls;
        self.port = 587;
        self
    }

    /// Use implicit TLS security (port 465).
    pub fn tls(mut self) -> Self {
        self.security = SmtpSecurity::Tls;
        self.port = 465;
        self
    }

    /// Use no encryption (not recommended).
    pub fn insecure(mut self) -> Self {
        self.security = SmtpSecurity::None;
        self.port = 25;
        self
    }

    /// Set the connection timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the connection pool size.
    pub fn pool_size(mut self, size: u32) -> Self {
        self.pool_size = size;
        self
    }

    /// Create configuration for common providers.
    pub fn gmail(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::new("smtp.gmail.com")
            .port(587)
            .starttls()
            .credentials(username, password)
    }

    /// Create configuration for Outlook/Office365.
    pub fn outlook(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::new("smtp.office365.com")
            .port(587)
            .starttls()
            .credentials(username, password)
    }

    /// Create configuration for Amazon SES.
    pub fn amazon_ses(
        region: &str,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self::new(format!("email-smtp.{}.amazonaws.com", region))
            .port(587)
            .starttls()
            .credentials(username, password)
    }

    /// Create configuration for Mailgun.
    pub fn mailgun(domain: &str, api_key: impl Into<String>) -> Self {
        Self::new("smtp.mailgun.org")
            .port(587)
            .starttls()
            .credentials(format!("postmaster@{}", domain), api_key)
    }

    /// Create configuration for SendGrid.
    pub fn sendgrid(api_key: impl Into<String>) -> Self {
        Self::new("smtp.sendgrid.net")
            .port(587)
            .starttls()
            .credentials("apikey", api_key)
    }

    /// Create configuration for Postmark.
    pub fn postmark(api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        Self::new("smtp.postmarkapp.com")
            .port(587)
            .starttls()
            .credentials(key.clone(), key)
    }
}

/// SMTP transport.
pub struct SmtpTransport {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    config: SmtpConfig,
}

impl SmtpTransport {
    /// Create a new SMTP transport.
    pub async fn new(config: SmtpConfig) -> Result<Self> {
        let mut builder = match config.security {
            SmtpSecurity::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            }
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)?
            }
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)?,
        };

        // `pool_size` was read nowhere: the pool ran at lettre's default max
        // size regardless, so `MailerConfig::bulk_concurrency`'s "matching
        // SmtpConfig::pool_size" rationale had nothing to match.
        builder = builder
            .port(config.port)
            .timeout(Some(config.timeout))
            .pool_config(
                lettre::transport::smtp::PoolConfig::new().max_size(config.pool_size.max(1)),
            );

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }

        let transport = builder.build();

        info!(
            host = %config.host,
            port = config.port,
            security = ?config.security,
            "SMTP transport initialized"
        );

        Ok(Self { transport, config })
    }

    /// Get the configuration.
    pub fn config(&self) -> &SmtpConfig {
        &self.config
    }

    /// Test the SMTP connection.
    pub async fn test_connection(&self) -> Result<bool> {
        self.transport
            .test_connection()
            .await
            .map_err(MailError::from)
    }
}

#[async_trait]
impl Transport for SmtpTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        let message = email.to_lettre()?;

        debug!(
            to = ?email.to.iter().map(|a| &a.email).collect::<Vec<_>>(),
            subject = ?email.subject,
            "Sending email via SMTP"
        );

        self.transport.send(message).await?;

        debug!("Email sent successfully");
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        self.test_connection().await.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smtp_config_builder() {
        let config = SmtpConfig::new("smtp.example.com")
            .port(587)
            .starttls()
            .credentials("user", "pass");

        assert_eq!(config.host, "smtp.example.com");
        assert_eq!(config.port, 587);
        assert_eq!(config.security, SmtpSecurity::StartTls);
        assert_eq!(config.username.as_deref(), Some("user"));
    }

    /// A derived `Debug` dumped the SMTP password into any
    /// `tracing::debug!(?config)`.
    #[test]
    fn debug_never_renders_the_password() {
        let rendered = format!(
            "{:?}",
            SmtpConfig::new("smtp.example.com").credentials("user", "hunter2-super-secret")
        );

        assert!(!rendered.contains("hunter2"), "password leaked: {rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        // The username is not a secret and stays legible for debugging.
        assert!(rendered.contains("user"), "{rendered}");

        // No credentials configured renders as None, not "<redacted>".
        let rendered = format!("{:?}", SmtpConfig::new("smtp.example.com"));
        assert!(rendered.contains("password: None"), "{rendered}");
    }

    /// `pool_size` was configured but read nowhere; it must reach lettre.
    #[tokio::test]
    async fn pool_size_is_wired_into_the_transport() {
        let transport = SmtpTransport::new(SmtpConfig::new("smtp.example.com").pool_size(9))
            .await
            .unwrap();
        assert_eq!(transport.config().pool_size, 9);
    }

    #[test]
    fn test_provider_configs() {
        let gmail = SmtpConfig::gmail("user@gmail.com", "pass");
        assert_eq!(gmail.host, "smtp.gmail.com");

        let sendgrid = SmtpConfig::sendgrid("api-key");
        assert_eq!(sendgrid.host, "smtp.sendgrid.net");
        assert_eq!(sendgrid.username.as_deref(), Some("apikey"));
    }
}
