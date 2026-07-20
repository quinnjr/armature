//! SendGrid email provider integration.

use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use tracing::debug;

use crate::{Email, MailError, Result, Transport};

/// SendGrid configuration.
#[derive(Debug, Clone)]
pub struct SendGridConfig {
    /// API key.
    pub api_key: String,
    /// API endpoint (defaults to production).
    pub endpoint: String,
}

impl SendGridConfig {
    /// Create a new SendGrid configuration.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: "https://api.sendgrid.com/v3/mail/send".to_string(),
        }
    }

    /// Set a custom endpoint (for testing).
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
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
    pub fn new(config: SendGridConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
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
            // Rate limited
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(60);
            Err(MailError::RateLimited(retry_after))
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(MailError::Provider(format!(
                "SendGrid error {}: {}",
                status, body
            )))
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
    /// Custom headers, plus the headers implied by `Email::priority`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    headers: std::collections::BTreeMap<String, String>,
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
            subject: email.subject.clone().unwrap_or_default(),
            content,
            attachments,
            headers: email.wire_headers().into_iter().collect(),
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

    /// WF6 findings 4 and 5: custom headers and priority were stored on `Email`
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

    #[test]
    fn payload_omits_headers_when_there_are_none() {
        let payload = SendGridPayload::from_email(&base()).unwrap();
        let json = serde_json::to_value(&payload).unwrap();
        assert!(json.get("headers").is_none());
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
