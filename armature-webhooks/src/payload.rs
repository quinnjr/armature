//! Webhook payload types

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A webhook payload to be sent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// Unique identifier for this webhook delivery
    pub id: String,

    /// Event type (e.g., "user.created", "order.updated")
    pub event: String,

    /// Timestamp when the event occurred
    pub timestamp: DateTime<Utc>,

    /// The actual event data
    pub data: serde_json::Value,

    /// Optional metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl WebhookPayload {
    /// Create a new webhook payload with the given event type
    pub fn new(event: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            event: event.into(),
            timestamp: Utc::now(),
            data: serde_json::Value::Null,
            metadata: None,
        }
    }

    /// Set the payload data
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = data;
        self
    }

    /// Set metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Set a custom ID
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Set a custom timestamp
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Convert to JSON bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Convert to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Convert to pretty JSON string
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Status of a webhook delivery attempt
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryStatus {
    /// Delivery is pending
    Pending,

    /// Delivery is in progress
    InProgress,

    /// Delivery succeeded
    Succeeded,

    /// Delivery failed but will be retried
    Failed,

    /// Delivery permanently failed (no more retries)
    PermanentlyFailed,
}

impl WebhookDeliveryStatus {
    /// Check if the delivery is complete (success or permanent failure)
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::PermanentlyFailed)
    }

    /// Check if the delivery succeeded
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

/// Record of a webhook delivery attempt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    /// Unique delivery ID
    pub id: String,

    /// The webhook payload
    pub payload: WebhookPayload,

    /// Target endpoint URL
    pub endpoint_url: String,

    /// Current status
    pub status: WebhookDeliveryStatus,

    /// Number of delivery attempts
    pub attempts: u32,

    /// Last attempt timestamp
    pub last_attempt: Option<DateTime<Utc>>,

    /// Next retry timestamp (if applicable)
    pub next_retry: Option<DateTime<Utc>>,

    /// HTTP status code from last attempt
    pub last_status_code: Option<u16>,

    /// Error message from last failed attempt
    pub last_error: Option<String>,

    /// Response body from last attempt (truncated)
    pub last_response_body: Option<String>,

    /// Created timestamp
    pub created_at: DateTime<Utc>,

    /// Updated timestamp
    pub updated_at: DateTime<Utc>,
}

impl WebhookDelivery {
    /// Create a new delivery record
    pub fn new(payload: WebhookPayload, endpoint_url: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            payload,
            endpoint_url: endpoint_url.into(),
            status: WebhookDeliveryStatus::Pending,
            attempts: 0,
            last_attempt: None,
            next_retry: None,
            last_status_code: None,
            last_error: None,
            last_response_body: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Record a successful delivery
    pub fn mark_succeeded(&mut self, status_code: u16, response_body: Option<String>) {
        self.status = WebhookDeliveryStatus::Succeeded;
        self.last_attempt = Some(Utc::now());
        self.last_status_code = Some(status_code);
        self.last_response_body = response_body.map(|s| truncate_string(&s, 1024));
        self.last_error = None;
        self.next_retry = None;
        self.updated_at = Utc::now();
        self.attempts += 1;
    }

    /// Record a failed delivery
    pub fn mark_failed(&mut self, error: String, next_retry: Option<DateTime<Utc>>) {
        self.status = if next_retry.is_some() {
            WebhookDeliveryStatus::Failed
        } else {
            WebhookDeliveryStatus::PermanentlyFailed
        };
        self.last_attempt = Some(Utc::now());
        self.last_error = Some(error);
        self.next_retry = next_retry;
        self.updated_at = Utc::now();
        self.attempts += 1;
    }

    /// Record a failed delivery with HTTP status
    pub fn mark_failed_with_status(
        &mut self,
        status_code: u16,
        response_body: Option<String>,
        next_retry: Option<DateTime<Utc>>,
    ) {
        self.status = if next_retry.is_some() {
            WebhookDeliveryStatus::Failed
        } else {
            WebhookDeliveryStatus::PermanentlyFailed
        };
        self.last_attempt = Some(Utc::now());
        self.last_status_code = Some(status_code);
        self.last_response_body = response_body.map(|s| truncate_string(&s, 1024));
        self.last_error = Some(format!("HTTP {}", status_code));
        self.next_retry = next_retry;
        self.updated_at = Utc::now();
        self.attempts += 1;
    }
}

/// Truncate a string to a maximum length (in bytes), respecting UTF-8 char boundaries
///
/// This is fed untrusted response bodies from external webhook targets, which may
/// contain multibyte UTF-8 characters. Slicing on a raw byte index can land in the
/// middle of a multibyte character and panic, so we find the largest valid char
/// boundary at or before `max_len - 3` (reserving 3 bytes for the "..." suffix).
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    let target = max_len.saturating_sub(3);
    let mut boundary = target.min(s.len());
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }

    format!("{}...", &s[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_creation() {
        let payload =
            WebhookPayload::new("user.created").with_data(serde_json::json!({"user_id": "123"}));

        assert_eq!(payload.event, "user.created");
        assert!(!payload.id.is_empty());
    }

    #[test]
    fn test_payload_serialization() {
        let payload =
            WebhookPayload::new("test.event").with_data(serde_json::json!({"key": "value"}));

        let json = payload.to_json().unwrap();
        assert!(json.contains("test.event"));
        assert!(json.contains("key"));
    }

    #[test]
    fn test_delivery_status() {
        assert!(WebhookDeliveryStatus::Succeeded.is_terminal());
        assert!(WebhookDeliveryStatus::PermanentlyFailed.is_terminal());
        assert!(!WebhookDeliveryStatus::Failed.is_terminal());
        assert!(!WebhookDeliveryStatus::Pending.is_terminal());
    }

    #[test]
    fn test_truncate_string_does_not_panic_on_multibyte_boundary() {
        // Each '€' is 3 bytes in UTF-8; with max_len=10 the naive byte-slice
        // `&s[..7]` would land mid-character and panic.
        let body: String = std::iter::repeat_n('€', 20).collect();
        let truncated = truncate_string(&body, 10);

        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 10);
        // Result must be valid UTF-8 (guaranteed by type), and must not have
        // corrupted the euro signs into replacement bytes.
        assert!(truncated.chars().all(|c| c == '€' || c == '.'));
    }

    #[test]
    fn test_truncate_string_via_mark_succeeded_multibyte_response() {
        let payload = WebhookPayload::new("test");
        let mut delivery = WebhookDelivery::new(payload, "https://example.com/webhook");

        let body: String = std::iter::repeat_n('日', 2000).collect();
        // Should not panic.
        delivery.mark_succeeded(200, Some(body));

        let truncated = delivery.last_response_body.unwrap();
        assert!(truncated.len() <= 1024);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_delivery_lifecycle() {
        let payload = WebhookPayload::new("test");
        let mut delivery = WebhookDelivery::new(payload, "https://example.com/webhook");

        assert_eq!(delivery.status, WebhookDeliveryStatus::Pending);
        assert_eq!(delivery.attempts, 0);

        // Mark as failed with retry
        delivery.mark_failed("Connection timeout".to_string(), Some(Utc::now()));
        assert_eq!(delivery.status, WebhookDeliveryStatus::Failed);
        assert_eq!(delivery.attempts, 1);

        // Mark as succeeded
        delivery.mark_succeeded(200, Some("OK".to_string()));
        assert_eq!(delivery.status, WebhookDeliveryStatus::Succeeded);
        assert_eq!(delivery.attempts, 2);
    }
}
