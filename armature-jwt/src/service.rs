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
    /// Claim key used to distinguish access tokens from refresh tokens. Set by
    /// [`generate_token_pair`] and checked by [`refresh_token`] so that a leaked access token
    /// cannot be presented as a refresh token to mint a fresh token pair.
    ///
    /// [`generate_token_pair`]: JwtService::generate_token_pair
    /// [`refresh_token`]: JwtService::refresh_token
    const TOKEN_USE_CLAIM: &'static str = "token_use";
    /// Value of [`TOKEN_USE_CLAIM`](Self::TOKEN_USE_CLAIM) minted onto access tokens.
    const ACCESS_TOKEN_USE: &'static str = "access";
    /// Value of [`TOKEN_USE_CLAIM`](Self::TOKEN_USE_CLAIM) minted onto refresh tokens.
    const REFRESH_TOKEN_USE: &'static str = "refresh";

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
    ///
    /// Each token also gets a `token_use` claim (`"access"` or `"refresh"`) so that
    /// [`refresh_token`] can tell the two apart and reject an access token presented where a
    /// refresh token is expected.
    ///
    /// [`refresh_token`]: JwtService::refresh_token
    pub fn generate_token_pair<T: Serialize + Clone>(&self, claims: &T) -> Result<TokenPair> {
        let now = chrono::Utc::now().timestamp();

        // Serialize the claims once and reuse the resulting `Value` for both tokens; only
        // `exp` and `token_use` differ between the access and refresh claims, so there is no
        // need to run `serde_json::to_value` twice.
        let base_claims = serde_json::to_value(claims)
            .map_err(|e| JwtError::SerializationError(e.to_string()))?;

        let access_claims = Self::set_expiration(
            base_claims.clone(),
            now + self.config.expires_in.as_secs() as i64,
        )?;
        let access_claims = Self::set_token_use(access_claims, Self::ACCESS_TOKEN_USE)?;

        let refresh_claims = Self::set_expiration(
            base_claims,
            now + self.config.refresh_expires_in.as_secs() as i64,
        )?;
        let refresh_claims = Self::set_token_use(refresh_claims, Self::REFRESH_TOKEN_USE)?;

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
    /// Verifies the incoming token, checks that it carries `token_use: "refresh"` (see
    /// [`generate_token_pair`]), and if so re-issues a brand new token pair from its claims.
    /// A currently-valid *access* token is rejected with [`JwtError::InvalidToken`] rather
    /// than being honored, so a leaked access token cannot be laundered into a fresh
    /// access+refresh pair. Both new tokens get freshly computed expirations, regardless of
    /// the `exp` carried by the old refresh token.
    ///
    /// [`generate_token_pair`]: JwtService::generate_token_pair
    pub fn refresh_token<T: DeserializeOwned + Serialize + Clone>(
        &self,
        refresh_token: &str,
    ) -> Result<TokenPair> {
        // Verify generically first so the `token_use` discriminator can be inspected before
        // the claims are trusted as coming from a genuine refresh token.
        let raw: serde_json::Value = self.verify(refresh_token)?;

        let token_use = raw.get(Self::TOKEN_USE_CLAIM).and_then(|v| v.as_str());
        if token_use != Some(Self::REFRESH_TOKEN_USE) {
            return Err(JwtError::InvalidToken(format!(
                "expected a refresh token (token_use = \"{}\"), got token_use = {:?}",
                Self::REFRESH_TOKEN_USE,
                token_use
            )));
        }

        let claims: T =
            serde_json::from_value(raw).map_err(|e| JwtError::SerializationError(e.to_string()))?;

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

    /// Set (add or overwrite) the [`TOKEN_USE_CLAIM`](Self::TOKEN_USE_CLAIM) field on an
    /// already-serialized claims `Value`.
    fn set_token_use(mut value: serde_json::Value, token_use: &str) -> Result<serde_json::Value> {
        match value.as_object_mut() {
            Some(obj) => {
                obj.insert(
                    Self::TOKEN_USE_CLAIM.to_string(),
                    serde_json::Value::from(token_use),
                );
                Ok(value)
            }
            None => Err(JwtError::SerializationError(
                "claims must serialize to a JSON object to carry a token_use field".to_string(),
            )),
        }
    }

    /// Reserved [`StandardClaims`] keys that `additional_claims` in [`sign_standard`] is not
    /// allowed to override. A caller who wants to set one of these should use the dedicated
    /// builder methods (`with_issuer`, `with_expiration`, etc.) instead of `additional_claims`.
    /// This also includes [`TOKEN_USE_CLAIM`](Self::TOKEN_USE_CLAIM) so that a caller cannot
    /// use `additional_claims` to forge a `token_use: "refresh"` claim that would pass
    /// [`refresh_token`](JwtService::refresh_token)'s discriminator check without having gone
    /// through [`generate_token_pair`](JwtService::generate_token_pair).
    ///
    /// [`sign_standard`]: JwtService::sign_standard
    const RESERVED_STANDARD_CLAIMS: [&'static str; 7] = [
        "sub",
        "iss",
        "aud",
        "exp",
        "iat",
        "jti",
        Self::TOKEN_USE_CLAIM,
    ];

    /// Sign with standard claims
    ///
    /// `additional_claims` is merged on top of the standard claims (subject, expiration,
    /// configured issuer/audience), but any key that collides with a reserved standard claim
    /// (see [`RESERVED_STANDARD_CLAIMS`](Self::RESERVED_STANDARD_CLAIMS)) is dropped from
    /// `additional_claims` first, so it can never silently override the configured value.
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

        if let Some(mut additional) = additional_claims {
            // Strip any key that collides with a reserved standard claim before merging, so
            // `additional_claims` can never silently clobber sub/iss/aud/exp/iat/jti.
            if let serde_json::Value::Object(ref mut add_obj) = additional {
                for key in Self::RESERVED_STANDARD_CLAIMS {
                    add_obj.remove(key);
                }
            }

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
        let expected = pair.refresh_expires_in;
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

    /// `additional_claims` colliding with a reserved standard-claim key (here: `exp`, `sub`)
    /// must not be able to override the configured value — those keys are stripped from
    /// `additional_claims` before merging (see `RESERVED_STANDARD_CLAIMS`).
    #[test]
    fn sign_standard_additional_claims_cannot_clobber_reserved_keys() {
        let config = JwtConfig::new("test-secret".to_string())
            .with_issuer("test-issuer".to_string())
            .with_audience(vec!["test-audience".to_string()]);
        let service = JwtService::new(config).unwrap();

        let additional = serde_json::json!({
            "exp": 0,
            "sub": "attacker-controlled",
            "iss": "attacker-issuer",
            "aud": "attacker-audience",
            "iat": 0,
            "jti": "attacker-jti",
            "token_use": "refresh",
            "role": "admin",
        });

        let token = service
            .sign_standard("user-123".to_string(), Some(additional))
            .unwrap();

        let decoded: serde_json::Value = service.verify(&token).unwrap();

        let exp = decoded.get("exp").and_then(|v| v.as_i64()).unwrap();
        assert_ne!(exp, 0, "colliding additional_claims exp must not win");
        assert_eq!(
            decoded.get("sub").and_then(|v| v.as_str()),
            Some("user-123"),
            "colliding additional_claims sub must not win"
        );
        assert_eq!(
            decoded.get("iss").and_then(|v| v.as_str()),
            Some("test-issuer"),
            "colliding additional_claims iss must not win"
        );
        assert_eq!(
            decoded.get("aud").and_then(|v| v.as_array()),
            Some(&vec![serde_json::Value::String(
                "test-audience".to_string()
            )]),
            "colliding additional_claims aud must not win"
        );
        if let Some(iat) = decoded.get("iat") {
            assert_ne!(
                iat.as_i64(),
                Some(0),
                "colliding additional_claims iat must not win"
            );
        }
        if let Some(jti) = decoded.get("jti") {
            assert_ne!(
                jti.as_str(),
                Some("attacker-jti"),
                "colliding additional_claims jti must not win"
            );
        }
        assert_ne!(
            decoded.get("token_use").and_then(|v| v.as_str()),
            Some("refresh"),
            "colliding additional_claims token_use must not win"
        );
        // Non-colliding keys must still be merged in.
        assert_eq!(decoded.get("role").and_then(|v| v.as_str()), Some("admin"));
    }

    /// Passing a currently-valid *access* token to `refresh_token` must be rejected — otherwise
    /// a leaked short-lived access token could be laundered into a fresh access+refresh pair.
    /// The genuine refresh token from the same pair must still work.
    #[test]
    fn refresh_token_rejects_an_access_token() {
        let service = JwtService::new(test_config()).unwrap();
        let claims = test_claims();

        let pair = service.generate_token_pair(&claims).unwrap();

        let result = service.refresh_token::<TestClaims>(&pair.access_token);
        assert!(
            matches!(result, Err(JwtError::InvalidToken(_))),
            "access token must be rejected by refresh_token, got {result:?}"
        );

        let refreshed = service.refresh_token::<TestClaims>(&pair.refresh_token);
        assert!(
            refreshed.is_ok(),
            "genuine refresh token should still work: {refreshed:?}"
        );
    }

    /// A hand-built `{"alg":"none"}` token (no real signature) must never verify. `Algorithm`
    /// has no `none` variant in `jsonwebtoken`, and `Validation` is pinned to the
    /// server-configured algorithm, so this should fail before any signature check happens.
    #[test]
    fn test_alg_none_token_rejected() {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let config = JwtConfig::new("test-secret".to_string());
        let service = JwtService::new(config).unwrap();

        let header = serde_json::json!({"alg": "none", "typ": "JWT"});
        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
        let payload = serde_json::json!({"sub": "attacker", "exp": exp});

        let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string());
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.to_string());
        // alg:none tokens conventionally carry an empty signature segment.
        let forged_token = format!("{header_b64}.{payload_b64}.");

        let result: Result<serde_json::Value> = service.verify(&forged_token);
        assert!(
            result.is_err(),
            "a hand-forged alg:none token must be rejected, got {result:?}"
        );

        // Variant: a 2-segment token with no trailing dot at all (no signature segment
        // present, rather than an empty one) must also be rejected.
        let no_trailing_dot = format!("{header_b64}.{payload_b64}");
        let no_dot_result: Result<serde_json::Value> = service.verify(&no_trailing_dot);
        assert!(
            no_dot_result.is_err(),
            "a hand-forged alg:none token with no trailing dot must be rejected, got {no_dot_result:?}"
        );

        // Variant: explicitly confirm rejection against a service configured for HS256 (the
        // common default algorithm for this crate's `JwtConfig::new`), matching `test_config`'s
        // default rather than relying on that default implicitly.
        let hs256_config = JwtConfig::new("test-secret".to_string());
        assert_eq!(
            hs256_config.algorithm,
            jsonwebtoken::Algorithm::HS256,
            "sanity check: JwtConfig::new should default to HS256"
        );
        let hs256_service = JwtService::new(hs256_config).unwrap();
        let hs256_result: Result<serde_json::Value> = hs256_service.verify(&forged_token);
        assert!(
            hs256_result.is_err(),
            "a hand-forged alg:none token must be rejected by an HS256-configured service, got {hs256_result:?}"
        );
    }

    /// Algorithm-confusion attack: a service configured for RS256 must reject a token forged
    /// as HS256 using the RSA *public* key bytes as the HMAC secret, even though that is a
    /// classic attack against libraries that pick the verification algorithm from the token
    /// header. `jsonwebtoken`'s `Validation` pins `validation.algorithms` to the single
    /// server-configured algorithm (RS256 here), so a token whose header says HS256 is
    /// rejected outright, regardless of what key material was used to "sign" it.
    #[test]
    fn test_algorithm_confusion_hs256_forged_with_rsa_public_key_rejected() {
        const RSA_PRIVATE_KEY_PEM: &str = include_str!("../tests/fixtures/test_rsa_priv.pem");
        const RSA_PUBLIC_KEY_PEM: &str = include_str!("../tests/fixtures/test_rsa_pub.pem");

        let rs256_config = JwtConfig::with_rsa(
            RSA_PRIVATE_KEY_PEM.to_string(),
            RSA_PUBLIC_KEY_PEM.to_string(),
        );
        let rs256_service = JwtService::new(rs256_config).unwrap();

        let exp = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
        let claims = serde_json::json!({"sub": "user-123", "exp": exp});

        // Sanity check: a normally RS256-signed token verifies fine against this service.
        let legit_token = rs256_service.sign(&claims).unwrap();
        let legit_result: Result<serde_json::Value> = rs256_service.verify(&legit_token);
        assert!(
            legit_result.is_ok(),
            "sanity check failed: a genuine RS256 token should verify"
        );

        // Forge an HS256 token, using the RSA *public key* PEM bytes as the HMAC secret — the
        // textbook algorithm-confusion attack, since the public key is not secret.
        let hs256_config = JwtConfig::new(RSA_PUBLIC_KEY_PEM.to_string());
        let hs256_service = JwtService::new(hs256_config).unwrap();
        let forged_token = hs256_service.sign(&claims).unwrap();

        // The RS256-configured service must reject this forged HS256 token outright.
        let forged_result: Result<serde_json::Value> = rs256_service.verify(&forged_token);
        assert!(
            forged_result.is_err(),
            "algorithm-confusion forged HS256 token must be rejected by an RS256-configured \
             service, got {forged_result:?}"
        );
    }

    /// A token whose `exp` is already in the past must be rejected with
    /// `JwtError::TokenExpired`.
    #[test]
    fn test_expired_token_rejected() {
        let config = JwtConfig::new("test-secret".to_string());
        let service = JwtService::new(config).unwrap();

        let exp = chrono::Utc::now().timestamp() - 1000;
        let claims = TestClaims {
            sub: "123".to_string(),
            name: "Test".to_string(),
            exp,
        };

        let token = service.sign(&claims).unwrap();
        let result: Result<TestClaims> = service.verify(&token);

        assert!(
            matches!(result, Err(JwtError::TokenExpired)),
            "expired token must be rejected with JwtError::TokenExpired, got {result:?}"
        );
    }
}
