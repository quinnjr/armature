//! Passwordless Authentication
//!
//! Provides magic link and WebAuthn passwordless authentication.
//!
//! # Features
//!
//! - Magic link generation and verification
//! - Email-based passwordless login
//! - WebAuthn registration and authentication
//! - Time-limited tokens
//!
//! # Usage
//!
//! ```no_run
//! use armature_auth::passwordless::*;
//!
//! # async fn example(presented_token: &str) -> Result<(), PasswordlessError> {
//! // Issue the link and store the record (keyed by identifier) somewhere the
//! // callback can load it from.
//! let mut token = MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))?;
//! let link = format!("https://myapp.com/auth/verify?token={}", token.token);
//!
//! // Send link via email...
//!
//! // In the callback: check the token the caller actually presented against
//! // the stored record. `verify_token` does the constant-time comparison as
//! // well as the expiry/single-use checks — the login decision must never be
//! // made without comparing the secret.
//! if token.verify_token(presented_token)? {
//!     token.mark_used(); // persist this, or the link stays redeemable
//!     println!("Magic link valid!");
//! }
//! # Ok(())
//! # }
//! ```

use chrono::{DateTime, Duration, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "webauthn")]
use webauthn_rs::prelude::*;

/// Passwordless authentication errors
#[derive(Debug, Error)]
pub enum PasswordlessError {
    #[error("Invalid token")]
    InvalidToken,

    #[error("Token expired")]
    TokenExpired,

    #[error("Token already used")]
    TokenUsed,

    #[error("WebAuthn error: {0}")]
    WebAuthn(String),

    #[error("Feature not enabled: {0}")]
    FeatureNotEnabled(&'static str),
}

/// Magic Link Token
///
/// Time-limited token for passwordless authentication via email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkToken {
    /// The token string
    pub token: String,

    /// User identifier (email, user ID, etc.)
    pub identifier: String,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Expiration timestamp
    pub expires_at: DateTime<Utc>,

    /// Whether token has been used
    pub used: bool,
}

impl MagicLinkToken {
    /// Generate a new magic link token
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::passwordless::*;
    ///
    /// # fn example() -> Result<(), PasswordlessError> {
    /// let token = MagicLinkToken::generate(
    ///     "user@example.com",
    ///     std::time::Duration::from_secs(3600) // 1 hour
    /// )?;
    ///
    /// println!("Token: {}", token.token);
    /// # Ok(())
    /// # }
    /// ```
    pub fn generate(
        identifier: impl Into<String>,
        ttl: std::time::Duration,
    ) -> Result<Self, PasswordlessError> {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
        let token = hex::encode(bytes);

        let now = Utc::now();
        let expires_at =
            now + Duration::from_std(ttl).map_err(|_| PasswordlessError::InvalidToken)?;

        Ok(Self {
            token,
            identifier: identifier.into(),
            created_at: now,
            expires_at,
            used: false,
        })
    }

    /// Check that this token is still redeemable — **not** that the caller
    /// presented it.
    ///
    /// Checks:
    /// - Not already used
    /// - Not expired
    ///
    /// This answers a question about the stored record only. It takes no
    /// candidate and therefore compares no secret, so on its own it authorises
    /// nothing: any caller who can name the identifier could log in. To
    /// authenticate a click on a magic link, look the record up and pass the
    /// token from the URL to [`Self::verify_token`].
    pub fn is_usable(&self) -> Result<bool, PasswordlessError> {
        if self.used {
            return Err(PasswordlessError::TokenUsed);
        }

        if Utc::now() > self.expires_at {
            return Err(PasswordlessError::TokenExpired);
        }

        Ok(true)
    }

    /// Verify a token presented by a caller against this stored record.
    ///
    /// This is the login check: it compares `candidate` against the issued
    /// token in constant time (so the secret cannot be recovered a byte at a
    /// time by timing the response) *and* enforces the single-use and expiry
    /// rules.
    ///
    /// Returns `Err(InvalidToken)` when the candidate does not match, and the
    /// corresponding error when the record is used or expired. On `Ok(true)`
    /// the caller must persist [`Self::mark_used`] before completing the login,
    /// otherwise the link stays redeemable.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::passwordless::*;
    ///
    /// # fn example() -> Result<(), PasswordlessError> {
    /// let mut token = MagicLinkToken::generate(
    ///     "user@example.com",
    ///     std::time::Duration::from_secs(900),
    /// )?;
    ///
    /// // The value that arrived in the callback URL.
    /// let presented = token.token.clone();
    ///
    /// if token.verify_token(&presented)? {
    ///     token.mark_used();
    /// }
    ///
    /// // Replaying the same link no longer works.
    /// assert!(token.verify_token(&presented).is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn verify_token(&self, candidate: &str) -> Result<bool, PasswordlessError> {
        // Compare first and unconditionally, so a mismatched candidate is not
        // distinguishable from a used/expired one by which check ran.
        let matches =
            armature_core::crypto::constant_time_eq(candidate.as_bytes(), self.token.as_bytes());

        self.is_usable()?;

        if matches {
            Ok(true)
        } else {
            Err(PasswordlessError::InvalidToken)
        }
    }

    /// Mark token as used
    pub fn mark_used(&mut self) {
        self.used = true;
    }

    /// Create verification URL
    pub fn to_url(&self, base_url: &str) -> String {
        format!("{}?token={}", base_url, self.token)
    }
}

/// WebAuthn Configuration
#[cfg(feature = "webauthn")]
#[derive(Debug, Clone)]
pub struct WebAuthnConfig {
    /// Relying party ID (your domain)
    pub rp_id: String,

    /// Relying party name
    pub rp_name: String,

