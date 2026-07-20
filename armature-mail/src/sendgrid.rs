//! SendGrid email provider integration.

use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use std::fmt;
use std::time::Duration;
use tracing::debug;

use crate::http::{DEFAULT_CONNECT_TIMEOUT, DEFAULT_TIMEOUT, build_client, retry_after_secs};
use crate::{Email, MailError, Result, Transport};

/// SendGrid configuration.
///
/// `#[non_exhaustive]`: this type gained `timeout` and then `connect_timeout`,
/// and each addition broke every downstream struct literal. Construct it with
/// [`SendGridConfig::new`] plus the builders.
#[derive(Clone)]
#[non_exhaustive]
pub struct SendGridConfig {
    /// API key.
    pub api_key: String,
    /// API endpoint (defaults to production).
    pub endpoint: String,
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
impl fmt::Debug for SendGridConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendGridConfig")
            .field("api_key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

impl SendGridConfig {
    /// Create a new SendGrid configuration.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.sendgrid.com/v3/mail/send".to_string(),
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Set a custom endpoint (for testing, or a proxy).
    ///
    /// Validated by [`SendGridTransport::new`]: the API key travels on every
    /// request, so a plaintext endpoint to a non-loopback host is rejected.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
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
}

/// SendGrid transport.
pub struct SendGridTransport {
    client: Client,
    config: SendGridConfig,
}

impl SendGridTransport {
    /// Create a new SendGrid transport.
    ///
    /// Fallible for two reasons. The endpoint is validated — `send` attaches
    /// `Authorization: Bearer <api key>`, so a plaintext endpoint to a remote
    /// host is a credential disclosure on every send. And a client that cannot
    /// be built is surfaced rather than swapped for `Client::new()`, which
    /// carries neither a request nor a connect timeout.
    pub fn new(config: SendGridConfig) -> Result<Self> {
        crate::http::require_encrypted_endpoint(&config.endpoint)?;
        let client = build_client(config.timeout, config.connect_timeout)?;

        Ok(Self { client, config })
    }
}

#[async_trait]
impl Transport for SendGridTransport {
    async fn send(&self, email: &Email) -> Result<()> {
        email.validate()?;

        let payload = SendGridPayload::from_email(email)?;

        debug!(
            to = ?email.to.iter().map(|a| &a.email).collect::<Vec<_>>(),
            subject = ?email.subject,
            "Sending email via SendGrid"
        );

        let response = self
            .client
            .post(&self.config.endpoint)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| MailError::Network(e.to_string()))?;

        let status = response.status();

        if status.is_success() {
            debug!("Email sent successfully via SendGrid");
            Ok(())
        } else if status.as_u16() == 429 {
            Err(MailError::RateLimited(
                retry_after_secs(response.headers()).unwrap_or(60),
            ))
        } else {
            // Not `unwrap_or_default()`: that renders a connection dropped
            // mid-body identically to a provider that genuinely sent none,
            // which is the difference between "retry this" and "this payload is
            // wrong". Surface the read failure in the message instead.
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<body unreadable: {e}>"));
            // The status is carried on the error so `is_retryable` can tell a
            // transient 5xx from a permanent 4xx.
            Err(MailError::provider(
                status.as_u16(),
                format!("SendGrid error {}: {}", status, body),
            ))
        }
    }
}

/// SendGrid API payload.
#[derive(Debug, Serialize)]
struct SendGridPayload {
    personalizations: Vec<Personalization>,
    from: EmailAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<EmailAddress>,
    subject: String,
    content: Vec<Content>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<SendGridAttachment>,
    /// Custom headers, the headers implied by `Email::priority`, and the
    /// threading headers. Kept as an ordered list and serialized as a JSON
    /// object so emission order matches `Email::wire_headers`.
    #[serde(
        skip_serializing_if = "Vec::is_empty",
        serialize_with = "serialize_headers_as_map"
    )]
    headers: Vec<(String, String)>,
}

/// Serialize an ordered header list as SendGrid's single-valued `headers` object.
fn serialize_headers_as_map<S>(
    headers: &[(String, String)],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(headers.len()))?;
    for (name, value) in headers {
        map.serialize_entry(name, value)?;
    }
    map.end()
}

