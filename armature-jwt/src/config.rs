// JWT configuration

use crate::Result;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// JWT configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtConfig {
    /// Secret key for HS256/HS384/HS512 algorithms
    pub secret: Option<String>,

    /// Public key for RS256/RS384/RS512/ES256/ES384 algorithms (PEM format)
    pub public_key: Option<String>,

    /// Private key for RS256/RS384/RS512/ES256/ES384 algorithms (PEM format)
    pub private_key: Option<String>,

    /// Algorithm to use (default: HS256)
    pub algorithm: Algorithm,

    /// Token expiration time (default: 1 hour)
    pub expires_in: Duration,

    /// Refresh token expiration (default: 7 days)
    pub refresh_expires_in: Duration,

    /// Issuer (iss claim)
    pub issuer: Option<String>,

    /// Audience (aud claim)
    pub audience: Option<Vec<String>>,

    /// Validate expiration
    pub validate_exp: bool,

    /// Validate not before.
    ///
    /// **On by default** in both [`JwtConfig::new`] and [`JwtConfig::with_rsa`] — a token
    /// carrying a future `nbf` claim is rejected unless this is explicitly turned off via
    /// [`JwtConfig::with_nbf_validation`]`(false)`, for tokens that don't need not-before
    /// enforcement.
    pub validate_nbf: bool,

    /// Leeway for time validation (seconds)
    pub leeway: u64,
}

impl JwtConfig {
    /// Create a new configuration with a secret
    pub fn new(secret: String) -> Self {
        Self {
            secret: Some(secret),
            public_key: None,
            private_key: None,
            algorithm: Algorithm::HS256,
            expires_in: Duration::from_secs(3600), // 1 hour
            refresh_expires_in: Duration::from_secs(604800), // 7 days
            issuer: None,
            audience: None,
            validate_exp: true,
            validate_nbf: true,
            leeway: 0,
        }
    }

    /// Create configuration with RSA keys
    pub fn with_rsa(private_key: String, public_key: String) -> Self {
        Self {
            secret: None,
            public_key: Some(public_key),
            private_key: Some(private_key),
            algorithm: Algorithm::RS256,
            expires_in: Duration::from_secs(3600),
            refresh_expires_in: Duration::from_secs(604800),
            issuer: None,
            audience: None,
            validate_exp: true,
            validate_nbf: true,
            leeway: 0,
        }
    }

    /// Set the algorithm
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Set expiration time
    pub fn with_expiration(mut self, duration: Duration) -> Self {
        self.expires_in = duration;
        self
    }

    /// Set refresh token expiration
    pub fn with_refresh_expiration(mut self, duration: Duration) -> Self {
        self.refresh_expires_in = duration;
        self
    }

    /// Set issuer
    pub fn with_issuer(mut self, issuer: String) -> Self {
        self.issuer = Some(issuer);
        self
    }

    /// Set audience
    pub fn with_audience(mut self, audience: Vec<String>) -> Self {
        self.audience = Some(audience);
        self
    }

    /// Set leeway
    pub fn with_leeway(mut self, leeway: u64) -> Self {
        self.leeway = leeway;
        self
    }

    /// Enable or disable `nbf` (not-before) validation.
    ///
    /// `validate_nbf` is `true` by default (see its doc comment), so a token with a future
    /// `nbf` claim is rejected unless this is called with `false` to opt out, for tokens that
    /// don't need not-before enforcement.
    pub fn with_nbf_validation(mut self, enabled: bool) -> Self {
        self.validate_nbf = enabled;
        self
    }

