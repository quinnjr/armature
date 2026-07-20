# armature-mail

Email sending for the Armature framework.

## Features

- **SMTP Support** — send via any SMTP server, with STARTTLS or implicit TLS
- **Templates** — Handlebars, Tera, or MiniJinja rendering of subject/HTML/text
- **Attachments** — file and inline attachments
- **Providers** — SendGrid, Mailgun, AWS SES
- **Async Queue** — non-blocking sending with retries and a dead-letter queue

## Installation

```toml
[dependencies]
armature-mail = "0.2"

# Providers, template engines, and the queue are optional features:
# armature-mail = { version = "0.2", features = ["sendgrid", "redis"] }
```

Features: `handlebars` (default), `tera`, `minijinja`, `sendgrid`, `mailgun`,
`ses`, `queue`, `redis`, and the bundles `all-providers`, `all-templates`,
`full`.

## Quick Start

`Mailer::smtp` takes an `SmtpConfig` and is async and fallible; the returned
`Mailer` is what you send through.

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_mail::{Email, Mailer, SmtpConfig};

let mailer = Mailer::smtp(
    SmtpConfig::new("smtp.example.com")
        .credentials("user@example.com", "password")
        .port(587)
        .starttls(),
)
.await?;

let email = Email::new()
    .from("sender@example.com")
    .to("recipient@example.com")
    .subject("Hello!")
    // Bodies are set with `text` and/or `html` — there is no `body`.
    .text("This is a test email.")
    .html("<h1>Hello!</h1>");

mailer.send(email).await?;
# Ok(())
# }
```

`SmtpConfig` also has presets for common hosts: `gmail`, `outlook`,
`amazon_ses`, `mailgun`, `sendgrid`, and `postmark`.

## Building emails

`Email::new()` is infallible and defers address validation to send time.
`Email::builder()` validates each address as it is added:

```rust
use armature_mail::Email;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let email = Email::builder()
    .from("sender@example.com")?
    .to("recipient@example.com")?
    .subject("Welcome!")
    .text("Thanks for signing up.")
    .build()?;

assert_eq!(email.subject.as_deref(), Some("Welcome!"));
# Ok(())
# }
```

The fluent setters take `impl IntoAddress` and so cannot return a `Result`.
Rather than silently dropping bad input, an address that fails to parse is
recorded on `Email::invalid_addresses`, and a header value carrying a CR/LF is
dropped and recorded on `Email::invalid_headers` — `Email::validate` then
fails instead of letting a shortened recipient list or an injected header
reach a transport:

```rust
use armature_mail::Email;

let email = Email::new().header("X-Bad", "value\r\nBcc: attacker@example.com");

assert!(email.headers.is_empty());
assert_eq!(email.invalid_headers.len(), 1);
assert!(email.validate().is_err());
```

## Attachments

`Attachment::from_file` reads the file synchronously and infers the MIME type
from the extension.

```rust,no_run
# fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_mail::{Attachment, Email};

let email = Email::new()
    .to("recipient@example.com")
    .subject("Your report")
    .text("See attached.")
    .attach(Attachment::from_file("report.pdf")?);
# Ok(())
# }
```

Use `Attachment::from_bytes` for in-memory data, and `.inline()` plus
`.content_id(..)` for images referenced from an HTML body.

## Templates

```rust,no_run
# #[cfg(feature = "handlebars")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_mail::{Mailer, SmtpConfig};
use serde_json::json;

let mailer = Mailer::smtp(SmtpConfig::new("smtp.example.com"))
    .await?
    .with_templates("./templates")?;

mailer
    .send_template("welcome", "user@example.com", json!({ "name": "John" }))
    .await?;
# Ok(())
# }
```

The rendered template supplies the subject, HTML, and text bodies, so
`send_template` only takes the recipient. `with_templates` requires the
`handlebars` feature; for Tera or MiniJinja, build the engine yourself and
pass it to `Mailer::with_template_engine`.

## Providers

Each provider is a `Transport`; construct one and hand it to `Mailer::new`.

### SendGrid

```rust
# #[cfg(feature = "sendgrid")]
# fn example() -> Result<(), armature_mail::MailError> {
use armature_mail::{Mailer, SendGridConfig, SendGridTransport};

// Fallible: the endpoint is rejected if it is not https (the API key rides on
// every request), and a client that cannot be built is surfaced rather than
// silently replaced by one with no timeouts.
let mailer = Mailer::new(SendGridTransport::new(SendGridConfig::new("SG.api-key"))?);
# Ok(())
# }
```

### Mailgun

```rust
# #[cfg(feature = "mailgun")]
# fn example() -> Result<(), armature_mail::MailError> {
use armature_mail::{MailgunConfig, MailgunTransport, Mailer};

let mailer = Mailer::new(MailgunTransport::new(MailgunConfig::new(
    "api-key",
    "mg.example.com",
))?);
# Ok(())
# }
```

### AWS SES

```rust,no_run
# #[cfg(feature = "ses")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_mail::{Mailer, SesConfig, SesTransport};

let transport = SesTransport::new(SesConfig::new().region("us-east-1")).await?;
let mailer = Mailer::new(transport);
# Ok(())
# }
```

Provider HTTP failures surface as `MailError::Provider { status, message }`,
carrying the upstream status code alongside the response body.

## Queue

```rust,no_run
# #[cfg(feature = "queue")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_mail::{Email, EmailQueue, EmailQueueConfig, Mailer, SmtpConfig};
use std::sync::Arc;

// Validated here: `visibility_timeout` must clear `job_timeout` by more than 2x.
let queue = EmailQueue::in_memory(EmailQueueConfig::default().concurrency(4))?;

let _job_id = queue
    .enqueue(Email::new().to("user@example.com").subject("Hi").text("Async"))
    .await?;

let mailer = Arc::new(Mailer::smtp(SmtpConfig::new("smtp.example.com")).await?);
tokio::spawn(queue.worker(mailer).run());
# Ok(())
# }
```

Jobs hold the email as an `Arc<Email>`, so a retry does not deep-copy its
attachments. Use `EmailQueue::redis(redis_service, config)` (feature `redis`)
for a backend that survives a restart.

## License

MIT OR Apache-2.0