/// Build SendGrid's `headers` list for a message.
///
/// SendGrid's `headers` is a genuinely single-valued JSON object, so duplicate
/// names must be *combined*, not dropped: collecting into a map silently kept
/// only the last value, and alphabetised the emission order. RFC 5322 §3.2.2
/// allows folding repeated field values into one comma-separated list, which is
/// what a duplicate name becomes here.
///
/// The threading headers live on dedicated `Email` fields rather than in
/// `headers`, and SendGrid has no structured field for them, so they are
/// appended here — otherwise `.in_reply_to(..)` was silently dropped.
fn build_headers(email: &Email) -> Vec<(String, String)> {
    let mut ordered: Vec<(String, String)> = Vec::new();

    let mut push = |name: String, value: String| match ordered
        .iter_mut()
        .find(|(n, _)| n.eq_ignore_ascii_case(&name))
    {
        Some((_, existing)) => {
            existing.push_str(", ");
            existing.push_str(&value);
        }
        None => ordered.push((name, value)),
    };

    for (name, value) in email.wire_headers() {
        push(name, value);
    }

    if let Some(id) = &email.message_id {
        push("Message-ID".to_string(), crate::email::angle_wrapped(id));
    }
    if let Some(id) = &email.in_reply_to {
        push("In-Reply-To".to_string(), crate::email::angle_wrapped(id));
    }
    if !email.references.is_empty() {
        let refs = email
            .references
            .iter()
            .map(|r| crate::email::angle_wrapped(r))
            .collect::<Vec<_>>()
            .join(" ");
        push("References".to_string(), refs);
    }

    ordered
}

#[derive(Debug, Serialize)]
struct Personalization {
    to: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cc: Vec<EmailAddress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bcc: Vec<EmailAddress>,
}

#[derive(Debug, Serialize)]
struct EmailAddress {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct Content {
    #[serde(rename = "type")]
    content_type: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct SendGridAttachment {
    content: String,
    filename: String,
    #[serde(rename = "type")]
    content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
    disposition: String,
}

impl SendGridPayload {
    fn from_email(email: &Email) -> Result<Self> {
        use base64::Engine;

        let from = email.from.as_ref().ok_or(MailError::MissingField("from"))?;

        let mut content = Vec::new();

        if let Some(text) = &email.text {
            content.push(Content {
                content_type: "text/plain".to_string(),
                value: text.clone(),
            });
        }

        if let Some(html) = &email.html {
            content.push(Content {
                content_type: "text/html".to_string(),
                value: html.clone(),
            });
        }

        let attachments: Vec<SendGridAttachment> = email
            .attachments
            .iter()
            .map(|a| SendGridAttachment {
                content: base64::engine::general_purpose::STANDARD.encode(&a.data),
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                content_id: a.content_id.clone(),
                disposition: match a.disposition {
                    crate::ContentDisposition::Attachment => "attachment",
                    crate::ContentDisposition::Inline => "inline",
                }
                .to_string(),
            })
            .collect();

        Ok(Self {
            personalizations: vec![Personalization {
                to: email
                    .to
                    .iter()
                    .map(|a| EmailAddress {
                        email: a.email.clone(),
                        name: a.name.clone(),
                    })
                    .collect(),
                cc: email
                    .cc
                    .iter()
                    .map(|a| EmailAddress {
                        email: a.email.clone(),
                        name: a.name.clone(),
                    })
                    .collect(),
                bcc: email
                    .bcc
                    .iter()
                    .map(|a| EmailAddress {
                        email: a.email.clone(),
                        name: a.name.clone(),
                    })
                    .collect(),
            }],
            from: EmailAddress {
                email: from.email.clone(),
                name: from.name.clone(),
            },
            reply_to: email.reply_to.as_ref().map(|a| EmailAddress {
                email: a.email.clone(),
                name: a.name.clone(),
            }),
            subject: email.wire_subject().unwrap_or_default().to_string(),
            content,
            attachments,
            headers: build_headers(email),
        })
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

    /// Custom headers and priority were stored on `Email`
    /// but never reached the SendGrid payload.
    #[test]
    fn payload_carries_custom_headers_and_priority() {
        let email = base()
            .header("X-Campaign-Id", "spring-2026")
            .high_priority();
        let payload = SendGridPayload::from_email(&email).unwrap();
        let json = serde_json::to_value(&payload).unwrap();

        let headers = json.get("headers").expect("headers field emitted");
        assert_eq!(headers["X-Campaign-Id"], "spring-2026");
        assert_eq!(headers["X-Priority"], "1 (Highest)");
        assert_eq!(headers["Importance"], "High");
    }

    /// Collecting into a `BTreeMap` meant
    /// `.header("X-Tag","a").header("X-Tag","b")` reached SendGrid as a single
    /// `X-Tag: b` — the first value silently dropped — and alphabetised the
    /// emission order. SendGrid's `headers` really is single-valued, so
    /// duplicates are folded per RFC 5322 §3.2.2 instead of discarded.
    #[test]
    fn duplicate_header_names_are_joined_not_dropped() {
        let email = base().header("X-Tag", "a").header("X-Tag", "b");
        let payload = SendGridPayload::from_email(&email).unwrap();
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["headers"]["X-Tag"], "a, b");
    }

