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
    /// Headers rejected by [`Email::header`], recorded for [`Email::validate`].
    ///
    /// Same rationale as [`Email::invalid_addresses`]: the fluent setter cannot
    /// return a `Result`, and a header carrying a CR/LF must never reach a
    /// transport that writes header values verbatim.
    #[serde(default)]
    pub invalid_headers: Vec<String>,
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
            invalid_headers: Vec::new(),
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
    ///
    /// The name and the value are both validated. A CR or LF in either is a
    /// header-injection primitive on every transport that writes headers by hand
    /// (Mailgun's form body, SendGrid's JSON); only the SMTP path is
    /// independently protected by lettre's encoder. An invalid header is
    /// recorded and surfaced by [`Email::validate`] rather than silently
    /// dropped — this setter cannot return a `Result`.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        match validate_header(&name, &value) {
            Ok(()) => self.headers.push((name, value)),
            Err(e) => self.invalid_headers.push(e.to_string()),
        }
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
    ///
    /// Headers that fail validation are omitted here; [`Email::validate`] — which
    /// every transport calls before touching this — has already failed the send,
    /// so nothing unvalidated can reach the wire even if a caller populated
    /// [`Email::headers`] directly instead of going through [`Email::header`].
    pub fn wire_headers(&self) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = self
            .headers
            .iter()
            .filter(|(n, v)| validate_header(n, v).is_ok())
            .cloned()
            .collect();
        headers.extend(self.priority_headers());
        headers
    }

    /// The subject as it should be written to the wire: [`Email::subject`] with
    /// any trailing whitespace removed.
    ///
    /// [`Email::validate`] validates this value rather than the raw field, so
    /// transports must emit this one to stay consistent with what was checked.
    pub(crate) fn wire_subject(&self) -> Option<&str> {
        self.subject.as_deref().map(str::trim_end)
    }

    /// Validate the email.
    pub fn validate(&self) -> Result<()> {
        if !self.invalid_addresses.is_empty() {
            return Err(MailError::InvalidAddress(self.invalid_addresses.join("; ")));
        }
        if !self.invalid_headers.is_empty() {
            return Err(MailError::Config(self.invalid_headers.join("; ")));
        }
        // `headers` is a public field, so a caller can bypass `Email::header`.
        // Re-check here: this is the single choke point every transport passes
        // through before serializing header values onto the wire.
        for (name, value) in &self.headers {
            validate_header(name, value)?;
        }
        if self.from.is_none() {
            return Err(MailError::MissingField("from"));
        }
        if self.to.is_empty() && self.cc.is_empty() && self.bcc.is_empty() {
            return Err(MailError::MissingField("to/cc/bcc"));
        }
        let Some(subject) = self.subject.as_deref() else {
            return Err(MailError::MissingField("subject"));
        };
        if self.text.is_none() && self.html.is_none() {
            return Err(MailError::MissingField("text/html body"));
        }

        // `subject` and the three threading fields all become header values on
        // the wire — `Subject:` and Mailgun's `h:`-prefixed form fields, and
        // SendGrid's `headers` JSON object. They bypassed `Email::header`, so a
        // control character in any of them reached those transports unchecked
        // and could inject a header exactly as a poisoned custom header would.
        // `trim_end`: a template that ends its `subject` file with a newline
        // produces a subject ending in `\n`. That is not an injection attempt —
        // it cannot introduce a second header because nothing follows it — and
        // rejecting it would refuse to send mail that 0.1.x delivered. The
        // trailing run is stripped here and by [`Email::wire_subject`], which is
        // what every transport emits, so it never reaches the wire either.
        validate_header_value("Subject", subject.trim_end())?;
        if let Some(id) = &self.message_id {
            validate_header_value("Message-ID", id)?;
        }
        if let Some(id) = &self.in_reply_to {
            validate_header_value("In-Reply-To", id)?;
        }
        for reference in &self.references {
            validate_header_value("References", reference)?;
        }

        Ok(())
    }

    /// Build a lettre message.
    pub(crate) fn to_lettre(&self) -> Result<lettre::Message> {
        self.validate()?;

        // `validate` above proved both are present; binding them once here keeps
        // the `unwrap_or_default()` that silently sent an empty subject — for an
        // email that had already failed validation — out of the build path.
        let from = self
            .from
            .as_ref()
            .expect("validated: from is present")
            .to_mailbox()?;
        let subject = self.wire_subject().expect("validated: subject is present");

        let mut builder = lettre::Message::builder().from(from).subject(subject);

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

        // Add message ID. RFC 5322 requires the angle brackets; lettre passes the
        // value through verbatim, so a bare `id@host` would go out malformed and
        // defeat downstream deduplication.
        if let Some(msg_id) = &self.message_id {
            builder = builder.message_id(Some(angle_wrapped(msg_id)));
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

/// Validate a custom header name and value.
///
/// Header names must be non-empty RFC 5322 `field-name` tokens: printable ASCII
/// excluding `:` and space. Header values must contain no CR, LF, NUL, or other
/// ASCII control character — a `\r\n` in a value lets a caller append arbitrary
/// headers (`Bcc:`, `Content-Type:`) to the message on any transport that writes
/// the value verbatim.
pub fn validate_header(name: &str, value: &str) -> Result<()> {
    if name.is_empty() {
        return Err(MailError::Config("Header name cannot be empty".to_string()));
    }
    if let Some(c) = name
        .chars()
        .find(|c| !c.is_ascii() || c.is_ascii_control() || *c == ':' || *c == ' ')
    {
        return Err(MailError::Config(format!(
            "Invalid character {:?} in header name {:?}",
            c, name
        )));
    }
    validate_header_value(name, value)
}

/// Validate a header *value* only, for the fields that become headers without
/// going through [`Email::header`] — `Subject`, `Message-ID`, `In-Reply-To` and
/// `References`.
///
/// Crate-internal: every caller is in this module, and [`validate_header`] is
/// the entry point a user actually has a name to check against.
pub(crate) fn validate_header_value(name: &str, value: &str) -> Result<()> {
    if let Some(c) = value.chars().find(|c| c.is_ascii_control()) {
        return Err(MailError::Config(format!(
            "Invalid control character {:?} in value of header {:?}",
            c, name
        )));
    }
    Ok(())
}

/// Wrap a message identifier in angle brackets if it is not already.
pub(crate) fn angle_wrapped(id: &str) -> String {
    let id = id.trim();
    if id.starts_with('<') && id.ends_with('>') {
        id.to_string()
    } else {
        format!("<{}>", id)
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
        (Some(cid), true) => lettre::message::Attachment::new_inline_with_name(
            cid.clone(),
            attachment.filename.clone(),
        ),
        // Inline disposition without a Content-ID: nothing can reference it via
        // `cid:`, but honor the requested disposition anyway.
        (None, true) => lettre::message::Attachment::new_inline_with_name(
            attachment.filename.clone(),
            attachment.filename.clone(),
        ),
        // An explicit `ContentDisposition::Attachment` wins over the presence of a
        // Content-ID. Matching `(Some(_), _)` above made any attachment carrying a
        // content-id inline, so a caller that asked for a downloadable attachment
        // got one no mail client would offer for download.
        (Some(_), false) | (None, false) => {
            lettre::message::Attachment::new(attachment.filename.clone())
        }
    };

    // lettre's `IntoBody` is implemented over `Into<MaybeString>`, which `Bytes`
    // is not, so the payload is materialized as a `Vec<u8>` here. `Attachment`
    // itself holds `Bytes`, so this is the *only* copy on the path — cloning the
    // `Email`, the `EmailJob`, or the attachment list no longer copies payloads.
    Ok(part.body(attachment.data.to_vec(), content_type))
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

    /// Inline attachments were built with `Attachment::new`, so they
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

    /// `Email::header` was stored but read by no transport.
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

    /// `Email::priority` was clamped and stored but never emitted.
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

    /// The fluent builders used to swallow parse errors with
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

    /// `(Some(cid), _)` made *any* attachment carrying a
    /// content-id inline, so an explicit `ContentDisposition::Attachment` was
    /// ignored and no mail client offered the file for download.
    #[test]
    fn content_id_does_not_override_an_explicit_attachment_disposition() {
        let email = base().attach(
            crate::Attachment::png("chart.png", vec![1, 2, 3])
                .content_id("chart1")
                .disposition(crate::ContentDisposition::Attachment),
        );

        let wire = formatted(&email);

        assert!(
            wire.contains(r#"Content-Disposition: attachment; filename="chart.png""#),
            "explicit attachment disposition was overridden by the content-id:\n{wire}"
        );
    }

    #[test]
    fn message_id_is_wrapped_in_angle_brackets() {
        let wire = formatted(&base().message_id("abc123@armature"));
        assert!(wire.contains("Message-ID: <abc123@armature>"), "{wire}");

        // An id that already has them is not double-wrapped.
        let wire = formatted(&base().message_id("<abc123@armature>"));
        assert!(wire.contains("Message-ID: <abc123@armature>"), "{wire}");
        assert!(!wire.contains("<<"), "{wire}");
    }

    /// Header values were never validated, so a `\r\n` in a
    /// value let a caller inject arbitrary headers on the Mailgun and SendGrid
    /// transports, which write the value verbatim.
    #[test]
    fn crlf_in_a_header_value_is_rejected() {
        let email = base().header("X-Tag", "ok\r\nBcc: evil@example.com");

        assert!(
            matches!(email.validate(), Err(MailError::Config(_))),
            "CRLF header value was accepted"
        );
        assert!(email.headers.is_empty(), "poisoned header was stored");
        assert!(!email.wire_headers().iter().any(|(_, v)| v.contains('\n')));
        assert!(email.to_lettre().is_err());
    }

    #[test]
    fn invalid_header_names_are_rejected() {
        for (name, value) in [
            ("X-Bad: Injected", "v"),
            ("X-Bad\r\nBcc", "v"),
            ("", "v"),
            ("X Bad", "v"),
            ("X-Bad\u{0}", "v"),
        ] {
            let email = base().header(name, value);
            assert!(
                email.validate().is_err(),
                "header name {name:?} was accepted"
            );
        }
    }

    /// A caller can populate the public `headers` field directly, bypassing
    /// `Email::header`; `validate` is the choke point every transport passes.
    #[test]
    fn directly_populated_headers_are_still_validated() {
        let mut email = base();
        email
            .headers
            .push(("X-Tag".to_string(), "a\r\nBcc: evil@x.com".to_string()));

        assert!(matches!(email.validate(), Err(MailError::Config(_))));
        assert!(
            email.wire_headers().is_empty(),
            "invalid header leaked into wire_headers()"
        );
    }

    #[test]
    fn ordinary_headers_still_pass() {
        let email = base().header("X-Campaign-Id", "spring-2026");
        assert!(email.validate().is_ok());
        assert_eq!(email.wire_headers().len(), 1);
    }

    /// `validate` is documented as the single choke point every transport passes
    /// through, but it checked only `headers` — `subject`, `message_id`,
    /// `in_reply_to` and `references` all become `h:`-prefixed Mailgun fields
    /// and entries in SendGrid's `headers` object, unchecked.
    #[test]
    fn control_characters_in_subject_and_threading_fields_are_rejected() {
        let poison = "ok\r\nBcc: evil@example.com";

        let cases: Vec<Email> = vec![
            base().subject(poison),
            base().message_id(poison),
            base().in_reply_to(poison),
            base().reference(poison),
        ];

        for email in cases {
            assert!(
                matches!(email.validate(), Err(MailError::Config(_))),
                "injected value was accepted: {email:?}"
            );
            assert!(email.to_lettre().is_err());
        }
    }

    /// A `subject.hbs` template ending in a newline renders a subject ending in
    /// `\n`. That is not header injection — nothing follows the trailing run —
    /// and rejecting it would refuse mail 0.1.x delivered. It must validate, and
    /// the newline must not reach the wire.
    #[test]
    fn a_trailing_newline_in_the_subject_is_trimmed_not_rejected() {
        for raw in ["Welcome Bob\n", "Welcome Bob\r\n", "Welcome Bob  \n\n"] {
            let email = base().subject(raw);
            assert!(
                email.validate().is_ok(),
                "trailing whitespace rejected: {raw:?}"
            );
            assert_eq!(email.wire_subject(), Some("Welcome Bob"));

            let wire = formatted(&email);
            assert!(wire.contains("Subject: Welcome Bob"), "{wire}");
        }
    }

    /// Trimming the tail must not weaken the injection check: a control
    /// character with anything after it is still rejected.
    #[test]
    fn a_trailing_trim_does_not_excuse_an_embedded_injection() {
        let email = base().subject("ok\r\nBcc: evil@example.com\n");
        assert!(matches!(email.validate(), Err(MailError::Config(_))));
        assert!(email.to_lettre().is_err());
    }

    #[test]
    fn ordinary_subjects_and_threading_fields_still_pass() {
        let email = base()
            .subject("Your receipt — order #42")
            .message_id("abc@armature")
            .in_reply_to("<parent@example.com>")
            .reference("<root@example.com>");
        assert!(email.validate().is_ok());
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
