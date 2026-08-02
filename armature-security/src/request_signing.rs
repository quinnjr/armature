//! Request Signing with HMAC
//!
//! Provides HMAC-based request signing and verification for API security.
//!
//! # Features
//!
//! - HMAC-SHA256 request signing
//! - Timestamp-based replay protection
//! - Custom header configuration
//! - Signature verification middleware
//!
//! # Usage
//!
//! ```
//! use armature_security::request_signing::*;
//! use std::time::{SystemTime, UNIX_EPOCH};
//!
//! // Get current timestamp
//! let timestamp = SystemTime::now()
//!     .duration_since(UNIX_EPOCH)
//!     .unwrap()
//!     .as_secs();
//!
//! // Generate signature
//! let signer = RequestSigner::new("secret-key");
//! let signature = signer.sign("POST", "/api/users", "request body", timestamp);
//!
//! // Verify signature
//! let verifier = RequestVerifier::new("secret-key");
//! assert!(verifier.verify("POST", "/api/users", "request body", timestamp, &signature).unwrap());
//! ```

use armature_core::{Error, HttpRequest, HttpResponse, Middleware};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Request signing errors
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Missing signature header")]
    MissingSignature,

    #[error("Missing timestamp header")]
    MissingTimestamp,

    #[error("Invalid timestamp")]
    InvalidTimestamp,

    #[error("Request expired (replay attack?)")]
    RequestExpired,
}

/// Request signer
///
/// Generates HMAC-SHA256 signatures for requests.
#[derive(Clone)]
pub struct RequestSigner {
    secret: String,
}

impl RequestSigner {
    /// Create a new request signer
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestSigner;
    ///
    /// let signer = RequestSigner::new("my-secret-key");
    /// ```
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Sign a request
    ///
    /// # Arguments
    ///
    /// * `method` - HTTP method
    /// * `path` - Request path
    /// * `body` - Request body
    /// * `timestamp` - Unix timestamp
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestSigner;
    ///
    /// let signer = RequestSigner::new("secret");
    /// let signature = signer.sign("POST", "/api/users", "request body", 1702468800);
    /// ```
    pub fn sign(&self, method: &str, path: &str, body: &str, timestamp: u64) -> String {
        let message = format!("{}:{}:{}:{}", method, path, body, timestamp);
        self.hmac_sha256(&message)
    }

    /// Generate HMAC-SHA256
    fn hmac_sha256(&self, message: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(self.secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(message.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

/// Request verifier
///
/// Verifies HMAC-SHA256 signatures on incoming requests.
pub struct RequestVerifier {
    secret: String,
    max_age_seconds: u64,
}

impl RequestVerifier {
    /// Create a new request verifier
    ///
    /// # Arguments
    ///
    /// * `secret` - Shared secret for HMAC
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestVerifier;
    ///
    /// let verifier = RequestVerifier::new("my-secret-key");
    /// ```
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            max_age_seconds: 300, // 5 minutes default
        }
    }

    /// Set maximum age for requests (replay protection)
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestVerifier;
    ///
    /// let verifier = RequestVerifier::new("secret")
    ///     .with_max_age(600); // 10 minutes
    /// ```
    pub fn with_max_age(mut self, seconds: u64) -> Self {
        self.max_age_seconds = seconds;
        self
    }

    /// Verify a signed request
    ///
    /// # Arguments
    ///
    /// * `method` - HTTP method
    /// * `path` - Request path
    /// * `body` - Request body
    /// * `timestamp` - Unix timestamp from request
    /// * `signature` - HMAC signature from request
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestVerifier;
    ///
    /// # fn example() -> Result<(), armature_security::request_signing::SigningError> {
    /// let verifier = RequestVerifier::new("secret");
    /// let is_valid = verifier.verify(
    ///     "POST",
    ///     "/api/users",
    ///     "request body",
    ///     1702468800,
    ///     "expected-signature"
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn verify(
        &self,
        method: &str,
        path: &str,
        body: &str,
        timestamp: u64,
        signature: &str,
    ) -> Result<bool, SigningError> {
        // Check timestamp (replay protection)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SigningError::InvalidTimestamp)?
            .as_secs();

        let age = now.saturating_sub(timestamp);
        if age > self.max_age_seconds {
            return Err(SigningError::RequestExpired);
        }

        // Generate expected signature
        let signer = RequestSigner::new(&self.secret);
        let expected = signer.sign(method, path, body, timestamp);

        // Constant-time comparison
        Ok(armature_core::crypto::constant_time_eq(
            signature.as_bytes(),
            expected.as_bytes(),
        ))
    }

    /// Verify request from HttpRequest
    pub fn verify_request(&self, request: &HttpRequest) -> Result<bool, SigningError> {
        let signature = request
            .headers
            .get("X-Signature")
            .ok_or(SigningError::MissingSignature)?;

        let timestamp_str = request
            .headers
            .get("X-Timestamp")
            .ok_or(SigningError::MissingTimestamp)?;

        let timestamp: u64 = timestamp_str
            .parse()
            .map_err(|_| SigningError::InvalidTimestamp)?;

        // Use `body_ref()` rather than the `body` field directly: the production
        // HTTP server stores incoming bodies via `HttpRequest::set_body_bytes`
        // (zero-copy), which explicitly clears the legacy `body` field. Reading
        // `request.body` here would see an empty body for every real request and
        // make every signature check fail (or, for callers who skip verification
        // on that basis, silently pass with the wrong data signed).
        let body_str = String::from_utf8_lossy(request.body_ref());

        self.verify(
            &request.method,
            &request.path,
            &body_str,
            timestamp,
            signature,
        )
    }
}

/// Request signing middleware
///
/// Automatically verifies HMAC signatures on incoming requests.
pub struct RequestSigningMiddleware {
    verifier: RequestVerifier,
    skip_paths: Vec<String>,
}

impl RequestSigningMiddleware {
    /// Create new request signing middleware
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::request_signing::RequestSigningMiddleware;
    ///
    /// let middleware = RequestSigningMiddleware::new("my-secret-key");
    /// ```
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            verifier: RequestVerifier::new(secret),
            skip_paths: vec!["/health".to_string(), "/metrics".to_string()],
        }
    }