    #[test]
    fn header_emission_order_follows_wire_headers() {
        let email = base()
            .header("Z-Last", "z")
            .header("A-First", "a")
            .high_priority();
        let names: Vec<String> = build_headers(&email).into_iter().map(|(n, _)| n).collect();

        assert_eq!(
            names,
            [
                "Z-Last",
                "A-First",
                "X-Priority",
                "X-MSMail-Priority",
                "Importance"
            ]
        );
    }

    /// SendGrid never read `message_id`/`in_reply_to`/
    /// `references`, so threading was silently lost.
    #[test]
    fn threading_headers_are_emitted() {
        let email = base()
            .message_id("abc123@armature")
            .in_reply_to("<parent@example.com>")
            .reference("<root@example.com>")
            .reference("<mid@example.com>");
        let payload = SendGridPayload::from_email(&email).unwrap();
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["headers"]["Message-ID"], "<abc123@armature>");
        assert_eq!(json["headers"]["In-Reply-To"], "<parent@example.com>");
        assert_eq!(
            json["headers"]["References"],
            "<root@example.com> <mid@example.com>"
        );
    }

    /// SendGrid writes header values into raw JSON, so a
    /// CRLF in a value would inject a header. `send` calls `validate` first;
    /// this asserts nothing poisoned can reach the payload regardless.
    #[test]
    fn crlf_injection_never_reaches_the_payload() {
        let email = base().header("X-Tag", "ok\r\nBcc: evil@example.com");

        assert!(email.validate().is_err(), "CRLF header value was accepted");

        let payload = SendGridPayload::from_email(&email).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            !json.contains("Bcc"),
            "injected header leaked into the SendGrid payload: {json}"
        );
    }

    #[test]
    fn payload_omits_headers_when_there_are_none() {
        let payload = SendGridPayload::from_email(&base()).unwrap();
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("headers").is_none());
    }

    /// `SendGridConfig::new(key).endpoint("http://collector.example.com/…")`
    /// shipped the live API key in cleartext on every send, because `send`
    /// attaches `Authorization: Bearer {api_key}`.
    #[test]
    fn a_cleartext_endpoint_is_rejected() {
        let config = SendGridConfig::new("SG.secret").endpoint("http://collector.example.com/v3");
        let err = SendGridTransport::new(config).err().expect("must reject");

        assert!(matches!(err, MailError::Config(_)), "{err}");
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn https_and_loopback_endpoints_are_accepted() {
        assert!(SendGridTransport::new(SendGridConfig::new("SG.secret")).is_ok());
        assert!(
            SendGridTransport::new(
                SendGridConfig::new("SG.secret").endpoint("http://127.0.0.1:8080/v3/mail/send")
            )
            .is_ok()
        );
    }

    /// The client fallback had no request *and* no connect timeout. Both are
    /// configured, and the connect timeout has a bounded default.
    #[test]
    fn both_timeouts_have_bounded_defaults() {
        let config = SendGridConfig::new("SG.secret");
        assert_eq!(config.timeout, std::time::Duration::from_secs(30));
        assert_eq!(config.connect_timeout, std::time::Duration::from_secs(10));

        let config = config.connect_timeout(std::time::Duration::from_millis(250));
        assert_eq!(
            config.connect_timeout,
            std::time::Duration::from_millis(250)
        );
    }

    /// A derived `Debug` dumped the API key into any `tracing::debug!(?config)`.
    #[test]
    fn debug_never_renders_the_api_key() {
        let rendered = format!("{:?}", SendGridConfig::new("SG.super-secret-key"));

        assert!(
            !rendered.contains("super-secret-key"),
            "API key leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn payload_marks_inline_attachments() {
        let email = base().attach(
            crate::Attachment::png("logo.png", vec![1, 2, 3]).content_id("cid1".to_string()),
        );
        let payload = SendGridPayload::from_email(&email).unwrap();
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["attachments"][0]["disposition"], "inline");
        assert_eq!(json["attachments"][0]["content_id"], "cid1");
    }
}
