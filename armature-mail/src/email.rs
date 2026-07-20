//! Email message types.

use crate::{Address, Attachment, IntoAddress, MailError, Result};
use serde::{Deserialize, Serialize};

/// Email message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    /// Sender address.
    pub from: Option<Address>,
    /// Reply-to address.
    pub reply_to: Option<Address>,
    /// To recipients.
    pub to: Vec<Address>,
    /// CC recipients.
    pub cc: Vec<Address>,
    /// BCC recipients.
    pub bcc: Vec<Address>,
    /// Email subject.
    pub subject: Option<String>,
    /// Plain text body.
    pub text: Option<String>,
    /// HTML body.
    pub html: Option<String>,
    /// Attachments.
    pub attachments: Vec<Attachment>,
    /// Custom headers.
    pub headers: Vec<(String, String)>,
    /// Message ID.
    pub message_id: Option<String>,
    /// References (for threading).
    pub references: Vec<String>,
    /// In-Reply-To header.
    pub in_reply_to: Option<String>,
    /// Priority (1-5, 1 highest).
    pub priority: Option<u8>,
    /// Addresses that failed to parse in the fluent builders.
    ///
    /// The fluent setters ([`Email::to`], [`Email::cc`], [`Email::from`], …) take
    /// `impl IntoAddress` and therefore cannot return a `Result`. Rather than
    /// silently dropping a recipient whose address does not parse, the offending
    /// input is recorded here and [`Email::validate`] fails with
    /// [`MailError::InvalidAddress`], so a caller never sends to fewer recipients
    /// than it asked for without a signal.
    #[serde(default)]
    pub invalid_addresses: Vec<String>,
}

impl Email {
    /// Create a new empty email.
    pub fn new() -> Self {
        Self {
            from: None,
            reply_to: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: None,
            text: None,
            html: None,
            attachments: Vec::new(),
            headers: Vec::new(),
            message_id: None,
            references: Vec::new(),
            in_reply_to: None,
            priority: None,
            invalid_addresses: Vec::new(),
        }
    }

    /// Record an address that failed to parse so `validate()` can surface it.
    fn record_invalid(&mut self, err: MailError) {
        self.invalid_addresses.push(err.to_string());
    }

    /// Create a builder.
    pub fn builder() -> EmailBuilder {
        EmailBuilder::new()
    }

    /// Set the from address.
    ///
    /// If `from` does not parse, the error is recorded and surfaced by
    /// [`Email::validate`] rather than being silently dropped.
    pub fn from(mut self, from: impl IntoAddress) -> Self {
        match from.into_address() {
            Ok(addr) => self.from = Some(addr),
            Err(e) => self.record_invalid(e),
        }
        self
    }

    /// Set the reply-to address.
    ///
    /// If `reply_to` does not parse, the error is recorded and surfaced by
    /// [`Email::validate`].
    pub fn reply_to(mut self, reply_to: impl IntoAddress) -> Self {
        match reply_to.into_address() {
            Ok(addr) => self.reply_to = Some(addr),
            Err(e) => self.record_invalid(e),
        }
        self
    }

    /// Add a to recipient.
    ///
    /// If the address does not parse, the error is recorded and surfaced by
    /// [`Email::validate`] instead of the recipient being silently dropped.
    pub fn to(mut self, to: impl IntoAddress) -> Self {
        match to.into_address() {
            Ok(addr) => self.to.push(addr),
            Err(e) => self.record_invalid(e),
        }
        self
    }

