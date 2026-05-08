//! API Key Management
//!
//! Provides API key generation, validation, and rotation with database injection.
//!
//! # Features
//!
//! - API key generation with secure random
//! - Key validation and verification
//! - Key rotation and expiration
//! - Rate limiting per key
//! - Scopes/permissions per key
//! - Database-agnostic with DI
//!
//! # Usage
//!
//! ```no_run
//! use armature_auth::api_key::*;
//! use chrono::{DateTime, Utc};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Implement your database storage
//! struct MyApiKeyStore;
//!
//! #[async_trait::async_trait]
//! impl ApiKeyStore for MyApiKeyStore {
//!     async fn save(&self, key: &ApiKey) -> Result<(), ApiKeyError> { Ok(()) }
//!     async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, ApiKeyError> { Ok(None) }
//!     async fn find_by_id(&self, id: &str) -> Result<Option<ApiKey>, ApiKeyError> { Ok(None) }
//!     async fn list_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, ApiKeyError> { Ok(vec![]) }
//!     async fn revoke(&self, key_id: &str) -> Result<(), ApiKeyError> { Ok(()) }
//!     async fn update_last_used(&self, key_id: &str, timestamp: DateTime<Utc>) -> Result<(), ApiKeyError> { Ok(()) }
//! }
//!
//! // Inject store via DI container
//! let store: Arc<dyn ApiKeyStore> = Arc::new(MyApiKeyStore);
//! let manager = ApiKeyManager::new(store);
//!
//! // Generate API key
//! let api_key = manager.generate("user_123", vec!["read".to_string(), "write".to_string()]).await?;
//! println!("API Key: {}", api_key.key);
//!
//! // Validate API key
//! if let Some(key) = manager.validate(&api_key.key).await? {
//!     println!("Valid key for user: {}", key.user_id);
//! }
//! # Ok(())
//! # }
//! ```

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use thiserror::Error;

/// API Key errors
#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error("Invalid API key")]
    Invalid,

    #[error("API key expired")]
    Expired,

    #[error("API key revoked")]
    Revoked,

    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,
}

/// API Key structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    /// Unique key ID
    pub id: String,

    /// The actual API key (store hash, not plaintext!)
    pub key: String,

    /// User/account ID this key belongs to
    pub user_id: String,

    /// Key name/description
    pub name: Option<String>,

    /// Scopes/permissions
    pub scopes: Vec<String>,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Expiration timestamp
    pub expires_at: Option<DateTime<Utc>>,

    /// Last used timestamp
    pub last_used_at: Option<DateTime<Utc>>,

    /// Whether key is revoked
    pub revoked: bool,

    /// Rate limit (requests per minute)
    pub rate_limit: Option<u32>,
}

/// API Key storage trait (implement with your database)
///
/// Users must provide their own implementation using their database of choice.
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    /// Save or update an API key
    async fn save(&self, key: &ApiKey) -> Result<(), ApiKeyError>;

    /// Find API key by the key string (should query by hash)
    async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, ApiKeyError>;

    /// Find API key by ID
    async fn find_by_id(&self, id: &str) -> Result<Option<ApiKey>, ApiKeyError>;

    /// List all keys for a user
    async fn list_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, ApiKeyError>;

    /// Revoke an API key
    async fn revoke(&self, key_id: &str) -> Result<(), ApiKeyError>;

    /// Update last used timestamp
    async fn update_last_used(
        &self,
        key_id: &str,
        timestamp: DateTime<Utc>,
    ) -> Result<(), ApiKeyError>;
}

/// API Key Manager
///
/// Manages API key lifecycle with injected storage.
pub struct ApiKeyManager {
    store: Arc<dyn ApiKeyStore>,
    key_prefix: String,
    default_expiration: Option<Duration>,
}

