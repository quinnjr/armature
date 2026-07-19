//! Webhook receiver for handling incoming webhooks

use crate::{Result, WebhookConfig, WebhookError, WebhookPayload, WebhookSignature};

/// Receiver for incoming webhooks
#[derive(Debug, Clone)]
pub struct WebhookReceiver {
    signature: WebhookSignature,
    timestamp_tolerance: u64,
}

impl WebhookReceiver {
    /// Create a new receiver with the given secret
    ///
    /// Uses [`WebhookConfig::default`] for the timestamp tolerance and signing
    /// algorithm. To have a `WebhookConfig`'s settings (e.g. a custom
    /// `timestamp_tolerance` or `signing_algorithm`) flow through, use
    /// [`WebhookReceiver::from_config`] instead.
    pub fn new(secret: impl Into<String>) -> Self {
        Self::from_config(secret, &WebhookConfig::default())
    }

    /// Create a new receiver from a [`WebhookConfig`], so its
    /// `timestamp_tolerance` and `signing_algorithm` are honored
    pub fn from_config(secret: impl Into<String>, config: &WebhookConfig) -> Self {
        Self {
            signature: WebhookSignature::new(secret).with_algorithm(config.signing_algorithm),
            timestamp_tolerance: config.timestamp_tolerance,
        }
    }

    /// Set the timestamp tolerance in seconds
    pub fn with_tolerance(mut self, seconds: u64) -> Self {
        self.timestamp_tolerance = seconds;
        self
    }

    /// Verify an incoming webhook signature
    pub fn verify(&self, payload: &[u8], signature: &str) -> Result<bool> {
        self.signature
            .verify(payload, signature, self.timestamp_tolerance)
    }

    /// Verify and parse an incoming webhook
    pub fn receive(&self, payload: &[u8], signature: &str) -> Result<WebhookPayload> {
        // Verify signature first
        if !self.verify(payload, signature)? {
            return Err(WebhookError::SignatureInvalid(
                "Signature verification failed".to_string(),
            ));
        }

        // Parse the payload
        serde_json::from_slice(payload).map_err(|e| WebhookError::PayloadError(e.to_string()))
    }

