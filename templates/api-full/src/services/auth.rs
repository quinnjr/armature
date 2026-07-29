//! Authentication service
//!
//! Wraps `armature-jwt` (token signing/verification) and `armature-auth`
//! (Argon2 password hashing) behind a small, template-friendly API.

use crate::models::{TokenClaims, User};
use armature::armature_auth::{PasswordHasher, PasswordVerifier};
use armature::armature_jwt::{JwtConfig, JwtError, JwtManager};
use chrono::{Duration, Utc};
use std::sync::OnceLock;

pub struct AuthService {
    jwt_manager: JwtManager,
    password_hasher: PasswordHasher,
    token_expiry_secs: u64,
}

impl AuthService {
    /// Build a new `AuthService`, wiring up the JWT manager.
    ///
    /// Returns an error (rather than panicking) if the JWT configuration is
    /// invalid, so callers can log the concrete cause and shut down
    /// cleanly instead of crashing with an opaque `expect()` message.
    pub fn new(secret: String, expiry_hours: u64) -> Result<Self, JwtError> {
        let jwt_config = JwtConfig::new(secret);
        let jwt_manager = JwtManager::new(jwt_config)?;

        Ok(Self {
            jwt_manager,
            password_hasher: PasswordHasher::default(),
            token_expiry_secs: expiry_hours.saturating_mul(3600),
        })
    }

    /// Hash a password with Argon2.
    pub fn hash_password(&self, password: &str) -> Result<String, String> {
        self.password_hasher
            .hash(password)
            .map_err(|e| e.to_string())
    }

    /// Verify a password against a previously-generated hash.
    pub fn verify_password(&self, password: &str, hash: &str) -> bool {
        // Any error here (malformed hash, hasher-library failure, etc.) is
        // treated as "wrong password" to the caller — but it's logged
        // server-side first so data corruption doesn't masquerade as a
        // routine failed login with zero visibility.
        self.password_hasher
            .verify(password, hash)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "password verification error");
                false
            })
    }

    /// Generate a signed JWT for a user.
    pub fn generate_token(&self, user: &User) -> Result<String, String> {
        let now = Utc::now();
        let exp = now + Duration::seconds(self.token_expiry_secs as i64);

        let claims = TokenClaims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            role: user.role,
            iat: now.timestamp() as usize,
            exp: exp.timestamp() as usize,
        };

        self.jwt_manager.sign(&claims).map_err(|e| e.to_string())
    }

    /// Verify a JWT (with or without a leading `Bearer ` prefix) and return
    /// its claims.
    pub fn verify_token(&self, token: &str) -> Result<TokenClaims, String> {
        let token = token.trim_start_matches("Bearer ").trim();
        let claims: TokenClaims = self.jwt_manager.verify(token).map_err(|e| e.to_string())?;
        Ok(claims)
    }

    /// Token expiry, in seconds.
    pub fn token_expiry_seconds(&self) -> u64 {
        self.token_expiry_secs
    }
}

// NOTE: this `OnceLock`-based singleton pattern (static + `init_x_service`/
// `get_x_service` pair) is hand-rolled and independently reinvented, with
// minor variation, across the `api-minimal` and `microservice` templates —
// a future pass should factor this into a shared helper instead of each
// template defining its own copy.
static AUTH_SERVICE: OnceLock<AuthService> = OnceLock::new();

/// Install the process-wide [`AuthService`].
///
/// Must be called exactly once from `main`, before the server starts
/// accepting requests. Returns the [`JwtError`] if the JWT configuration is
/// invalid, so the caller can log the concrete cause and exit cleanly
/// instead of crashing with an opaque panic.
pub fn init_auth_service(secret: String, expiry_hours: u64) -> Result<(), JwtError> {
    let service = AuthService::new(secret, expiry_hours)?;
    if AUTH_SERVICE.set(service).is_err() {
        panic!("AuthService already initialized");
    }
    Ok(())
}

/// Access the process-wide [`AuthService`].
pub fn get_auth_service() -> &'static AuthService {
    AUTH_SERVICE
        .get()
        .expect("AuthService not initialized — call init_auth_service() in main()")
}