impl ApiKeyManager {
    /// Create new API key manager with injected store
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_auth::api_key::*;
    /// use chrono::{DateTime, Utc};
    /// use std::sync::Arc;
    ///
    /// # struct MyStore;
    /// # #[async_trait::async_trait]
    /// # impl ApiKeyStore for MyStore {
    /// #     async fn save(&self, key: &ApiKey) -> Result<(), ApiKeyError> { Ok(()) }
    /// #     async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, ApiKeyError> { Ok(None) }
    /// #     async fn find_by_id(&self, id: &str) -> Result<Option<ApiKey>, ApiKeyError> { Ok(None) }
    /// #     async fn list_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, ApiKeyError> { Ok(vec![]) }
    /// #     async fn revoke(&self, key_id: &str) -> Result<(), ApiKeyError> { Ok(()) }
    /// #     async fn update_last_used(&self, key_id: &str, timestamp: DateTime<Utc>) -> Result<(), ApiKeyError> { Ok(()) }
    /// # }
    /// let store: Arc<dyn ApiKeyStore> = Arc::new(MyStore);
    /// let manager = ApiKeyManager::new(store);
    /// ```
    pub fn new(store: Arc<dyn ApiKeyStore>) -> Self {
        Self {
            store,
            key_prefix: "ak".to_string(),
            default_expiration: Some(Duration::days(365)),
        }
    }

    /// Set key prefix (default: "ak")
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Set default expiration duration
    pub fn with_expiration(mut self, duration: Option<Duration>) -> Self {
        self.default_expiration = duration;
        self
    }

