//! AWS SES email provider integration.

use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    types::{Body, Content, Destination, EmailContent, Message},
};
use tracing::debug;

use crate::{Email, MailError, Result, Transport};

/// AWS SES configuration.
#[derive(Debug, Clone, Default)]
pub struct SesConfig {
    /// AWS region.
    pub region: Option<String>,
    /// Configuration set name (optional).
    pub configuration_set: Option<String>,
}

impl SesConfig {
    /// Create a new SES configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the AWS region.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set the configuration set.
    pub fn configuration_set(mut self, name: impl Into<String>) -> Self {
        self.configuration_set = Some(name.into());
        self
    }
}

/// AWS SES transport.
pub struct SesTransport {
    client: Client,
    config: SesConfig,
}

impl SesTransport {
    /// Create a new SES transport.
    pub async fn new(config: SesConfig) -> Result<Self> {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

        let ses_config = if let Some(region) = &config.region {
            aws_sdk_sesv2::config::Builder::from(&aws_config)
                .region(aws_sdk_sesv2::config::Region::new(region.clone()))
                .build()
        } else {
            aws_sdk_sesv2::config::Builder::from(&aws_config).build()
        };

        let client = Client::from_conf(ses_config);

        Ok(Self { client, config })
    }

    /// Create from an existing AWS SDK client.
    pub fn from_client(client: Client, config: SesConfig) -> Self {
        Self { client, config }
    }
}

#[async_trait]
impl Transport for SesTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        email.validate()?;

        let from = email
            .from
            .as_ref()
            .ok_or(MailError::MissingField("from"))?
            .to_string();

        let to_addresses: Vec<String> = email.to.iter().map(|a| a.to_string()).collect();
        let cc_addresses: Vec<String> = email.cc.iter().map(|a| a.to_string()).collect();
        let bcc_addresses: Vec<String> = email.bcc.iter().map(|a| a.to_string()).collect();

        debug!(
            to = ?to_addresses,
            subject = ?email.subject,
            "Sending email via AWS SES"
        );

        // Build destination
        let mut destination = Destination::builder();
        for addr in &to_addresses {
            destination = destination.to_addresses(addr);
        }
        for addr in &cc_addresses {
            destination = destination.cc_addresses(addr);
        }
        for addr in &bcc_addresses {
            destination = destination.bcc_addresses(addr);
        }

        // Build body
        let mut body = Body::builder();

        if let Some(text) = &email.text {
            body = body.text(
                Content::builder()
                    .data(text)
                    .charset("UTF-8")
                    .build()
                    .map_err(|e| MailError::Smtp(e.to_string()))?,
            );
        }

        if let Some(html) = &email.html {
            body = body.html(
                Content::builder()
                    .data(html)
                    .charset("UTF-8")
                    .build()
                    .map_err(|e| MailError::Smtp(e.to_string()))?,
            );
        }

        // Build message
        let message = Message::builder()
            .subject(
                Content::builder()
                    .data(email.subject.as_deref().unwrap_or_default())
                    .charset("UTF-8")
                    .build()
                    .map_err(|e| MailError::Smtp(e.to_string()))?,
            )
            .body(body.build())
            .build();

        // Build email content
        let email_content = EmailContent::builder().simple(message).build();

        // Build request
        let mut request = self
            .client
            .send_email()
            .from_email_address(&from)
            .destination(destination.build())
            .content(email_content);

        if let Some(config_set) = &self.config.configuration_set {
            request = request.configuration_set_name(config_set);
        }

        if let Some(reply_to) = &email.reply_to {
            request = request.reply_to_addresses(reply_to.to_string());
        }

        // Send
        request
            .send()
            .await
            .map_err(|e| MailError::Provider(e.to_string()))?;

        debug!("Email sent successfully via AWS SES");
        Ok(())
    }

    async fn is_healthy(&self) -> bool {
        // Try to get account info as a health check
        self.client.get_account().send().await.is_ok()
    }
}
