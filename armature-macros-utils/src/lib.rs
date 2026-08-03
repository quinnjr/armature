//! Utility macros for the Armature framework
//!
//! This crate provides convenient macros to make working with Armature easier
//! and reduce boilerplate code.
//!
//! ## Response Macros
//!
//! Quick HTTP response creation:
//! - `json!` - Create JSON responses
//! - `html!` - Create HTML responses
//! - `text!` - Create plain text responses
//!
//! ## Validation Macros
//!
//! Input validation helpers:
//! - `validate!` - Validate fields with custom rules
//! - `validate_required!` - Check required fields
//! - `validate_email!` - Validate email format
//!
//! ## Test Macros
//!
//! Testing utilities:
//! - `test_request!` - Create test HTTP requests
//! - `assert_json!` - Assert JSON response equality
//! - `assert_status!` - Assert HTTP status codes
//!
//! ## Model Macros
//!
//! Database model helpers:
//! - `#[derive(Model)]` - field-wise `Debug` + `Clone` and a `Default`-bounded
//!   `new()` (add serde derives yourself)
//! - `#[derive(ApiModel)]` - `to_json`/`from_json`, honoring `#[api(skip)]`
//! - `#[derive(Resource)]` - table metadata (`table_name()`/`primary_key()`)
//!
//! ## Error Handling
//!
//! Quick error creation:
//! - `bail!` - Return early with an error
//! - `ensure!` - Conditional error return

use proc_macro::TokenStream;

mod error_helpers;
mod model;
mod response;
mod test_helpers;
mod validation;

// ============================================================================
// Response Macros
// ============================================================================

/// Create a JSON response with automatic serialization.
///
/// The value is any `serde::Serialize` expression. An optional leading status
/// (numeric literal, `u16` expression, or the `ok` alias) selects the response
/// status; it defaults to `200`. Expands to `Result<HttpResponse, Error>`.
///
/// Requires the caller to depend on `armature-core` (and `serde_json` if you
/// build the value with `serde_json::json!`).
///
/// # Examples
///
/// ```ignore
/// // Defaults to 200 OK.
/// json!(serde_json::json!({ "items": items }))
///
/// // Explicit numeric status.
/// json!(201, serde_json::json!({ "id": 123 }))
///
/// // `ok` status alias.
/// json!(ok, user_data)
/// ```
#[proc_macro]
pub fn json(input: TokenStream) -> TokenStream {
    response::json_impl(input)
}

/// Create an HTML response
///
/// # Examples
///
/// ```ignore
/// html!(200, "<h1>Hello</h1>")
/// html!(ok, "<p>Content</p>")
/// html!("<html>...</html>")
/// ```
#[proc_macro]
pub fn html(input: TokenStream) -> TokenStream {
    response::html_impl(input)
}

/// Create a plain text response
///
/// # Examples
///
/// ```ignore
/// text!(200, "Plain text content")
/// text!(ok, "Success")
/// text!("Hello, world!")
/// ```
#[proc_macro]
pub fn text(input: TokenStream) -> TokenStream {
    response::text_impl(input)
}

/// Create a redirect response
///
/// # Examples
///
/// ```ignore
/// redirect!("/home")
/// redirect!(301, "/new-location")
/// redirect!(temporary, "/temp")
/// ```
#[proc_macro]
pub fn redirect(input: TokenStream) -> TokenStream {
    response::redirect_impl(input)
}

// ============================================================================
// Validation Macros
// ============================================================================

/// Validate a condition, returning `Error::Validation` with the caller's
/// message when it fails.
///
/// Supported forms:
/// - `validate!(condition)` — generic message.
/// - `validate!(condition, "message")` — custom message.
/// - `validate!(value, validator_fn, "message")` — calls `validator_fn(&value)`.
///
/// Used inside a function returning `Result<_, armature_core::Error>`.
///
/// # Examples
///
/// ```ignore
/// validate!(age >= 18, "Must be 18 or older");
/// validate!(password.len() >= 8, "Password too short");
/// validate!(email, is_valid_email, "Invalid email format");
/// ```
#[proc_macro]
pub fn validate(input: TokenStream) -> TokenStream {
    validation::validate_impl(input)
}

/// Validate that required fields are present
///
/// # Examples
///
/// ```ignore
/// validate_required!(name, email, password);
/// ```
#[proc_macro]
pub fn validate_required(input: TokenStream) -> TokenStream {
    validation::validate_required_impl(input)
}

/// Validate email format against a compiled-once regex.
///
/// Returns `Error::Validation` when the value does not match. The regex is
/// stored in a `static LazyLock`, so it is compiled a single time rather than
/// on every call.
///
/// **Dependency:** the expansion references `regex::Regex`, so the calling
/// crate must depend on the `regex` crate.
///
/// # Examples
///
/// ```ignore
/// validate_email!(user_email);
/// ```
#[proc_macro]
pub fn validate_email(input: TokenStream) -> TokenStream {
    validation::validate_email_impl(input)
}