    /// Add multiple to recipients.
    ///
    /// Addresses that fail to parse are recorded and surfaced by
    /// [`Email::validate`].
    pub fn to_many<I, A>(mut self, recipients: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: IntoAddress,
    {
        for r in recipients {
            match r.into_address() {
                Ok(addr) => self.to.push(addr),
                Err(e) => self.record_invalid(e),
            }
        }
        self
    }

    /// Add a CC recipient.
    ///
    /// If the address does not parse, the error is recorded and surfaced by
    /// [`Email::validate`].
    pub fn cc(mut self, cc: impl IntoAddress) -> Self {
        match cc.into_address() {
            Ok(addr) => self.cc.push(addr),
            Err(e) => self.record_invalid(e),
        }
        self
    }

    /// Add a BCC recipient.
    ///
    /// If the address does not parse, the error is recorded and surfaced by
    /// [`Email::validate`].
    pub fn bcc(mut self, bcc: impl IntoAddress) -> Self {
        match bcc.into_address() {
            Ok(addr) => self.bcc.push(addr),
            Err(e) => self.record_invalid(e),
        }
        self
    }

    /// Set the subject.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Set the plain text body.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Set the HTML body.
    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Add an attachment.
    pub fn attach(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Add a custom header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Set the message ID.
    pub fn message_id(mut self, id: impl Into<String>) -> Self {
        self.message_id = Some(id.into());
        self
    }

    /// Set the in-reply-to header (for threading).
    pub fn in_reply_to(mut self, id: impl Into<String>) -> Self {
        self.in_reply_to = Some(id.into());
        self
    }

    /// Add a reference (for threading).
    pub fn reference(mut self, id: impl Into<String>) -> Self {
        self.references.push(id.into());
        self
    }

    /// Set the priority (1-5, 1 being highest).
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = Some(priority.clamp(1, 5));
        self
    }

    /// Set high priority.
    pub fn high_priority(self) -> Self {
        self.priority(1)
    }

    /// Set low priority.
    pub fn low_priority(self) -> Self {
        self.priority(5)
    }

    /// Headers implied by [`Email::priority`], in the order they should be emitted.
    ///
    /// Returns an empty vector when no priority was set. `X-Priority` /
    /// `X-MSMail-Priority` are the de-facto Outlook convention; `Importance` is
    /// the RFC 4021 registered header. Emitting all three is what mail clients
    /// actually key off.
    pub fn priority_headers(&self) -> Vec<(String, String)> {
        let Some(priority) = self.priority else {
            return Vec::new();
        };
        let (x_priority, importance, ms) = match priority {
            1 => ("1 (Highest)", "High", "High"),
            2 => ("2 (High)", "High", "High"),
            3 => ("3 (Normal)", "Normal", "Normal"),
            4 => ("4 (Low)", "Low", "Low"),
            _ => ("5 (Lowest)", "Low", "Low"),
        };
        vec![
            ("X-Priority".to_string(), x_priority.to_string()),
            ("X-MSMail-Priority".to_string(), ms.to_string()),
            ("Importance".to_string(), importance.to_string()),
        ]
    }

    /// All custom headers to emit on the wire: the caller's [`Email::headers`]
    /// followed by the headers implied by [`Email::priority`].
    ///
    /// Transports should use this rather than reading `headers` directly, so
    /// priority is never dropped.
    pub fn wire_headers(&self) -> Vec<(String, String)> {
        let mut headers = self.headers.clone();
        headers.extend(self.priority_headers());
        headers
    }

    /// Validate the email.
    pub fn validate(&self) -> Result<()> {
        if !self.invalid_addresses.is_empty() {
            return Err(MailError::InvalidAddress(self.invalid_addresses.join("; ")));
        }
        if self.from.is_none() {
            return Err(MailError::MissingField("from"));
        }
        if self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() {
            return Err(MailError::MissingField("to/cc/bcc"));
        }
        if self.subject.is_none() {
            return Err(MailError::MissingField("subject"));
        }
        if self.text.is_none() && self.html.is_none() {
            return Err(MailError::MissingField("text/html body"));
        }
        Ok(())
    }

    /// Build a lettre message.
    pub(crate) fn to_lettre(&self) -> Result<lettre::Message> {
        self.validate()?;

        let from = self.from.as_ref().unwrap().to_mailbox()?;

        let mut builder = lettre::Message::builder()
            .from(from)
            .subject(self.subject.as_deref().unwrap_or_default());

        // Add recipients
        for addr in &self.to {
            builder = builder.to(addr.to_mailbox()?);
        }
        for addr in &self.cc {
            builder = builder.cc(addr.to_mailbox()?);
        }
        for addr in &self.bcc {
            builder = builder.bcc(addr.to_mailbox()?);
        }

        // Add reply-to
        if let Some(reply_to) = &self.reply_to {
            builder = builder.reply_to(reply_to.to_mailbox()?);
        }

        // Add message ID
        if let Some(msg_id) = &self.message_id {
            builder = builder.message_id(Some(msg_id.clone()));
        }

        // Add in-reply-to
        if let Some(in_reply_to) = &self.in_reply_to {
            builder = builder.in_reply_to(in_reply_to.clone());
        }

        // Add references
        for reference in &self.references {
            builder = builder.references(reference.clone());
        }

        // Emit custom headers and the headers implied by `priority`. lettre has no
        // typed header for arbitrary names, so these go in as raw header values.
        for (name, value) in self.wire_headers() {
            let header_name = lettre::message::header::HeaderName::new_from_ascii(name.clone())
                .map_err(|_| MailError::Smtp(format!("Invalid header name: {}", name)))?;
            builder = builder.raw_header(lettre::message::header::HeaderValue::new(
                header_name,
                value,
            ));
        }

        // Build body
        let body = match (&self.html, &self.text) {
            (Some(html), Some(text)) => {
                lettre::message::MultiPart::alternative_plain_html(text.clone(), html.clone())
            }
            (Some(html), None) => {
                lettre::message::MultiPart::alternative_plain_html(String::new(), html.clone())
            }
            (None, Some(text)) => {
                lettre::message::MultiPart::alternative_plain_html(text.clone(), String::new())
            }
            (None, None) => unreachable!(), // Validated above
        };

        // Split attachments: inline parts must live in a `multipart/related`
        // alongside the alternative body so that `cid:` references in the HTML
        // resolve; everything else goes in the outer `multipart/mixed`.
        let (inline, attached): (Vec<_>, Vec<_>) = self
            .attachments
            .iter()
            .partition(|a| a.is_inline() && a.content_id.is_some());

        let body = if inline.is_empty() {
            body
        } else {
            let mut related = lettre::message::MultiPart::related().multipart(body);
            for attachment in inline {
                related = related.singlepart(build_part(attachment)?);
            }
            related
        };

        let body = if attached.is_empty() {
            body
        } else {
            let mut mixed = lettre::message::MultiPart::mixed().multipart(body);
            for attachment in attached {
                mixed = mixed.singlepart(build_part(attachment)?);
            }
            mixed
        };

        builder
            .multipart(body)
            .map_err(|e| MailError::Smtp(e.to_string()))
    }
}

/// Build a lettre `SinglePart` for one attachment.
///
/// Attachments carrying a `content_id` (or explicitly marked
/// [`ContentDisposition::Inline`]) are emitted with `Content-Disposition: inline`
/// and a `Content-ID` header so that `<img src="cid:...">` references in the HTML
/// body resolve. Everything else becomes a normal `Content-Disposition:
/// attachment` part.
fn build_part(attachment: &Attachment) -> Result<lettre::message::SinglePart> {
    let content_type: lettre::message::header::ContentType =
        attachment.content_type.parse().map_err(|_| {
            MailError::Attachment(format!(
                "Invalid content type '{}' for attachment '{}'",
                attachment.content_type, attachment.filename
            ))
        })?;

    let part = match (&attachment.content_id, attachment.is_inline()) {
        (Some(cid), _) => lettre::message::Attachment::new_inline_with_name(
            cid.clone(),
            attachment.filename.clone(),
        ),
        // Inline disposition without a Content-ID: nothing can reference it via
        // `cid:`, but honor the requested disposition anyway.
        (None, true) => lettre::message::Attachment::new_inline_with_name(
            attachment.filename.clone(),
            attachment.filename.clone(),
        ),
        (None, false) => lettre::message::Attachment::new(attachment.filename.clone()),
    };

    Ok(part.body(attachment.data.clone(), content_type))
}

impl Default for Email {
    fn default() -> Self {
        Self::new()
    }
}

/// Email builder with validation.
#[derive(Default)]
pub struct EmailBuilder {
    email: Email,
}

impl EmailBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the from address.
    pub fn from(mut self, from: &str) -> Result<Self> {
        self.email.from = Some(Address::parse(from)?);
        Ok(self)
    }

