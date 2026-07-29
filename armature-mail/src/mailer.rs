//! High-level mailer interface.

use std::sync::Arc;
use tracing::debug;

use crate::{
    Address, Email, MailError, Result, SmtpConfig, SmtpTransport, TemplateEngine, Transport,
};

/// Mailer configuration.
///
/// `#[non_exhaustive]`: this type gained `bulk_concurrency` last round, which
/// broke every downstream struct literal. Construct it with
/// `MailerConfig::default()` plus the builders.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MailerConfig {
    /// Default from address.
    pub default_from: Option<Address>,
    /// Default reply-to address.
    pub default_reply_to: Option<Address>,
    /// Retry count for failed sends.
    pub retry_count: u32,
    /// Retry delay.
    pub retry_delay: std::time::Duration,
    /// Maximum number of concurrent sends in [`Mailer::send_bulk`].
    ///
    /// Defaults to 4, matching `SmtpConfig::pool_size` — the pool exists to be
    /// used concurrently, and a fully sequential bulk send never exercises it.
    pub bulk_concurrency: usize,
}

impl Default for MailerConfig {
    fn default() -> Self {
        Self {
            default_from: None,
            default_reply_to: None,
            retry_count: 3,
            retry_delay: std::time::Duration::from_secs(1),
            bulk_concurrency: 4,
        }
    }
}

impl MailerConfig {
    /// Set the default from address.
    pub fn from(mut self, from: &str) -> Result<Self> {
        self.default_from = Some(Address::parse(from)?);
        Ok(self)
    }

    /// Set the default reply-to address.
    pub fn reply_to(mut self, reply_to: &str) -> Result<Self> {
        self.default_reply_to = Some(Address::parse(reply_to)?);
        Ok(self)
    }

    /// Set the retry count.
    pub fn retries(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    /// Set the maximum number of concurrent sends in [`Mailer::send_bulk`].
    ///
    /// Clamped to at least 1: a concurrency of 0 would make `send_bulk` hang.
    pub fn bulk_concurrency(mut self, concurrency: usize) -> Self {
        self.bulk_concurrency = concurrency.max(1);
        self
    }
}

/// High-level mailer for sending emails.
pub struct Mailer {
    transport: Arc<dyn Transport>,
    config: MailerConfig,
    templates: Option<Arc<dyn TemplateEngine>>,
}

impl Mailer {
    /// Create a new mailer with an SMTP transport.
    pub async fn smtp(smtp_config: SmtpConfig) -> Result<Self> {
        let transport = SmtpTransport::new(smtp_config).await?;
        Ok(Self {
            transport: Arc::new(transport),
            config: MailerConfig::default(),
            templates: None,
        })
    }

    /// Create a new mailer with a custom transport.
    pub fn new(transport: impl Transport + 'static) -> Self {
        Self {
            transport: Arc::new(transport),
            config: MailerConfig::default(),
            templates: None,
        }
    }

