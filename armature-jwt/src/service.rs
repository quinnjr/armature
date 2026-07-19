// JWT service implementation

use crate::{JwtConfig, JwtError, Result, StandardClaims, TokenPair};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, TokenData, Validation, decode, encode};
use serde::{Serialize, de::DeserializeOwned};

/// JWT service for token operations
#[derive(Clone)]
pub struct JwtService {
    config: JwtConfig,
    /// Absent for services constructed via [`JwtService::verify_only`], which
    /// only need a decoding key (e.g. a public key with no matching private
    /// key on hand). `sign` fails cleanly in that case rather than requiring
    /// signing material a verifier does not have.
    encoding_key: Option<EncodingKey>,
    decoding_key: DecodingKey,
    validation: Validation,
}

impl JwtService {
    /// Create a new JWT service
    pub fn new(config: JwtConfig) -> Result<Self> {
        let encoding_key = config.encoding_key()?;
        let decoding_key = config.decoding_key()?;
        let validation = config.validation();

        Ok(Self {
            config,
            encoding_key: Some(encoding_key),
            decoding_key,
            validation,
        })
    }

    /// Create a verify-only JWT service.
    ///
    /// Only builds the decoding key, so it works with configs that carry a
    /// public key (or secret) but no private key — the common case for a
    /// service that verifies tokens issued elsewhere. Calling [`sign`] on the
    /// result returns an error instead of panicking or requiring signing
    /// material that was never provided.
    ///
    /// [`sign`]: JwtService::sign
    pub fn verify_only(config: JwtConfig) -> Result<Self> {
        let decoding_key = config.decoding_key()?;
        let validation = config.validation();

        Ok(Self {
            config,
            encoding_key: None,
            decoding_key,
            validation,
        })
    }

    /// Sign a token with claims
    pub fn sign<T: Serialize>(&self, claims: &T) -> Result<String> {
        let encoding_key = self.encoding_key.as_ref().ok_or_else(|| {
            JwtError::ConfigError(
                "signing key not available: this JwtService was constructed with \
                 JwtService::verify_only and can only verify tokens"
                    .to_string(),
            )
        })?;
        let header = Header::new(self.config.algorithm);
        encode(&header, claims, encoding_key).map_err(JwtError::from)
    }

    /// Verify and decode a token
    pub fn verify<T: DeserializeOwned>(&self, token: &str) -> Result<T> {
        let token_data: TokenData<T> = decode(token, &self.decoding_key, &self.validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => JwtError::TokenExpired,
                jsonwebtoken::errors::ErrorKind::InvalidSignature => JwtError::InvalidSignature,
                _ => JwtError::EncodingError(e),
            })?;

