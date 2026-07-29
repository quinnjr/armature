#![doc = include_str!("../README.md")]
//!
//! # Armature Mail
//!
//! Email sending with SMTP, templates, and cloud provider integrations.
//!
//! ## Features
//!
//! - **SMTP Transport**: Direct SMTP email sending with TLS support
//! - **Email Templates**: HTML and text templates with Handlebars, Tera, or MiniJinja
//! - **Cloud Providers**: SendGrid, Mailgun, AWS SES integrations
//! - **Attachments**: File and inline attachments
//! - **Async Queue**: Non-blocking email sending with retries
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use armature_mail::{Email, Mailer, SmtpConfig};
//!
//! // Configure SMTP
//! let config = SmtpConfig::new("smtp.example.com")
//!     .credentials("user@example.com", "password")
//!     .port(587)
//!     .starttls();
//!
//! let mailer = Mailer::smtp(config).await?;
//!
//! // Send an email
//! let email = Email::new()
//!     .from("sender@example.com")
//!     .to("recipient@example.com")
//!     .subject("Hello from Armature!")
//!     .text("This is a test email.")
//!     .html("<h1>Hello!</h1><p>This is a test email.</p>");
//!
//! mailer.send(email).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## With Templates
//!
//! `with_templates` loads a directory of Handlebars templates (it requires the
//! `handlebars` feature); `with_template_engine` accepts any [`TemplateEngine`].
//!
//! ```rust,no_run
//! # #[cfg(feature = "handlebars")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use armature_mail::{Mailer, SmtpConfig};
//! use serde_json::json;
//!
//! let mailer = Mailer::smtp(SmtpConfig::new("smtp.example.com"))
//!     .await?
//!     .with_templates("./templates")?;
//!
//! // Send using a template. The rendered template supplies the subject and
//! // body, so only the recipient is passed in.
//! mailer
//!     .send_template(
//!         "welcome",
//!         "user@example.com",
//!         json!({
//!             "name": "John",
//!             "activation_link": "https://example.com/activate/abc123"
//!         }),
//!     )
//!     .await?;
//! # Ok(())
//! # }
//! ```

mod address;
mod attachment;
mod email;
mod error;
mod mailer;
mod transport;

// Gated on the transports that use it, not on `reqwest` itself: `reqwest` is
// only ever enabled *by* `sendgrid`/`mailgun`, and both import every item in
// here unconditionally, so gating on the feature that actually implies a caller
// is what keeps the module from needing a blanket `#[allow(dead_code)]`.
#[cfg(any(feature = "sendgrid", feature = "mailgun"))]
mod http;

#[cfg(any(feature = "handlebars", feature = "tera", feature = "minijinja"))]
mod template_dir;

#[cfg(feature = "handlebars")]
mod template_handlebars;

#[cfg(feature = "tera")]
mod template_tera;

#[cfg(feature = "minijinja")]
mod template_minijinja;

#[cfg(feature = "sendgrid")]
mod sendgrid;

#[cfg(feature = "ses")]
mod ses;

#[cfg(feature = "mailgun")]
mod mailgun;

#[cfg(feature = "queue")]
mod queue;

pub use address::{Address, IntoAddress, Mailbox};
pub use attachment::{Attachment, ContentDisposition, DEFAULT_MAX_ATTACHMENT_BYTES};
pub use email::{Email, EmailBuilder, validate_header};
pub use error::{MailError, Result};
pub use mailer::{Mailer, MailerConfig};
pub use transport::{SmtpConfig, SmtpSecurity, SmtpTransport, Transport};

#[cfg(feature = "handlebars")]
pub use template_handlebars::HandlebarsEngine;

#[cfg(feature = "tera")]
pub use template_tera::TeraEngine;

#[cfg(feature = "minijinja")]
pub use template_minijinja::MiniJinjaEngine;

#[cfg(feature = "sendgrid")]
pub use sendgrid::{SendGridConfig, SendGridTransport};

#[cfg(feature = "ses")]
pub use ses::{SesConfig, SesTransport};

#[cfg(feature = "mailgun")]
pub use mailgun::{MailgunConfig, MailgunTransport};

#[cfg(feature = "queue")]
pub use queue::{
    EmailJob, EmailQueue, EmailQueueBackend, EmailQueueConfig, EmailQueueWorker, InMemoryBackend,
    MAX_SERVER_RETRY_AFTER, MailerQueueExt, QueueStats,
};

#[cfg(feature = "redis")]
pub use queue::RedisBackend;

/// Template engine trait for rendering email templates.
pub trait TemplateEngine: Send + Sync {
    /// Render a template with the given name and context.
    fn render(&self, name: &str, context: &serde_json::Value) -> Result<RenderedTemplate>;

    /// Check if a template exists.
    fn has_template(&self, name: &str) -> bool;

    /// Register a template from a string.
    fn register_template(&mut self, name: &str, content: &str) -> Result<()>;
}

/// Rendered template output.
#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    /// HTML content (if available).
    pub html: Option<String>,
    /// Plain text content (if available).
    pub text: Option<String>,
    /// Subject line (if available).
    pub subject: Option<String>,
}

impl RenderedTemplate {
    /// Create a new rendered template with HTML content.
    pub fn html(html: impl Into<String>) -> Self {
        Self {
            html: Some(html.into()),
            text: None,
            subject: None,
        }
    }

    /// Create a new rendered template with text content.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            html: None,
            text: Some(text.into()),
            subject: None,
        }
    }

    /// Create a new rendered template with both HTML and text.
    pub fn both(html: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            html: Some(html.into()),
            text: Some(text.into()),
            subject: None,
        }
    }

    /// Set the subject.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
}

/// Prelude for common imports.
///
/// ```
/// use armature_mail::prelude::*;
/// ```
pub mod prelude {
    pub use crate::address::{Address, IntoAddress, Mailbox};
    pub use crate::attachment::{Attachment, ContentDisposition};
    pub use crate::email::{Email, EmailBuilder};
    pub use crate::error::{MailError, Result};
    pub use crate::mailer::{Mailer, MailerConfig};
    pub use crate::transport::{SmtpConfig, SmtpSecurity, SmtpTransport, Transport};
    pub use crate::{RenderedTemplate, TemplateEngine};

    #[cfg(feature = "handlebars")]
    pub use crate::HandlebarsEngine;

    #[cfg(feature = "tera")]
    pub use crate::TeraEngine;

    #[cfg(feature = "minijinja")]
    pub use crate::MiniJinjaEngine;

    #[cfg(feature = "sendgrid")]
    pub use crate::sendgrid::{SendGridConfig, SendGridTransport};

    #[cfg(feature = "ses")]
    pub use crate::ses::{SesConfig, SesTransport};

    #[cfg(feature = "mailgun")]
    pub use crate::mailgun::{MailgunConfig, MailgunTransport};

    #[cfg(feature = "queue")]
    pub use crate::queue::{EmailJob, EmailQueue, EmailQueueBackend, EmailQueueConfig};
}
