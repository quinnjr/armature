# Stateless Architecture

Armature is designed as a **completely stateless** framework, following RESTful and cloud-native best practices.

## Core Principles

### 1. No Server-Side Sessions

Armature does **not** implement or provide:
- Session storage mechanisms
- In-memory session management
- Session cookies
- Server-side user state persistence
- Session stores (Redis, memcached, etc.)

### 2. Stateless Authentication

All authentication is token-based and stateless:

```rust
// JWT tokens carry all user information
let token = jwt_manager.create_token(user_claims)?;

// Each request is independent
// Extract user from token on every request
let claims = jwt_manager.verify_token(&token)?;
```

**Key Points:**
- User identity is embedded in tokens (JWT)
- No server-side session lookup required
- Each request contains all necessary auth information
- Tokens can be validated without database lookups

### 3. Request Context Only

User information exists only for the duration of a single request:

```rust
#[controller("/api")]
struct UserController;

impl UserController {
    async fn get_profile(&self, req: HttpRequest) -> Result<Json<User>, Error> {
        // Extract user from JWT token in request
        let token = extract_bearer_token(&req)?;
        let claims = verify_token(token)?;

        // Use claims for this request only
        // No session storage
        Ok(Json(get_user_from_db(claims.user_id)?))
    }
}
```

## Authentication Patterns

### JWT-Based Authentication

```rust
use armature_jwt::{JwtManager, JwtConfig};

// Create token at login
let token = jwt_manager.create_token(UserClaims {
    user_id: user.id,
    email: user.email,
    roles: user.roles,
})?;

// Client stores token (localStorage, etc.)
// Client sends token with each request
// Authorization: Bearer <token>

// Server verifies token on each request
let claims = jwt_manager.verify_token(&token)?;
// Use claims.user_id, claims.roles, etc.
```

### OAuth2/OIDC Integration

OAuth2 flows are stateless using PKCE:

```rust
use armature_auth::providers::GoogleProvider;

// 1. Generate auth URL with PKCE
let (auth_url, pkce_verifier) = provider
    .authorization_url_with_pkce()
    .map_err(|e| Error::Internal(e.to_string()))?;

// 2. Store PKCE verifier client-side (NOT on server)
// Client handles the PKCE flow

// 3. Exchange code for token (stateless)
let token = provider.exchange_code_pkce(code, pkce_verifier).await?;

// 4. Create JWT from user info
let user_info = provider.get_user_info(&token).await?;
let jwt = jwt_manager.create_token(user_info)?;

// Return JWT to client
```

**Important:** CSRF/state tokens are handled client-side or embedded in redirect URLs, not stored on server.

### SAML 2.0

SAML integration is stateless:

```rust
use armature_auth::saml::SamlServiceProvider;

// 1. Generate authentication request
let authn_request = saml_provider.create_authn_request()?;

// 2. Redirect user with request embedded in URL
// No server-side state stored

// 3. Validate SAML response (stateless validation)
let assertion = saml_provider.validate_response(&saml_response)?;

// 4. Create JWT from assertion
let jwt = jwt_manager.create_token(UserClaims {
    user_id: assertion.name_id,
    email: assertion.attributes.get("email"),
    // ...
})?;

// Return JWT to client
```

## Why Stateless?

### Benefits

1. **Horizontal Scalability**
   - Any server can handle any request
   - No session affinity (sticky sessions) needed
   - Load balancing is trivial

2. **Cloud-Native**
   - Works seamlessly with containers
   - No shared state between instances
   - Perfect for Kubernetes, serverless, etc.

3. **Reliability**
   - No session store to fail
   - No session synchronization issues
   - Server restarts don't affect users

4. **Performance**
   - No session lookups
   - No database queries for auth
   - Token validation is cryptographic (fast)

5. **Security**
   - No session hijacking
   - No session fixation attacks
   - Token expiration is built-in

### Trade-offs

1. **Token Size**
   - JWTs can be larger than session IDs
   - Include only necessary claims

2. **Token Revocation**
   - Tokens are valid until expiry
   - Use short expiration times (15-60 minutes)
   - Implement token refresh flow
   - For immediate revocation, maintain token blacklist (separate concern)

3. **Client Responsibility**
   - Client must store and manage tokens
   - Client must handle token refresh

## Anti-Patterns to Avoid

### ❌ Don't Create Session Storage

```rust
// BAD - Don't do this
lazy_static! {
    static ref SESSIONS: Arc<Mutex<HashMap<String, UserSession>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

// This breaks stateless architecture
fn store_session(session_id: String, user: User) {
    let mut sessions = SESSIONS.lock().unwrap();
    sessions.insert(session_id, UserSession { user });
}
```

### ❌ Don't Cache User Data Server-Side

```rust
// BAD - Don't do this
lazy_static! {
    static ref USER_CACHE: Arc<Mutex<HashMap<String, User>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

// This creates state
fn cache_user(user_id: String, user: User) {
    let mut cache = USER_CACHE.lock().unwrap();
    cache.insert(user_id, user);
}
```

### ❌ Don't Store Request-Specific State

```rust
// BAD - Don't do this
static mut CURRENT_USER: Option<User> = None;

// This breaks concurrent requests
fn set_current_user(user: User) {
    unsafe {
        CURRENT_USER = Some(user);
    }
}
```

