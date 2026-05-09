# Armature Macros Overview

Complete reference for all macros available in Armature.

## Three Macro Crates

Armature provides macros through three complementary crates:

### 1. armature-macro (Procedural Attributes)

**Purpose**: Decorators for routes, controllers, and dependency injection

**Type**: Proc-macro attributes (`#[decorator]`)

**Key Features**:
- HTTP method decorators (`#[get]`, `#[post]`, etc.)
- Controller and module organization
- Timeout and body limit decorators
- Cache decorators
- Injectable and DI integration

### 2. armature-macros (Declarative)

**Purpose**: Pattern-based macros for common operations

**Type**: Declarative macros (`macro_rules!`)

**Key Features**:
- Quick response creation (`ok_json!`, `created_json!`, etc.)
- Parameter extraction (`path_param!`, `query_param!`)
- Error responses (`bad_request!`, `not_found!`)
- Validation helpers (`guard!`, `validation_error!`)
- Utility macros (`json_object!`, `paginated_response!`)

### 3. armature-macros-utils (Procedural Utilities)

**Purpose**: Additional procedural macros for convenience

**Type**: Proc-macros

**Key Features**:
- Response builders (`json!`, `html!`, `text!`, `redirect!`)
- Validation macros (`validate!`, `validate_email!`)
- Model derives (`#[derive(Model)]`, `#[derive(ApiModel)]`)
- Test helpers (`test_request!`, `assert_json!`)
- Error handling (`bail!`, `ensure!`)

## Complete Macro List

### Route Decorators (armature-macro)

| Macro | Purpose | Example |
|-------|---------|---------|
| `#[get]` | GET route | `#[get("/users")]` |
| `#[post]` | POST route | `#[post("/users")]` |
| `#[put]` | PUT route | `#[put("/users/:id")]` |
| `#[delete]` | DELETE route | `#[delete("/users/:id")]` |
| `#[patch]` | PATCH route | `#[patch("/users/:id")]` |
| `#[controller]` | Controller class | `#[controller("/api")]` |
| `#[module]` | Module organization | `#[module(providers = [...])]` |
| `#[injectable]` | DI injectable | `#[injectable]` |
| `#[timeout]` | Request timeout | `#[timeout(30)]` |
| `#[body_limit]` | Body size limit | `#[body_limit("10mb")]` |
| `#[cache]` | Result caching | `#[cache(ttl = 300)]` |

### Response Macros (armature-macros)

| Macro | Status | Purpose |
|-------|--------|---------|
| `ok_json!()` | 200 | Success JSON response |
| `created_json!()` | 201 | Created JSON response |
| `json_response!()` | Custom | Custom status JSON |
| `bad_request!()` | 400 | Bad request error |
| `not_found!()` | 404 | Not found error |
| `internal_error!()` | 500 | Server error |

### Parameter Extraction (armature-macros)

| Macro | Purpose | Returns |
|-------|---------|---------|
| `path_param!(req, "id")` | Single path param | `T` (parsed) |
| `path_params!(req, "id": i64, ...)` | Multiple path params | `(T1, T2, ...)` |
| `query_param!(req, "page")` | Query parameter | `Option<T>` |
| `header!(req, "Auth")` | Header value | `Result<&String>` |

### Validation (armature-macros)

| Macro | Purpose |
|-------|---------|
| `validate!(condition)` | Validate condition |
| `validate_required!(field)` | Check required field |
| `validate_email!(email)` | Validate email format |
| `guard!(cond, msg)` | Guard with 403 error |
| `validation_error!(msg)` | Create validation error |

### Utilities (armature-macros)

| Macro | Purpose |
|-------|---------|
| `json_object!{}` | Build JSON object |
| `paginated_response!()` | Create paginated response |
| `log_error!(msg)` | Log and return error |
| `routes!{}` | Define multiple routes |

### Procedural Utilities (armature-macros-utils)

| Macro | Type | Purpose |
|-------|------|---------|
| `json!()` | Proc | JSON response |
| `html!()` | Proc | HTML response |
| `text!()` | Proc | Text response |
| `redirect!()` | Proc | Redirect response |
| `validate!()` | Proc | Validate expression |
| `bail!()` | Proc | Return with error |
| `ensure!()` | Proc | Ensure condition |
| `#[derive(Model)]` | Derive | Model traits |
| `#[derive(ApiModel)]` | Derive | API model traits |
| `#[derive(Resource)]` | Derive | Database resource |
| `test_request!()` | Proc | Create test request |
| `assert_json!()` | Proc | Assert JSON equality |
| `assert_status!()` | Proc | Assert HTTP status |

