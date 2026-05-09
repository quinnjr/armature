# OAuth2 Providers Guide

Complete guide to using OAuth2/OIDC providers with Armature authentication.

## Table of Contents

1. [Overview](#overview)
2. [Supported Providers](#supported-providers)
3. [Google OAuth2](#google-oauth2)
4. [Microsoft Entra](#microsoft-entra-azure-ad)
5. [AWS Cognito](#aws-cognito)
6. [Okta](#okta)
7. [Auth0](#auth0)
8. [OAuth2 Flow](#oauth2-flow)
9. [Complete Example](#complete-example)
10. [Best Practices](#best-practices)

## Overview

`armature-auth` provides built-in support for major enterprise OAuth2 and OIDC identity providers. This allows you to integrate with existing authentication systems without building your own user management.

### Benefits

- **Single Sign-On (SSO)**: Users authenticate with their existing accounts
- **Enterprise Ready**: Integrates with corporate identity providers
- **Security**: Leverages battle-tested OAuth2/OIDC protocols
- **No User Management**: Delegate user storage to providers
- **Multi-Provider**: Support multiple providers simultaneously

## Supported Providers

| Provider | Type | Use Case |
|----------|------|----------|
| **Google** | Consumer | Gmail users, Google Workspace |
| **Microsoft Entra** | Enterprise | Azure AD, Office 365, Enterprise SSO |
| **AWS Cognito** | Cloud | AWS-native applications |
| **Okta** | Enterprise | Enterprise SSO, SAML bridge |
| **Auth0** | Universal | Multi-provider, custom branding |

## Google OAuth2

### Setup

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select existing
3. Navigate to "APIs & Services" > "Credentials"
4. Create OAuth 2.0 Client ID
5. Add authorized redirect URI: `http://localhost:3000/auth/google/callback`

### Configuration

```rust
use armature_auth::providers::{GoogleConfig, GoogleProvider};

let config = GoogleConfig::new(
    "your-client-id.apps.googleusercontent.com".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/google/callback".to_string(),
);

// Optional: Custom scopes
let config = config.with_scopes(vec![
    "openid".to_string(),
    "email".to_string(),
    "profile".to_string(),
    "https://www.googleapis.com/auth/userinfo.profile".to_string(),
]);

let provider = GoogleProvider::new(config)?;
```

### User Info

Google returns:
- `sub`: Google user ID
- `email`: User's Gmail address
- `name`: Full name
- `picture`: Profile picture URL
- `email_verified`: Email verification status

## Microsoft Entra (Azure AD)

### Setup

1. Go to [Azure Portal](https://portal.azure.com/)
2. Navigate to "Azure Active Directory"
3. Select "App registrations" > "New registration"
4. Add redirect URI: `http://localhost:3000/auth/microsoft/callback`
5. Create a client secret in "Certificates & secrets"

### Configuration

```rust
use armature_auth::providers::{MicrosoftEntraConfig, MicrosoftEntraProvider};

// Option 1: Common tenant (any Azure AD account)
let config = MicrosoftEntraConfig::common(
    "your-application-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/microsoft/callback".to_string(),
);

// Option 2: Organizations only (work/school accounts)
let config = MicrosoftEntraConfig::organizations(
    "your-application-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/microsoft/callback".to_string(),
);

// Option 3: Consumers only (personal Microsoft accounts)
let config = MicrosoftEntraConfig::consumers(
    "your-application-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/microsoft/callback".to_string(),
);

// Option 4: Specific tenant
let config = MicrosoftEntraConfig::new(
    "your-application-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/microsoft/callback".to_string(),
    "your-tenant-id".to_string(),
);

let provider = MicrosoftEntraProvider::new(config)?;
```

### Tenant Types

- **common**: Any Azure AD account (personal or work/school)
- **organizations**: Work or school accounts only
- **consumers**: Personal Microsoft accounts only
- **{tenant-id}**: Specific Azure AD tenant

### User Info

Microsoft Graph returns:
- `id`: Azure AD user ID (mapped to `sub`)
- `userPrincipalName`: User's UPN
- `displayName`: Full name
- `mail`: Email address
- `jobTitle`: Job title

## AWS Cognito

### Setup

1. Go to [AWS Console](https://console.aws.amazon.com/cognito/)
2. Create a User Pool
3. Configure "App integration" > "App client"
4. Enable "Hosted UI"
5. Add callback URL: `http://localhost:3000/auth/cognito/callback`
6. Note your domain: `your-app.auth.{region}.amazoncognito.com`

### Configuration

```rust
use armature_auth::providers::{AwsCognitoConfig, AwsCognitoProvider};

let config = AwsCognitoConfig::new(
    "your-client-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/cognito/callback".to_string(),
    "your-app.auth.us-east-1.amazoncognito.com".to_string(),
    "us-east-1".to_string(),
);

// Optional: Custom scopes
let config = config.with_scopes(vec![
    "openid".to_string(),
    "email".to_string(),
    "profile".to_string(),
    "aws.cognito.signin.user.admin".to_string(),
]);

let provider = AwsCognitoProvider::new(config)?;
```

### User Info

Cognito returns:
- `sub`: Cognito user UUID
- `email`: User's email
- `email_verified`: Email verification status
- Custom attributes you've configured

## Okta

### Setup

1. Go to [Okta Admin Console](https://your-domain.okta.com/admin)
2. Navigate to "Applications" > "Create App Integration"
3. Select "OIDC - OpenID Connect"
4. Choose "Web Application"
5. Add redirect URI: `http://localhost:3000/auth/okta/callback`
6. Note your Okta domain: `dev-12345.okta.com`

### Configuration

```rust
use armature_auth::providers::{OktaConfig, OktaProvider};

let config = OktaConfig::new(
    "your-client-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/okta/callback".to_string(),
    "dev-12345.okta.com".to_string(),
);

// Optional: Additional scopes
let config = config.with_scopes(vec![
    "openid".to_string(),
    "email".to_string(),
    "profile".to_string(),
    "groups".to_string(), // Include group membership
]);

let provider = OktaProvider::new(config)?;
```

### User Info

Okta returns:
- `sub`: Okta user ID
- `email`: User's email
- `name`: Full name
- `preferred_username`: Username
- `groups`: Group membership (if requested)

## Auth0

### Setup

1. Go to [Auth0 Dashboard](https://manage.auth0.com/)
2. Navigate to "Applications" > "Create Application"
3. Select "Regular Web Applications"
4. Add callback URL: `http://localhost:3000/auth/auth0/callback`
5. Note your domain: `your-tenant.us.auth0.com`

### Configuration

```rust
use armature_auth::providers::{Auth0Config, Auth0Provider};

let config = Auth0Config::new(
    "your-client-id".to_string(),
    "your-client-secret".to_string(),
    "http://localhost:3000/auth/auth0/callback".to_string(),
    "your-tenant.us.auth0.com".to_string(),
);

// Optional: API audience for access tokens
let config = config.with_audience("https://api.example.com".to_string());

// Optional: Custom scopes
let config = config.with_scopes(vec![
    "openid".to_string(),
    "email".to_string(),
    "profile".to_string(),
    "offline_access".to_string(), // For refresh tokens
]);

let provider = Auth0Provider::new(config)?;
```

### User Info

Auth0 returns:
- `sub`: Auth0 user ID (format: `provider|id`)
- `email`: User's email
- `name`: Full name
- `picture`: Profile picture
- `email_verified`: Email verification
- Custom user metadata

## OAuth2 Flow

### Step 1: Generate Authorization URL

```rust
use armature_auth::OAuth2Provider;

let (auth_url, csrf_token) = provider.authorization_url()?;

// IMPORTANT: Armature is stateless - no server-side sessions
// Option 1: Use PKCE (recommended for public clients)
let (auth_url, pkce_verifier) = provider.authorization_url_with_pkce()?;
// Return pkce_verifier to client securely (in redirect state param or response)

// Option 2: Store CSRF in signed cookie (client-side)
// Or embed in the redirect URL state parameter

// Redirect user to auth_url
response.redirect(auth_url.as_str());
```

### Step 2: Handle Callback

```rust
// Extract code and state from query parameters
let code = request.query("code")?;
let state = request.query("state")?;

// Verify CSRF token (stateless approach)
// Option 1: With PKCE (no CSRF needed - cryptographically secure)
let token = provider.exchange_code_pkce(code.to_string(), pkce_verifier).await?;

// Option 2: Verify state parameter if not using PKCE
// Extract state from redirect, compare with original (stored client-side or signed)

// Exchange code for token
// let token = provider.exchange_code(code.to_string()).await?;
```

### Step 3: Get User Info

```rust
// Fetch user information
let user_info = provider.get_user_info(&token).await?;

// Create JWT token (stateless authentication)
let user_claims = UserClaims {
    sub: user_info.sub,
    email: user_info.email.unwrap_or_default(),
    name: user_info.name,
};

// Generate JWT instead of session
let jwt_token = jwt_manager.create_token(user_claims)?;

// Return JWT to client
// Client stores token and sends it with each request
// No server-side session needed
```

### Step 4: Token Refresh

```rust
// When access token expires
if let Some(refresh_token) = token.refresh_token {
    let new_token = provider.refresh_token(refresh_token).await?;
    // Update stored token
}
```

## Complete Example

```rust
use armature_framework::prelude::*;
use armature_auth::providers::{GoogleConfig, GoogleProvider};
use armature_auth::OAuth2Provider;

#[controller("/auth")]
struct AuthController {
    google_provider: Arc<GoogleProvider>,
}

impl AuthController {
    #[get("/google")]
    async fn google_login(&self) -> Result<Response> {
        // Generate authorization URL
        let (auth_url, csrf_token) = self.google_provider
            .authorization_url()
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Armature is stateless - use PKCE instead of CSRF tokens
        // PKCE provides cryptographic protection without server-side state

        // Redirect to Google
        Ok(Response::redirect(auth_url.as_str()))
    }

    #[get("/google/callback")]
    async fn google_callback(&self, request: HttpRequest) -> Result<Response> {
        // Extract authorization code
        let code = request.query_param("code")
            .ok_or_else(|| Error::BadRequest("Missing code".into()))?;

        // Exchange code for token
        let token = self.google_provider
            .exchange_code(code.to_string())
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Get user info
        let user_info = self.google_provider
            .get_user_info(&token)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;

        // Create user session
        let user = UserContext::new(user_info.sub)
            .with_email(user_info.email.unwrap_or_default());

        // In production:
        // 1. Find or create user in database
        // 2. Generate JWT token (stateless auth)
        // 3. Return JWT to client
        // 4. Client stores JWT and includes in subsequent requests

        Ok(Response::json(serde_json::json!({
            "user": user,
            "token": token.access_token
        })))
    }
}
```

## Best Practices

### 1. CSRF Protection

Use PKCE for stateless CSRF protection:

```rust
// Generate with PKCE (recommended - stateless and secure)
let (auth_url, pkce_verifier) = provider.authorization_url_with_pkce()?;

// On callback, exchange with PKCE verifier
let token = provider.exchange_code_pkce(code, pkce_verifier).await?;

// PKCE provides cryptographic protection without server-side state
// The pkce_verifier can be passed through the OAuth flow securely
```

**Note:** Armature is stateless. PKCE is preferred over traditional state parameters
because it provides cryptographic security without requiring server-side session storage.

### 2. Secure Redirect URIs

- Use HTTPS in production
- Whitelist exact URIs (no wildcards)
- Use separate URIs per environment

```rust
let redirect_url = if cfg!(debug_assertions) {
    "http://localhost:3000/callback"
} else {
    "https://app.example.com/callback"
};
```

### 3. Token Storage

```rust
// ✓ Store tokens securely
let encrypted_token = encrypt(token.access_token)?;
db.save_user_token(user_id, encrypted_token)?;

// ✓ Use refresh tokens
if token_expired && refresh_token.is_some() {
    let new_token = provider.refresh_token(refresh_token).await?;
}

// ✗ Don't expose tokens to client
// ✗ Don't log tokens
```

### 4. Error Handling

```rust
match provider.exchange_code(code).await {
    Ok(token) => { /* Success */ },
    Err(e) => {
        log::error!("OAuth2 error: {}", e);
        // Show user-friendly message
        return Err(Error::AuthenticationFailed);
    }
}
```

### 5. Multi-Provider Support

```rust
enum Provider {
    Google(GoogleProvider),
    Microsoft(MicrosoftEntraProvider),
    Okta(OktaProvider),
}

impl Provider {
    async fn authenticate(&self, code: String) -> Result<OAuth2Token> {
        match self {
            Provider::Google(p) => p.exchange_code(code).await,
            Provider::Microsoft(p) => p.exchange_code(code).await,
            Provider::Okta(p) => p.exchange_code(code).await,
        }
    }
}
```

### 6. User Linking

```rust
async fn link_oauth_user(
    oauth_sub: String,
    provider: String,
    user_info: OAuth2UserInfo,
) -> Result<User> {
    // Check if OAuth account already linked
    if let Some(user) = db.find_by_oauth(provider, oauth_sub).await? {
        return Ok(user);
    }

    // Check if email matches existing user
    if let Some(email) = user_info.email {
        if let Some(user) = db.find_by_email(&email).await? {
            // Link OAuth account to existing user
            db.link_oauth(user.id, provider, oauth_sub).await?;
            return Ok(user);
        }
    }

    // Create new user
    let user = User::new(user_info);
    db.save_user(&user).await?;
    db.link_oauth(user.id, provider, oauth_sub).await?;

    Ok(user)
}
```

## Summary

The `armature-auth` OAuth2 provider system offers:

- ✅ **5 major providers** out of the box
- ✅ **Consistent API** across all providers
- ✅ **Type-safe** configuration
- ✅ **Async-first** design
- ✅ **Production-ready** error handling
- ✅ **Extensible** for custom providers

For more information:
- [Auth Guide](auth-guide.md) - Complete authentication documentation
- [JWT Guide](jwt-guide.md) - JWT token management (coming soon)
- [Examples](../examples/) - Working code examples

## See Also

- [Google OAuth2 Documentation](https://developers.google.com/identity/protocols/oauth2)
- [Microsoft Identity Platform](https://docs.microsoft.com/en-us/azure/active-directory/develop/)
- [AWS Cognito Documentation](https://docs.aws.amazon.com/cognito/)
- [Okta Developer Documentation](https://developer.okta.com/)
- [Auth0 Documentation](https://auth0.com/docs)

