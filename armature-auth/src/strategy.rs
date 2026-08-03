// Authentication strategies

use crate::password::PasswordHasher;
use crate::user::UserContext;
use crate::{AuthError, AuthUser, PasswordVerifier, Result};
use armature_jwt::JwtManager;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Authentication strategy trait
#[async_trait]
pub trait AuthStrategy<T: AuthUser>: Send + Sync {
    /// Authenticate and return user
    async fn authenticate(&self, credentials: &(dyn std::any::Any + Send + Sync)) -> Result<T>;
}

/// Storage lookup for [`LocalStrategy`]: given a username, returns the user
/// record plus its stored password hash (never the plaintext password).
#[async_trait]
pub trait LocalUserStore<T: AuthUser>: Send + Sync {
    /// Find a user and their password hash by username.
    async fn find_by_username(&self, username: &str) -> Result<Option<(T, String)>>;
}

/// Local authentication strategy (username/password).
///
/// Verifies credentials against a user store's password hash using the
/// hashing already provided by this crate ([`PasswordHasher`]/
/// [`PasswordVerifier`]). Without a store attached (via
/// [`LocalStrategy::with_store`]), authentication always fails —
/// [`LocalStrategy::new`] alone has nothing to check credentials against.
pub struct LocalStrategy<T: AuthUser> {
    store: Option<Arc<dyn LocalUserStore<T>>>,
    hasher: PasswordHasher,
    /// Hash of a fixed, unusable password, verified against whenever no real
    /// hash is available. Built on first use because it costs a full KDF run
    /// and the hasher can still be swapped after construction.
    dummy_hash: std::sync::OnceLock<String>,
}

/// Plaintext behind [`LocalStrategy::dummy_hash`]. Its value is irrelevant —
/// nothing ever compares equal to it on purpose — but it must be a constant so
/// the hash is computed once and reused.
const DUMMY_PASSWORD: &str = "armature-local-strategy-dummy-password";

impl<T: AuthUser> LocalStrategy<T> {
    pub fn new() -> Self {
        Self {
            store: None,
            hasher: PasswordHasher::default(),
            dummy_hash: std::sync::OnceLock::new(),
        }
    }

    /// Attach the user store to verify credentials against.
    pub fn with_store(store: Arc<dyn LocalUserStore<T>>) -> Self {
        Self {
            store: Some(store),
            hasher: PasswordHasher::default(),
            dummy_hash: std::sync::OnceLock::new(),
        }
    }

    /// Override the password hasher/verifier (default: [`PasswordHasher::default`]).
    pub fn with_password_hasher(mut self, hasher: PasswordHasher) -> Self {
        self.hasher = hasher;
        self.dummy_hash = std::sync::OnceLock::new();
        self
    }

    /// Spend the same KDF work an existing user's login would, then discard the
    /// result.
    ///
    /// Returning as soon as the username misses makes the "no such user" path
    /// orders of magnitude faster than the "wrong password" path — Argon2 and
    /// bcrypt are deliberately slow — which turns the login endpoint into an
    /// account-enumeration oracle measurable over the network. Verifying
    /// against a fixed dummy hash makes both paths cost the same.
    fn equalize_timing(&self, password: &str) {
        let dummy = self
            .dummy_hash
            .get_or_init(|| self.hasher.hash(DUMMY_PASSWORD).unwrap_or_default());

        if !dummy.is_empty() {
            let _ = self.hasher.verify(password, dummy);
        }
    }
}

impl<T: AuthUser> Default for LocalStrategy<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<T: AuthUser + 'static> AuthStrategy<T> for LocalStrategy<T> {
    async fn authenticate(&self, credentials: &(dyn std::any::Any + Send + Sync)) -> Result<T> {
        let creds = credentials
            .downcast_ref::<LocalCredentials>()
            .ok_or_else(|| {
                AuthError::AuthenticationFailed(
                    "expected LocalCredentials for LocalStrategy".to_string(),
                )
            })?;

        let Some(store) = self.store.as_ref() else {
            self.equalize_timing(&creds.password);
            return Err(AuthError::InvalidCredentials);
        };

        let Some((user, password_hash)) = store.find_by_username(&creds.username).await? else {
            // An unknown username must cost what a known one costs; see
            // `equalize_timing`.
            self.equalize_timing(&creds.password);
            return Err(AuthError::InvalidCredentials);
        };

        if self.hasher.verify(&creds.password, &password_hash)? {
            Ok(user)
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }
}

/// Local credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCredentials {
    pub username: String,
    pub password: String,
}

