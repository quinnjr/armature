# Use Guard Decorator Guide

Armature provides the `#[use_guard]` and `#[guard]` decorators for protecting routes with authorization checks. Guards determine whether a request should be allowed to proceed before the handler executes.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Guards vs Middleware](#guards-vs-middleware)
- [Basic Usage](#basic-usage)
- [Multiple Guards](#multiple-guards)
- [Guard with Configuration](#guard-with-configuration)
- [Controller-Level Guards](#controller-level-guards)
- [Built-in Guards](#built-in-guards)
- [Custom Guards](#custom-guards)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

Guards are authorization checks that run before a route handler. They answer the question: "Should this request be allowed to proceed?"

- **Return `Ok(true)`** → Request proceeds to handler
- **Return `Ok(false)`** → Request is denied (403 Forbidden)
- **Return `Err(e)`** → Request fails with the error

## Features

- ✅ Route-level guard protection
- ✅ Controller-level guard inheritance
- ✅ Multiple guard chaining (all must pass)
- ✅ Type-safe guard configuration
- ✅ Access to request context (headers, params)
- ✅ Works with all HTTP method decorators

## Guards vs Middleware

| Aspect | Guards | Middleware |
|--------|--------|------------|
| Purpose | Authorization (allow/deny) | Request/Response processing |
| Return | `Result<bool, Error>` | `Result<HttpResponse, Error>` |
| Response modification | No | Yes |
| Short-circuit on deny | Yes | Optional |
| Use case | Auth, permissions, rate limiting | Logging, CORS, compression |

**Use guards when** you need to allow or deny access.
**Use middleware when** you need to process requests/responses.

## Basic Usage

### Simple Type-Based Guard

When your guard implements `Default`:

```rust
use armature_framework::{get, use_guard};
use armature_core::{HttpRequest, HttpResponse, Error, guard::AuthenticationGuard};

#[use_guard(AuthenticationGuard)]
#[get("/protected")]
async fn protected_endpoint(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok().with_json(&serde_json::json!({
        "message": "You have access!"
    }))?)
}
```

### Guard Order

Guards are checked in order. If any guard fails, subsequent guards are not checked:

```rust
#[use_guard(AuthenticationGuard, AdminGuard)]
#[get("/admin")]
async fn admin_endpoint(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Only reaches here if BOTH guards pass
    Ok(HttpResponse::ok())
}
```

## Multiple Guards

Chain multiple guards - all must pass:

```rust
use armature_framework::{get, use_guard};
use armature_core::{
    HttpRequest, HttpResponse, Error,
    guard::{AuthenticationGuard, RolesGuard}
};

#[derive(Default)]
struct PremiumGuard;

#[async_trait::async_trait]
impl armature_core::guard::Guard for PremiumGuard {
    async fn can_activate(
        &self,
        context: &armature_core::guard::GuardContext
    ) -> Result<bool, Error> {
        // Check if user has premium subscription
        Ok(context.get_header("x-premium-user").is_some())
    }
}

#[use_guard(AuthenticationGuard, PremiumGuard)]
#[get("/premium-content")]
async fn premium_content(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

## Guard with Configuration

Use `#[guard(...)]` for guards that need configuration:

```rust
use armature_framework::{get, guard};
use armature_core::{HttpRequest, HttpResponse, Error, guard::ApiKeyGuard};

#[guard(ApiKeyGuard::new(vec!["secret-key-1".into(), "secret-key-2".into()]))]
#[get("/api/data")]
async fn api_data(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

### Combining Type and Instance Guards

```rust
use armature_framework::{get, guard};
use armature_core::{
    HttpRequest, HttpResponse, Error,
    guard::{AuthenticationGuard, RolesGuard}
};

#[guard(
    AuthenticationGuard,
    RolesGuard::new(vec!["admin".into(), "moderator".into()])
)]
#[get("/moderation")]
async fn moderation_panel(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

## Controller-Level Guards

Apply guards to all routes in a controller:

```rust
use armature_framework::{controller, get, post, guard};
use armature_core::{HttpRequest, HttpResponse, Error, guard::AuthenticationGuard};

#[guard(AuthenticationGuard)]
#[controller("/api/users")]
struct UserController;

impl UserController {
    // All routes require authentication

    #[get("")]
    async fn list_users(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[get("/:id")]
    async fn get_user(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[post("")]
    async fn create_user(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::created())
    }
}
```

### Combining Controller and Route Guards

Route guards add to controller guards:

```rust
use armature_framework::{controller, get, guard, use_guard};
use armature_core::{
    HttpRequest, HttpResponse, Error,
    guard::AuthenticationGuard
};

#[derive(Default)]
struct AdminGuard;

#[async_trait::async_trait]
impl armature_core::guard::Guard for AdminGuard {
    async fn can_activate(
        &self,
        context: &armature_core::guard::GuardContext
    ) -> Result<bool, Error> {
        // Check for admin role
        Ok(context.get_header("x-user-role")
            .map(|r| r == "admin")
            .unwrap_or(false))
    }
}

#[guard(AuthenticationGuard)]  // All routes require auth
#[controller("/api")]
struct ApiController;

impl ApiController {
    #[get("/public")]
    async fn public_data(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        // Only needs AuthenticationGuard
        Ok(HttpResponse::ok())
    }

    #[use_guard(AdminGuard)]  // Additional guard
    #[get("/admin")]
    async fn admin_data(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        // Needs AuthenticationGuard + AdminGuard
        Ok(HttpResponse::ok())
    }
}
```

## Built-in Guards

Armature provides several built-in guards:

| Guard | Purpose | Default |
|-------|---------|---------|
| `AuthenticationGuard` | Check for Bearer token | ✅ |
| `RolesGuard` | Check user roles | ❌ (needs roles) |
| `ApiKeyGuard` | Validate API keys | ❌ (needs keys) |
| `CustomGuard<F>` | Custom predicate | ❌ (needs fn) |

### AuthenticationGuard

Checks for `Authorization: Bearer <token>` header:

```rust
#[use_guard(AuthenticationGuard)]
#[get("/me")]
async fn get_current_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let token = req.headers.get("authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap(); // Safe after guard passes

    Ok(HttpResponse::ok())
}
```

### RolesGuard

Checks for specific roles (requires role extraction logic):

```rust
#[guard(RolesGuard::new(vec!["admin".into()]))]
#[get("/admin/settings")]
async fn admin_settings(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

### ApiKeyGuard

Validates `X-API-Key` header:

```rust
#[guard(ApiKeyGuard::new(vec![
    "key-production".into(),
    "key-staging".into()
]))]
#[get("/api/v1/data")]
async fn api_data(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

## Custom Guards

Create custom guards by implementing the `Guard` trait:

```rust
use armature_core::{Error, guard::{Guard, GuardContext}};
use async_trait::async_trait;

/// Rate limiting guard
pub struct RateLimitGuard {
    max_requests: u32,
}

impl RateLimitGuard {
    pub fn new(max_requests: u32) -> Self {
        Self { max_requests }
    }
}

#[async_trait]
impl Guard for RateLimitGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        // Get client identifier (IP, API key, etc.)
        let client_id = context.get_header("x-client-id")
            .cloned()
            .unwrap_or_else(|| "anonymous".into());

        // Check rate limit (implement your logic)
        let request_count = get_request_count(&client_id).await;

        if request_count >= self.max_requests {
            Err(Error::TooManyRequests(format!(
                "Rate limit exceeded: {} requests",
                self.max_requests
            )))
        } else {
            increment_request_count(&client_id).await;
            Ok(true)
        }
    }
}
```

### Guard with Request Data Access

Guards have full access to the request context:

```rust
pub struct OwnershipGuard;

impl Default for OwnershipGuard {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Guard for OwnershipGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        // Access path parameters
        let resource_id = context.get_param("id")
            .ok_or_else(|| Error::BadRequest("Missing resource ID".into()))?;

        // Access headers
        let user_id = context.get_header("x-user-id")
            .ok_or_else(|| Error::Unauthorized("Missing user ID".into()))?;

        // Check ownership
        let resource = fetch_resource(resource_id).await?;

        if resource.owner_id == *user_id {
            Ok(true)
        } else {
            Err(Error::Forbidden("You don't own this resource".into()))
        }
    }
}
```

### IP Whitelist Guard

```rust
pub struct IpWhitelistGuard {
    allowed_ips: Vec<String>,
}

impl IpWhitelistGuard {
    pub fn new(ips: Vec<String>) -> Self {
        Self { allowed_ips: ips }
    }
}

#[async_trait]
impl Guard for IpWhitelistGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        let client_ip = context.get_header("x-forwarded-for")
            .or_else(|| context.get_header("x-real-ip"))
            .ok_or_else(|| Error::BadRequest("Cannot determine client IP".into()))?;

        if self.allowed_ips.contains(client_ip) {
            Ok(true)
        } else {
            Err(Error::Forbidden(format!(
                "IP {} not in whitelist",
                client_ip
            )))
        }
    }
}
```

### Time-Based Guard

```rust
use chrono::{Local, Timelike};

pub struct BusinessHoursGuard;

impl Default for BusinessHoursGuard {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Guard for BusinessHoursGuard {
    async fn can_activate(&self, _context: &GuardContext) -> Result<bool, Error> {
        let now = Local::now();
        let hour = now.hour();

        // Allow access only during business hours (9 AM - 5 PM)
        if hour >= 9 && hour < 17 {
            Ok(true)
        } else {
            Err(Error::Forbidden(
                "This endpoint is only available during business hours".into()
            ))
        }
    }
}
```

## Best Practices

### 1. Keep Guards Focused

Each guard should check one thing:

```rust
// ✅ Good: Single responsibility
pub struct AuthenticationGuard;  // Only checks auth
pub struct AdminRoleGuard;       // Only checks admin role
pub struct ResourceOwnerGuard;   // Only checks ownership

// ❌ Bad: Multiple responsibilities
pub struct EverythingGuard;  // Checks auth + role + ownership + ...
```

### 2. Use Descriptive Errors

```rust
// ✅ Good: Helpful error messages
Err(Error::Forbidden("Admin role required to access this resource".into()))
Err(Error::Unauthorized("API key expired".into()))

// ❌ Bad: Generic errors
Err(Error::Forbidden("Access denied".into()))
```

### 3. Order Guards Logically

```rust
// ✅ Good: Check auth before permissions
#[use_guard(AuthenticationGuard, AdminGuard)]

// ❌ Bad: Checking permissions before auth
#[use_guard(AdminGuard, AuthenticationGuard)]
```

### 4. Implement Default for Simple Guards

```rust
// ✅ Good: Allows #[use_guard(MyGuard)]
impl Default for MyGuard {
    fn default() -> Self {
        Self
    }
}

// Then use as:
#[use_guard(MyGuard)]
```

### 5. Use Configuration for Complex Guards

```rust
// ✅ Good: Use #[guard(...)] for configured guards
#[guard(RolesGuard::new(vec!["admin".into()]))]

// Instead of:
// #[use_guard(RolesGuard)]  // Won't work without Default
```

## Common Pitfalls

### ❌ Guard Without Default

```rust
// This will fail if RolesGuard doesn't implement Default
#[use_guard(RolesGuard)]  // Error!

// Use #[guard(...)] instead for configured guards
#[guard(RolesGuard::new(vec!["admin".into()]))]  // Works!
```

### ❌ Returning `Ok(false)` Without Error

```rust
// Bad: Returns generic 403
async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
    Ok(false)  // User gets "Access denied by guard"
}

// Good: Return specific error
async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
    Err(Error::Forbidden("Premium subscription required".into()))
}
```

### ❌ Heavy Operations in Guards

```rust
// Bad: Database query on every request
async fn can_activate(&self, ctx: &GuardContext) -> Result<bool, Error> {
    let user = db.fetch_user_with_all_relations().await?;  // Expensive!
    Ok(user.has_permission("admin"))
}

// Good: Use cached data or lightweight checks
async fn can_activate(&self, ctx: &GuardContext) -> Result<bool, Error> {
    // Check JWT claims instead of database
    let role = ctx.get_header("x-user-role");
    Ok(role == Some(&"admin".into()))
}
```

## API Reference

### Decorators

| Decorator | Target | Description |
|-----------|--------|-------------|
| `#[use_guard(Type, ...)]` | Function | Apply guards by type (requires Default) |
| `#[guard(expr, ...)]` | Function/Struct | Apply guard instances with configuration |

### GuardContext Methods

| Method | Description |
|--------|-------------|
| `get_header(name)` | Get request header by name |
| `get_param(name)` | Get path parameter by name |
| `request` | Access full `HttpRequest` |

### Built-in Guard Types

| Type | Constructor | Description |
|------|-------------|-------------|
| `AuthenticationGuard` | (unit struct) | Bearer token check |
| `RolesGuard` | `new(roles: Vec<String>)` | Role-based access |
| `ApiKeyGuard` | `new(keys: Vec<String>)` | API key validation |
| `CustomGuard<F>` | `new(predicate)` | Custom predicate |

## Summary

**Key Points:**

1. **`#[use_guard(Type)]`** for guards implementing `Default`
2. **`#[guard(expr)]`** for guards with configuration
3. Guards return `Result<bool, Error>`:
   - `Ok(true)` → Allow
   - `Ok(false)` → Deny (generic 403)
   - `Err(e)` → Deny with specific error
4. All guards must pass (AND logic)
5. Guards run in order; first failure stops execution
6. Use guards for authorization, middleware for processing

**Quick Reference:**

```rust
// Simple guard (Default required)
#[use_guard(AuthenticationGuard)]
#[get("/protected")]
async fn protected(req: HttpRequest) -> Result<HttpResponse, Error> { ... }

// Configured guard
#[guard(ApiKeyGuard::new(vec!["key1".into()]))]
#[get("/api")]
async fn api(req: HttpRequest) -> Result<HttpResponse, Error> { ... }

// Multiple guards
#[use_guard(AuthenticationGuard, AdminGuard)]
#[get("/admin")]
async fn admin(req: HttpRequest) -> Result<HttpResponse, Error> { ... }

// Controller-level guard
#[guard(AuthenticationGuard)]
#[controller("/api")]
struct ApiController;
```


