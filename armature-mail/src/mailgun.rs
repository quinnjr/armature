//! Mailgun email provider integration.

use async_trait::async_trait;
use reqwest::{Client, multipart::Form};
use std::fmt;
use std::time::Duration;
use tracing::debug;

use crate::http::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_TIMEOUT, build_client, retry_after_secs};
use crate::{Email, MailError, Result, Transport};

/// Mailgun configuration.
///
/// `#[non_exhaustive]`: this type gained `timeout` and then `connect_timeout`
/// and `base_url`, and each addition broke every downstream struct literal.
/// Construct it with [`MailgunConfig::new`] plus the builders.
#[derive(Clone)]
#[non_exhaustive]
pub struct MailgunConfig {
    /// API key.
    pub api_key: String,
    /// Domain.
    pub domain: String,
    /// API endpoint region (US or EU).
    pub region: MailgunRegion,
    /// Base URL override, bypassing [`MailgunConfig::region`].
    ///
    /// For pointing tests or an egress proxy somewhere other than Mailgun.
    /// Validated by [`MailgunTransport::new`].
    pub base_url: Option<String>,
    /// Per-request timeout.
    ///
    /// Set at the transport level so a hung request tears the connection down
    /// deterministically. The queue worker's `job_timeout` only drops the
    /// future, which does not un-send anything — keep this below it.
    pub timeout: Duration,
    /// Connect-phase timeout.
    ///
    /// Defaults to 10 seconds. Bounded separately from [`Self::timeout`] so an
    /// unreachable endpoint fails quickly rather than consuming the whole
    /// request budget.
    pub connect_timeout: Duration,
}

/// Hand-written so the API key is never rendered — a derived `Debug` put the
/// full key into any `tracing::debug!(?config)`.
impl fmt::Debug for MailgunConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailgunConfig")
            .field("api_key", &"<redacted>")
            .field("domain", &self.domain)
            .field("region", &self.region)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
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
            base_url: None,
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Set the per-request timeout (default: 30 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the connect-phase timeout (default: 10 seconds).
    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// Override the API base URL, bypassing [`MailgunConfig::region`].
    ///
    /// Validated by [`MailgunTransport::new`]: the API key is sent as HTTP basic
    /// auth on every request, so a plaintext base URL to a non-loopback host is
    /// rejected.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
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
    pub(crate) fn endpoint(&self) -> String {
        let base = match &self.base_url {
            Some(base) => base.trim_end_matches('/'),
            None => match self.region {
                MailgunRegion::Us => "https://api.mailgun.net",
                MailgunRegion::Eu => "https://api.eu.mailgun.net",
            },
        };
        format!("{}/v3/{}/messages", base, self.domain)
    }
}

/// Build Mailgun's form text fields for a message, in emission order.
///
/// Extracted as a seam so the `h:` prefixing — which is what makes Mailgun
/// honor a header at all, and whose loss is silent — can be asserted on without
/// an HTTP round-trip. Attachments are added separately by the caller since they
/// are multipart file parts rather than text fields.
fn build_form(email: &Email) -> Result<Vec<(String, String)>> {
    let from = email
        .from
        .as_ref()
        .ok_or(MailError::MissingField("from"))?
        .to_string();

    let mut fields = vec![
        ("from".to_string(), from),
        (
            "subject".to_string(),
            email.wire_subject().unwrap_or_default().to_string(),
        ),
    ];

    for addr in &email.to {
        fields.push(("to".to_string(), addr.to_string()));
    }
    for addr in &email.cc {
        fields.push(("cc".to_string(), addr.to_string()));
    }
    for addr in &email.bcc {
        fields.push(("bcc".to_string(), addr.to_string()));
    }

    if let Some(text) = &email.text {
        fields.push(("text".to_string(), text.clone()));
    }
    if let Some(html) = &email.html {
        fields.push(("html".to_string(), html.clone()));
    }

    if let Some(reply_to) = &email.reply_to {
        fields.push(("h:Reply-To".to_string(), reply_to.to_string()));
    }

    // Custom headers and the headers implied by `priority` are passed through
    // Mailgun's `h:<name>` convention. Duplicate names are preserved: Mailgun
    // accepts repeated fields.
    for (name, value) in email.wire_headers() {
        fields.push((format!("h:{}", name), value));
    }

    // Threading headers live on dedicated `Email` fields and used to be read by
    // no transport at all, so `.in_reply_to(..)` was silently dropped.
    if let Some(id) = &email.message_id {
        fields.push(("h:Message-Id".to_string(), crate::email::angle_wrapped(id)));
    }
    if let Some(id) = &email.in_reply_to {
        fields.push(("h:In-Reply-To".to_string(), crate::email::angle_wrapped(id)));
    }
    if !email.references.is_empty() {
        let refs = email
            .references
            .iter()
            .map(|r| crate::email::angle_wrapped(r))
            .collect::<Vec<_>>()
            .join(" ");
        fields.push(("h:References".to_string(), refs));
    }

    Ok(fields)
}

