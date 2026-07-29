# Armature Macros Utils

Procedural utility macros for the Armature framework.

## Features

- ✅ **Response Builders** - Procedural macros for responses
- ✅ **Validation Helpers** - Compile-time validation macros
- ✅ **Model Derives** - Auto-implement common traits
- ✅ **Test Helpers** - Testing utility macros
- ✅ **Error Handling** - Bail and ensure macros

## Installation

```toml
[dependencies]
armature-macros-utils = { path = "../armature-macros-utils" }
```

## Procedural Macros

### Response Macros

```rust
use armature_macros_utils::{json, html, text, redirect};

// JSON response
json!({ "message": "Hello" })

// HTML response
html!("<h1>Welcome</h1>")

// Text response
text!("Hello, world!")

// Redirect
redirect!("/new-location")
```

### Validation Macros

```rust
use armature_macros_utils::{validate, validate_required, validate_email};

// Conditional validation with a custom message
validate!(age >= 18, "Must be 18 or older");

// Custom validator function: calls is_valid_email(&email)
validate!(email, is_valid_email, "Invalid email format");

// Required field validation (one or more Option fields; names the missing one)
validate_required!(name, email, password);

// Email validation
validate_email!(user_email);
```

> `validate_email!` expands to code using `regex::Regex`, so the calling crate
> must depend on the `regex` crate. The regex is compiled once via a
> `static LazyLock`.

### Error Handling

```rust
use armature_macros_utils::{bail, ensure};

// Return early with error
if !found {
    bail!("User not found");
}

// Ensure condition or error
ensure!(user.is_active(), "User account is inactive");
```

### Model Derives

`#[derive(Model)]` supplies **field-wise `Debug` + `Clone`** and a
`Default`-bounded `new()`. It does **not** implement `Serialize`/`Deserialize`
(a derive macro cannot attach other derives) — add those yourself.
`#[derive(Resource)]` exposes table metadata (`table_name()`/`primary_key()`),
not CRUD queries.

```rust
use armature_macros_utils::{Model, ApiModel, Resource};
use serde::{Serialize, Deserialize};

// Basic model: Model gives Debug + Clone + new(); you add Default + serde.
#[derive(Model, Default, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub name: String,
}

// API model with field visibility
#[derive(ApiModel, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: i64,
    pub name: String,
    #[api(skip)]
    pub password_hash: String,  // Excluded from API
}

// Resource model for database
#[derive(Resource, Serialize, Deserialize)]
#[resource(table = "users")]
pub struct UserEntity {
    #[resource(primary_key)]
    pub id: i64,
    pub name: String,
}
```

### Test Helpers

```rust
use armature_macros_utils::{test_request, assert_json, assert_status};

#[tokio::test]
async fn test_endpoint() {
    let req = test_request!(GET "/users/1");
    let resp = handler(req).await.unwrap();

    assert_status!(resp, 200);
    assert_json!(resp, { "id": 1, "name": "Alice" });
}
```

## Use with armature-macros

These crates work together:

```rust
use armature_proc_macro::get;

// Declarative macros
use armature_macros::prelude::*;

// Procedural macros
use armature_macros_utils::*;

#[get("/users/:id")]
async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id: i64 = path_param!(req, "id")?;  // From armature-macros

    match db.find(id).await {
        Some(user) => ok_json!(user),  // From armature-macros
        None => not_found!("User not found"),  // From armature-macros
    }
}
```

## Documentation

See the [Macros Guide](../../docs/guides/macros-guide.md) for comprehensive documentation.

## License

MIT OR Apache-2.0

