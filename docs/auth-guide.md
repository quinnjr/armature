# Authentication & Authorization Guide

Complete guide to authentication and authorization in Armature using `armature-auth`.

## Table of Contents

1. [Overview](#overview)
2. [Installation](#installation)
3. [Password Hashing](#password-hashing)
4. [Authentication Service](#authentication-service)
5. [User Management](#user-management)
6. [Request Authentication (JwtAuthMiddleware)](#request-authentication-jwtauthmiddleware)
7. [Guards](#guards)
8. [Authentication Strategies](#authentication-strategies)
9. [API Keys](#api-keys)
10. [Magic Links](#magic-links)
11. [Two-Factor Authentication](#two-factor-authentication)
12. [SAML](#saml)
13. [Complete Example](#complete-example)
14. [Best Practices](#best-practices)

## Overview

`armature-auth` provides a comprehensive authentication and authorization system inspired by NestJS:

- **Password Hashing**: Argon2id (default) and bcrypt, with transparent rehashing
- **JWT Integration**: `JwtAuthMiddleware` verifies access tokens and attaches the verified identity
- **Guards**: Fail-closed route protection that only trusts a verified identity
- **Role-Based Access Control (RBAC)**: Role and permission checking
- **Authentication Strategies**: Pluggable authentication methods
- **User Context**: Type-safe user information extraction

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["auth"] }
```

The `auth` feature automatically includes `armature-jwt`.

## Password Hashing

### Supported Algorithms

`armature-auth` supports two password hashing algorithms:

1. **Argon2id** (default, recommended)
   - Modern, memory-hard algorithm
   - Winner of Password Hashing Competition
   - Defaults: m = 64 MiB, t = 3, p = 1 (RFC 9106 / OWASP guidance)

2. **Bcrypt**
   - Battle-tested, widely used
   - Passwords longer than 72 bytes are **rejected** rather than silently
     truncated (`BCRYPT_MAX_PASSWORD_BYTES`)

### Basic Usage

```rust
use armature_auth::{PasswordHasher, PasswordVerifier};

// Default (Argon2)
let hasher = PasswordHasher::default();
let hash = hasher.hash("my-password")?;

// Verify
let is_valid = hasher.verify("my-password", &hash)?;
assert!(is_valid);

// Specific algorithm
use armature_auth::password::HashAlgorithm;
let bcrypt_hasher = PasswordHasher::new(HashAlgorithm::Bcrypt);
let hash = bcrypt_hasher.hash("my-password")?;

// Tune the cost parameters (both return `Result`)
let hasher = PasswordHasher::new(HashAlgorithm::Argon2)
    .with_argon2_params(128 * 1024, 3, 1)?; // memory in KiB, iterations, parallelism
let bcrypt_hasher = PasswordHasher::new(HashAlgorithm::Bcrypt).with_bcrypt_cost(12)?;
```

### Hashing from Async Code

Hashing is CPU- and memory-heavy by design. From async handlers use the
`_async` variants, which run on Tokio's blocking pool instead of stalling a
runtime worker:

```rust
let hash = hasher.hash_async("my-password").await?;
let ok = hasher.verify_async("my-password", &hash).await?;
```

### Upgrading Old Hashes

`needs_rehash` reports whether a stored hash uses a different algorithm or
weaker parameters than the hasher is configured with. After a successful
login, rehash the just-verified password and store the result:

```rust
if hasher.verify_async(&password, &stored_hash).await? {
    if hasher.needs_rehash(&stored_hash) {
        let upgraded = hasher.hash_async(&password).await?;
        db.update_password_hash(&user_id, &upgraded).await?;
    }
}
```

`LocalStrategy` does this automatically through
`LocalUserStore::update_password_hash` (see
[Local Strategy](#local-strategy)).

### Auto-Detection

The hasher automatically detects the algorithm from the hash format:

```rust
let hasher = PasswordHasher::default();

// Can verify both Bcrypt and Argon2 hashes
let bcrypt_hash = "$2b$12$...";
let argon2_hash = "$argon2id$v=19$...";

hasher.verify("password", bcrypt_hash)?; // Works
hasher.verify("password", argon2_hash)?; // Also works
```

## Authentication Service

The `AuthService` is the central authentication component:

```rust
use armature_auth::{AuthService, PasswordHasher};
use armature_jwt::{JwtConfig, JwtManager};

// Basic setup
let auth_service = AuthService::new();

// With JWT. `JwtConfig::new` returns `Result`: the HMAC secret must be at
// least 32 bytes and must not be a placeholder or low-entropy string.
// Generate one with e.g. `openssl rand -base64 48`.
let jwt_config = JwtConfig::new(std::env::var("JWT_SECRET")?)?;
let jwt_manager = JwtManager::new(jwt_config)?;
let auth_service = AuthService::with_jwt(jwt_manager);

// Custom password hasher
let hasher = PasswordHasher::new(HashAlgorithm::Bcrypt);
let auth_service = AuthService::new()
    .with_password_hasher(hasher);
```

### Service Methods

```rust
// Hash a password
let hash = auth_service.hash_password("password")?;

// Verify a password
let is_valid = auth_service.verify_password("password", &hash)?;

// Validate a user
auth_service.validate(&user)?;

// Access JWT manager
if let Some(jwt) = auth_service.jwt_manager() {
    let token = jwt.sign(&claims)?;
}
```

`JwtConfig` has no `Default` (it used to sign with a public placeholder
secret), and `JwtManager::new` re-validates the config, so a weak secret
assigned to the public `secret` field or loaded from a config file is rejected
too. For asymmetric keys use `JwtConfig::with_rsa(private, public)` or
`JwtConfig::with_key_pair(private, public, algorithm)` (RS*, PS*, ES*, EdDSA).

### Verifying Tokens

| Method | Use |
|--------|-----|
| `verify_access::<T>(token)` | Authenticate a request. Rejects refresh tokens (`token_use` other than `"access"`, or the `refresh+jwt` `typ` header). |
| `verify::<T>(token)` | Verify signature and claims of any token type. |
| `refresh_token::<T>(token)` | Exchange a refresh token for a new pair; requires the refresh `typ` header. |
| `decode_unverified_insecure::<T>(token)` | Read claims **without** verification. The claims are attacker-controlled; never base a decision on them. |

`generate_token_pair` marks access tokens `typ: "at+jwt"` and refresh tokens
`typ: "refresh+jwt"`, so a refresh token can never be presented as a bearer
credential.

## User Management

### Implementing AuthUser

Define your user type and implement the `AuthUser` trait:

```rust
use armature_auth::AuthUser;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: String,
    email: String,
    password_hash: String,
    roles: Vec<String>,
    permissions: Vec<String>,
    active: bool,
}

impl AuthUser for User {
    fn user_id(&self) -> String {
        self.id.clone()
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    fn has_permission(&self, permission: &str) -> bool {
        self.permissions.iter().any(|p| p == permission)
    }
}
```

### Using UserContext

For simple use cases, use the built-in `UserContext`:

```rust
use armature_auth::UserContext;

let user = UserContext::new("user123".to_string())
    .with_email("user@example.com".to_string())
    .with_roles(vec!["admin".to_string(), "user".to_string()])
    .with_permissions(vec!["read".to_string(), "write".to_string()])
    .with_metadata(serde_json::json!({
        "name": "John Doe",
        "department": "Engineering"
    }));

// UserContext implements AuthUser
assert!(user.has_role("admin"));
assert!(user.has_permission("write"));
```

## Request Authentication (JwtAuthMiddleware)

`JwtAuthMiddleware` turns an `Authorization: Bearer …` header into a verified,
request-scoped identity. On a valid **access** token it inserts three request
extensions:

1. `armature_auth::UserContext` built from the `sub`, `roles` and
   `permissions` claims (all claims are kept in `UserContext::metadata`);
2. `armature_core::RequestRoles`, for core's `RolesGuard`;
3. `armature_core::VerifiedIdentity` (the subject), for core's
   `AuthenticationGuard`.

It rejects refresh tokens (it verifies with `JwtManager::verify_access`),
tokens without a subject claim, and requests carrying more than one
`Authorization` header. The `Bearer` scheme is matched case-insensitively.

Register it as **global** middleware with `Application::use_middleware`, so it
runs before every guard:

```rust
use armature_auth::JwtAuthMiddleware;
use armature_core::Application;
use armature_jwt::{JwtConfig, JwtManager};

let manager = JwtManager::new(JwtConfig::new(std::env::var("JWT_SECRET")?)?)?;

let app = Application::create::<AppModule>()
    .await
    // Optional mode: anonymous requests continue without an identity,
    // and guards on individual routes decide.
    .use_middleware(JwtAuthMiddleware::optional(manager));

app.listen(3000).await?;
```

- `JwtAuthMiddleware::new(manager)` (required mode) answers `401` for a
  missing or invalid token before anything else runs.
- `JwtAuthMiddleware::optional(manager)` lets anonymous requests through. A
  token that **fails** verification is not forwarded: the `Authorization`
  header is stripped and a `RejectedCredentials` marker extension is inserted,
  so nothing downstream can mistake it for a credential.
- Claim names are configurable with `with_subject_claim`, `with_roles_claim`
  and `with_permissions_claim`.

### Pipeline Order

Armature follows the NestJS order:

1. Global middleware (`Application::use_middleware`), in registration order
   (first = outermost)
2. Global guards (`Application::with_guard`)
3. Route-scoped middleware and guards (module guards, controller
   `#[middleware]` / `#[guard]`, `RouteGroup` layers)
4. The handler

Controller-level `#[middleware]` runs **before** controller-level `#[guard]`,
so an authentication middleware attached to a controller also populates the
identity its guards read.

## Guards

Guards protect routes by enforcing authentication and authorization rules.
They **fail closed**: they only trust the verified identity an authentication
layer (in practice `JwtAuthMiddleware`) attached to the request. A bare
`Authorization` header proves nothing and is not consulted.

`AuthGuard`, `RoleGuard` and `PermissionGuard` implement both
`armature_auth::Guard` and `armature_core::Guard`, so they can be used in
`#[guard(...)]`, `#[use_guard(...)]` and module guards. In the core pipeline a
missing identity becomes `401 Unauthorized` and a missing role/permission
`403 Forbidden`.

### Authentication Guard

Passes only when a verified `UserContext` is present:

```rust
use armature_auth::{AuthError, AuthGuard, Guard, UserContext};

let guard = AuthGuard::new();

// With both `armature_auth::Guard` and `armature_core::Guard` in scope,
// call the method through the trait you mean:
match Guard::can_activate(&guard, &request).await {
    Ok(true) => { /* authenticated */ }
    Err(AuthError::Unauthorized) => { /* no verified identity */ }
    _ => {}
}

// Read the verified user in a handler
let user: UserContext = guard.extract_user(&request)?;
```

The core equivalent, `armature_core::AuthenticationGuard`, requires a
`VerifiedIdentity` (or `RequestRoles`) extension.

### Role Guard

Requires specific roles from the verified identity:

```rust
use armature_auth::RoleGuard;

// Require ANY of these roles
let guard = RoleGuard::any(vec!["admin".to_string(), "moderator".to_string()]);

// Require ALL of these roles
let guard = RoleGuard::all(vec!["admin".to_string(), "verified".to_string()]);

// Check an already-extracted user
let has_access = guard.check_roles(&user);
```

An **empty** requirement (`RoleGuard::any(vec![])`) means "any authenticated
caller", not "anyone": it still requires a verified identity.

On a controller:

```rust
#[controller("/admin")]
#[guard(RoleGuard::any(vec!["admin".to_string()]))]
#[derive(Default)]
struct AdminController;
```

With `JwtAuthMiddleware` registered through `Application::use_middleware`, a
token carrying the `admin` role passes; a token without it gets `403`, and a
request without a valid token gets `401`.

### Permission Guard

Requires specific permissions:

```rust
use armature_auth::PermissionGuard;

// Require ANY of these permissions
let guard = PermissionGuard::any(vec![
    "posts:read".to_string(),
    "posts:list".to_string()
]);

// Require ALL of these permissions
let guard = PermissionGuard::all(vec![
    "posts:read".to_string(),
    "posts:write".to_string()
]);

let has_access = guard.check_permissions(&user);
```

As with roles, `PermissionGuard::any(vec![])` requires authentication.

### Custom Guards

Implement `armature_core::Guard` for custom logic. Read the verified identity
from the request extensions, never from raw headers:

```rust
use armature_auth::UserContext;
use armature_core::{Error, Guard, GuardContext};
use async_trait::async_trait;

struct ActiveAccountGuard;

#[async_trait]
impl Guard for ActiveAccountGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        let user = context
            .request
            .extensions
            .get::<UserContext>()
            .ok_or_else(|| Error::Unauthorized("authentication required".into()))?;
        Ok(user.metadata.get("active").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}
```

## Authentication Strategies

Strategies define how users are authenticated.

### Local Strategy

Username/password authentication against a `LocalUserStore`. Without a store,
authentication always fails. The strategy hashes on the blocking pool, runs a
dummy verification for unknown users (so timing does not reveal which
usernames exist), and upgrades outdated hashes through
`LocalUserStore::update_password_hash` (default: no-op).

```rust
use armature_auth::{AuthStrategy, LocalCredentials, LocalStrategy, UserContext};
use armature_auth::strategy::LocalUserStore;
use std::sync::Arc;

struct DbUserStore { /* ... */ }

#[async_trait::async_trait]
impl LocalUserStore<UserContext> for DbUserStore {
    async fn find_by_username(&self, username: &str)
        -> armature_auth::Result<Option<(UserContext, String)>> {
        // Return the user and their stored password hash
        todo!()
    }

    async fn update_password_hash(&self, username: &str, new_hash: &str)
        -> armature_auth::Result<()> {
        // Persist the upgraded hash
        todo!()
    }
}

let strategy = LocalStrategy::with_store(Arc::new(DbUserStore { /* ... */ }));

let credentials = LocalCredentials {
    username: "user@example.com".to_string(),
    password: "password123".to_string(),
};
let user: UserContext = strategy.authenticate(&credentials).await?;
```

`LocalCredentials` redacts the password from `Debug` and skips it when
serializing.

### JWT Strategy

Token-based authentication:

```rust
use armature_auth::JwtStrategy;
use armature_jwt::JwtManager;

let jwt_manager = JwtManager::new(jwt_config)?;
let strategy = JwtStrategy::<User>::new(jwt_manager);

// Extract token from header
let token = strategy.extract_token("Bearer eyJhbGc...")?;

// Verify (access tokens only) and build a UserContext from the claims
let user: UserContext = strategy.authenticate(&token.to_string()).await?;
```

`JwtStrategy` verifies with `verify_access`, so refresh tokens are rejected.

## API Keys

API keys are never stored in plaintext. `ApiKeyManager` stores only
`ApiKey::key_hash` = hex `HMAC-SHA256(pepper, key)`, where the pepper is a
server-side secret of at least 32 bytes kept outside the key database
(rotating it invalidates every issued key). The plaintext exists once, in the
`GeneratedApiKey` returned by `generate`/`rotate`.

```rust
use armature_auth::api_key::{ApiKeyManager, ApiKeyStore};
use std::sync::Arc;

let store: Arc<dyn ApiKeyStore> = Arc::new(MyApiKeyStore::new(pool));
let pepper = std::env::var("API_KEY_PEPPER")?;
let manager = ApiKeyManager::new(store, pepper.into_bytes())?; // Err if < 32 bytes

// Issue: show `secret()` to the user once; only the hash reaches the store.
let generated = manager.generate("user_123", vec!["read".to_string()]).await?;
let plaintext = generated.secret().to_string();

// Validate: hashes the presented key and re-checks it in constant time.
if let Some(key) = manager.validate(&plaintext).await? {
    assert!(manager.has_scope(&key, "read"));
}

// Rotate: revokes the old key and returns a new GeneratedApiKey
let rotated = manager.rotate(&generated.id).await?;
```

Stores implement `ApiKeyStore::find_by_key_hash` (it replaced
`find_by_key`). `GeneratedApiKey` is not `Serialize` and redacts its secret
from `Debug`; `ApiKey` redacts its hash. Keys issued before this scheme cannot
be converted, so re-issue them.

## Magic Links

Only a SHA-256 hash of a magic-link token is stored. Redemption is a single
atomic step, so a link works at most once even under concurrent clicks:

```rust
use armature_auth::passwordless::{
    InMemoryMagicLinkStore, MagicLinkStore, MagicLinkToken, redeem_magic_link,
};
use std::time::Duration;

let store = InMemoryMagicLinkStore::new(); // or your own MagicLinkStore

// Issue: store the hashed record, e-mail the URL
let issued = MagicLinkToken::generate("user@example.com", Duration::from_secs(900))?;
store.save(&issued.record).await?;
send_email(&issued.to_url("https://myapp.com/auth/verify"));

// Callback: verify and consume in one step; a replay fails
let record = redeem_magic_link(&store, &presented_token).await?;
let email = record.identifier;
```

A custom store implements `MagicLinkStore::consume` atomically (for example a
single `DELETE … RETURNING` or Redis `GETDEL`). For a record you hold
exclusively, `MagicLinkToken::consume(candidate)` performs the same check.

## Two-Factor Authentication

Enabled with the `two-factor` feature.

### TOTP with Replay Protection

A TOTP code stays valid for its whole time step plus the ±1-step skew window.
To stop a code from being used twice, `verify` takes the last accepted time
step and never accepts a step at or before it:

```rust
use armature_auth::two_factor::TotpSecret;

// Enrolment: persist `secret.to_base32()` (encrypted); TotpSecret itself is
// not Serialize and redacts Debug.
let secret = TotpSecret::generate();
let qr_url = secret.to_qr_url("user@example.com", "MyApp")?;

// Login: load the secret and the last accepted step from the user record
let secret = TotpSecret::from_base32(stored_base32)?; // validates
let mut last_step: Option<u64> = user.totp_last_step;

if secret.verify_and_record(code, 30, &mut last_step)? {
    user.totp_last_step = last_step; // persist before answering
    // 2FA passed
}

// Or: `verify(code, 30, last_step)? -> Option<u64>` returns the matched step
```

A `time_step` of `0` is a `TwoFactorError::InvalidTimeStep`.

### Backup Codes

Backup codes are stored hashed and carry 80 bits each:

```rust
use armature_auth::two_factor::BackupCodes;

let generated = BackupCodes::generate(10);
show_once(generated.plaintext()); // display to the user exactly once
let mut stored: BackupCodes = generated.stored; // persist this

// Accepts codes with or without hyphens, any case; each works once
if stored.verify_and_consume(&submitted) {
    save(&stored);
}
```

## SAML

Enabled with the `saml` feature. Every SP-initiated response must be
correlated with the AuthnRequest that started the login, and accepted
assertion IDs are recorded so a captured assertion cannot be replayed.

```rust
use armature_auth::saml::{InMemoryReplayCache, SamlProvider, SamlServiceProvider};
use std::sync::Arc;

let sp = SamlServiceProvider::new("okta".to_string(), saml_config)?
    // The default cache is per process; share one across instances
    .with_replay_cache(Arc::new(InMemoryReplayCache::new()));

// 1. Start login: store request_id (and relay_state) in the user's session
let request = sp.create_auth_request()?;
session.set("saml_request_id", &request.request_id)?;
session.set("saml_relay_state", &request.relay_state)?;
// redirect to request.redirect_url

// 2. ACS endpoint: InResponseTo must match the stored request ID
let assertion = sp
    .validate_response(
        &saml_response,
        &stored_request_id,
        stored_relay_state.as_deref(),
        received_relay_state.as_deref(),
    )
    .await?;
```

IdP-initiated SSO has no request ID to correlate and is refused unless
`SamlConfig::allow_idp_initiated(true)` is set; it then goes through
`SamlServiceProvider::validate_unsolicited_response`. Behind a load balancer,
implement `AssertionReplayCache` on shared storage (for example Redis
`SET key 1 NX PXAT <expiry>`); the in-memory cache only protects one process.
`SamlConfig` redacts the SP private key from `Debug`.

## Complete Example

### User Registration and Login

```rust
use armature_auth::{AuthService, AuthUser, UserContext};
use armature_jwt::{Claims, JwtConfig, JwtManager};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    user: UserInfo,
}

#[derive(Serialize)]
struct UserInfo {
    id: String,
    email: String,
    roles: Vec<String>,
}

async fn register(
    auth_service: &AuthService,
    req: RegisterRequest,
) -> Result<User, Error> {
    // Hash password (Argon2id). In async code prefer
    // `PasswordHasher::hash_async`, which runs on the blocking pool.
    let password_hash = auth_service.hash_password(&req.password)?;

    // Create user
    let user = User {
        id: generate_id(),
        email: req.email,
        password_hash,
        roles: vec!["user".to_string()],
        active: true,
    };

    // Save to database
    save_user(&user).await?;

    Ok(user)
}

async fn login(
    auth_service: &AuthService,
    req: LoginRequest,
) -> Result<AuthResponse, Error> {
    // Find user
    let user = find_user_by_email(&req.email).await?;

    // Verify password
    if !auth_service.verify_password(&req.password, &user.password_hash)? {
        return Err(Error::InvalidCredentials);
    }

    // Validate user
    auth_service.validate(&user)?;

    // Generate tokens
    let jwt_manager = auth_service.jwt_manager().unwrap();

    // `sub` must be the stable user ID: JwtAuthMiddleware reads it, and
    // rejects tokens without one.
    let claims = Claims::new(UserContext::new(user.id.clone())
        .with_email(user.email.clone())
        .with_roles(user.roles.clone()))
        .with_subject(user.id.clone())
        .with_expiration(3600);

    let token_pair = jwt_manager.generate_token_pair(&claims)?;

    Ok(AuthResponse {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        user: UserInfo {
            id: user.id,
            email: user.email,
            roles: user.roles,
        },
    })
}
```

### Protected Routes

Register `JwtAuthMiddleware` globally and put guards on the controllers. The
middleware verifies the token before any guard runs; handlers read the
verified `UserContext` from the request:

```rust
use armature_auth::{AuthGuard, JwtAuthMiddleware, RoleGuard, UserContext};
use armature::prelude::*;

#[controller("/admin")]
#[guard(RoleGuard::any(vec!["admin".to_string()]))]
#[derive(Default)]
struct AdminController;

#[routes]
impl AdminController {
    #[get("/stats")]
    async fn stats(req: HttpRequest) -> Result<HttpResponse, Error> {
        let user: UserContext = AuthGuard::new()
            .extract_user(&req)
            .map_err(|_| Error::Unauthorized("authentication required".into()))?;
        HttpResponse::ok().json(&serde_json::json!({ "user": user.user_id }))
    }
}

#[module(controllers: [AdminController])]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manager = JwtManager::new(JwtConfig::new(std::env::var("JWT_SECRET")?)?)?;

    Application::create::<AppModule>()
        .await
        .use_middleware(JwtAuthMiddleware::optional(manager))
        .listen(3000)
        .await?;
    Ok(())
}
```

Do not hand-roll authentication by checking for an `Authorization` header in a
handler or middleware: a header is a claim, not a proof. Verify it
(`JwtAuthMiddleware`, `JwtStrategy`, `JwtManager::verify_access`) and rely on
the extensions the verifier attaches.

## Best Practices

### 1. Password Security

```rust
// ✓ Use Argon2 (default)
let hasher = PasswordHasher::default();

// ✓ Or explicitly choose Argon2
let hasher = PasswordHasher::new(HashAlgorithm::Argon2);

// ⚠️ Bcrypt is OK but Argon2 is preferred
let hasher = PasswordHasher::new(HashAlgorithm::Bcrypt);
```

### 2. Token Management

```rust
// ✓ Random secret of at least 32 bytes, from the environment / a secret store
let secret = std::env::var("JWT_SECRET")?;

// ✓ Short-lived access tokens
let jwt_config = JwtConfig::new(secret)?
    .with_expiration(Duration::from_secs(900)); // 15 minutes

// ✓ Long-lived refresh tokens
let jwt_config = jwt_config
    .with_refresh_expiration(Duration::from_secs(604800)); // 7 days

// ✓ Authenticate requests with verify_access (rejects refresh tokens)
let claims: serde_json::Value = jwt_manager.verify_access(token)?;

// ✗ Never trust decode_unverified_insecure for a decision
```

### 3. User Validation

```rust
// Always validate users after authentication
auth_service.validate(&user)?;

// Check user is active
if !user.is_active() {
    return Err(Error::InactiveUser);
}
```

### 4. Guard Composition

Authenticate once, globally, then compose guards declaratively. Guards listed
on a controller run in order; the first that denies short-circuits:

```rust
#[controller("/admin/reports")]
#[guard(
    AuthGuard,
    RoleGuard::any(vec!["admin".to_string()]),
    PermissionGuard::all(vec!["reports:read".to_string()])
)]
#[derive(Default)]
struct ReportsController;
```

### 5. Error Handling

```rust
use armature_auth::AuthError;

match auth_service.verify_password(password, hash) {
    Ok(true) => { /* Success */ },
    Ok(false) => return Err(AuthError::InvalidCredentials),
    Err(AuthError::PasswordVerifyError(e)) => {
        log::error!("Password verification error: {}", e);
        return Err(AuthError::AuthenticationFailed("Internal error".into()));
    },
    Err(e) => return Err(e),
}
```

### 6. Database Integration

```rust
// Store hashed passwords only
async fn create_user(email: String, password: String) -> Result<User> {
    let auth_service = AuthService::new();

    // Hash password
    let password_hash = auth_service.hash_password(&password)?;

    // NEVER store plain password
    let user = User {
        id: generate_id(),
        email,
        password_hash, // Store hash, not password
        roles: vec!["user".to_string()],
        active: true,
    };

    db.save(&user).await?;
    Ok(user)
}
```

### 7. Rate Limiting

Prefer `armature-ratelimit`'s `RateLimitMiddleware` (see the
[Rate Limiting Guide](rate-limiting-guide.md)), which keys on the socket peer
or the verified identity rather than spoofable headers. A minimal in-process
sketch:

```rust
// Implement rate limiting for auth endpoints
use std::collections::HashMap;
use std::time::{Duration, Instant};

struct RateLimiter {
    attempts: HashMap<String, (u32, Instant)>,
    max_attempts: u32,
    window: Duration,
}

impl RateLimiter {
    fn check(&mut self, email: &str) -> Result<(), AuthError> {
        let now = Instant::now();
        let entry = self.attempts.entry(email.to_string())
            .or_insert((0, now));

        if now.duration_since(entry.1) > self.window {
            *entry = (1, now);
            Ok(())
        } else if entry.0 >= self.max_attempts {
            Err(AuthError::AuthenticationFailed(
                "Too many attempts".into()
            ))
        } else {
            entry.0 += 1;
            Ok(())
        }
    }
}
```

## Summary

The `armature-auth` module provides:

- ✅ **Secure password hashing** with Argon2id and bcrypt, plus `needs_rehash`
- ✅ **JWT integration** via `JwtAuthMiddleware` and access-only verification
- ✅ **Fail-closed guards** for route protection
- ✅ **API keys, magic links, TOTP and backup codes** stored only as hashes
- ✅ **RBAC** with roles and permissions
- ✅ **Flexible strategies** for different auth methods
- ✅ **Type-safe** user context
- ✅ **DI integration** with the Armature framework

For more examples, see:
- `examples/auth_complete.rs` - Complete authentication demo
- `examples/jwt_simple.rs` - JWT basics
- `docs/jwt-guide.md` - JWT details (coming soon)

## See Also

- [JWT Guide](jwt-guide.md) (coming soon)
- [Configuration Guide](config-guide.md)
- [API Reference](../README.md)