    /// Generate a new API key
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_auth::api_key::*;
    /// # use std::sync::Arc;
    /// # async fn example(manager: ApiKeyManager) -> Result<(), ApiKeyError> {
    /// let key = manager.generate(
    ///     "user_123",
    ///     vec!["read".to_string(), "write".to_string()]
    /// ).await?;
    ///
    /// println!("API Key: {}", key.key);
    /// println!("Key ID: {}", key.id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn generate(
        &self,
        user_id: impl Into<String>,
        scopes: Vec<String>,
    ) -> Result<ApiKey, ApiKeyError> {
        let key_id = uuid::Uuid::new_v4().to_string();
        let raw_key = self.generate_random_key();
        let key_string = format!("{}_{}", self.key_prefix, raw_key);

        let expires_at = self.default_expiration.map(|d| Utc::now() + d);

        let api_key = ApiKey {
            id: key_id,
            key: key_string.clone(),
            user_id: user_id.into(),
            name: None,
            scopes,
            created_at: Utc::now(),
            expires_at,
            last_used_at: None,
            revoked: false,
            rate_limit: None,
        };

        // Store key (implementation should hash the key!)
        self.store.save(&api_key).await?;

        Ok(api_key)
    }

    /// Validate an API key
    ///
    /// Returns the API key if valid, None if invalid, or an error.
    pub async fn validate(&self, key: &str) -> Result<Option<ApiKey>, ApiKeyError> {
        let api_key = match self.store.find_by_key(key).await? {
            Some(k) => k,
            None => return Ok(None),
        };

        // Check if revoked
        if api_key.revoked {
            return Err(ApiKeyError::Revoked);
        }

        // Check if expired
        if let Some(expires_at) = api_key.expires_at
            && Utc::now() > expires_at
        {
            return Err(ApiKeyError::Expired);
        }

        // Update last used timestamp
        self.store.update_last_used(&api_key.id, Utc::now()).await?;

        Ok(Some(api_key))
    }

    /// Check if key has required scope
    pub fn has_scope(&self, api_key: &ApiKey, required_scope: &str) -> bool {
        api_key
            .scopes
            .iter()
            .any(|s| s == required_scope || s == "*")
    }

    /// Revoke an API key
    pub async fn revoke(&self, key_id: &str) -> Result<(), ApiKeyError> {
        self.store.revoke(key_id).await
    }

    /// List all keys for a user
    pub async fn list_user_keys(&self, user_id: &str) -> Result<Vec<ApiKey>, ApiKeyError> {
        self.store.list_by_user(user_id).await
    }

    /// Rotate an API key (revoke old, generate new)
    pub async fn rotate(&self, old_key_id: &str) -> Result<ApiKey, ApiKeyError> {
        // Get old key
        let old_key = self
            .store
            .find_by_id(old_key_id)
            .await?
            .ok_or(ApiKeyError::Invalid)?;

        // Revoke old key
        self.revoke(old_key_id).await?;

        // Generate new key with same scopes
        self.generate(&old_key.user_id, old_key.scopes).await
    }

    /// Generate random key string
    fn generate_random_key(&self) -> String {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Hash an API key (for storage)
    pub fn hash_key(key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct InMemoryStore {
        keys: std::sync::Mutex<Vec<ApiKey>>,
    }

    impl InMemoryStore {
        fn new() -> Self {
            Self {
                keys: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ApiKeyStore for InMemoryStore {
        async fn save(&self, key: &ApiKey) -> Result<(), ApiKeyError> {
            let mut keys = self.keys.lock().unwrap();
            keys.push(key.clone());
            Ok(())
        }

        async fn find_by_key(&self, key: &str) -> Result<Option<ApiKey>, ApiKeyError> {
            let keys = self.keys.lock().unwrap();
            Ok(keys.iter().find(|k| k.key == key).cloned())
        }

        async fn find_by_id(&self, id: &str) -> Result<Option<ApiKey>, ApiKeyError> {
            let keys = self.keys.lock().unwrap();
            Ok(keys.iter().find(|k| k.id == id).cloned())
        }

        async fn list_by_user(&self, user_id: &str) -> Result<Vec<ApiKey>, ApiKeyError> {
            let keys = self.keys.lock().unwrap();
            Ok(keys
                .iter()
                .filter(|k| k.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn revoke(&self, key_id: &str) -> Result<(), ApiKeyError> {
            let mut keys = self.keys.lock().unwrap();
            if let Some(key) = keys.iter_mut().find(|k| k.id == key_id) {
                key.revoked = true;
            }
            Ok(())
        }

        async fn update_last_used(
            &self,
            key_id: &str,
            timestamp: DateTime<Utc>,
        ) -> Result<(), ApiKeyError> {
            let mut keys = self.keys.lock().unwrap();
            if let Some(key) = keys.iter_mut().find(|k| k.id == key_id) {
                key.last_used_at = Some(timestamp);
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_generate_api_key() {
        let store = Arc::new(InMemoryStore::new());
        let manager = ApiKeyManager::new(store);

        let key = manager
            .generate("user_123", vec!["read".to_string()])
            .await
            .unwrap();

        assert!(key.key.starts_with("ak_"));
        assert_eq!(key.user_id, "user_123");
        assert_eq!(key.scopes, vec!["read"]);
    }

    #[tokio::test]
    async fn test_validate_api_key() {
        let store = Arc::new(InMemoryStore::new());
        let manager = ApiKeyManager::new(store);

        let key = manager
            .generate("user_123", vec!["read".to_string()])
            .await
            .unwrap();
        let validated = manager.validate(&key.key).await.unwrap();

        assert!(validated.is_some());
        assert_eq!(validated.unwrap().user_id, "user_123");
    }

    #[tokio::test]
    async fn test_revoke_api_key() {
        let store = Arc::new(InMemoryStore::new());
        let manager = ApiKeyManager::new(store);

        let key = manager
            .generate("user_123", vec!["read".to_string()])
            .await
            .unwrap();
        manager.revoke(&key.id).await.unwrap();

        let result = manager.validate(&key.key).await;
        assert!(matches!(result, Err(ApiKeyError::Revoked)));
    }

    #[tokio::test]
    async fn test_has_scope() {
        let store = Arc::new(InMemoryStore::new());
        let manager = ApiKeyManager::new(store);

        let key = manager
            .generate("user_123", vec!["read".to_string(), "write".to_string()])
            .await
            .unwrap();

        assert!(manager.has_scope(&key, "read"));
        assert!(manager.has_scope(&key, "write"));
        assert!(!manager.has_scope(&key, "admin"));
    }
}