// ============================================================================
// Test Helper Macros
// ============================================================================

/// Create a test HTTP request
///
/// # Examples
///
/// The body (when present) is serialized to JSON via `serde_json`, and the
/// `Content-Type` defaults to `application/json` (overridable via `headers`).
///
/// ```ignore
/// let req = test_request!(GET "/users");
/// let req = test_request!(POST "/users", serde_json::json!({ "name": "Alice" }));
/// let req = test_request!(GET "/users/123", headers: { "Authorization": "Bearer token" });
/// ```
#[proc_macro]
pub fn test_request(input: TokenStream) -> TokenStream {
    test_helpers::test_request_impl(input)
}

/// Assert JSON response equality
///
/// # Examples
///
/// ```ignore
/// assert_json!(response, { "id": 1, "name": "Alice" });
/// ```
#[proc_macro]
pub fn assert_json(input: TokenStream) -> TokenStream {
    test_helpers::assert_json_impl(input)
}

/// Assert HTTP status code
///
/// # Examples
///
/// ```ignore
/// assert_status!(response, 200);
/// assert_status!(response, ok);
/// ```
#[proc_macro]
pub fn assert_status(input: TokenStream) -> TokenStream {
    test_helpers::assert_status_impl(input)
}

// ============================================================================
// Error Handling Macros
// ============================================================================

/// Return early with an error
///
/// # Examples
///
/// ```ignore
/// bail!("User not found");
/// bail!(NotFound, "User {} not found", id);
/// ```
#[proc_macro]
pub fn bail(input: TokenStream) -> TokenStream {
    error_helpers::bail_impl(input)
}

/// Ensure a condition is true, otherwise return an error
///
/// # Examples
///
/// ```ignore
/// ensure!(user.is_active(), "User account is inactive");
/// ensure!(age >= 18, BadRequest, "Must be 18 or older");
/// ```
#[proc_macro]
pub fn ensure(input: TokenStream) -> TokenStream {
    error_helpers::ensure_impl(input)
}

// ============================================================================
// Model Derive Macros
// ============================================================================

/// Derive common model traits.
///
/// Implements `Debug` and `Clone` field-wise, and adds a `new()` constructor
/// bounded on `Self: Default`.
///
/// This macro does **not** implement `Serialize`/`Deserialize` — a derive
/// macro cannot make a type derive *other* traits. Add those derives yourself:
///
/// ```ignore
/// use serde::{Serialize, Deserialize};
///
/// // `Model` supplies Debug + Clone + new(); you add Default + serde.
/// #[derive(Model, Default, Serialize, Deserialize)]
/// pub struct User {
///     pub id: i64,
///     pub name: String,
///     pub email: String,
/// }
///
/// let user = User::new(); // requires Default
/// ```
///
/// The generated `Debug`/`Clone` impls do **not** add per-generic-parameter
/// bounds (unlike `#[derive(Debug, Clone)]`, which bounds every type
/// parameter): a generic struct needs its own explicit `T: Clone + Debug`
/// bounds (e.g. on the struct or via a `where` clause) for the emitted impls
/// to compile.
#[proc_macro_derive(Model, attributes(model))]
pub fn derive_model(input: TokenStream) -> TokenStream {
    model::derive_model_impl(input)
}

/// Derive API model traits with field visibility control
///
/// # Examples
///
/// ```ignore
/// #[derive(ApiModel)]
/// pub struct User {
///     pub id: i64,
///     pub name: String,
///     #[api(skip)]
///     pub password: String,
/// }
/// ```
///
/// `skip` is the only accepted `api` option, and it is a bare flag. Anything
/// else (`#[api(skipp)]`, `#[api(skip = true)]`) is a compile error — the
/// attribute exists to redact secrets, so a typo must not quietly leave the
/// field in the payload.
#[proc_macro_derive(ApiModel, attributes(api))]
pub fn derive_api_model(input: TokenStream) -> TokenStream {
    model::derive_api_model_impl(input)
}

/// Derive resource table metadata.
///
/// Parses `#[resource(table = "..")]` and `#[resource(primary_key)]` and
/// exposes `table_name()` (falling back to the snake_case struct name) and
/// `primary_key()` (falling back to `"id"`).
///
/// This macro provides table metadata only — it does **not** generate CRUD
/// query methods.
///
/// # Examples
///
/// ```ignore
/// #[derive(Resource)]
/// #[resource(table = "users")]
/// pub struct User {
///     #[resource(primary_key)]
///     pub id: i64,
///     pub name: String,
/// }
///
/// assert_eq!(User::table_name(), "users");
/// assert_eq!(User::primary_key(), "id");
/// ```
#[proc_macro_derive(Resource, attributes(resource))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    model::derive_resource_impl(input)
}
