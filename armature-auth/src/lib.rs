//! Authentication and authorization for Armature.
//!
//! This crate provides comprehensive authentication and authorization
//! capabilities including JWT, OAuth2, SAML, password hashing, and guards.
//!
//! ## Features
//!
//! - 🔐 **JWT Authentication** - Token-based auth with `armature-jwt`
//! - 🌐 **OAuth2** - Google, Auth0, Microsoft, AWS Cognito, Okta
//! - 🔑 **SAML** - Enterprise SAML 2.0 authentication (requires `saml` feature)
//! - 🔒 **Password Hashing** - Argon2 (default) and bcrypt
//! - 🛡️ **Guards** - Route protection with auth and role guards
//! - 👤 **User Context** - Request-scoped user information
//!
//! ## Cargo Features
//!
//! - `default` - Core authentication (JWT, OAuth2, password hashing)
//! - `saml` - SAML 2.0 support (requires openssl/xmlsec1 system libraries)
//!
//! ### SAML System Requirements
//!
//! The `saml` feature requires system libraries for XML signature verification:
//! - **Ubuntu/Debian**: `apt-get install libxml2-dev libxmlsec1-dev libxmlsec1-openssl`
//! - **macOS**: `brew install libxmlsec1`
//! - **Windows**: Not recommended (complex setup)
//!
//! ## Quick Start - Password Authentication
//!
//! ```
//! use armature_auth::{AuthService, PasswordHasher, PasswordVerifier};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let auth_service = AuthService::new();
//!
//! // Hash a password
//! let hash = auth_service.hash_password("secret123")?;
//!
//! // Verify password
//! let is_valid = auth_service.verify_password("secret123", &hash)?;
//! assert!(is_valid);
//! # Ok(())
//! # }
//! ```
//!
//! ## JWT Authentication
//!
//! ```no_run
//! use armature_auth::AuthService;
//! use armature_jwt::{JwtConfig, JwtManager};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create JWT manager
//! let jwt_config = JwtConfig::new("your-secret-key".to_string());
//! let jwt_manager = JwtManager::new(jwt_config)?;
//!
//! // Create auth service with JWT
//! let auth_service = AuthService::with_jwt(jwt_manager);
//!
//! // JWT manager is now available
//! assert!(auth_service.jwt_manager().is_some());
//! # Ok(())
//! # }
//! ```
//!
//! ## OAuth2 - Google Example
//!
//! ```no_run
//! use armature_auth::{GoogleProvider, providers::GoogleConfig, OAuth2Provider};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GoogleConfig::new(
//!     "client-id".to_string(),
//!     "client-secret".to_string(),
//!     "https://myapp.com/callback".to_string(),
//! );
//!
//! let provider = GoogleProvider::new(config)?;
//!
//! // Generate authorization URL
//! let (auth_url, state) = provider.authorization_url()?;
//! println!("Redirect user to: {}", auth_url);
//! println!("State: {}", state.secret());
//!
//! // After callback, exchange code for token
//! let token = provider.exchange_code("auth-code".to_string()).await?;
//! println!("Access token: {}", token.access_token);
//! # Ok(())
//! # }
//! ```
//!
//! ## Route Guards
//!
//! ```ignore
//! use armature_auth::{AuthGuard, RoleGuard};
//! use armature_core::{Controller, Get};
//!
//! #[controller("/admin")]
//! struct AdminController;
//!
//! impl AdminController {
//!     // Require authentication
//!     #[get("/dashboard")]
//!     #[guard(AuthGuard)]
//!     async fn dashboard(&self) -> Result<HttpResponse, Error> {
//!         Ok(HttpResponse::ok())
//!     }
//!
//!     // Require specific role
//!     #[get("/users")]
//!     #[guard(RoleGuard::new("admin"))]
//!     async fn users(&self) -> Result<HttpResponse, Error> {
//!         Ok(HttpResponse::ok())
//!     }
//! }
//! ```
//!
//! ## SAML Authentication
//!
//! ```ignore
//! use armature_auth::{SamlServiceProvider, SamlConfig, IdpMetadata};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let config = SamlConfig {
//!     entity_id: "https://myapp.com".to_string(),
//!     acs_url: "https://myapp.com/callback".to_string(),
//!     sls_url: None,
//!     // IdP metadata must be provided as XML; URL-based fetching is not yet supported.
//!     idp_metadata: IdpMetadata::Xml("<EntityDescriptor>...</EntityDescriptor>".to_string()),
//!     sp_certificate: None,
//!     sp_private_key: None,
//!     contact_person: None,
//!     allow_unsigned_assertions: false,
//!     required_attributes: vec![],
//! };
//!
//! let provider = SamlServiceProvider::new("my-sp".to_string(), config)?;
//!
//! // Generate SAML auth request
//! let auth_request = provider.create_auth_request()?;
//! println!("SAML Request: {}", auth_request.saml_request);
//! # Ok(())
//! # }
//! ```

pub mod api_key;
pub mod error;
pub mod guard;
pub mod middleware;
pub mod oauth2;
pub mod password;
pub mod passwordless;
pub mod providers;
#[cfg(feature = "saml")]
pub mod saml;
pub mod strategy;
#[cfg(feature = "two-factor")]
pub mod two_factor;
pub mod user;

