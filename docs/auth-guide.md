# Authentication & Authorization Guide

Complete guide to authentication and authorization in Armature using `armature-auth`.

## Table of Contents

1. [Overview](#overview)
2. [Installation](#installation)
3. [Password Hashing](#password-hashing)
4. [Authentication Service](#authentication-service)
5. [User Management](#user-management)
6. [Guards](#guards)
7. [Authentication Strategies](#authentication-strategies)
8. [Complete Example](#complete-example)
9. [Best Practices](#best-practices)

## Overview

`armature-auth` provides a comprehensive authentication and authorization system inspired by NestJS:

- **Password Hashing**: Bcrypt and Argon2 support
- **JWT Integration**: Seamless integration with `armature-jwt`
- **Guards**: Route protection with authentication and authorization
- **Role-Based Access Control (RBAC)**: Role and permission checking
- **Authentication Strategies**: Pluggable authentication methods
- **User Context**: Type-safe user information extraction

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["auth"] }
```

The `auth` feature automatically includes `armature-jwt`.

## Password Hashing

### Supported Algorithms

`armature-auth` supports two password hashing algorithms:

1. **Argon2** (default, recommended)
   - Modern, memory-hard algorithm
   - Winner of Password Hashing Competition
   - Resistant to GPU cracking attacks

2. **Bcrypt**
   - Battle-tested, widely used
   - Good compatibility
   - Slower than some modern alternatives

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
```

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

// With JWT
let jwt_config = JwtConfig::new("your-secret".to_string());
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

## Guards

Guards protect routes by enforcing authentication and authorization rules.

### Authentication Guard

Ensures the request has a valid token:

```rust
use armature_auth::AuthGuard;

let guard = AuthGuard::new();

// Check if request can proceed
if guard.can_activate(&request).await? {
    // Request is authenticated
}
```

### Role Guard

Requires specific roles:

```rust
use armature_auth::RoleGuard;

// Require ANY of these roles
let guard = RoleGuard::any(vec!["admin".to_string(), "moderator".to_string()]);

// Require ALL of these roles
let guard = RoleGuard::all(vec!["admin".to_string(), "verified".to_string()]);

// Check user roles
let has_access = guard.check_roles(&user);

// Use in request handler
if guard.can_activate(&request).await? {
    // User has required roles
}
```

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

### Custom Guards

Implement the `Guard` trait for custom logic:

```rust
use armature_auth::Guard;
use async_trait::async_trait;

struct CustomGuard {
    // Your fields
}

#[async_trait]
impl Guard for CustomGuard {
    async fn can_activate(&self, request: &HttpRequest) -> Result<bool> {
        // Your custom logic
        Ok(true)
    }
}
```

## Authentication Strategies

Strategies define how users are authenticated.

### Local Strategy

Username/password authentication:

```rust
use armature_auth::{LocalStrategy, LocalCredentials};

let strategy = LocalStrategy::<User>::new();

let credentials = LocalCredentials {
    username: "user@example.com".to_string(),
    password: "password123".to_string(),
};

// In your implementation:
// 1. Find user by username
// 2. Verify password
// 3. Return authenticated user
```

### JWT Strategy

Token-based authentication:

```rust
use armature_auth::JwtStrategy;
use armature_jwt::JwtManager;

let jwt_manager = JwtManager::new(jwt_config)?;
let strategy = JwtStrategy::<User>::new(jwt_manager);

// Extract token from header
let token = strategy.extract_token("Bearer eyJhbGc...")?;

// Verify and decode token
// Load user from database
// Return authenticated user
```

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
    // Hash password
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

    let claims = Claims::new(UserContext::new(user.id.clone())
        .with_email(user.email.clone())
        .with_roles(user.roles.clone()))
        .with_subject(user.email.clone())
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

```rust
use armature_auth::{AuthGuard, RoleGuard};

async fn protected_handler(request: HttpRequest) -> Result<Response, Error> {
    // Check authentication
    let auth_guard = AuthGuard::new();
    if !auth_guard.can_activate(&request).await? {
        return Err(Error::Unauthorized);
    }

    // Extract user
    let user = extract_user_from_request(&request)?;

    // Check roles
    let role_guard = RoleGuard::any(vec!["admin".to_string()]);
    if !role_guard.check_roles(&user) {
        return Err(Error::Forbidden);
    }

    // Handle request
    Ok(Response::ok("Protected resource"))
}
```

### Middleware Pattern

```rust
async fn auth_middleware(
    request: HttpRequest,
    next: impl Fn(HttpRequest) -> Future<Output = Result<Response>>,
) -> Result<Response> {
    let guard = AuthGuard::new();

    if !guard.can_activate(&request).await? {
        return Err(Error::Unauthorized);
    }

    next(request).await
}
```

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
// ✓ Short-lived access tokens
let jwt_config = JwtConfig::new(secret)
    .with_expiration(Duration::from_secs(900)); // 15 minutes

// ✓ Long-lived refresh tokens
let jwt_config = jwt_config
    .with_refresh_expiration(Duration::from_secs(604800)); // 7 days

// ✓ Store secrets in environment variables
let secret = std::env::var("JWT_SECRET")?;
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

```rust
// Combine multiple guards
async fn admin_only_handler(request: HttpRequest) -> Result<Response> {
    // First: Authentication
    AuthGuard::new().can_activate(&request).await?;

    // Second: Authorization
    let user = extract_user(&request)?;
    let role_guard = RoleGuard::any(vec!["admin".to_string()]);

    if !role_guard.check_roles(&user) {
        return Err(Error::Forbidden);
    }

    // Handler logic
    Ok(Response::ok("Admin resource"))
}
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

- ✅ **Secure password hashing** with Bcrypt and Argon2
- ✅ **JWT integration** for stateless authentication
- ✅ **Guards** for route protection
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

