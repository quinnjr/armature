//! Webhook signature generation and verification

use crate::config::SigningAlgorithm;
use crate::{Result, WebhookError};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Sha256, Sha512};

type HmacSha256 = Hmac<Sha256>;
type HmacSha512 = Hmac<Sha512>;

/// Webhook signature utilities
#[derive(Debug, Clone)]
pub struct WebhookSignature {
    secret: String,
    algorithm: SigningAlgorithm,
}

impl WebhookSignature {
    /// Create a new signature utility with the given secret
    ///
    /// Uses HMAC-SHA256 by default. Use [`WebhookSignature::with_algorithm`]
    /// to select a different signing algorithm (e.g. HMAC-SHA512).
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            algorithm: SigningAlgorithm::HmacSha256,
        }
    }

    /// Set the signing algorithm to use for signing and verification
    pub fn with_algorithm(mut self, algorithm: SigningAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Generate a signature for the given payload using the configured algorithm
    pub fn sign(&self, payload: &[u8]) -> String {
        self.sign_with_timestamp(payload, &Self::current_timestamp())
    }

    /// Generate a signature with a specific timestamp
    pub fn sign_with_timestamp(&self, payload: &[u8], timestamp: &str) -> String {
        let signature = self.compute_hmac_over_timestamped_payload(timestamp.as_bytes(), payload);
        format!("t={},v1={}", timestamp, signature)
    }

    /// Verify a signature against the payload
    ///
    /// Supports two signature schemes:
    /// - Stripe-style `t=<timestamp>,v1=<hex>`: verified with timestamp-tolerance
    ///   replay protection.
    /// - GitHub-style `sha256=<hex>` (no timestamp): verified as a timestampless
    ///   HMAC-SHA256 over the raw body. Since there is no timestamp, replay
    ///   protection does not apply to this scheme.
    pub fn verify(&self, payload: &[u8], signature: &str, tolerance_secs: u64) -> Result<bool> {
        if let Some(hex_sig) = signature.strip_prefix("sha256=") {
            // GitHub-style signature: timestampless HMAC-SHA256 over the raw body.
            let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes())
                .expect("HMAC can take any size key");
            mac.update(payload);
            let expected = hex::encode(mac.finalize().into_bytes());
            return Ok(constant_time_compare(hex_sig, &expected));
        }

        let parts = Self::parse_signature(signature)?;

        // Verify timestamp is within tolerance
        let timestamp: i64 = parts
            .timestamp
            .parse()
            .map_err(|_| WebhookError::TimestampInvalid("Invalid timestamp format".to_string()))?;

        let now = chrono::Utc::now().timestamp();
        let age = (now - timestamp).unsigned_abs();

        if age > tolerance_secs {
            return Err(WebhookError::TimestampInvalid(format!(
                "Timestamp too old: {} seconds (tolerance: {} seconds)",
                age, tolerance_secs
            )));
        }

        // Compute expected signature
        let expected =
            self.compute_hmac_over_timestamped_payload(parts.timestamp.as_bytes(), payload);

        // Constant-time comparison to prevent timing attacks
        Ok(constant_time_compare(&parts.signature, &expected))
    }

    /// Compute the HMAC over `timestamp || "." || payload`, using the configured
    /// signing algorithm, feeding raw bytes directly into the MAC (no lossy
    /// UTF-8 conversion or intermediate `String` allocation).
    fn compute_hmac_over_timestamped_payload(&self, timestamp: &[u8], payload: &[u8]) -> String {
        match self.algorithm {
            SigningAlgorithm::HmacSha256 => {
                self.compute_hmac_sha256_over_timestamped_payload(timestamp, payload)
            }
            SigningAlgorithm::HmacSha512 => {
                self.compute_hmac_sha512_over_timestamped_payload(timestamp, payload)
            }
        }
    }

    /// Compute HMAC-SHA256 over `timestamp || "." || payload`
    fn compute_hmac_sha256_over_timestamped_payload(
        &self,
        timestamp: &[u8],
        payload: &[u8],
    ) -> String {
        let mut mac =
            HmacSha256::new_from_slice(self.secret.as_bytes()).expect("HMAC can take any size key");
        mac.update(timestamp);
        mac.update(b".");
        mac.update(payload);
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Compute HMAC-SHA512 over `timestamp || "." || payload`
    fn compute_hmac_sha512_over_timestamped_payload(
        &self,
        timestamp: &[u8],
        payload: &[u8],
    ) -> String {
        let mut mac =
            HmacSha512::new_from_slice(self.secret.as_bytes()).expect("HMAC can take any size key");
        mac.update(timestamp);
        mac.update(b".");
        mac.update(payload);
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Get current Unix timestamp as string
    fn current_timestamp() -> String {
        chrono::Utc::now().timestamp().to_string()
    }

    /// Parse a signature string into components
    fn parse_signature(signature: &str) -> Result<SignatureParts> {
        let mut timestamp = None;
        let mut sig = None;

        for part in signature.split(',') {
            let mut kv = part.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("t"), Some(t)) => timestamp = Some(t.to_string()),
                (Some("v1"), Some(v)) => sig = Some(v.to_string()),
                _ => {}
            }
        }

        match (timestamp, sig) {
            (Some(t), Some(s)) => Ok(SignatureParts {
                timestamp: t,
                signature: s,
            }),
            _ => Err(WebhookError::SignatureInvalid(
                "Missing timestamp or signature".to_string(),
            )),
        }
    }
}

/// Parsed signature components
struct SignatureParts {
    timestamp: String,
    signature: String,
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        result |= x ^ y;
    }
    result == 0
}