/// JWT authentication strategy: verifies a bearer token via
/// [`armature_jwt::JwtManager`] and builds a [`UserContext`] from its claims,
/// the same mapping [`crate::JwtAuthMiddleware`] uses (`sub`, `roles`,
/// `permissions`).
pub struct JwtStrategy<T: AuthUser> {
    jwt_manager: JwtManager,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: AuthUser> JwtStrategy<T> {
    pub fn new(jwt_manager: JwtManager) -> Self {
        Self {
            jwt_manager,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Extract token from Authorization header
    pub fn extract_token<'a>(&self, header: &'a str) -> Result<&'a str> {
        header
            .strip_prefix("Bearer ")
            .ok_or_else(|| AuthError::InvalidToken("Invalid Bearer token format".to_string()))
    }
}

#[async_trait]
impl AuthStrategy<UserContext> for JwtStrategy<UserContext> {
    async fn authenticate(
        &self,
        credentials: &(dyn std::any::Any + Send + Sync),
    ) -> Result<UserContext> {
        let token = credentials
            .downcast_ref::<JwtCredentials>()
            .map(|c| c.token.as_str())
            .or_else(|| credentials.downcast_ref::<String>().map(String::as_str))
            .ok_or_else(|| {
                AuthError::AuthenticationFailed(
                    "expected JwtCredentials for JwtStrategy".to_string(),
                )
            })?;

        let claims: serde_json::Value = self.jwt_manager.verify(token)?;

        let subject = claims
            .get("sub")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let roles = string_list(claims.get("roles"));
        let permissions = string_list(claims.get("permissions"));

        Ok(UserContext::new(subject)
            .with_roles(roles)
            .with_permissions(permissions)
            .with_metadata(claims))
    }
}

/// Coerce a JSON claim value into a list of strings (array of strings, or a
/// single string); anything else (including absence) yields an empty list.
fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// JWT credentials
#[derive(Debug, Clone)]
pub struct JwtCredentials {
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_credentials() {
        let creds = LocalCredentials {
            username: "user@example.com".to_string(),
            password: "password123".to_string(),
        };

        assert_eq!(creds.username, "user@example.com");
        assert_eq!(creds.password, "password123");
    }

    #[test]
    fn test_jwt_token_extraction() {
        use crate::UserContext;
        use armature_jwt::JwtConfig;

        let config = JwtConfig::new("test-secret".to_string());
        let jwt_manager = JwtManager::new(config).unwrap();
        let strategy = JwtStrategy::<UserContext>::new(jwt_manager);

        let valid_header = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...";
        let token = strategy.extract_token(valid_header);
        assert!(token.is_ok());

        let invalid_header = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...";
        let token = strategy.extract_token(invalid_header);
        assert!(token.is_err());
    }

    struct InMemoryUserStore {
        username: String,
        password_hash: String,
    }

    #[async_trait]
    impl LocalUserStore<UserContext> for InMemoryUserStore {
        async fn find_by_username(&self, username: &str) -> Result<Option<(UserContext, String)>> {
            if username == self.username {
                Ok(Some((
                    UserContext::new(username.to_string()),
                    self.password_hash.clone(),
                )))
            } else {
                Ok(None)
            }
        }
    }

    #[tokio::test]
    async fn local_strategy_authenticates_valid_credentials() {
        let hasher = PasswordHasher::default();
        let hash = hasher.hash("correct-password").unwrap();
        let store = Arc::new(InMemoryUserStore {
            username: "alice".to_string(),
            password_hash: hash,
        });
        let strategy = LocalStrategy::<UserContext>::with_store(store);

        let creds = LocalCredentials {
            username: "alice".to_string(),
            password: "correct-password".to_string(),
        };
        let user = strategy.authenticate(&creds).await.unwrap();
        assert_eq!(user.user_id, "alice");
    }

    #[tokio::test]
    async fn local_strategy_rejects_invalid_password() {
        let hasher = PasswordHasher::default();
        let hash = hasher.hash("correct-password").unwrap();
        let store = Arc::new(InMemoryUserStore {
            username: "alice".to_string(),
            password_hash: hash,
        });
        let strategy = LocalStrategy::<UserContext>::with_store(store);

        let creds = LocalCredentials {
            username: "alice".to_string(),
            password: "wrong-password".to_string(),
        };
        let result = strategy.authenticate(&creds).await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn local_strategy_without_store_always_fails() {
        let strategy = LocalStrategy::<UserContext>::new();
        let creds = LocalCredentials {
            username: "alice".to_string(),
            password: "anything".to_string(),
        };
        let result = strategy.authenticate(&creds).await;
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn jwt_strategy_authenticates_valid_token() {
        use armature_jwt::JwtConfig;

        let manager = JwtManager::new(JwtConfig::new("test-secret".to_string())).unwrap();
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = manager
            .sign(&serde_json::json!({ "sub": "user123", "roles": ["admin"], "exp": exp }))
            .unwrap();

        let strategy = JwtStrategy::<UserContext>::new(manager);
        let creds = JwtCredentials { token };
        let user = strategy.authenticate(&creds).await.unwrap();

        assert_eq!(user.user_id, "user123");
        assert!(user.has_role("admin"));
    }

    #[tokio::test]
    async fn jwt_strategy_rejects_invalid_token() {
        use armature_jwt::JwtConfig;

        let manager = JwtManager::new(JwtConfig::new("test-secret".to_string())).unwrap();
        let strategy = JwtStrategy::<UserContext>::new(manager);
        let creds = JwtCredentials {
            token: "not.a.valid.token".to_string(),
        };

        let result = strategy.authenticate(&creds).await;
        assert!(result.is_err());
    }
}