    /// Set the mailer configuration.
    pub fn with_config(mut self, config: MailerConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the default from address.
    pub fn default_from(mut self, from: &str) -> Result<Self> {
        self.config.default_from = Some(Address::parse(from)?);
        Ok(self)
    }

    /// Set a template engine.
    pub fn with_template_engine(mut self, engine: impl TemplateEngine + 'static) -> Self {
        self.templates = Some(Arc::new(engine));
        self
    }

    /// Load templates from a directory (requires handlebars feature).
    #[cfg(feature = "handlebars")]
    pub fn with_templates(mut self, path: impl AsRef<std::path::Path>) -> Result<Self> {
        let engine = crate::HandlebarsEngine::from_directory(path)?;
        self.templates = Some(Arc::new(engine));
        Ok(self)
    }

    /// Send an email.
    pub async fn send(&self, email: Email) -> Result<()> {
        let email = self.apply_defaults(email);
        self.send_with_retry(&email).await
    }

    /// Send a shared email without taking ownership.
    ///
    /// Used by the queue worker, where the same `Email` — attachments and all —
    /// is sent once per attempt. The email is only cloned when a default
    /// actually has to be applied to it, so the common case (an email that
    /// already has a `from`) copies nothing.
    pub async fn send_shared(&self, email: Arc<Email>) -> Result<()> {
        let needs_defaults = (email.from.is_none() && self.config.default_from.is_some())
            || (email.reply_to.is_none() && self.config.default_reply_to.is_some());

        if needs_defaults {
            let owned = self.apply_defaults((*email).clone());
            return self.send_with_retry(&owned).await;
        }

        self.send_with_retry(&email).await
    }

    /// Send an email using a template.
    pub async fn send_template(
        &self,
        template_name: &str,
        to: &str,
        context: serde_json::Value,
    ) -> Result<()> {
        let templates = self
            .templates
            .as_ref()
            .ok_or_else(|| MailError::Template("No template engine configured".to_string()))?;

        let rendered = templates.render(template_name, &context)?;
        let to_addr = Address::parse(to)?;

        let mut email = Email::new().to(to_addr);

        if let Some(subject) = rendered.subject {
            email = email.subject(subject);
        }
        if let Some(html) = rendered.html {
            email = email.html(html);
        }
        if let Some(text) = rendered.text {
            email = email.text(text);
        }

        self.send(email).await
    }

    /// Send a simple text email.
    pub async fn send_text(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let email = Email::new().to(to).subject(subject).text(body);

        self.send(email).await
    }

    /// Send a simple HTML email.
    pub async fn send_html(&self, to: &str, subject: &str, html: &str) -> Result<()> {
        let email = Email::new().to(to).subject(subject).html(html);

        self.send(email).await
    }

    /// Send to multiple recipients.
    ///
    /// Up to [`MailerConfig::bulk_concurrency`] sends are in flight at once, so
    /// one slow recipient does not stall the whole batch. Results are returned
    /// in the same order as `emails` regardless of completion order.
    pub async fn send_bulk(&self, emails: Vec<Email>) -> Vec<Result<()>> {
        use futures_util::stream::StreamExt;

        let concurrency = self.config.bulk_concurrency.max(1);

        futures_util::stream::iter(emails)
            .map(|email| self.send(email))
            // `buffered` (not `buffer_unordered`) preserves input ordering.
            .buffered(concurrency)
            .collect()
            .await
    }

    /// Check if the transport is healthy.
    pub async fn is_healthy(&self) -> bool {
        self.transport.is_healthy().await
    }

    /// Apply default configuration to an email.
    fn apply_defaults(&self, mut email: Email) -> Email {
        if email.from.is_none() {
            email.from = self.config.default_from.clone();
        }
        if email.reply_to.is_none() {
            email.reply_to = self.config.default_reply_to.clone();
        }
        email
    }

    /// Send with retry logic.
    ///
    /// When the provider tells us how long to wait (`Retry-After`, surfaced as
    /// [`MailError::retry_after`]) that delay is honored instead of the local
    /// one: retrying a `Retry-After: 300` after the configured 1s just gets
    /// re-throttled and burns quota.
    async fn send_with_retry(&self, email: &Email) -> Result<()> {
        let mut last_error: Option<MailError> = None;

        for attempt in 0..=self.config.retry_count {
            if attempt > 0 {
                let delay = last_error
                    .as_ref()
                    .and_then(|e| e.retry_after())
                    .unwrap_or(self.config.retry_delay);
                debug!(attempt, ?delay, "Retrying email send");
                tokio::time::sleep(delay).await;
            }

            match self.transport.send(email).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if !e.is_retryable() || attempt >= self.config.retry_count {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        // Unreachable: the loop runs `0..=retry_count`, and on its final
        // iteration `attempt >= retry_count` holds, so every path through the
        // last iteration returns — `Ok(())` or `Err(e)`. Kept as an `Err` rather
        // than a panic (`unreachable!()`, `.expect(..)`): trading a
        // guaranteed-unused error return for a panic in a library is a strictly
        // worse failure mode if the invariant above is ever broken by an edit to
        // the loop.
        Err(last_error.unwrap_or_else(|| {
            MailError::provider(
                None,
                "send_with_retry exhausted its attempts without recording an error",
            )
        }))
    }
}

/// Escape a value for interpolation into HTML text or a quoted attribute.
///
/// The convenience builders below assemble HTML with `format!`, bypassing the
/// crate's own escaping template path. Without this, a `name` or link containing
/// markup breaks the layout at best and injects a phishing link into
/// transactional mail sent to another user at worst.
///
/// Both quote styles are escaped so the result is also safe inside `href="…"`.
fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convenience functions for quick email sending.
impl Mailer {
    /// Create a welcome email.
    ///
    /// `name` and `activation_link` are HTML-escaped before interpolation.
    pub fn welcome_email(to: &str, name: &str, activation_link: &str) -> Email {
        Email::new()
            .to(to)
            .subject("Welcome! Please activate your account")
            .html(format!(
                r#"
                <h1>Welcome, {}!</h1>
                <p>Thank you for signing up. Please click the link below to activate your account:</p>
                <p><a href="{}">Activate Account</a></p>
                "#,
                escape_html(name),
                escape_html(activation_link)
            ))
            .text(format!(
                "Welcome, {}!\n\nPlease visit this link to activate your account: {}",
                name, activation_link
            ))
    }

    /// Create a password reset email.
    ///
    /// `reset_link` and `expires_in` are HTML-escaped before interpolation;
    /// `reset_link` lands inside a quoted `href`, so an unescaped quote would let
    /// a caller-supplied value add attributes to the anchor.
    pub fn password_reset_email(to: &str, reset_link: &str, expires_in: &str) -> Email {
        Email::new()
            .to(to)
            .subject("Password Reset Request")
            .html(format!(
                r#"
                <h1>Password Reset</h1>
                <p>We received a request to reset your password.</p>
                <p><a href="{}">Reset Password</a></p>
                <p>This link will expire in {}.</p>
                <p>If you didn't request this, please ignore this email.</p>
                "#,
                escape_html(reset_link),
                escape_html(expires_in)
            ))
            .text(format!(
                "Password Reset\n\nVisit this link to reset your password: {}\n\nThis link will expire in {}.\n\nIf you didn't request this, please ignore this email.",
                reset_link, expires_in
            ))
    }

    /// Create a notification email.
    ///
    /// `title` and `message` are HTML-escaped before interpolation into the HTML
    /// part. The plain-text part and the subject keep the original values.
    pub fn notification_email(to: &str, title: &str, message: &str) -> Email {
        Email::new()
            .to(to)
            .subject(title)
            .text(message)
            .html(format!(
                "<h2>{}</h2><p>{}</p>",
                escape_html(title),
                escape_html(message)
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XSS: &str = "<script>alert(1)</script>";

    fn assert_escaped(html: &str) {
        assert!(html.contains("&lt;script&gt;"), "not escaped:\n{html}");
        assert!(
            !html.contains("<script>"),
            "raw script tag present:\n{html}"
        );
    }

    /// The convenience builders interpolated caller values
    /// into HTML with `format!`, bypassing the crate's own escaping template
    /// path, so markup in a name hijacked the rendered body.
    #[test]
    fn welcome_email_escapes_the_name_and_link() {
        let email = Mailer::welcome_email("to@example.com", XSS, "https://x.test/a?b=1&c=2");
        let html = email.html.as_deref().unwrap();

        assert_escaped(html);
        assert!(html.contains("b=1&amp;c=2"), "link not escaped:\n{html}");
    }

    #[test]
    fn password_reset_email_escapes_the_link() {
        // A quote in the link would otherwise break out of the `href="…"`.
        let email = Mailer::password_reset_email(
            "to@example.com",
            r#"https://x.test/" onclick="steal()"#,
            XSS,
        );
        let html = email.html.as_deref().unwrap();

        assert_escaped(html);
        assert!(
            !html.contains(r#"" onclick=""#),
            "broke out of the href attribute:\n{html}"
        );
        assert!(html.contains("&quot;"), "quote not escaped:\n{html}");
    }

    #[test]
    fn notification_email_escapes_title_and_message() {
        let email = Mailer::notification_email("to@example.com", XSS, XSS);
        assert_escaped(email.html.as_deref().unwrap());

        // The plain-text part is not HTML and must not be mangled.
        assert_eq!(email.text.as_deref(), Some(XSS));
        assert_eq!(email.subject.as_deref(), Some(XSS));
    }

    #[test]
    fn escape_html_covers_every_dangerous_character() {
        assert_eq!(
            escape_html(r#"<a href="x" & 'y'>"#),
            "&lt;a href=&quot;x&quot; &amp; &#x27;y&#x27;&gt;"
        );
        assert_eq!(escape_html("plain text"), "plain text");
    }

    /// Transport that records how many sends were in flight simultaneously.
    struct ConcurrencyProbe {
        in_flight: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
        order: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Transport for ConcurrencyProbe {
        async fn send(&self, email: &Email) -> Result<()> {
            use std::sync::atomic::Ordering;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.order
                .lock()
                .unwrap()
                .push(email.subject.clone().unwrap_or_default());
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn bulk_emails(n: usize) -> Vec<Email> {
        (0..n)
            .map(|i| {
                Email::new()
                    .from("s@example.com")
                    .to("r@example.com")
                    .subject(format!("m{i}"))
                    .text("hi")
            })
            .collect()
    }

    /// `send_bulk` was fully sequential, so one slow
    /// recipient stalled the whole batch and `pool_size` was never exercised.
    #[tokio::test]
    async fn send_bulk_runs_concurrently_up_to_the_configured_bound() {
        use std::sync::atomic::Ordering;

        let probe = Arc::new(ConcurrencyProbe {
            in_flight: Default::default(),
            peak: Default::default(),
            order: Default::default(),
        });

        struct Shared(Arc<ConcurrencyProbe>);
        #[async_trait::async_trait]
        impl Transport for Shared {
            async fn send(&self, email: &Email) -> Result<()> {
                self.0.send(email).await
            }
        }

        let mailer = Mailer::new(Shared(probe.clone()))
            .with_config(MailerConfig::default().bulk_concurrency(4));

        let results = mailer.send_bulk(bulk_emails(8)).await;

        assert_eq!(results.len(), 8);
        assert!(results.iter().all(|r| r.is_ok()));
        assert!(
            probe.peak.load(Ordering::SeqCst) > 1,
            "send_bulk ran sequentially (peak in-flight = 1)"
        );
        assert!(
            probe.peak.load(Ordering::SeqCst) <= 4,
            "send_bulk exceeded bulk_concurrency: {}",
            probe.peak.load(Ordering::SeqCst)
        );
    }

    /// Concurrency must not reorder results relative to the input.
    #[tokio::test]
    async fn send_bulk_preserves_result_ordering() {
        struct Slow;
        #[async_trait::async_trait]
        impl Transport for Slow {
            async fn send(&self, email: &Email) -> Result<()> {
                // The first message is the slowest, so an unordered
                // implementation would surface its result last.
                let subject = email.subject.clone().unwrap_or_default();
                let delay = if subject == "m0" { 60 } else { 5 };
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                if subject == "m2" {
                    return Err(MailError::provider(400u16, "rejected"));
                }
                Ok(())
            }
        }

        let mailer = Mailer::new(Slow).with_config(MailerConfig::default().bulk_concurrency(4));
        let results = mailer.send_bulk(bulk_emails(5)).await;

        assert_eq!(results.len(), 5);
        // The failure stays at index 2 even though it completed before m0.
        assert!(results[2].is_err(), "results were reordered");
        for i in [0, 1, 3, 4] {
            assert!(results[i].is_ok(), "unexpected failure at {i}");
        }
    }

    #[test]
    fn bulk_concurrency_defaults_to_the_pool_size_and_clamps_to_one() {
        assert_eq!(MailerConfig::default().bulk_concurrency, 4);
        assert_eq!(
            MailerConfig::default().bulk_concurrency(0).bulk_concurrency,
            1
        );
        assert_eq!(
            MailerConfig::default()
                .bulk_concurrency(16)
                .bulk_concurrency,
            16
        );
    }
}
