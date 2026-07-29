//! AWS SES email provider integration.

use async_trait::async_trait;
use aws_sdk_sesv2::{
    Client,
    config::http::HttpResponse,
    error::SdkError,
    primitives::Blob,
    types::{Body, Content, Destination, EmailContent, Message, RawMessage},
};
use tracing::debug;

use crate::{Email, MailError, Result, Transport};

/// Map an AWS SDK error onto the closest [`MailError`] variant.
///
/// Collapsing everything into [`MailError::Provider`] made transport-level
/// failures indistinguishable from message rejections: a dispatch failure or a
/// timeout is transient and must retry, and a service error carries an HTTP
/// status that decides retryability. `Provider` without a status is the
/// conservative fallback and does *not* retry.
fn map_sdk_error<E>(err: SdkError<E, HttpResponse>) -> MailError
where
    SdkError<E, HttpResponse>: std::fmt::Display,
{
    match &err {
        // "An HTTP response was not received. The request MAY have been sent."
        SdkError::DispatchFailure(_) => MailError::Network(err.to_string()),
        SdkError::TimeoutError(_) => MailError::Timeout,
        SdkError::ResponseError(_) => MailError::Network(err.to_string()),
        SdkError::ServiceError(ctx) => {
            let status = ctx.raw().status().as_u16();
            MailError::provider(status, err.to_string())
        }
        // Construction failures are our own bug, never transient.
        _ => MailError::provider(None, err.to_string()),
    }
}

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

/// Whether a message must be sent as raw MIME rather than SES's structured
/// `simple` content.
///
/// `EmailContent::simple` can only carry subject/text/html. It has no
/// representation for attachments, custom headers, *or* the threading headers
/// (`Message-ID`, `In-Reply-To`, `References`) — an email with only
/// `.in_reply_to(..)` set used to take the simple path and lose its threading
/// silently.
fn needs_raw_mime(email: &Email) -> bool {
    !email.attachments.is_empty()
        || !email.wire_headers().is_empty()
        || email.message_id.is_some()
        || email.in_reply_to.is_some()
        || !email.references.is_empty()
}

/// Build the SES content for a message, choosing the simple or raw-MIME path.
///
/// Extracted as a seam so the choice — not a restatement of the condition — can
/// be asserted on directly.
fn ses_content(email: &Email) -> Result<EmailContent> {
    if needs_raw_mime(email) {
        let mime = email.to_lettre()?.formatted();
        let raw = RawMessage::builder()
            .data(Blob::new(mime))
            .build()
            .map_err(|e| MailError::Smtp(e.to_string()))?;

        debug!(
            attachments = email.attachments.len(),
            "Sending raw MIME message via AWS SES"
        );

        return Ok(EmailContent::builder().raw(raw).build());
    }

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

    let message = Message::builder()
        .subject(
            Content::builder()
                // `wire_subject` rather than the raw field: a template-derived
                // subject commonly ends in a newline, and every other transport
                // strips that trailing run before it reaches the wire. Reading
                // `email.subject` here made SES the one backend that emitted it.
                .data(email.wire_subject().unwrap_or_default())
                .charset("UTF-8")
                .build()
                .map_err(|e| MailError::Smtp(e.to_string()))?,
        )
        .body(body.build())
        .build();

    Ok(EmailContent::builder().simple(message).build())
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

        let email_content = ses_content(email)?;

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
        request.send().await.map_err(map_sdk_error)?;

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
    use super::*;
    use crate::{Attachment, Email};

    fn base() -> Email {
        Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello")
    }

    /// Bytes SES would receive on the raw path.
    fn raw_bytes(content: &EmailContent) -> String {
        let raw = content.raw().expect("raw content");
        String::from_utf8_lossy(raw.data().as_ref()).into_owned()
    }

    /// A template-derived subject commonly ends in a newline. Every transport
    /// strips that trailing run before it reaches the wire; SES read the raw
    /// `email.subject` field directly and so was the one backend that emitted
    /// it, which would have been a bare `\n` inside a `Subject:` header.
    #[test]
    fn subject_trailing_newline_is_stripped_on_the_simple_path() {
        let email = Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Welcome\n")
            .text("Hello");

        let content = ses_content(&email).expect("simple content builds");
        let simple = content.simple().expect("no attachments or wire headers");
        assert_eq!(
            simple.subject().unwrap().data(),
            "Welcome",
            "SES must send the wire subject, not the raw field"
        );
    }

    /// `SesTransport::send` built `EmailContent::simple` from
    /// subject/text/html only and never read `email.attachments`, so attachments
    /// vanished with no error. Assert on the content `send` actually builds, not
    /// on a restatement of the branch condition.
    #[test]
    fn raw_mime_path_carries_attachments_and_headers() {
        let email = base()
            .attach(Attachment::text("report.csv", "a,b\n1,2\n"))
            .header("X-Campaign-Id", "spring-2026")
            .high_priority();

        let content = ses_content(&email).unwrap();
        assert!(
            content.raw().is_some(),
            "attachments must force the raw MIME path"
        );

        let mime = raw_bytes(&content);
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
        let content = ses_content(&base()).unwrap();
        assert!(
            content.simple().is_some(),
            "plain message should not go raw"
        );
        assert!(content.raw().is_none());

        let simple = content.simple().unwrap();
        assert_eq!(simple.subject().unwrap().data(), "Test");
        assert_eq!(simple.body().unwrap().text().unwrap().data(), "Hello");
    }

    /// An email carrying only threading headers took the
    /// simple path, which has no representation for them, so `In-Reply-To` /
    /// `References` / `Message-ID` were dropped with no error.
    #[test]
    fn threading_headers_force_the_raw_path_and_survive() {
        let email = base()
            .message_id("abc123@armature")
            .in_reply_to("<parent@example.com>")
            .reference("<root@example.com>");

        let content = ses_content(&email).unwrap();
        assert!(
            content.raw().is_some(),
            "threading headers must force the raw MIME path"
        );

        let mime = raw_bytes(&content);
        assert!(mime.contains("Message-ID: <abc123@armature>"), "{mime}");
        assert!(mime.contains("In-Reply-To: <parent@example.com>"), "{mime}");
        assert!(mime.contains("References: <root@example.com>"), "{mime}");
    }

    #[test]
    fn each_threading_header_alone_forces_raw() {
        assert!(needs_raw_mime(&base().in_reply_to("<x@y>")));
        assert!(needs_raw_mime(&base().reference("<x@y>")));
        assert!(needs_raw_mime(&base().message_id("x@y")));
        assert!(!needs_raw_mime(&base()));
    }
}
