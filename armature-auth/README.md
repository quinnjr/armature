# armature-auth

Authentication and authorization for the Armature framework.

## Features

- **Password Hashing** - bcrypt and Argon2 support
- **OAuth2/OIDC** - Google, GitHub, Microsoft, custom providers
- **JWT Integration** - Works with `armature-jwt`
- **Role-Based Access** - Guards and middleware for authorization
- **WebAuthn/FIDO2** - Passwordless authentication (optional)
- **SAML 2.0** - Enterprise SSO support (optional)

## Installation

```toml
[dependencies]
armature-auth = "0.1"
```

## Quick Start

```rust
use armature_auth::{PasswordHasher, PasswordVerifier};
use armature_auth::password::HashAlgorithm;

// Hash a password (Argon2 is the default; `PasswordHasher::default()` also uses Argon2)
let hasher = PasswordHasher::new(HashAlgorithm::Argon2);
let hash = hasher.hash("my_password")?;

// Verify a password
assert!(hasher.verify("my_password", &hash)?);

// OAuth2 flow
use armature_auth::oauth2::{GenericOAuth2Provider, OAuth2Config, OAuth2Provider};

// Configure the client from the provider's endpoints (client id/secret, auth URL,
// token URL, redirect URL), then add scopes and the userinfo endpoint.
let config = OAuth2Config::new(
    "your-client-id".to_string(),
    "your-secret".to_string(),
    "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
    "https://oauth2.googleapis.com/token".to_string(),
    "http://localhost:3000/callback".to_string(),
)
.with_scopes(vec!["openid".to_string(), "email".to_string()])
.with_user_info_url("https://www.googleapis.com/oauth2/v2/userinfo".to_string());

// `authorization_url()` is called on the provider (e.g. GoogleProvider), not the config.
let provider = GenericOAuth2Provider::new("google".to_string(), config)?;
let (auth_url, csrf_token) = provider.authorization_url()?;
```

## Features Flags

- `oauth2` - OAuth2/OIDC support (default)
- `webauthn` - WebAuthn/FIDO2 passwordless auth
- `saml` - SAML 2.0 enterprise SSO

## License

MIT OR Apache-2.0