## Quick Reference by Use Case

### Building Routes

```rust
use armature_macro::{controller, get, post, put, delete};
use armature_macros::prelude::*;

#[controller("/api/users")]
pub struct UserController;

impl UserController {
    #[get("/")]
    async fn list(req: HttpRequest) -> Result<HttpResponse, Error> {
        ok_json!({ "users": [] })
    }

    #[get("/:id")]
    async fn get(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: i64 = path_param!(req, "id")?;
        ok_json!({ "id": id })
    }

    #[post("/")]
    async fn create(req: HttpRequest) -> Result<HttpResponse, Error> {
        created_json!({ "id": 1 })
    }
}
```

### Parameter Extraction

```rust
// Path parameters
let id: i64 = path_param!(req, "id")?;
let (user_id, post_id) = path_params!(req, "user_id": i64, "post_id": i64)?;

// Query parameters
let page: u32 = query_param!(req, "page").unwrap_or(1);
let limit: u32 = query_param!(req, "limit").unwrap_or(20);

// Headers
let auth: &String = header!(req, "Authorization")?;
```

### Validation

```rust
// Field validation
validate_required!(name);
validate_email!(email);
validate!(age >= 18);

// Authorization guard
guard!(user.is_admin(), "Admin required");

// Custom validation
if !is_valid(&data) {
    return validation_error!("Invalid data format");
}
```

### Error Responses

```rust
// Quick error returns
return bad_request!("Invalid input");
return not_found!("User not found");
return internal_error!("Database error");

// With formatting
return not_found!("User {} not found", user_id);
return bad_request!("Field '{}' is required", field);
```

### Testing

```rust
#[tokio::test]
async fn test_get_user() {
    let req = test_request!(GET "/users/1");
    let resp = get_user(req).await.unwrap();

    assert_status!(resp, 200);
    assert_json!(resp, { "id": 1 });
}
```

## Macro Categories Summary

### By Frequency of Use

**Very Common (Use Daily)**:
- `ok_json!()` - Success responses
- `path_param!()` - Path parameter extraction
- `#[get]`, `#[post]`, etc. - Route decorators
- `created_json!()` - Creation responses
- `bad_request!()`, `not_found!()` - Error responses

**Common (Use Regularly)**:
- `query_param!()` - Query parameters
- `guard!()` - Authorization checks
- `validate_email!()` - Email validation
- `#[controller]` - Controller organization
- `paginated_response!()` - Pagination

**Occasional (Use When Needed)**:
- `#[timeout]`, `#[body_limit]` - Request limits
- `#[cache]` - Result caching
- `json_object!{}` - JSON building
- `log_error!()` - Error logging
- `#[derive(Model)]` - Model generation

**Advanced (Special Cases)**:
- `#[module]` - Module organization
- `#[injectable]` - DI configuration
- `routes!{}` - Bulk route definition
- `#[derive(Resource)]` - Database models
- Test helpers - Testing utilities

## Code Reduction Examples

### Example 1: User Creation

**Without Macros (45 lines)**:

```rust
async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let name = req.path_params.get("name")
        .ok_or_else(|| Error::BadRequest("Missing name".to_string()))?;
    let email = req.path_params.get("email")
        .ok_or_else(|| Error::BadRequest("Missing email".to_string()))?;

    if name.is_empty() {
        return Err(Error::Validation("Name is required".to_string()));
    }

    let email_regex = regex::Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap();
    if !email_regex.is_match(email) {
        return Err(Error::Validation("Invalid email format".to_string()));
    }

    let user = match db.create_user(name, email).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to create user: {}", e);
            return Err(Error::InternalServerError("Database error".to_string()));
        }
    };

    let json = serde_json::json!({
        "id": user.id,
        "name": user.name,
        "email": user.email
    });

    let mut response = HttpResponse::created();
    response.body = serde_json::to_vec(&json)
        .map_err(|e| Error::Serialization(e.to_string()))?;
    response.headers.insert(
        "Content-Type".to_string(),
        "application/json".to_string()
    );

    Ok(response)
}
```

**With Macros (18 lines - 60% reduction)**:

```rust
async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let name: String = path_param!(req, "name")?;
    let email: String = path_param!(req, "email")?;

    validate_required!(name);
    validate_email!(email);

    let user = db.create_user(name, email).await
        .map_err(|e| log_error!("Failed to create user: {}", e))?;

    created_json!({
        "id": user.id,
        "name": user.name,
        "email": user.email
    })
}
```

### Example 2: List with Pagination

**Without Macros (30 lines)**:

```rust
async fn list_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    let page = req.query_params.get("page")
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(1);

    let limit = req.query_params.get("limit")
        .and_then(|l| l.parse::<u32>().ok())
        .unwrap_or(20)
        .min(100);

    let users = db.list_users(page, limit).await?;
    let total = db.count_users().await?;

    let json = serde_json::json!({
        "data": users,
        "pagination": {
            "page": page,
            "total": total,
            "per_page": users.len()
        }
    });

    let mut response = HttpResponse::ok();
    response.body = serde_json::to_vec(&json)?;
    response.headers.insert("Content-Type".to_string(), "application/json".to_string());

    Ok(response)
}
```

**With Macros (11 lines - 63% reduction)**:

```rust
async fn list_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    let page: u32 = query_param!(req, "page").unwrap_or(1);
    let limit: u32 = query_param!(req, "limit").unwrap_or(20).min(100);

    let users = db.list_users(page, limit).await?;
    let total = db.count_users().await?;

    paginated_response!(users, page, total)
}
```

## Performance Impact

**Compile Time**: Macros are expanded at compile time
- Zero runtime overhead
- Type checking at compile time
- Same performance as hand-written code

**Code Size**: Significant reduction
- 30-60% less boilerplate
- Better readability
- Easier maintenance

**Developer Experience**:
- Faster development
- Fewer errors
- Consistent patterns
- Better documentation

## Best Practices

### 1. Import via Prelude

```rust
// Instead of individual imports
use armature_macros::{ok_json, not_found, path_param, guard};

// Use prelude
use armature_macros::prelude::*;
```

### 2. Combine Decorators

```rust
#[timeout(30)]
#[body_limit("5mb")]
#[cache(ttl = 300)]
#[post("/process")]
async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Handler code
}
```

### 3. Type-Safe Extraction

```rust
// Explicit types for clarity
let id: i64 = path_param!(req, "id")?;
let page: u32 = query_param!(req, "page").unwrap_or(1);

// Let compiler infer when obvious
let name = path_param!(req, "name")?;  // Infers String
```

### 4. Consistent Error Handling

```rust
// Use macro error responses for consistency
match result {
    Some(data) => ok_json!(data),
    None => not_found!("Resource not found"),
}

// Not mixed styles
match result {
    Some(data) => ok_json!(data),
    None => Err(Error::NotFound("...".to_string())),  // ❌ Inconsistent
}
```

## Migration Guide

### From Manual Response Creation

**Before**:
```rust
let mut response = HttpResponse::ok();
response.body = serde_json::to_vec(&data)?;
response.headers.insert("Content-Type".to_string(), "application/json".to_string());
Ok(response)
```

**After**:
```rust
ok_json!(data)
```

### From Manual Parameter Extraction

**Before**:
```rust
let id = req.path_params.get("id")
    .ok_or_else(|| Error::BadRequest("Missing id".to_string()))?
    .parse::<i64>()
    .map_err(|_| Error::BadRequest("Invalid id".to_string()))?;
```

**After**:
```rust
let id: i64 = path_param!(req, "id")?;
```

### From Manual Validation

**Before**:
```rust
if email.is_empty() {
    return Err(Error::Validation("Email required".to_string()));
}
let email_regex = Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap();
if !email_regex.is_match(&email) {
    return Err(Error::Validation("Invalid email".to_string()));
}
```

**After**:
```rust
validate_required!(email);
validate_email!(email);
```

## Summary

Armature's macro system provides:

✅ **3 complementary macro crates**
✅ **30+ useful macros**
✅ **30-60% code reduction**
✅ **Zero runtime overhead**
✅ **Type-safe abstractions**
✅ **Consistent patterns**
✅ **Better readability**

The macros are designed to work together seamlessly, creating a powerful
and ergonomic development experience while maintaining type safety and
performance.

For detailed guides, see:
- [Macros Guide](guides/macros-guide.md)
- [armature-macros README](../armature-macros/README.md)
- [armature-macros-utils README](../armature-macros-utils/README.md)