    /// Verify signature from HTTP headers
    ///
    /// Looks for the signature in common header names, matched
    /// case-insensitively (HTTP header names are case-insensitive per RFC
    /// 7230 section 3.2):
    /// - X-Webhook-Signature
    /// - X-Hub-Signature-256
    pub fn verify_from_headers(
        &self,
        payload: &[u8],
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<bool> {
        const CANDIDATES: &[&str] = &["x-webhook-signature", "x-hub-signature-256"];

        let signature = headers
            .iter()
            .find(|(key, _)| CANDIDATES.contains(&key.to_lowercase().as_str()))
            .map(|(_, value)| value)
            .ok_or(WebhookError::SignatureMissing)?;

        self.verify(payload, signature)
    }

    /// Receive and parse webhook from HTTP headers and body
    pub fn receive_from_request(
        &self,
        payload: &[u8],
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<WebhookPayload> {
        // Verify signature
        if !self.verify_from_headers(payload, headers)? {
            return Err(WebhookError::SignatureInvalid(
                "Signature verification failed".to_string(),
            ));
        }

        // Parse the payload
        serde_json::from_slice(payload).map_err(|e| WebhookError::PayloadError(e.to_string()))
    }

    /// Create a handler for specific event types
    pub fn handler<F>(&self, event_filter: &str, callback: F) -> WebhookHandler<F>
    where
        F: Fn(WebhookPayload) -> Result<()>,
    {
        WebhookHandler {
            receiver: self.clone(),
            event_filter: event_filter.to_string(),
            callback,
        }
    }
}

/// A webhook handler that filters and processes specific events
pub struct WebhookHandler<F>
where
    F: Fn(WebhookPayload) -> Result<()>,
{
    receiver: WebhookReceiver,
    event_filter: String,
    callback: F,
}

impl<F> WebhookHandler<F>
where
    F: Fn(WebhookPayload) -> Result<()>,
{
    /// Handle an incoming webhook request
    pub fn handle(&self, payload: &[u8], signature: &str) -> Result<bool> {
        // Verify and parse
        let webhook = self.receiver.receive(payload, signature)?;

        // Check if event matches filter
        if !self.matches_event(&webhook.event) {
            return Ok(false);
        }

        // Call the handler
        (self.callback)(webhook)?;
        Ok(true)
    }

    /// Check if an event matches the filter
    fn matches_event(&self, event: &str) -> bool {
        if self.event_filter == "*" {
            return true;
        }

        if self.event_filter.ends_with(".*") {
            let prefix = &self.event_filter[..self.event_filter.len() - 2];
            return event.starts_with(prefix);
        }

        self.event_filter == event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receiver_creation() {
        let receiver = WebhookReceiver::new("test-secret");
        assert_eq!(receiver.timestamp_tolerance, 300);
    }

    #[test]
    fn test_receiver_with_tolerance() {
        let receiver = WebhookReceiver::new("test-secret").with_tolerance(60);
        assert_eq!(receiver.timestamp_tolerance, 60);
    }

    #[test]
    fn test_verify_valid_signature() {
        let secret = "test-secret";
        let receiver = WebhookReceiver::new(secret);
        let signer = WebhookSignature::new(secret);

        let payload = b"test payload";
        let signature = signer.sign(payload);

        let result = receiver.verify(payload, &signature);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_verify_invalid_signature() {
        let receiver = WebhookReceiver::new("correct-secret");

        let payload = b"test payload";
        let wrong_signer = WebhookSignature::new("wrong-secret");
        let signature = wrong_signer.sign(payload);

        let result = receiver.verify(payload, &signature);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_receive_and_parse() {
        let secret = "test-secret";
        let receiver = WebhookReceiver::new(secret);
        let signer = WebhookSignature::new(secret);

        let webhook =
            WebhookPayload::new("test.event").with_data(serde_json::json!({"key": "value"}));
        let payload_bytes = webhook.to_bytes().unwrap();
        let signature = signer.sign(&payload_bytes);

        let result = receiver.receive(&payload_bytes, &signature);
        assert!(result.is_ok());

        let received = result.unwrap();
        assert_eq!(received.event, "test.event");
    }

    #[test]
    fn test_verify_from_headers() {
        let secret = "test-secret";
        let receiver = WebhookReceiver::new(secret);
        let signer = WebhookSignature::new(secret);

        let payload = b"test payload";
        let signature = signer.sign(payload);

        let mut headers = std::collections::HashMap::new();
        headers.insert("X-Webhook-Signature".to_string(), signature);

        let result = receiver.verify_from_headers(payload, &headers);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_verify_from_headers_mixed_case() {
        let secret = "test-secret";
        let receiver = WebhookReceiver::new(secret);
        let signer = WebhookSignature::new(secret);

        let payload = b"test payload";
        let signature = signer.sign(payload);

        // HTTP header names are case-insensitive; a mixed-case variant
        // like "x-webhook-Signature" must still be found.
        let mut headers = std::collections::HashMap::new();
        headers.insert("x-webhook-Signature".to_string(), signature.clone());

        let result = receiver.verify_from_headers(payload, &headers);
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Also check the GitHub-style alternate header in a different case
        let mut headers2 = std::collections::HashMap::new();
        headers2.insert("X-HUB-SIGNATURE-256".to_string(), signature);

        let result2 = receiver.verify_from_headers(payload, &headers2);
        assert!(result2.is_ok());
        assert!(result2.unwrap());
    }

    #[test]
    fn test_from_config_threads_timestamp_tolerance() {
        let config = crate::WebhookConfig::builder()
            .timestamp_tolerance(60)
            .build();
        let receiver = WebhookReceiver::from_config("test-secret", &config);

        assert_eq!(receiver.timestamp_tolerance, 60);
    }

    #[test]
    fn test_verify_missing_signature() {
        let receiver = WebhookReceiver::new("test-secret");
        let headers = std::collections::HashMap::new();

        let result = receiver.verify_from_headers(b"payload", &headers);
        assert!(matches!(result, Err(WebhookError::SignatureMissing)));
    }

    #[test]
    fn test_handler_event_filter() {
        let receiver = WebhookReceiver::new("test-secret");
        let handler = receiver.handler("user.*", |_| Ok(()));

        // Matches
        assert!(handler.matches_event("user.created"));
        assert!(handler.matches_event("user.updated"));
        assert!(handler.matches_event("user.deleted"));

        // Doesn't match
        assert!(!handler.matches_event("order.created"));
        assert!(!handler.matches_event("product.updated"));
    }

    #[test]
    fn test_handler_all_events() {
        let receiver = WebhookReceiver::new("test-secret");
        let handler = receiver.handler("*", |_| Ok(()));

        assert!(handler.matches_event("user.created"));
        assert!(handler.matches_event("order.shipped"));
        assert!(handler.matches_event("anything"));
    }

    #[test]
    fn test_handler_exact_match() {
        let receiver = WebhookReceiver::new("test-secret");
        let handler = receiver.handler("user.created", |_| Ok(()));

        assert!(handler.matches_event("user.created"));
        assert!(!handler.matches_event("user.updated"));
    }
}
