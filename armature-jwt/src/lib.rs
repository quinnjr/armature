//! JWT authentication and authorization for Armature
//!
//! This crate provides JSON Web Token (JWT) support for the Armature framework,
//! including token generation, verification, and management.
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```
//! use armature_jwt::{JwtConfig, JwtManager, StandardClaims};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create JWT configuration
//! let config = JwtConfig::new("your-secret-key".to_string());
//! let manager = JwtManager::new(config)?;
//!
//! // Create claims
//! let claims = StandardClaims::new()
//!     .with_subject("user123".to_string())
//!     .with_expiration(3600); // 1 hour
//!
//! // Sign a token
//! let token = manager.sign(&claims)?;
//!
//! // Verify and decode
//! let decoded: StandardClaims = manager.verify(&token)?;
//! assert_eq!(decoded.sub, Some("user123".to_string()));
//! # Ok(())
//! # }
//! ```
//!
//! ## Custom Claims
//!
//! ```
//! use armature_jwt::{Claims, JwtConfig, JwtManager};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct UserClaims {
//!     email: String,
//!     role: String,
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = JwtConfig::new("secret".to_string());
//! let manager = JwtManager::new(config)?;
//!
//! let claims = Claims::new(UserClaims {
//!     email: "user@example.com".to_string(),
//!     role: "admin".to_string(),
//! })
//! .with_subject("123".to_string())
//! .with_expiration(7200);
//!
//! let token = manager.sign(&claims)?;
//! let decoded: Claims<UserClaims> = manager.verify(&token)?;
//! assert_eq!(decoded.custom.email, "user@example.com");
//! # Ok(())
//! # }
//! ```

pub mod claims;
pub mod config;
pub mod error;
pub mod service;
pub mod token;

pub use claims::{Claims, StandardClaims};
pub use config::JwtConfig;
pub use error::{JwtError, Result};
pub use service::JwtService;
pub use token::{Token, TokenPair};

// Re-export jsonwebtoken types
pub use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};

use armature_log::{debug, info, trace};

/// JWT service for token management
#[derive(Clone)]
pub struct JwtManager {
    service: JwtService,
}

impl JwtManager {
    /// Create a new JWT manager
    ///
    /// # Example
    ///
    /// ```
    /// use armature_jwt::{JwtConfig, JwtManager};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = JwtConfig::new("my-secret".to_string());
    /// let manager = JwtManager::new(config)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(config: JwtConfig) -> Result<Self> {
        info!("Initializing JWT manager");
        debug!("JWT algorithm: {:?}", config.algorithm);
        let service = JwtService::new(config)?;
        Ok(Self { service })
    }

    /// Create a verify-only JWT manager.
    ///
    /// Unlike [`JwtManager::new`], this does not require signing (encoding)
    /// key material — only a decoding key (secret for HS*, public key for
    /// RS*/ES*) is needed. This is the right constructor for a pure token
    /// *verifier* that never mints tokens itself, e.g. one configured with
    /// only a public key and no corresponding private key.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_jwt::{JwtConfig, JwtManager, StandardClaims};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let signer = JwtManager::new(JwtConfig::new("shared-secret".to_string()))?;
    /// let token = signer.sign(
    ///     &StandardClaims::new()
    ///         .with_subject("user".to_string())
    ///         .with_expiration(3600),
    /// )?;
    ///
    /// let verifier = JwtManager::verify_only(JwtConfig::new("shared-secret".to_string()))?;
    /// let claims: StandardClaims = verifier.verify(&token)?;
    /// assert_eq!(claims.sub, Some("user".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn verify_only(config: JwtConfig) -> Result<Self> {
        info!("Initializing verify-only JWT manager");
        debug!("JWT algorithm: {:?}", config.algorithm);
        let service = JwtService::verify_only(config)?;
        Ok(Self { service })
    }

    /// Sign a token with claims
    ///
    /// # Example
    ///
    /// ```
    /// use armature_jwt::{JwtConfig, JwtManager, StandardClaims};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = JwtManager::new(JwtConfig::new("secret".to_string()))?;
    /// let claims = StandardClaims::new().with_subject("user".to_string());
    /// let token = manager.sign(&claims)?;
    /// assert!(!token.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn sign<T: serde::Serialize>(&self, claims: &T) -> Result<String> {
        trace!("Signing JWT token");
        self.service.sign(claims)
    }

    /// Verify and decode a token
    ///
    /// # Example
    ///
    /// ```
    /// use armature_jwt::{JwtConfig, JwtManager, StandardClaims};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let manager = JwtManager::new(JwtConfig::new("secret".to_string()))?;
    /// let claims = StandardClaims::new()
    ///     .with_subject("user".to_string())
    ///     .with_expiration(3600); // Add expiration
    /// let token = manager.sign(&claims)?;
    /// let decoded: StandardClaims = manager.verify(&token)?;
    /// assert_eq!(decoded.sub, Some("user".to_string()));
    /// # Ok(())
    /// # }
    /// ```
    pub fn verify<T: serde::de::DeserializeOwned>(&self, token: &str) -> Result<T> {
        trace!("Verifying JWT token");
        match self.service.verify(token) {
            Ok(claims) => {
                trace!("JWT token verified successfully");
                Ok(claims)
            }
            Err(e) => {
                debug!("JWT verification failed: {}", e);
                Err(e)
            }
        }
    }

    /// Generate a token pair (access + refresh)
    pub fn generate_token_pair<T: serde::Serialize + Clone>(
        &self,
        claims: &T,
    ) -> Result<TokenPair> {
        debug!("Generating JWT token pair");
        self.service.generate_token_pair(claims)
    }

    /// Refresh an access token using a refresh token
    #[allow(dead_code)]
    pub fn refresh_token<T: serde::de::DeserializeOwned + serde::Serialize + Clone>(
        &self,
        refresh_token: &str,
    ) -> Result<TokenPair> {
        self.service.refresh_token::<T>(refresh_token)
    }

    /// Decode a token without verification (useful for inspecting expired tokens)
    pub fn decode_unverified<T: serde::de::DeserializeOwned>(&self, token: &str) -> Result<T> {
        self.service.decode_unverified(token)
    }

    /// Get the configuration
    pub fn config(&self) -> &JwtConfig {
        self.service.config()
    }
}

impl Default for JwtManager {
    fn default() -> Self {
        Self::new(JwtConfig::default()).expect("Failed to create default JwtManager")
    }
}

/// Prelude for common imports.
///
/// ```
/// use armature_jwt::prelude::*;
/// ```
pub mod prelude {
    pub use crate::JwtManager;
    pub use crate::claims::{Claims, StandardClaims};
    pub use crate::config::JwtConfig;
    pub use crate::error::{JwtError, Result};
    pub use crate::service::JwtService;
    pub use crate::token::{Token, TokenPair};
    pub use jsonwebtoken::Algorithm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestClaims {
        sub: String,
        name: String,
        exp: i64,
    }

    #[test]
    fn test_sign_and_verify() {
        let config = JwtConfig::new("test-secret".to_string());
        let manager = JwtManager::new(config).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();

        let claims = TestClaims {
            sub: "123".to_string(),
            name: "John Doe".to_string(),
            exp,
        };

        let token = manager.sign(&claims).unwrap();
        let decoded: TestClaims = manager.verify(&token).unwrap();

        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.name, claims.name);
    }

    #[test]
    fn test_invalid_token() {
        let config = JwtConfig::new("test-secret".to_string());
        let manager = JwtManager::new(config).unwrap();

        let result: Result<TestClaims> = manager.verify("invalid.token.here");
        assert!(result.is_err());
    }
}