/// Header names for webhook signatures (Stripe-style)
#[allow(dead_code)]
pub mod headers {
    /// The signature header name
    pub const SIGNATURE: &str = "X-Webhook-Signature";

    /// Alternative signature header (GitHub style)
    pub const SIGNATURE_ALT: &str = "X-Hub-Signature-256";

    /// Timestamp header (if sent separately)
    pub const TIMESTAMP: &str = "X-Webhook-Timestamp";

    /// Webhook ID header
    pub const WEBHOOK_ID: &str = "X-Webhook-Id";

    /// Event type header
    pub const EVENT_TYPE: &str = "X-Webhook-Event";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let signer = WebhookSignature::new("test-secret");
        let payload = b"Hello, World!";

        let signature = signer.sign(payload);
        assert!(signature.starts_with("t="));
        assert!(signature.contains(",v1="));

        // Verify should pass with reasonable tolerance
        let result = signer.verify(payload, &signature, 300);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_sign_with_timestamp() {
        let signer = WebhookSignature::new("test-secret");
        let payload = b"test payload";
        let timestamp = "1234567890";

        let sig1 = signer.sign_with_timestamp(payload, timestamp);
        let sig2 = signer.sign_with_timestamp(payload, timestamp);

        // Same input should produce same signature
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_verify_invalid_signature() {
        let signer = WebhookSignature::new("test-secret");
        let payload = b"test payload";

        let result = signer.verify(payload, "invalid", 300);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_wrong_secret() {
        let signer1 = WebhookSignature::new("secret1");
        let signer2 = WebhookSignature::new("secret2");

        let payload = b"test payload";
        let signature = signer1.sign(payload);

        let result = signer2.verify(payload, &signature, 300);
        assert!(result.is_ok());
        assert!(!result.unwrap()); // Should not match
    }

    #[test]
    fn test_verify_expired_timestamp() {
        let signer = WebhookSignature::new("test-secret");
        let payload = b"test payload";

        // Create signature with old timestamp
        let old_timestamp = (chrono::Utc::now().timestamp() - 1000).to_string();
        let signature = signer.sign_with_timestamp(payload, &old_timestamp);

        // Should fail with small tolerance
        let result = signer.verify(payload, &signature, 60);
        assert!(result.is_err());
    }

    #[test]
    fn test_sha512_algorithm_selection_produces_sha512_signature() {
        let secret = "test-secret";
        let payload = b"test payload";
        let timestamp = "1234567890";

        let sha256_signer = WebhookSignature::new(secret);
        let sha512_signer =
            WebhookSignature::new(secret).with_algorithm(SigningAlgorithm::HmacSha512);

        let sha256_sig = sha256_signer.sign_with_timestamp(payload, timestamp);
        let sha512_sig = sha512_signer.sign_with_timestamp(payload, timestamp);

        // The two algorithms must produce different signatures
        assert_ne!(sha256_sig, sha512_sig);

        // Compute the expected raw HMAC-SHA512 value directly and confirm it's embedded
        let expected_sha512 = sha512_signer
            .compute_hmac_sha512_over_timestamped_payload(timestamp.as_bytes(), payload);
        assert!(sha512_sig.contains(&expected_sha512));

        // SHA-512 hex digest is 128 chars, SHA-256 is 64 chars
        let v1_sha512 = sha512_sig.split("v1=").nth(1).unwrap();
        let v1_sha256 = sha256_sig.split("v1=").nth(1).unwrap();
        assert_eq!(v1_sha512.len(), 128);
        assert_eq!(v1_sha256.len(), 64);

        // And verification round-trips correctly for the sha512 signer
        // (use a fresh, current-timestamp signature so it's within tolerance)
        let live_sha512_sig = sha512_signer.sign(payload);
        let result = sha512_signer.verify(payload, &live_sha512_sig, 300);
        assert!(result.is_ok());
        assert!(result.unwrap());

        // A sha256 signer must reject a sha512-produced signature (mismatched digest)
        let cross_result = sha256_signer.verify(payload, &live_sha512_sig, 300);
        assert!(cross_result.is_ok());
        assert!(!cross_result.unwrap());
    }

    #[test]
    fn test_verify_github_style_sha256_signature() {
        // GitHub sends `X-Hub-Signature-256: sha256=<hex>` with no timestamp:
        // a raw HMAC-SHA256 over the exact request body.
        let secret = "test-secret";
        let signer = WebhookSignature::new(secret);
        let payload = b"{\"action\":\"opened\"}";

        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take any size key");
        mac.update(payload);
        let expected_hex = hex::encode(mac.finalize().into_bytes());
        let github_signature = format!("sha256={}", expected_hex);

        let result = signer.verify(payload, &github_signature, 300);
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Wrong secret must not verify
        let wrong_signer = WebhookSignature::new("wrong-secret");
        let wrong_result = wrong_signer.verify(payload, &github_signature, 300);
        assert!(wrong_result.is_ok());
        assert!(!wrong_result.unwrap());

        // Tampered payload must not verify
        let tampered_result = signer.verify(b"{\"action\":\"closed\"}", &github_signature, 300);
        assert!(tampered_result.is_ok());
        assert!(!tampered_result.unwrap());
    }

    #[test]
    fn test_constant_time_compare() {
        assert!(constant_time_compare("abc", "abc"));
        assert!(!constant_time_compare("abc", "abd"));
        assert!(!constant_time_compare("abc", "ab"));
        assert!(!constant_time_compare("", "a"));
    }
}