## Correct Patterns

### ✅ Extract User from Token Each Request

```rust
// GOOD - Stateless
async fn get_user(req: HttpRequest) -> Result<User, Error> {
    let token = extract_bearer_token(&req)?;
    let claims = jwt_manager.verify_token(token)?;

    // Optionally fetch from database
    // (database is external state, not server state)
    let user = database.find_user(&claims.user_id).await?;

    Ok(user)
}
```

### ✅ Use Guards for Auth

```rust
// GOOD - Validates on each request
use armature_framework::{Guard, GuardContext};

pub struct AuthenticationGuard;

#[async_trait]
impl Guard for AuthenticationGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        let header = context.get_header("authorization")
            .ok_or_else(|| Error::Forbidden("Missing auth".to_string()))?;

        let token = extract_bearer_token(header)?;
        let _claims = jwt_manager.verify_token(token)?;

        // Token is valid
        Ok(true)
    }
}
```

### ✅ Use Middleware for Request Context

```rust
// GOOD - Attach user to request (not persistent)
pub struct AuthMiddleware {
    jwt_manager: JwtManager,
}

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        if let Some(auth_header) = req.headers.get("authorization") {
            if let Ok(token) = extract_bearer_token(auth_header) {
                if let Ok(claims) = self.jwt_manager.verify_token(token) {
                    // Add to request headers (request-scoped only)
                    req.headers.insert("x-user-id".to_string(), claims.sub);
                }
            }
        }

        next(req).await
    }
}
```

## Token Refresh Pattern

For long-lived sessions without server-side state:

```rust
#[derive(Serialize)]
struct TokenPair {
    access_token: String,  // Short-lived (15-60 min)
    refresh_token: String, // Long-lived (7-30 days)
}

// Login: return both tokens
let access_token = jwt_manager.create_token(user_claims)?;
let refresh_token = jwt_manager.create_refresh_token(user_claims)?;

// When access token expires:
// Client calls /auth/refresh with refresh_token
// Server validates refresh_token
// Server issues new access_token
// No session lookup needed
```

## Rate Limiting (Stateless)

Even rate limiting can be stateless using distributed stores:

```rust
// Use external store (Redis, etc.) not in-memory
use redis::AsyncCommands;

async fn check_rate_limit(ip: &str) -> Result<bool, Error> {
    let mut conn = redis_client.get_async_connection().await?;
    let key = format!("rate:{}",  ip);

    let count: u32 = conn.get(&key).await.unwrap_or(0);

    if count > 100 {
        return Err(Error::TooManyRequests("Rate limit exceeded".into()));
    }

    conn.incr(&key, 1).await?;
    conn.expire(&key, 60).await?; // 60 seconds

    Ok(true)
}
```

**Note:** This uses external state (Redis), not server state. Any server instance can check the rate limit.

## WebSockets and SSE

Even real-time features remain stateless:

```rust
// WebSocket connections are ephemeral
// No user state persists beyond connection lifetime

async fn handle_websocket(req: HttpRequest) -> Result<(), Error> {
    // Authenticate via token in initial request
    let token = req.query_params.get("token")
        .ok_or_else(|| Error::Forbidden("Missing token".into()))?;

    let claims = jwt_manager.verify_token(token)?;

    // Connection is scoped to this handler
    // When connection closes, all state is gone

    websocket_upgrade(req, move |socket| async move {
        // Handle messages
        // User info from claims (not stored globally)
    }).await
}
```

## Deployment Considerations

### Multiple Instances

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│ Server 1 │     │ Server 2 │     │ Server 3 │
│ (stateless)    │ (stateless)    │ (stateless)
└────┬─────┘     └────┬─────┘     └────┬─────┘
     │                │                │
     └────────────────┴────────────────┘
                      │
              ┌───────▼────────┐
              │  Load Balancer │
              └───────┬────────┘
                      │
              ┌───────▼────────┐
              │     Client      │
              │  (stores JWT)   │
              └─────────────────┘
```

Each server instance:
- Can handle any request
- Validates JWT independently
- No shared state needed
- No session synchronization

### Container/Kubernetes Friendly

```yaml
# Perfect for Kubernetes
apiVersion: apps/v1
kind: Deployment
metadata:
  name: armature-app
spec:
  replicas: 10  # Scale freely
  template:
    spec:
      containers:
      - name: app
        image: myapp:latest
        # No volume mounts for sessions
        # No sticky sessions needed
```

## Summary

Armature enforces stateless architecture by:

1. **No session framework** - Not provided, not supported
2. **JWT-based auth** - All user context in tokens
3. **Request-scoped data** - No persistence between requests
4. **External stores only** - Database, cache (Redis) are external, not in-server memory
5. **Cloud-native design** - Horizontal scaling without shared state

This design ensures your Armature application can:
- Scale horizontally with ease
- Deploy anywhere (containers, serverless, VMs)
- Handle millions of requests across multiple instances
- Recover from failures without user impact
- Maintain security without session vulnerabilities

**Remember:** If you need to "remember" something about a user, put it in the JWT or look it up from a database on each request. Never store it in server memory.

