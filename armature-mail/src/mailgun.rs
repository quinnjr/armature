//! Mailgun email provider integration.

use async_trait::async_trait;
use reqwest::{Client, multipart::Form};
use tracing::debug;

use crate::{Email, MailError, Result, Transport};

/// Mailgun configuration.
#[derive(Debug, Clone)]
pub struct MailgunConfig {
    /// API key.
    pub api_key: String,
    /// Domain.
    pub domain: String,
    /// API endpoint region (US or EU).
    pub region: MailgunRegion,
}

/// Mailgun API region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MailgunRegion {
    /// US region (default).
    #[default]
    Us,
    /// EU region.
    Eu,
}

impl MailgunConfig {
    /// Create a new Mailgun configuration.
    pub fn new(api_key: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            domain: domain.into(),
            region: MailgunRegion::Us,
        }
    }

    /// Set the API region.
    pub fn region(mut self, region: MailgunRegion) -> Self {
        self.region = region;
        self
    }

    /// Use EU region.
    pub fn eu(mut self) -> Self {
        self.region = MailgunRegion::Eu;
        self
    }

    /// Get the API endpoint.
    fn endpoint(&self) -> String {
        let base = match self.region {
            MailgunRegion::Us => "https://api.mailgun.net",
            MailgunRegion::Eu => "https://api.eu.mailgun.net",
        };
        format!("{}/v3/{}/messages", base, self.domain)
    }
}

/// Mailgun transport.
pub struct MailgunTransport {
    client: Client,
    config: MailgunConfig,
}

impl MailgunTransport {
    /// Create a new Mailgun transport.
    pub fn new(config: MailgunConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }
}

#[async_trait]
impl Transport for MailgunTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        email.validate()?;

        let from = email
            .from
            .as_ref()
            .ok_or(MailError::MissingField("from"))?
            .to_string();

        debug!(
            to = ?email.to.iter().map(|a| &a.email).collect::<Vec<_>>(),
            subject = ?email.subject,
            "Sending email via Mailgun"
        );

        let mut form = Form::new()
            .text("from", from)
            .text("subject", email.subject.clone().unwrap_or_default());

        // Add recipients
        for addr in &email.to {
            form = form.text("to", addr.to_string());
        }
        for addr in &email.cc {
            form = form.text("cc", addr.to_string());
        }
        for addr in &email.bcc {
            form = form.text("bcc", addr.to_string());
        }

        // Add body
        if let Some(text) = &email.text {
            form = form.text("text", text.clone());
        }
        if let Some(html) = &email.html {
            form = form.text("html", html.clone());
        }

        // Add reply-to
        if let Some(reply_to) = &email.reply_to {
            form = form.text("h:Reply-To", reply_to.to_string());
        }

        // Custom headers and the headers implied by `priority` are passed through
        // Mailgun's `h:<name>` convention.
        for (name, value) in email.wire_headers() {
            form = form.text(format!("h:{}", name), value);
        }

        // Add attachments
        for attachment in &email.attachments {
            let part = reqwest::multipart::Part::bytes(attachment.data.clone())
                .file_name(attachment.filename.clone())
                .mime_str(&attachment.content_type)
                .map_err(|e| MailError::Attachment(e.to_string()))?;

            form = if attachment.content_id.is_some() {
                form.part("inline", part)
            } else {
                form.part("attachment", part)
            };
        }

        // Send request
        let response = self
            .client
            .post(self.config.endpoint())
            .basic_auth("api", Some(&self.config.api_key))
            .multipart(form)
            .send()
            .await
            .map_err(|e| MailError::Network(e.to_string()))?;

        let status = response.status();

        if status.is_success() {
            debug!("Email sent successfully via Mailgun");
            Ok(())
        } else if status.as_u16() == 429 {
            Err(MailError::RateLimited(60))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(MailError::Provider(format!(
                "Mailgun error {}: {}",
                status, body
            )))
        }
    }
}