pub use api_key::{ApiKey, ApiKeyError, ApiKeyManager, ApiKeyStore};
pub use error::{AuthError, Result};
pub use guard::{AuthGuard, Guard, PermissionGuard, RoleGuard};
pub use middleware::JwtAuthMiddleware;
pub use oauth2::{OAuth2Provider, OAuth2Token, OAuth2UserInfo};
pub use password::{PasswordHasher, PasswordVerifier};
pub use passwordless::{MagicLinkToken, PasswordlessError, WebAuthnManager};
#[cfg(feature = "saml")]
pub use saml::{
    ContactInfo, IdpMetadata, SamlAssertion, SamlAuthRequest, SamlConfig, SamlProvider,
    SamlServiceProvider,
};
pub use strategy::{AuthStrategy, JwtStrategy, LocalStrategy};
#[cfg(feature = "two-factor")]
pub use two_factor::{BackupCodes, TotpSecret, TwoFactorError};
pub use user::{AuthUser, UserContext};

// Re-export providers
pub use providers::{
    Auth0Provider, AwsCognitoProvider, DiscordProvider, DiscordUser, GitHubProvider, GitHubUser,
    GitLabProvider, GitLabUser, GoogleProvider, LinkedInProvider, LinkedInUser,
    MicrosoftEntraProvider, OktaProvider,
};

use armature_jwt::JwtManager;
use armature_log::{debug, trace};
use std::sync::Arc;

/// Authentication service for managing user authentication.
///
/// Provides password hashing, verification, and optional JWT token management.
///
/// # Examples
///
/// Basic password authentication:
///
/// ```
/// use armature_auth::{AuthService, PasswordVerifier};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let service = AuthService::new();
///
/// // Hash password
/// let hash = service.hash_password("mypassword")?;
///
/// // Verify password
/// assert!(service.verify_password("mypassword", &hash)?);
/// assert!(!service.verify_password("wrongpassword", &hash)?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AuthService {
    jwt_manager: Option<Arc<JwtManager>>,
    password_hasher: PasswordHasher,
}

impl AuthService {
    /// Create a new authentication service
    pub fn new() -> Self {
        debug!("Creating new AuthService");
        Self {
            jwt_manager: None,
            password_hasher: PasswordHasher::default(),
        }
    }

    /// Create with JWT manager
    pub fn with_jwt(jwt_manager: JwtManager) -> Self {
        debug!("Creating AuthService with JWT manager");
        Self {
            jwt_manager: Some(Arc::new(jwt_manager)),
            password_hasher: PasswordHasher::default(),
        }
    }

    /// Set password hasher
    pub fn with_password_hasher(mut self, hasher: PasswordHasher) -> Self {
        debug!("Setting password hasher");
        self.password_hasher = hasher;
        self
    }

    /// Hash a password
    pub fn hash_password(&self, password: &str) -> Result<String> {
        trace!("Hashing password");
        self.password_hasher.hash(password)
    }

    /// Verify a password
    pub fn verify_password(&self, password: &str, hash: &str) -> Result<bool> {
        trace!("Verifying password");
        self.password_hasher.verify(password, hash)
    }

    /// Get JWT manager
    pub fn jwt_manager(&self) -> Option<&JwtManager> {
        self.jwt_manager.as_deref()
    }

    /// Validate authentication
    pub fn validate<T: AuthUser>(&self, user: &T) -> Result<()> {
        debug!("Validating user authentication");
        if !user.is_active() {
            debug!("User is inactive");
            return Err(AuthError::InactiveUser);
        }

        trace!("User validation successful");
        Ok(())
    }
}

impl Default for AuthService {
    fn default() -> Self {
        Self::new()
    }
}

/// Prelude for common imports.
///
/// ```
/// use armature_auth::prelude::*;
/// ```
pub mod prelude {
    pub use crate::AuthService;
    pub use crate::api_key::{ApiKey, ApiKeyManager, ApiKeyStore};
    pub use crate::error::{AuthError, Result};
    pub use crate::guard::{AuthGuard, Guard, PermissionGuard, RoleGuard};
    pub use crate::middleware::JwtAuthMiddleware;
    pub use crate::oauth2::{OAuth2Provider, OAuth2Token, OAuth2UserInfo};
    pub use crate::password::{PasswordHasher, PasswordVerifier};
    pub use crate::strategy::{AuthStrategy, JwtStrategy, LocalStrategy};
    pub use crate::user::{AuthUser, UserContext};

    // OAuth2 providers
    pub use crate::providers::{
        Auth0Provider, AwsCognitoProvider, DiscordProvider, GitHubProvider, GitLabProvider,
        GoogleProvider, LinkedInProvider, MicrosoftEntraProvider, OktaProvider,
    };

    #[cfg(feature = "saml")]
    pub use crate::saml::{SamlConfig, SamlProvider, SamlServiceProvider};

    #[cfg(feature = "two-factor")]
    pub use crate::two_factor::{BackupCodes, TotpSecret, TwoFactorError};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hashing() {
        let service = AuthService::new();

        let password = "test-password-123";
        let hash = service.hash_password(password).unwrap();

        assert!(service.verify_password(password, &hash).unwrap());
        assert!(!service.verify_password("wrong-password", &hash).unwrap());
    }
}
