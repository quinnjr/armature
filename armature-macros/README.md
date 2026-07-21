# Armature Macros

Declarative macros for the Armature framework that reduce boilerplate and improve code readability.

## Features

- ✅ **Response Macros** - Quick HTTP response creation
- ✅ **Parameter Extraction** - Type-safe param extraction
- ✅ **Error Handling** - Consistent error responses
- ✅ **Utilities** - JSON building, pagination, logging

> Looking for `validate!`, `validate_required!`, or `validate_email!`? Those
> are proc-macro attribute/derive-style validation helpers and live in
> [`armature-macros-utils`](../armature-macros-utils), not in this crate.

## Installation

```toml
[dependencies]
armature-macros = { path = "../armature-macros" }
```

## Quick Start

```rust
use armature_macros::prelude::*;

#[get("/users/:id")]
async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Extract path param with automatic parsing
    let id: i64 = path_param!(req, "id")?;

    // Query database
    match db.find_user(id).await {
        Some(user) => ok_json!(user),  // 200 OK with JSON
        None => not_found!("User {} not found", id),  // 404 error
    }
}
```

## Response Macros

### Success Responses

```rust
// 200 OK JSON
ok_json!({ "message": "Success", "data": data })

// 201 Created JSON
created_json!({ "id": new_id })

// Custom status JSON
json_response!(202, { "status": "processing" })
```

### Error Responses

```rust
// 400 Bad Request
bad_request!("Invalid input")
bad_request!("Field '{}' is invalid", field_name)

// 404 Not Found
not_found!("Resource not found")
not_found!("User {} not found", user_id)

// 500 Internal Server Error
internal_error!("Database connection failed")
```

## Parameter Extraction

### Path Parameters

```rust
// Single parameter
let id: i64 = path_param!(req, "id")?;

// Multiple parameters (the macro's own expansion already resolves to the
// parsed tuple — no trailing `?` needed, unlike the single-value
// `path_param!`, whose expansion is `Result<T, Error>`)
let (user_id, post_id) = path_params!(req, "user_id": i64, "post_id": i64);
```

### Query Parameters

```rust
// With default value
let page: u32 = query_param!(req, "page").unwrap_or(1);
let limit: u32 = query_param!(req, "limit").unwrap_or(20);

// Optional
let filter: Option<String> = query_param!(req, "filter");
```

### Headers

```rust
// Required header (returns error if missing)
let auth: &String = header!(req, "Authorization")?;

// Optional header
let content_type = header!(req, "Content-Type")
    .unwrap_or(&"text/plain".to_string());
```

## Validation

For field-level validation (`validate!`, `validate_required!`, `validate_email!`),
see [`armature-macros-utils`](../armature-macros-utils). `armature-macros` itself
provides two lightweight condition-checking macros:

```rust
// Guard condition (returns 403 if false)
guard!(user.is_admin(), "Admin access required");

// Validation error
if !is_valid(data) {
    return validation_error!("Data validation failed");
}
```

## Utilities

### JSON Object Builder

```rust
let response = json_object! {
    "id" => user.id,
    "name" => user.name,
    "email" => user.email,
    "active" => user.is_active,
};
```

### Paginated Response

```rust
let users = db.list_users(page, limit).await?;
let total = db.count_users().await?;

paginated_response!(users, page, total)
```

### Error Logging

```rust
match db.query().await {
    Ok(data) => ok_json!(data),
    Err(e) => log_error!("Database query failed: {}", e),
}
```

## Complete Example

```rust
use armature_macro::{controller, get, post, put, delete};
use armature_macros::prelude::*;

#[controller("/api/users")]
pub struct UserController;

impl UserController {
    #[get("/")]
    async fn list(req: HttpRequest) -> Result<HttpResponse, Error> {
        let page: u32 = query_param!(req, "page").unwrap_or(1);
        let limit: u32 = query_param!(req, "limit").unwrap_or(20).min(100);

        let users = db.list_users(page, limit).await?;
        let total = db.count_users().await?;

        paginated_response!(users, page, total)
    }

    #[get("/:id")]
    async fn get(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: i64 = path_param!(req, "id")?;

        match db.find_user(id).await {
            Some(user) => ok_json!(user),
            None => not_found!("User {} not found", id),
        }
    }

    #[post("/")]
    async fn create(req: HttpRequest) -> Result<HttpResponse, Error> {
        let name: String = extract_field(&req, "name")?;
        let email: String = extract_field(&req, "email")?;

        guard!(!name.is_empty(), "Name is required");
        guard!(email.contains('@'), "Invalid email format");

        let user = db.create_user(name, email).await?;
        created_json!({ "user": user })
    }

    #[put("/:id")]
    async fn update(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: i64 = path_param!(req, "id")?;
        let user = get_current_user(&req).await?;

        guard!(user.id == id || user.is_admin(), "Unauthorized");

        // Update user...
        ok_json!({ "message": "User updated" })
    }

    #[delete("/:id")]
    async fn delete(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id: i64 = path_param!(req, "id")?;
        let user = get_current_user(&req).await?;

        guard!(user.is_admin(), "Admin access required");

        db.delete_user(id).await?;
        Ok(HttpResponse::no_content())
    }
}
```

## Documentation

See the [Macros Guide](../../docs/guides/macros-guide.md) for comprehensive documentation.

## License

MIT OR Apache-2.0