    /// Set maximum age for signed requests
    pub fn with_max_age(mut self, seconds: u64) -> Self {
        self.verifier = self.verifier.with_max_age(seconds);
        self
    }

    /// Add path to skip signature verification
    pub fn skip_path(mut self, path: impl Into<String>) -> Self {
        self.skip_paths.push(path.into());
        self
    }

    /// Check if path should be skipped
    fn should_skip(&self, path: &str) -> bool {
        self.skip_paths.iter().any(|p| path.starts_with(p))
    }
}

#[async_trait::async_trait]
impl Middleware for RequestSigningMiddleware {
    async fn handle(
        &self,
        request: HttpRequest,
        next: armature_core::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        // Skip certain paths (health checks, metrics)
        if !self.should_skip(&request.path) {
            // Verify signature
            match self.verifier.verify_request(&request) {
                Ok(true) => {
                    // Signature valid, proceed
                }
                Ok(false) => {
                    return Err(Error::Unauthorized("Invalid signature".to_string()));
                }
                Err(SigningError::RequestExpired) => {
                    return Err(Error::BadRequest("Request expired".to_string()));
                }
                Err(SigningError::MissingSignature) => {
                    return Err(Error::BadRequest("Missing X-Signature header".to_string()));
                }
                Err(SigningError::MissingTimestamp) => {
                    return Err(Error::BadRequest("Missing X-Timestamp header".to_string()));
                }
                Err(e) => {
                    return Err(Error::BadRequest(format!(
                        "Signature verification failed: {}",
                        e
                    )));
                }
            }
        }

        next(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_request() {
        let signer = RequestSigner::new("secret");
        let sig = signer.sign("POST", "/api/users", "body", 1702468800);
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // SHA256 hex = 64 chars
    }

    #[test]
    fn test_verify_request() {
        let secret = "test-secret";
        let signer = RequestSigner::new(secret);
        let verifier = RequestVerifier::new(secret);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let signature = signer.sign("POST", "/api/test", "test body", timestamp);

        assert!(
            verifier
                .verify("POST", "/api/test", "test body", timestamp, &signature)
                .unwrap()
        );
    }

    #[test]
    fn test_verify_wrong_signature() {
        let verifier = RequestVerifier::new("secret");

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = verifier.verify(
            "POST",
            "/api/test",
            "test body",
            timestamp,
            "wrong-signature",
        );

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_verify_expired_request() {
        let verifier = RequestVerifier::new("secret").with_max_age(10);

        // Timestamp from 1 hour ago
        let old_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 3600;

        let result = verifier.verify("POST", "/api/test", "body", old_timestamp, "signature");

        assert!(matches!(result, Err(SigningError::RequestExpired)));
    }

    #[test]
    fn test_verify_request_with_zero_copy_body() {
        // Regression test: the production HTTP server stores incoming bodies
        // via `HttpRequest::set_body_bytes` (zero-copy), which clears the
        // legacy `body: Vec<u8>` field. `verify_request` must read the body
        // through `body_ref()`/`body_bytes()` so it sees the real payload
        // instead of treating every zero-copy request as having an empty
        // body (which would make every real signed request fail
        // verification).
        let secret = "test-secret";
        let signer = RequestSigner::new(secret);
        let verifier = RequestVerifier::new(secret);

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let body = "{\"test\":\"data\"}";
        let signature = signer.sign("POST", "/api/secure", body, timestamp);

        let mut request = HttpRequest::new("POST".to_string(), "/api/secure".to_string());
        request.headers.insert("X-Signature", signature);
        request.headers.insert("X-Timestamp", timestamp.to_string());
        // Simulate the real server path: body arrives as zero-copy Bytes,
        // which empties the legacy `body` field.
        request.set_body_bytes(bytes::Bytes::from_static(body.as_bytes()));
        assert!(request.body.is_empty());

        assert!(verifier.verify_request(&request).unwrap());
    }

    #[test]
    fn sign_is_real_hmac_sha256() {
        // Reference value computed independently via Python's hmac module:
        // hmac.new(b"secret", b"POST:/api/test:test body:1700000000", hashlib.sha256).hexdigest()
        let signer = RequestSigner::new("secret");
        let sig = signer.sign("POST", "/api/test", "test body", 1700000000);
        assert_eq!(
            sig,
            "b6979fc53be88b92dba411138047627e6b501ca24b1ae6f55dd7e87ee524755c"
        );
    }

    #[test]
    fn test_constant_time_eq() {
        use armature_core::crypto::constant_time_eq;
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
