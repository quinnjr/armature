//! AWS SES email provider integration.

use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    primitives::Blob,
    types::{Body, Content, Destination, EmailContent, Message, RawMessage},
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

        // `EmailContent::simple` can only carry subject/text/html — it has no
        // representation for attachments or custom headers. Whenever the message
        // needs either, assemble the full MIME document locally and send it as
        // raw content so nothing is silently dropped.
        let email_content = if email.attachments.is_empty() && email.wire_headers().is_empty() {
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

            EmailContent::builder().simple(message).build()
        } else {
            let mime = email.to_lettre()?.formatted();
            let raw = RawMessage::builder()
                .data(Blob::new(mime))
                .build()
                .map_err(|e| MailError::Smtp(e.to_string()))?;

            debug!(
                attachments = email.attachments.len(),
                "Sending raw MIME message via AWS SES"
            );

            EmailContent::builder().raw(raw).build()
        };

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

#[cfg(test)]
mod tests {
    use crate::{Attachment, Email};

    fn base() -> Email {
        Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello")
    }

    /// WF6 finding 3: `SesTransport::send` built `EmailContent::simple` from
    /// subject/text/html only and never read `email.attachments`, so attachments
    /// vanished with no error. It now switches to raw MIME whenever the message
    /// carries attachments or custom headers — this asserts on the exact bytes
    /// that path hands to `RawMessage`.
    #[test]
    fn raw_mime_path_carries_attachments_and_headers() {
        let email = base()
            .attach(Attachment::text("report.csv", "a,b\n1,2\n"))
            .header("X-Campaign-Id", "spring-2026")
            .high_priority();

        // The condition `send` uses to choose raw over simple content.
        assert!(!email.attachments.is_empty() || !email.wire_headers().is_empty());

        let mime = String::from_utf8_lossy(&email.to_lettre().unwrap().formatted()).into_owned();
        assert!(
            mime.contains(r#"Content-Disposition: attachment; filename="report.csv""#),
            "attachment missing from raw SES payload:\n{mime}"
        );
        assert!(mime.contains("X-Campaign-Id: spring-2026"), "{mime}");
        assert!(mime.contains("X-Priority: 1 (Highest)"), "{mime}");
    }

    /// A plain message with no attachments and no custom headers still takes the
    /// cheaper `EmailContent::simple` path.
    #[test]
    fn plain_message_takes_the_simple_path() {
        let email = base();
        assert!(email.attachments.is_empty() && email.wire_headers().is_empty());
    }
}