    /// Get encoding key
    pub fn encoding_key(&self) -> Result<EncodingKey> {
        match self.algorithm {
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                let secret = self.secret.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError("Secret required for HMAC algorithms".to_string())
                })?;
                Ok(EncodingKey::from_secret(secret.as_bytes()))
            }
            Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => {
                let private_key = self.private_key.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError(
                        "Private key required for RSA algorithms".to_string(),
                    )
                })?;
                EncodingKey::from_rsa_pem(private_key.as_bytes())
                    .map_err(|e| crate::JwtError::ConfigError(e.to_string()))
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                let private_key = self.private_key.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError(
                        "Private key required for ECDSA algorithms".to_string(),
                    )
                })?;
                EncodingKey::from_ec_pem(private_key.as_bytes())
                    .map_err(|e| crate::JwtError::ConfigError(e.to_string()))
            }
            _ => Err(crate::JwtError::ConfigError(
                "Unsupported algorithm".to_string(),
            )),
        }
    }

    /// Get decoding key
    pub fn decoding_key(&self) -> Result<DecodingKey> {
        match self.algorithm {
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                let secret = self.secret.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError("Secret required for HMAC algorithms".to_string())
                })?;
                Ok(DecodingKey::from_secret(secret.as_bytes()))
            }
            Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 => {
                let public_key = self.public_key.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError(
                        "Public key required for RSA algorithms".to_string(),
                    )
                })?;
                DecodingKey::from_rsa_pem(public_key.as_bytes())
                    .map_err(|e| crate::JwtError::ConfigError(e.to_string()))
            }
            Algorithm::ES256 | Algorithm::ES384 => {
                let public_key = self.public_key.as_ref().ok_or_else(|| {
                    crate::JwtError::ConfigError(
                        "Public key required for ECDSA algorithms".to_string(),
                    )
                })?;
                DecodingKey::from_ec_pem(public_key.as_bytes())
                    .map_err(|e| crate::JwtError::ConfigError(e.to_string()))
            }
            _ => Err(crate::JwtError::ConfigError(
                "Unsupported algorithm".to_string(),
            )),
        }
    }

    /// Get validation config
    pub fn validation(&self) -> Validation {
        let mut validation = Validation::new(self.algorithm);
        validation.validate_exp = self.validate_exp;
        validation.validate_nbf = self.validate_nbf;
        validation.leeway = self.leeway;

        if let Some(ref iss) = self.issuer {
            validation.set_issuer(&[iss]);
        }

        if let Some(ref aud) = self.audience {
            validation.set_audience(aud);
        }

        validation
    }
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self::new("change-me-in-production".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = JwtConfig::default();
        assert!(config.secret.is_some());
        assert_eq!(config.algorithm, Algorithm::HS256);
        assert!(config.validate_exp);
    }

    #[test]
    fn test_config_builder() {
        let config = JwtConfig::new("secret".to_string())
            .with_algorithm(Algorithm::HS512)
            .with_expiration(Duration::from_secs(7200))
            .with_issuer("my-app".to_string())
            .with_leeway(60);

        assert_eq!(config.algorithm, Algorithm::HS512);
        assert_eq!(config.expires_in, Duration::from_secs(7200));
        assert_eq!(config.issuer, Some("my-app".to_string()));
        assert_eq!(config.leeway, 60);
    }

    #[test]
    fn test_encoding_key() {
        let config = JwtConfig::new("test-secret".to_string());
        let key = config.encoding_key();
        assert!(key.is_ok());
    }

    #[test]
    fn test_decoding_key() {
        let config = JwtConfig::new("test-secret".to_string());
        let key = config.decoding_key();
        assert!(key.is_ok());
    }

    #[test]
    fn test_validation() {
        let config = JwtConfig::new("secret".to_string())
            .with_issuer("my-issuer".to_string())
            .with_audience(vec!["my-audience".to_string()]);

        let validation = config.validation();
        assert!(validation.iss.is_some());
        assert!(validation.aud.is_some());
        assert!(validation.validate_exp);
    }

    #[test]
    fn test_algorithm_hs256() {
        let config = JwtConfig::new("secret".to_string()).with_algorithm(Algorithm::HS256);
        assert_eq!(config.algorithm, Algorithm::HS256);
    }

    #[test]
    fn test_algorithm_hs384() {
        let config = JwtConfig::new("secret".to_string()).with_algorithm(Algorithm::HS384);
        assert_eq!(config.algorithm, Algorithm::HS384);
    }

    #[test]
    fn test_algorithm_hs512() {
        let config = JwtConfig::new("secret".to_string()).with_algorithm(Algorithm::HS512);
        assert_eq!(config.algorithm, Algorithm::HS512);
    }

    #[test]
    fn test_validation_enabled_by_default() {
        let config = JwtConfig::new("secret".to_string());
        assert!(config.validate_exp);

        let validation = config.validation();
        assert!(validation.validate_exp);
    }

    #[test]
    fn test_leeway_configuration() {
        let config = JwtConfig::new("secret".to_string()).with_leeway(120);

        assert_eq!(config.leeway, 120);

        let validation = config.validation();
        assert_eq!(validation.leeway, 120);
    }

    #[test]
    fn test_audience_configuration() {
        let config = JwtConfig::new("secret".to_string()).with_audience(vec!["app1".to_string()]);

        assert_eq!(config.audience, Some(vec!["app1".to_string()]));

        let validation = config.validation();
        assert!(validation.aud.is_some());
    }

    #[test]
    fn test_issuer_configuration() {
        let config = JwtConfig::new("secret".to_string()).with_issuer("auth-server".to_string());

        assert_eq!(config.issuer, Some("auth-server".to_string()));

        let validation = config.validation();
        assert!(validation.iss.is_some());
    }

    #[test]
    fn test_expiration_duration() {
        let one_hour = Duration::from_secs(3600);
        let config = JwtConfig::new("secret".to_string()).with_expiration(one_hour);

        assert_eq!(config.expires_in, one_hour);
    }

    #[test]
    fn test_default_expiration() {
        let config = JwtConfig::default();
        assert_eq!(config.expires_in, Duration::from_secs(3600));
    }

    #[test]
    fn test_config_clone() {
        let config1 = JwtConfig::new("secret".to_string()).with_issuer("issuer".to_string());

        let config2 = config1.clone();
        assert_eq!(config1.issuer, config2.issuer);
        assert_eq!(config1.algorithm, config2.algorithm);
    }

    #[test]
    fn test_config_with_all_options() {
        let config = JwtConfig::new("secret".to_string())
            .with_algorithm(Algorithm::HS512)
            .with_expiration(Duration::from_secs(7200))
            .with_issuer("issuer".to_string())
            .with_audience(vec!["audience".to_string()])
            .with_leeway(60);

        assert_eq!(config.algorithm, Algorithm::HS512);
        assert_eq!(config.expires_in, Duration::from_secs(7200));
        assert_eq!(config.issuer, Some("issuer".to_string()));
        assert_eq!(config.audience, Some(vec!["audience".to_string()]));
        assert_eq!(config.leeway, 60);
        assert!(config.validate_exp);
    }

    #[test]
    fn test_validation_requirements() {
        let config = JwtConfig::new("secret".to_string())
            .with_issuer("required-issuer".to_string())
            .with_audience(vec!["required-audience".to_string()]);

        let validation = config.validation();
        assert!(validation.iss.is_some());
        assert!(validation.aud.is_some());

        let expected_iss: std::collections::HashSet<String> =
            std::collections::HashSet::from(["required-issuer".to_string()]);
        assert_eq!(validation.iss.as_ref().unwrap(), &expected_iss);
    }

    #[test]
    fn test_zero_leeway() {
        let config = JwtConfig::new("secret".to_string()).with_leeway(0);

        assert_eq!(config.leeway, 0);
    }

    #[test]
    fn test_long_expiration() {
        let one_year = Duration::from_secs(365 * 24 * 3600);
        let config = JwtConfig::new("secret".to_string()).with_expiration(one_year);

        assert_eq!(config.expires_in, one_year);
    }

    #[test]
    fn test_nbf_validation_on_by_default() {
        let config = JwtConfig::new("secret".to_string());
        assert!(config.validate_nbf);

        let validation = config.validation();
        assert!(validation.validate_nbf);
    }

    #[test]
    fn test_with_nbf_validation_builder() {
        let config = JwtConfig::new("secret".to_string()).with_nbf_validation(false);
        assert!(!config.validate_nbf);

        let validation = config.validation();
        assert!(!validation.validate_nbf);
    }

    /// A token with a future `nbf` is rejected when `validate_nbf` is left at its default (on)
    /// — this is the current, secure-by-default posture.
    #[test]
    fn future_nbf_token_rejected_by_default() {
        let config = JwtConfig::new("nbf-test-secret".to_string());
        assert!(config.validate_nbf);

        let service = crate::service::JwtService::new(config).unwrap();

        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "sub": "user-1",
            "nbf": now + 3600,
            "exp": now + 7200,
        });

        let token = service.sign(&claims).unwrap();
        let result: crate::Result<serde_json::Value> = service.verify(&token);
        assert!(
            result.is_err(),
            "future nbf should be rejected when validate_nbf is on (default): {result:?}"
        );
    }

    /// The same future-`nbf` token is accepted once `validate_nbf` is explicitly disabled via
    /// [`JwtConfig::with_nbf_validation`]`(false)`.
    #[test]
    fn future_nbf_token_accepted_when_validation_explicitly_disabled() {
        let config = JwtConfig::new("nbf-test-secret".to_string()).with_nbf_validation(false);
        assert!(!config.validate_nbf);

        let service = crate::service::JwtService::new(config).unwrap();

        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "sub": "user-1",
            "nbf": now + 3600,
            "exp": now + 7200,
        });

        let token = service.sign(&claims).unwrap();
        let result: crate::Result<serde_json::Value> = service.verify(&token);
        assert!(
            result.is_ok(),
            "future nbf should be accepted when validate_nbf is explicitly disabled"
        );
    }
}