    /// Origin URL
    pub origin: Url,
}

#[cfg(feature = "webauthn")]
impl WebAuthnConfig {
    pub fn new(
        rp_id: impl Into<String>,
        rp_name: impl Into<String>,
        origin: impl Into<String>,
    ) -> Result<Self, PasswordlessError> {
        Ok(Self {
            rp_id: rp_id.into(),
            rp_name: rp_name.into(),
            origin: Url::parse(&origin.into())
                .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))?,
        })
    }
}

/// WebAuthn Manager
#[cfg(feature = "webauthn")]
pub struct WebAuthnManager {
    webauthn: Webauthn,
}

#[cfg(feature = "webauthn")]
impl WebAuthnManager {
    /// Create new WebAuthn manager
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_auth::passwordless::*;
    ///
    /// # #[cfg(feature = "webauthn")]
    /// # fn example() -> Result<(), PasswordlessError> {
    /// let config = WebAuthnConfig::new(
    ///     "example.com",
    ///     "Example App",
    ///     "https://example.com"
    /// )?;
    ///
    /// let manager = WebAuthnManager::new(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(config: WebAuthnConfig) -> Result<Self, PasswordlessError> {
        let rp_origin = config.origin.clone();
        let builder = WebauthnBuilder::new(&config.rp_id, &rp_origin)
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))?;

        let builder = builder.rp_name(&config.rp_name);

        let webauthn = builder
            .build()
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))?;

        Ok(Self { webauthn })
    }

    /// Start registration (credential creation)
    ///
    /// Returns challenge that should be sent to client.
    pub fn start_registration(
        &self,
        user_id: &[u8],
        username: &str,
        display_name: &str,
    ) -> Result<(CreationChallengeResponse, PasskeyRegistration), PasswordlessError> {
        // webauthn-rs 0.5 uses a Uuid for the user handle.
        let user_unique_id = Uuid::from_slice(user_id)
            .map_err(|e| PasswordlessError::WebAuthn(format!("user_id must be 16 bytes: {e}")))?;

        let (ccr, reg_state) = self
            .webauthn
            .start_passkey_registration(user_unique_id, username, display_name, None)
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))?;

        Ok((ccr, reg_state))
    }

    /// Finish registration
    ///
    /// Verifies client response and returns credential.
    pub fn finish_registration(
        &self,
        reg: &RegisterPublicKeyCredential,
        state: &PasskeyRegistration,
    ) -> Result<Passkey, PasswordlessError> {
        self.webauthn
            .finish_passkey_registration(reg, state)
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))
    }

    /// Start authentication
    ///
    /// Returns challenge that should be sent to client.
    pub fn start_authentication(
        &self,
        passkeys: &[Passkey],
    ) -> Result<(RequestChallengeResponse, PasskeyAuthentication), PasswordlessError> {
        self.webauthn
            .start_passkey_authentication(passkeys)
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))
    }

    /// Finish authentication
    ///
    /// Verifies client response.
    pub fn finish_authentication(
        &self,
        auth: &PublicKeyCredential,
        state: &PasskeyAuthentication,
    ) -> Result<AuthenticationResult, PasswordlessError> {
        self.webauthn
            .finish_passkey_authentication(auth, state)
            .map_err(|e| PasswordlessError::WebAuthn(e.to_string()))
    }
}

#[cfg(not(feature = "webauthn"))]
pub struct WebAuthnManager;

#[cfg(not(feature = "webauthn"))]
impl WebAuthnManager {
    pub fn new(_config: ()) -> Result<Self, PasswordlessError> {
        Err(PasswordlessError::FeatureNotEnabled("webauthn"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_magic_link() {
        let token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();

        assert!(!token.token.is_empty());
        assert_eq!(token.identifier, "user@example.com");
        assert!(!token.used);
    }

    #[test]
    fn test_verify_magic_link() {
        let token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();

        assert!(token.is_usable().is_ok());
    }

    #[test]
    fn test_verify_token_requires_the_presented_secret() {
        // The login check must compare the value the caller presented. A check
        // that only inspects `used`/`expires_at` authorises anyone who can name
        // the identifier.
        let token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();

        assert!(token.verify_token(&token.token).unwrap());
        assert!(matches!(
            token.verify_token("not-the-token"),
            Err(PasswordlessError::InvalidToken)
        ));
        assert!(matches!(
            token.verify_token(""),
            Err(PasswordlessError::InvalidToken)
        ));
    }

    #[test]
    fn test_verify_token_rejects_used_and_expired() {
        let mut token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();
        let presented = token.token.clone();
        token.mark_used();
        assert!(matches!(
            token.verify_token(&presented),
            Err(PasswordlessError::TokenUsed)
        ));

        let token = MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(0))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(matches!(
            token.verify_token(&token.token),
            Err(PasswordlessError::TokenExpired)
        ));
    }

    #[test]
    fn test_magic_link_url() {
        let token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();

        let url = token.to_url("https://example.com/auth/verify");
        assert!(url.starts_with("https://example.com/auth/verify?token="));
    }

    #[test]
    fn test_magic_link_used() {
        let mut token =
            MagicLinkToken::generate("user@example.com", std::time::Duration::from_secs(3600))
                .unwrap();

        token.mark_used();
        assert!(matches!(
            token.is_usable(),
            Err(PasswordlessError::TokenUsed)
        ));
    }

    #[test]
    fn test_expired_magic_link() {
        let token = MagicLinkToken::generate(
            "user@example.com",
            std::time::Duration::from_secs(0), // Already expired
        )
        .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(matches!(
            token.is_usable(),
            Err(PasswordlessError::TokenExpired)
        ));
    }
}
