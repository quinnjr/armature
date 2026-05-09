# Session Storage Guide

Server-side session storage for Armature framework with Redis, Memcached, and CouchDB backends.

## Table of Contents

- [Important: Stateless Architecture Preferred](#important-stateless-architecture-preferred)
- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [When to Use Sessions](#when-to-use-sessions)
- [Quick Start](#quick-start)
- [Session Backends](#session-backends)
- [Session API](#session-api)
- [Integration with Handlers](#integration-with-handlers)
- [Best Practices](#best-practices)
- [Migration from Sessions to JWT](#migration-from-sessions-to-jwt)
- [API Reference](#api-reference)
- [Summary](#summary)

## Important: Stateless Architecture Preferred

> ⚠️ **Armature strongly recommends stateless architecture using JWT tokens instead of server-side sessions.**

### Why Stateless?

| Aspect | Stateless (JWT) | Sessions |
|--------|-----------------|----------|
| **Scalability** | ✅ Any server handles any request | ❌ Requires shared session store |
| **Performance** | ✅ No session lookups | ❌ External store on every request |
| **Reliability** | ✅ No session store to fail | ❌ Session store is SPOF |
| **Cloud-Native** | ✅ Perfect for K8s/serverless | ❌ Requires stateful infrastructure |
| **Security** | ✅ No session hijacking | ❌ Session vulnerabilities possible |

### Preferred Approach: JWT Authentication

```rust
use armature_jwt::JwtManager;

// Create token at login - contains all user info
let token = jwt_manager.create_token(UserClaims {
    user_id: user.id,
    email: user.email,
    roles: user.roles,
})?;

// Client stores token (localStorage, cookie, etc.)
// Server validates on each request - no session lookup needed
let claims = jwt_manager.verify_token(&token)?;
```

See the [Stateless Architecture Guide](./stateless-architecture.md) for comprehensive details.

## Overview

The `armature-session` module provides server-side session storage for cases where sessions are absolutely necessary. It supports multiple backends with a unified API.

**Use this module only when:**
- Integrating with legacy systems requiring sessions
- Compliance mandates server-side session tracking
- You need immediate session invalidation
- Storing large amounts of temporary user data

## Features

- ✅ **Redis backend** - High-performance, default option
- ✅ **Memcached backend** - Optional, feature-gated
- ✅ **CouchDB backend** - Document-based sessions
- ✅ Unified `SessionStore` trait across all backends
- ✅ Automatic session expiration
- ✅ Session metadata (IP, user agent)
- ✅ Configurable TTL with max limits
- ✅ Typed session data storage

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
# Redis only (default)
armature-session = "0.1"

# With Memcached support
armature-session = { version = "0.1", features = ["memcached"] }

# With CouchDB support
armature-session = { version = "0.1", features = ["couchdb"] }

# All backends
armature-session = { version = "0.1", features = ["full"] }
```

Or via the main armature crate:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["session"] }

# With Memcached
armature-framework = { version = "0.1", features = ["session", "session-memcached"] }
```

## When to Use Sessions

### ✅ Appropriate Use Cases

1. **Legacy System Integration**
   ```rust
   // Old system expects session IDs
   let session = store.create(None).await?;
   legacy_api.set_session_id(&session.id)?;
   ```

2. **Compliance Requirements**
   ```rust
   // Audit trail requires server-side session tracking
   session.set("audit_trail", audit_entry)?;
   store.save(&session).await?;
   ```

3. **Immediate Logout (All Devices)**
   ```rust
   // Delete all sessions for a user
   for session_id in user_sessions {
       store.delete(&session_id).await?;
   }
   ```

4. **Large Temporary Data**
   ```rust
   // Shopping cart, multi-step wizard data
   session.set("cart", large_cart_data)?;
   session.set("wizard_step", 3)?;
   ```

### ❌ Inappropriate Use Cases

- **Authentication only** → Use JWT
- **Simple user identification** → Use JWT
- **API authentication** → Use JWT/API keys
- **Microservices** → Use JWT for service-to-service

## Quick Start

### Redis Session Store (Default)

```rust
use armature_session::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), SessionError> {
    // Configure Redis session store
    let config = SessionConfig::redis("redis://localhost:6379")?
        .with_namespace("myapp:session")
        .with_default_ttl(Duration::from_secs(3600))  // 1 hour
        .with_max_ttl(Duration::from_secs(86400));    // 24 hour max

    let store = RedisSessionStore::new(config).await?;

    // Create a new session
    let mut session = store.create(None).await?;
    println!("Session ID: {}", session.id);

    // Store typed data
    session.set("user_id", 123)?;
    session.set("username", "alice")?;
    session.set("roles", vec!["admin", "user"])?;
    store.save(&session).await?;

    // Retrieve session later
    if let Some(session) = store.get(&session.id).await? {
        let user_id: Option<i32> = session.get("user_id");
        let roles: Option<Vec<String>> = session.get("roles");
        println!("User: {:?}, Roles: {:?}", user_id, roles);
    }

    // Delete session (logout)
    store.delete(&session.id).await?;

    Ok(())
}
```

## Session Backends

### Redis (Default)

High-performance, recommended for most use cases.

```rust
use armature_session::*;

let config = SessionConfig::redis("redis://localhost:6379")?
    .with_namespace("myapp:session")
    .with_default_ttl(Duration::from_secs(3600));

let store = RedisSessionStore::new(config).await?;
```

**Pros:**
- Very fast (in-memory)
- Automatic expiration (TTL)
- Clustering support
- Persistence options

**Cons:**
- Additional infrastructure
- Memory-based (limited by RAM)

### Memcached (Feature-gated)

Requires `memcached` feature flag.

```toml
[dependencies]
armature-session = { version = "0.1", features = ["memcached"] }
```

```rust
use armature_session::*;

let config = SessionConfig::memcached("memcache://localhost:11211")?
    .with_namespace("myapp:session")
    .with_default_ttl(Duration::from_secs(3600));

let store = MemcachedSessionStore::new(config).await?;
```

**Pros:**
- Simple protocol
- Very fast
- Distributed by design

**Cons:**
- No persistence (volatile)
- No prefix scanning (count/clear_all limited)
- Memory-only

### CouchDB

Document-based sessions for environments already using CouchDB.

```toml
[dependencies]
armature-session = { version = "0.1", features = ["couchdb"] }
```

```rust
use armature_session::*;

let config = SessionConfig::couchdb("http://localhost:5984", "sessions")?
    .with_namespace("myapp")
    .with_default_ttl(Duration::from_secs(3600))
    .with_auth("admin", "password");

let store = CouchDbSessionStore::new(config).await?;
```

**Setup CouchDB database:**

```bash
# Create database
curl -X PUT http://admin:password@localhost:5984/sessions

# Create view for expiration cleanup
curl -X PUT http://admin:password@localhost:5984/sessions/_design/sessions \
  -H "Content-Type: application/json" \
  -d '{
    "views": {
      "by_expiration": {
        "map": "function(doc) { if(doc.expires_at) emit(doc.expires_at, null); }"
      }
    }
  }'
```

**Pros:**
- Persistent storage
- Query capabilities
- Good for document-oriented data

**Cons:**
- Slower than Redis/Memcached
- More complex setup
- Higher resource usage

## Session API

### Session Data Structure

```rust
pub struct Session {
    pub id: String,                              // Unique session ID
    pub data: HashMap<String, serde_json::Value>, // Session data
    pub created_at: DateTime<Utc>,               // Creation time
    pub last_accessed_at: DateTime<Utc>,         // Last access time
    pub expires_at: DateTime<Utc>,               // Expiration time
    pub user_agent: Option<String>,              // Browser/client info
    pub ip_address: Option<String>,              // Client IP
}
```

### Session Methods

```rust
// Get typed value
let user_id: Option<i32> = session.get("user_id");

// Set typed value
session.set("user_id", 123)?;

// Remove value
session.remove("temp_data");

// Check if key exists
if session.contains("user_id") { ... }

// Get all keys
let keys = session.keys();

// Clear all data
session.clear();

// Update last accessed time
session.touch();

// Extend expiration
session.extend(Duration::from_secs(3600));

// Check if expired
if session.is_expired() { ... }

// Add metadata
let session = Session::new(id, ttl)
    .with_user_agent("Mozilla/5.0...")
    .with_ip_address("192.168.1.1");
```

### SessionStore Trait

```rust
#[async_trait]
pub trait SessionStore: Send + Sync {
    // Create new session
    async fn create(&self, ttl: Option<Duration>) -> SessionResult<Session>;

    // Get session by ID
    async fn get(&self, session_id: &str) -> SessionResult<Option<Session>>;

    // Save/update session
    async fn save(&self, session: &Session) -> SessionResult<()>;

    // Delete session
    async fn delete(&self, session_id: &str) -> SessionResult<()>;

    // Check if session exists
    async fn exists(&self, session_id: &str) -> SessionResult<bool>;

    // Extend session TTL
    async fn extend(&self, session_id: &str, ttl: Duration) -> SessionResult<()>;

    // Touch session (update last accessed)
    async fn touch(&self, session_id: &str) -> SessionResult<()>;

    // Clear all sessions (dangerous!)
    async fn clear_all(&self) -> SessionResult<()>;

    // Count active sessions
    async fn count(&self) -> SessionResult<usize>;

    // Cleanup expired sessions
    async fn cleanup_expired(&self) -> SessionResult<usize>;
}
```

## Integration with Handlers

### Middleware Approach

```rust
use armature_framework::prelude::*;
use armature_session::*;

pub struct SessionMiddleware {
    store: RedisSessionStore,
}

#[async_trait]
impl Middleware for SessionMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Extract session ID from cookie
        let session_id = req.cookie("session_id");

        if let Some(id) = session_id {
            if let Some(session) = self.store.get(&id).await? {
                // Session valid - attach to request
                req.headers.insert("x-session-id".to_string(), id);
                req.headers.insert("x-user-id".to_string(),
                    session.get::<String>("user_id").unwrap_or_default());
            }
        }

        next(req).await
    }
}
```

### Controller with Sessions

```rust
#[controller("/api")]
struct UserController {
    session_store: RedisSessionStore,
}

impl UserController {
    #[post("/login")]
    async fn login(
        &self,
        #[body] credentials: Body<LoginRequest>,
    ) -> Result<HttpResponse, Error> {
        // Validate credentials...
        let user = validate_user(&credentials)?;

        // Create session
        let mut session = self.session_store.create(None).await?;
        session.set("user_id", user.id)?;
        session.set("roles", user.roles)?;
        self.session_store.save(&session).await?;

        // Return session ID in cookie
        HttpResponse::ok()
            .with_cookie("session_id", &session.id, CookieOptions::secure())
            .with_json(&LoginResponse { success: true })
    }

    #[post("/logout")]
    async fn logout(
        &self,
        #[header("Cookie")] cookie: Header,
    ) -> Result<HttpResponse, Error> {
        if let Some(session_id) = parse_cookie(&cookie, "session_id") {
            self.session_store.delete(&session_id).await?;
        }

        HttpResponse::ok()
            .with_cleared_cookie("session_id")
            .with_json(&LogoutResponse { success: true })
    }
}
```

## Best Practices

### 1. Always Set Reasonable TTLs

```rust
// ✅ Good - Short default, enforced maximum
let config = SessionConfig::redis("redis://localhost:6379")?
    .with_default_ttl(Duration::from_secs(3600))   // 1 hour
    .with_max_ttl(Duration::from_secs(86400));     // 24 hour max

// ❌ Bad - Very long sessions are security risks
let config = SessionConfig::redis("redis://localhost:6379")?
    .with_default_ttl(Duration::from_secs(86400 * 30)); // 30 days!
```

### 2. Store Minimal Data in Sessions

```rust
// ✅ Good - Only essential data
session.set("user_id", user.id)?;
session.set("roles", user.roles)?;

// ❌ Bad - Full user object with sensitive data
session.set("user", full_user_object)?; // Contains password hash!
```

### 3. Use Secure Session IDs

```rust
// Session IDs are automatically generated as UUIDs
// Never allow user-provided session IDs
let session = store.create(None).await?; // ID auto-generated
```

### 4. Regenerate Sessions on Privilege Changes

```rust
async fn elevate_privileges(
    store: &impl SessionStore,
    old_session_id: &str,
) -> Result<Session, Error> {
    // Get old session data
    let old_session = store.get(old_session_id).await?.unwrap();

    // Delete old session
    store.delete(old_session_id).await?;

    // Create new session with elevated privileges
    let mut new_session = store.create(None).await?;
    new_session.set("user_id", old_session.get::<i32>("user_id"))?;
    new_session.set("is_admin", true)?;
    store.save(&new_session).await?;

    Ok(new_session)
}
```

### 5. Clean Up Expired Sessions

```rust
// Run periodically (cron job)
async fn cleanup_sessions(store: &impl SessionStore) {
    match store.cleanup_expired().await {
        Ok(count) => println!("Cleaned up {} expired sessions", count),
        Err(e) => eprintln!("Session cleanup failed: {}", e),
    }
}
```

## Migration from Sessions to JWT

If you're currently using sessions and want to migrate to stateless JWT:

### Step 1: Add JWT Support

```rust
use armature_jwt::JwtManager;

let jwt_manager = JwtManager::new(JwtConfig {
    secret: "your-secret-key",
    expiration: Duration::from_secs(3600),
    ..Default::default()
})?;
```

### Step 2: Dual Authentication Period

```rust
async fn authenticate(
    req: &HttpRequest,
    session_store: &impl SessionStore,
    jwt_manager: &JwtManager,
) -> Result<UserId, Error> {
    // Try JWT first (new method)
    if let Some(token) = extract_bearer_token(req) {
        if let Ok(claims) = jwt_manager.verify_token(token) {
            return Ok(claims.user_id);
        }
    }

    // Fall back to session (legacy)
    if let Some(session_id) = req.cookie("session_id") {
        if let Some(session) = session_store.get(&session_id).await? {
            return Ok(session.get("user_id").unwrap());
        }
    }

    Err(Error::Unauthorized)
}
```

### Step 3: Issue JWT on Login

```rust
async fn login(
    credentials: LoginRequest,
    jwt_manager: &JwtManager,
) -> Result<LoginResponse, Error> {
    let user = validate_user(&credentials)?;

    // Issue JWT instead of creating session
    let token = jwt_manager.create_token(UserClaims {
        user_id: user.id,
        roles: user.roles,
    })?;

    Ok(LoginResponse { token })
}
```

### Step 4: Remove Session Support

Once all clients are migrated, remove session dependencies.

## Common Pitfalls

- ❌ **Don't** use sessions for simple authentication - use JWT
- ❌ **Don't** store sensitive data (passwords, keys) in sessions
- ❌ **Don't** use very long session TTLs
- ❌ **Don't** trust session IDs from untrusted sources
- ✅ **Do** regenerate session ID after login
- ✅ **Do** set secure cookie flags
- ✅ **Do** implement session cleanup
- ✅ **Do** consider JWT for new applications

## API Reference

### Types

| Type | Description |
|------|-------------|
| `Session` | Session data structure |
| `SessionConfig` | Configuration for session stores |
| `SessionBackend` | Enum of backend types |
| `SessionError` | Error type for session operations |
| `SessionResult<T>` | Result type alias |

### Traits

| Trait | Description |
|-------|-------------|
| `SessionStore` | Main trait for session storage backends |

### Structs

| Struct | Feature | Description |
|--------|---------|-------------|
| `RedisSessionStore` | `redis` (default) | Redis-backed sessions |
| `MemcachedSessionStore` | `memcached` | Memcached-backed sessions |
| `CouchDbSessionStore` | `couchdb` | CouchDB-backed sessions |

### Functions

| Function | Description |
|----------|-------------|
| `generate_session_id()` | Generate a new UUID session ID |

## Summary

**Key Takeaways:**

1. ⚠️ **Prefer stateless JWT authentication** - Sessions add complexity
2. Use sessions only when absolutely necessary (legacy, compliance, immediate logout)
3. Redis is the recommended backend for performance
4. Memcached requires `memcached` feature flag
5. Always set reasonable TTLs with enforced maximums
6. Store minimal data in sessions
7. Consider migrating to JWT for new applications

**Quick Reference:**

```rust
use armature_session::prelude::*;

// Redis (default)
let store = RedisSessionStore::new(
    SessionConfig::redis("redis://localhost:6379")?
).await?;

// Create session
let session = store.create(None).await?;

// Store data
session.set("user_id", 123)?;
store.save(&session).await?;

// Retrieve
if let Some(session) = store.get(&session.id).await? {
    let user_id: Option<i32> = session.get("user_id");
}

// Delete (logout)
store.delete(&session.id).await?;
```

**Remember:** If you're starting a new project, use JWT. Sessions are for legacy compatibility and specific use cases only.