        Ok(token_data.claims)
    }

    /// Decode without verification (useful for inspecting tokens)
    pub fn decode_unverified<T: DeserializeOwned>(&self, token: &str) -> Result<T> {
        let token_data: TokenData<T> =
            jsonwebtoken::dangerous::insecure_decode(token).map_err(JwtError::from)?;

        Ok(token_data.claims)
    }

    /// Generate a token pair (access + refresh)
    ///
    /// The access and refresh tokens carry the same custom claims but each gets its own
    /// `exp`, computed fresh from `config.expires_in` / `config.refresh_expires_in`. This
    /// guarantees the refresh token always outlives the access token and that the two
    /// tokens are never byte-identical.
    pub fn generate_token_pair<T: Serialize + Clone>(&self, claims: &T) -> Result<TokenPair> {
        let now = chrono::Utc::now().timestamp();

        // Serialize the claims once and reuse the resulting `Value` for both tokens; only
        // `exp` differs between the access and refresh claims, so there is no need to run
        // `serde_json::to_value` twice.
        let base_claims = serde_json::to_value(claims)
            .map_err(|e| JwtError::SerializationError(e.to_string()))?;

        let access_claims = Self::set_expiration(
            base_claims.clone(),
            now + self.config.expires_in.as_secs() as i64,
        )?;
        let refresh_claims = Self::set_expiration(
            base_claims,
            now + self.config.refresh_expires_in.as_secs() as i64,
        )?;

        let access_token = self.sign(&access_claims)?;
        let refresh_token = self.sign(&refresh_claims)?;

        Ok(TokenPair::new(
            access_token,
            refresh_token,
            self.config.expires_in.as_secs() as i64,
            self.config.refresh_expires_in.as_secs() as i64,
        ))
    }

    /// Refresh an access token
    ///
    /// Verifies the incoming refresh token, then re-issues a brand new token pair from its
    /// claims. Both the new access and refresh tokens get freshly computed expirations (via
    /// `generate_token_pair`), regardless of the `exp` carried by the old refresh token.
    pub fn refresh_token<T: DeserializeOwned + Serialize + Clone>(
        &self,
        refresh_token: &str,
    ) -> Result<TokenPair> {
        // Verify the refresh token
        let claims: T = self.verify(refresh_token)?;

        // Generate new token pair with fresh expirations
        self.generate_token_pair(&claims)
    }

    /// Set (add or overwrite) the `exp` field on an already-serialized claims `Value` to the
    /// given Unix timestamp.
    fn set_expiration(mut value: serde_json::Value, exp: i64) -> Result<serde_json::Value> {
        match value.as_object_mut() {
            Some(obj) => {
                obj.insert("exp".to_string(), serde_json::Value::from(exp));
                Ok(value)
            }
            None => Err(JwtError::SerializationError(
                "claims must serialize to a JSON object to carry an exp field".to_string(),
            )),
        }
    }

    /// Sign with standard claims
    pub fn sign_standard(
        &self,
        sub: String,
        additional_claims: Option<serde_json::Value>,
    ) -> Result<String> {
        let mut claims = StandardClaims::new()
            .with_subject(sub)
            .with_expiration(self.config.expires_in.as_secs() as i64);

        if let Some(iss) = &self.config.issuer {
            claims = claims.with_issuer(iss.clone());
        }

        if let Some(aud) = &self.config.audience {
            claims = claims.with_audience(aud.clone());
        }

        if let Some(additional) = additional_claims {
            // Merge additional claims
            let mut combined = serde_json::to_value(&claims)
                .map_err(|e| JwtError::SerializationError(e.to_string()))?;

            if let (Some(obj), serde_json::Value::Object(add_obj)) =
                (combined.as_object_mut(), additional)
            {
                obj.extend(add_obj);
            }

            let token = self.sign(&combined)?;
            return Ok(token);
        }

        self.sign(&claims)
    }

    /// Get the configuration
    pub fn config(&self) -> &JwtConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestClaims {
        sub: String,
        name: String,
        exp: i64,
    }

    fn test_config() -> JwtConfig {
        JwtConfig::new("test-secret".to_string())
    }

    fn test_claims() -> TestClaims {
        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
        TestClaims {
            sub: "123".to_string(),
            name: "Test".to_string(),
            exp,
        }
    }

    #[test]
    fn test_sign_and_verify() {
        let config = JwtConfig::new("test-secret".to_string());
        let service = JwtService::new(config).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();

        let claims = TestClaims {
            sub: "123".to_string(),
            name: "Test User".to_string(),
            exp,
        };

        let token = service.sign(&claims).unwrap();
        let decoded: TestClaims = service.verify(&token).unwrap();

        assert_eq!(decoded.sub, claims.sub);
        assert_eq!(decoded.name, claims.name);
    }

    #[test]
    fn test_invalid_signature() {
        let config1 = JwtConfig::new("secret1".to_string());
        let service1 = JwtService::new(config1).unwrap();

        let config2 = JwtConfig::new("secret2".to_string());
        let service2 = JwtService::new(config2).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();

        let claims = TestClaims {
            sub: "123".to_string(),
            name: "Test".to_string(),
            exp,
        };

        let token = service1.sign(&claims).unwrap();
        let result: Result<TestClaims> = service2.verify(&token);

        assert!(matches!(result, Err(JwtError::InvalidSignature)));
    }

    #[test]
    fn test_token_pair_generation() {
        let config = JwtConfig::new("test-secret".to_string());
        let service = JwtService::new(config).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();

        let claims = TestClaims {
            sub: "123".to_string(),
            name: "Test".to_string(),
            exp,
        };

        let pair = service.generate_token_pair(&claims).unwrap();

        assert!(!pair.access_token.is_empty());
        assert!(!pair.refresh_token.is_empty());
        assert_eq!(pair.token_type, "Bearer");
    }

    #[test]
    fn refresh_token_outlives_access_token() {
        let service = JwtService::new(test_config()).unwrap();
        let claims = test_claims();

        let pair = service.generate_token_pair(&claims).unwrap();
        let access: serde_json::Value = service.verify(&pair.access_token).unwrap();
        let refresh: serde_json::Value = service.verify(&pair.refresh_token).unwrap();

        let a_exp = access.get("exp").and_then(|v| v.as_i64()).unwrap();
        let r_exp = refresh.get("exp").and_then(|v| v.as_i64()).unwrap();

        assert!(
            r_exp > a_exp,
            "refresh exp {r_exp} must exceed access exp {a_exp}"
        );
        assert_ne!(pair.access_token, pair.refresh_token);

        // Verify that the metadata refresh_expires_in matches the actual token lifetime
        // (refresh_token.exp - now ≈ refresh_expires_in, within tolerance of ±2 seconds)
        let now = chrono::Utc::now().timestamp();
        let actual_lifetime = r_exp - now;
        let expected = pair.refresh_expires_in as i64;
        assert!(
            (actual_lifetime - expected).abs() <= 2,
            "TokenPair.refresh_expires_in {} must match refresh token lifetime {} (±2s)",
            expected,
            actual_lifetime
        );
    }

    #[test]
    fn refresh_reissues_a_fresh_access_token() {
        let service = JwtService::new(test_config()).unwrap();
        let claims = test_claims();

        let pair = service.generate_token_pair(&claims).unwrap();
        let new_pair = service
            .refresh_token::<TestClaims>(&pair.refresh_token)
            .unwrap();

        assert!(
            service
                .verify::<serde_json::Value>(&new_pair.access_token)
                .is_ok()
        );

        let old_access: serde_json::Value = service.verify(&pair.access_token).unwrap();
        let new_access: serde_json::Value = service.verify(&new_pair.access_token).unwrap();
        let old_exp = old_access.get("exp").and_then(|v| v.as_i64()).unwrap();
        let new_exp = new_access.get("exp").and_then(|v| v.as_i64()).unwrap();
        assert!(
            new_exp >= old_exp,
            "refreshed access token exp should not regress"
        );
    }

    #[test]
    fn test_decode_unverified() {
        let config = JwtConfig::new("test-secret".to_string());
        let service = JwtService::new(config).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();

        let claims = TestClaims {
            sub: "123".to_string(),
            name: "Test".to_string(),
            exp,
        };

        let token = service.sign(&claims).unwrap();
        let decoded: TestClaims = service.decode_unverified(&token).unwrap();

        assert_eq!(decoded, claims);
    }

    #[test]
    fn sign_standard_merges_configured_and_additional_claims() {
        let config = JwtConfig::new("test-secret".to_string())
            .with_issuer("test-issuer".to_string())
            .with_audience(vec!["test-audience".to_string()]);
        let service = JwtService::new(config).unwrap();

        let before = chrono::Utc::now().timestamp();

        let additional = serde_json::json!({
            "role": "admin",
            "org_id": "org-42",
        });

        let token = service
            .sign_standard("user-123".to_string(), Some(additional))
            .unwrap();

        let decoded: serde_json::Value = service.verify(&token).unwrap();

        assert_eq!(
            decoded.get("sub").and_then(|v| v.as_str()),
            Some("user-123")
        );
        assert_eq!(
            decoded.get("iss").and_then(|v| v.as_str()),
            Some("test-issuer")
        );
        assert_eq!(
            decoded.get("aud").and_then(|v| v.as_array()),
            Some(&vec![serde_json::Value::String(
                "test-audience".to_string()
            )])
        );
        assert_eq!(decoded.get("role").and_then(|v| v.as_str()), Some("admin"));
        assert_eq!(
            decoded.get("org_id").and_then(|v| v.as_str()),
            Some("org-42")
        );

        // `additional_claims` must not clobber the standard exp/iss fields.
        let exp = decoded.get("exp").and_then(|v| v.as_i64()).unwrap();
        assert!(
            exp > before,
            "exp {exp} should be a valid future timestamp (now was {before})"
        );
        assert_eq!(
            decoded.get("iss").and_then(|v| v.as_str()),
            Some("test-issuer"),
            "additional_claims must not clobber the configured issuer"
        );
    }
}