    /// Set the to address.
    pub fn to(mut self, to: &str) -> Result<Self> {
        self.email.to.push(Address::parse(to)?);
        Ok(self)
    }

    /// Set the subject.
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.email.subject = Some(subject.into());
        self
    }

    /// Set the text body.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.email.text = Some(text.into());
        self
    }

    /// Set the HTML body.
    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.email.html = Some(html.into());
        self
    }

    /// Add an attachment.
    pub fn attach(mut self, attachment: Attachment) -> Self {
        self.email.attachments.push(attachment);
        self
    }

    /// Build and validate the email.
    pub fn build(self) -> Result<Email> {
        self.email.validate()?;
        Ok(self.email)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_builder() {
        let email = Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello, world!");

        assert!(email.validate().is_ok());
    }

    #[test]
    fn test_email_missing_from() {
        let email = Email::new()
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello");

        assert!(email.validate().is_err());
    }

    /// Base email used by the MIME-assembly regression tests.
    fn base() -> Email {
        Email::new()
            .from("sender@example.com")
            .to("recipient@example.com")
            .subject("Test")
            .text("Hello")
            .html("<p>Hello</p>")
    }

    fn formatted(email: &Email) -> String {
        String::from_utf8_lossy(&email.to_lettre().unwrap().formatted()).into_owned()
    }

    /// WF6: a regular attachment must actually appear in the built MIME message.
    #[test]
    fn attachment_is_present_in_built_message() {
        let email = base().attach(crate::Attachment::text("report.csv", "a,b\n1,2\n"));

        let wire = formatted(&email);

        assert!(
            wire.contains(r#"Content-Disposition: attachment; filename="report.csv""#),
            "attachment part missing:\n{wire}"
        );
        assert!(wire.contains("multipart/mixed"), "no mixed part:\n{wire}");
    }

    /// WF6 finding 6: inline attachments were built with `Attachment::new`, so they
    /// carried no Content-ID and no inline disposition — every documented `cid:`
    /// reference in the HTML failed to resolve. They must now be emitted inside a
    /// `multipart/related` with both headers.
    #[test]
    fn inline_attachment_carries_content_id_and_inline_disposition() {
        let logo = crate::Attachment::png("logo.png", vec![0x89, 0x50, 0x4E, 0x47])
            .content_id("logo123".to_string());
        let email = base()
            .html(r#"<img src="cid:logo123">"#)
            .attach(logo.clone());

        let wire = formatted(&email);

        assert!(
            wire.contains("Content-ID: <logo123>"),
            "no Content-ID header:\n{wire}"
        );
        assert!(
            wire.contains("Content-Disposition: inline"),
            "no inline disposition:\n{wire}"
        );
        assert!(
            wire.contains("multipart/related"),
            "inline part not in a related container:\n{wire}"
        );
        // An inline part must not be advertised as a downloadable attachment.
        assert!(
            !wire.contains(r#"Content-Disposition: attachment; filename="logo.png""#),
            "inline part emitted as attachment:\n{wire}"
        );
    }

    /// Inline and regular attachments coexist: related nested inside mixed.
    #[test]
    fn inline_and_regular_attachments_coexist() {
        let email = base()
            .attach(
                crate::Attachment::png("logo.png", vec![1, 2, 3]).content_id("cid1".to_string()),
            )
            .attach(crate::Attachment::text("notes.txt", "hi"));

        let wire = formatted(&email);

        assert!(wire.contains("multipart/mixed"), "{wire}");
        assert!(wire.contains("multipart/related"), "{wire}");
        assert!(wire.contains("Content-ID: <cid1>"), "{wire}");
        assert!(
            wire.contains(r#"Content-Disposition: attachment; filename="notes.txt""#),
            "{wire}"
        );
    }

    /// WF6 finding 4: `Email::header` was stored but read by no transport.
    #[test]
    fn custom_headers_are_emitted() {
        let email = base()
            .header("X-Campaign-Id", "spring-2026")
            .header("X-Entity-Ref-Id", "abc-123");

        let wire = formatted(&email);

        assert!(
            wire.contains("X-Campaign-Id: spring-2026"),
            "custom header dropped:\n{wire}"
        );
        assert!(
            wire.contains("X-Entity-Ref-Id: abc-123"),
            "custom header dropped:\n{wire}"
        );
    }

    /// WF6 finding 5: `Email::priority` was clamped and stored but never emitted.
    #[test]
    fn priority_is_emitted_as_headers() {
        let wire = formatted(&base().high_priority());
        assert!(wire.contains("X-Priority: 1 (Highest)"), "{wire}");
        assert!(wire.contains("Importance: High"), "{wire}");
        assert!(wire.contains("X-MSMail-Priority: High"), "{wire}");

        let wire = formatted(&base().low_priority());
        assert!(wire.contains("X-Priority: 5 (Lowest)"), "{wire}");
        assert!(wire.contains("Importance: Low"), "{wire}");
    }

    #[test]
    fn no_priority_means_no_priority_headers() {
        let email = base();
        assert!(email.priority_headers().is_empty());
        assert!(!formatted(&email).contains("X-Priority"));
    }

    /// WF6 finding 11: the fluent builders used to swallow parse errors with
    /// `.ok()` / `if let Ok`, so a caller could silently send to fewer recipients
    /// than it asked for. The error must now reach `validate()`.
    #[test]
    fn invalid_recipient_surfaces_instead_of_being_dropped() {
        let email = Email::new()
            .from("sender@example.com")
            .to("good@example.com")
            .to("not-an-email")
            .subject("Test")
            .text("Hello");

        // The valid recipient is still recorded...
        assert_eq!(email.to.len(), 1);
        // ...but the invalid one is not silently forgotten.
        assert_eq!(email.invalid_addresses.len(), 1);
        assert!(matches!(
            email.validate(),
            Err(MailError::InvalidAddress(_))
        ));
        assert!(email.to_lettre().is_err());
    }

    #[test]
    fn invalid_from_and_cc_and_bcc_surface() {
        for email in [
            Email::new().from("nope"),
            Email::new().from("s@example.com").cc("nope"),
            Email::new().from("s@example.com").bcc("nope"),
            Email::new().from("s@example.com").reply_to("nope"),
            Email::new()
                .from("s@example.com")
                .to_many(["nope", "x@y.com"]),
        ] {
            let email = email.subject("s").text("t").to("ok@example.com");
            assert!(
                matches!(email.validate(), Err(MailError::InvalidAddress(_))),
                "invalid address was swallowed: {:?}",
                email.invalid_addresses
            );
        }
    }

    /// An attachment whose declared content type is unparseable must be rejected
    /// rather than silently downgraded to `text/plain` (which corrupts binaries).
    #[test]
    fn invalid_attachment_content_type_is_rejected() {
        let email = base().attach(crate::Attachment::new(
            "x.bin",
            "not a mime type",
            vec![1, 2],
        ));
        assert!(matches!(email.to_lettre(), Err(MailError::Attachment(_))));
    }

    #[test]
    fn wire_headers_combine_custom_and_priority() {
        let email = base().header("X-Custom", "v").priority(2);
        let wire = email.wire_headers();
        assert_eq!(wire[0], ("X-Custom".to_string(), "v".to_string()));
        assert!(
            wire.iter()
                .any(|(n, v)| n == "X-Priority" && v == "2 (High)")
        );
    }
}