/// Mailgun transport.
pub struct MailgunTransport {
    client: Client,
    config: MailgunConfig,
}

impl MailgunTransport {
    /// Create a new Mailgun transport.
    ///
    /// Fallible for two reasons. The endpoint is validated — `send` attaches the
    /// API key as HTTP basic auth, so a plaintext endpoint to a remote host is a
    /// credential disclosure on every send. And a client that cannot be built is
    /// surfaced rather than swapped for `Client::new()`, which carries neither a
    /// request nor a connect timeout.
    pub fn new(config: MailgunConfig) -> Result<Self> {
        crate::http::require_encrypted_endpoint(&config.endpoint())?;
        let client = build_client(config.timeout, config.connect_timeout)?;

        Ok(Self { client, config })
    }
}

#[async_trait]
impl Transport for MailgunTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        email.validate()?;

        debug!(
            to = ?email.to.iter().map(|a| &a.email).collect::<Vec<_>>(),
            subject = ?email.subject,
            "Sending email via Mailgun"
        );

        let mut form = Form::new();
        for (name, value) in build_form(email)? {
            form = form.text(name, value);
        }

        // Add attachments
        for attachment in &email.attachments {
            // `Part::stream_with_length` over the `Bytes` rather than
            // `Part::bytes`, which needs a `Cow<'static, [u8]>` and so would
            // copy the whole payload on every send attempt. The length is passed
            // explicitly so the part still carries a Content-Length.
            let len = attachment.data.len() as u64;
            let part = reqwest::multipart::Part::stream_with_length(attachment.data.clone(), len)
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
            // Previously hardcoded 60 and ignored the header. The queue's whole
            // retry schedule for a throttled job is keyed off this value, so
            // discarding a `Retry-After: 300` guaranteed a re-throttle.
            Err(MailError::RateLimited(
                retry_after_secs(response.headers()).unwrap_or(60),
            ))
        } else {
            // Not `unwrap_or_default()`: a truncated read would be
            // indistinguishable from a provider that sent an empty body.
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
            Err(MailError::provider(
                status.as_u16(),
                format!("Mailgun error {}: {}", status, body),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Email;

    fn base() -> Email {
        Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello")
    }

    fn find<'a>(fields: &'a [(String, String)], name: &str) -> Vec<&'a str> {
        fields
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The `h:` prefix is what makes Mailgun honor a
    /// custom header at all, and dropping it fails silently — the message is
    /// still accepted, just without the header.
    #[test]
    fn custom_headers_and_priority_carry_the_h_prefix() {
        let email = base()
            .header("X-Campaign-Id", "spring-2026")
            .high_priority();
        let fields = build_form(&email).unwrap();

        assert_eq!(find(&fields, "h:X-Campaign-Id"), ["spring-2026"]);
        assert_eq!(find(&fields, "h:X-Priority"), ["1 (Highest)"]);
        assert_eq!(find(&fields, "h:Importance"), ["High"]);
        assert_eq!(find(&fields, "h:X-MSMail-Priority"), ["High"]);

        // The unprefixed name must not appear: Mailgun would ignore it.
        assert!(find(&fields, "X-Campaign-Id").is_empty());
    }

    /// Mailgun accepts repeated form fields, so duplicate header names survive
    /// as distinct headers (SendGrid, whose `headers` object is single-valued,
    /// joins them instead).
    #[test]
    fn duplicate_header_names_are_preserved() {
        let email = base().header("X-Tag", "a").header("X-Tag", "b");
        let fields = build_form(&email).unwrap();

        assert_eq!(find(&fields, "h:X-Tag"), ["a", "b"]);
    }

    /// Mailgun never read `message_id`/`in_reply_to`/
    /// `references`, so threading was silently lost.
    #[test]
    fn threading_headers_are_emitted() {
        let email = base()
            .message_id("abc123@armature")
            .in_reply_to("<parent@example.com>")
            .reference("<root@example.com>")
            .reference("<mid@example.com>");
        let fields = build_form(&email).unwrap();

        assert_eq!(find(&fields, "h:Message-Id"), ["<abc123@armature>"]);
        assert_eq!(find(&fields, "h:In-Reply-To"), ["<parent@example.com>"]);
        assert_eq!(
            find(&fields, "h:References"),
            ["<root@example.com> <mid@example.com>"]
        );
    }

    #[test]
    fn basic_fields_and_recipients_are_present() {
        let email = base()
            .to("second@example.com")
            .cc("cc@example.com")
            .bcc("bcc@example.com")
            .html("<p>Hello</p>")
            .reply_to("reply@example.com");
        let fields = build_form(&email).unwrap();

        assert_eq!(find(&fields, "from"), ["sender@example.com"]);
        assert_eq!(find(&fields, "subject"), ["Test"]);
        assert_eq!(
            find(&fields, "to"),
            ["recipient@example.com", "second@example.com"]
        );
        assert_eq!(find(&fields, "cc"), ["cc@example.com"]);
        assert_eq!(find(&fields, "bcc"), ["bcc@example.com"]);
        assert_eq!(find(&fields, "text"), ["Hello"]);
        assert_eq!(find(&fields, "html"), ["<p>Hello</p>"]);
        assert_eq!(find(&fields, "h:Reply-To"), ["reply@example.com"]);
    }

    /// Mailgun writes header values into the form body
    /// verbatim, so a CRLF would inject a header. `Email::validate` — which
    /// `send` calls first — must reject it, and nothing may reach the form.
    #[test]
    fn crlf_injection_never_reaches_the_form() {
        let email = base().header("X-Tag", "ok\r\nBcc: evil@example.com");

        assert!(email.validate().is_err(), "CRLF header value was accepted");

        let fields = build_form(&email).unwrap();
        assert!(
            !fields
                .iter()
                .any(|(_, v)| v.contains('\r') || v.contains('\n')),
            "CRLF leaked into the Mailgun form: {fields:?}"
        );
        assert!(find(&fields, "h:X-Tag").is_empty());
    }

    /// The API key travels as HTTP basic auth on every request, so a plaintext
    /// base URL to a remote host discloses it.
    #[test]
    fn a_cleartext_base_url_is_rejected() {
        let config = MailgunConfig::new("key-secret", "example.com")
            .base_url("http://collector.example.com");
        let err = MailgunTransport::new(config).err().expect("must reject");

        assert!(matches!(err, MailError::Config(_)), "{err}");
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn https_and_loopback_endpoints_are_accepted() {
        assert!(MailgunTransport::new(MailgunConfig::new("key", "example.com")).is_ok());
        assert!(MailgunTransport::new(MailgunConfig::new("key", "example.com").eu()).is_ok());
        assert!(
            MailgunTransport::new(
                MailgunConfig::new("key", "example.com").base_url("http://127.0.0.1:8080")
            )
            .is_ok()
        );
    }

    #[test]
    fn both_timeouts_have_bounded_defaults() {
        let config = MailgunConfig::new("key", "example.com");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.connect_timeout, Duration::from_secs(10));

        let config = config.connect_timeout(Duration::from_millis(250));
        assert_eq!(config.connect_timeout, Duration::from_millis(250));
    }

    /// A derived `Debug` dumped the API key into any `tracing::debug!(?config)`.
    #[test]
    fn debug_never_renders_the_api_key() {
        let rendered = format!(
            "{:?}",
            MailgunConfig::new("key-super-secret", "example.com")
        );

        assert!(
            !rendered.contains("super-secret"),
            "API key leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn base_url_overrides_the_region() {
        let config = MailgunConfig::new("key", "example.com")
            .eu()
            .base_url("https://proxy.internal/");
        assert_eq!(
            config.endpoint(),
            "https://proxy.internal/v3/example.com/messages"
        );
    }

    #[test]
    fn crlf_in_a_display_name_never_reaches_the_form() {
        // The address layer rejects it, so it is recorded and `validate` fails.
        let email = base().to("Evil\r\nBcc: x@y.com <e@example.com>");
        assert!(email.validate().is_err());

        let fields = build_form(&email).unwrap();
        assert!(
            !fields
                .iter()
                .any(|(_, v)| v.contains('\r') || v.contains('\n')),
            "CRLF leaked into the Mailgun form: {fields:?}"
        );
    }
}
