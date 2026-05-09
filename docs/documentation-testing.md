# Documentation Testing Guide

Comprehensive guide for writing and testing documentation in the Armature framework.

## Overview

Documentation testing ensures that code examples in documentation actually work. Rust's `cargo test --doc` runs all code blocks in doc comments as tests.

## Why Document Tests?

✅ **Ensures examples work** - Code in docs stays up-to-date
✅ **Prevents bit rot** - Breaking changes caught immediately
✅ **Living documentation** - Examples are always tested
✅ **Better onboarding** - New users get working code

## Writing Doc Tests

### Basic Example

```rust
/// Add two numbers together.
///
/// # Examples
///
/// ```
/// use armature_core::utils::add;
///
/// let result = add(2, 3);
/// assert_eq!(result, 5);
/// ```
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

### Async Examples

```rust
/// Render a template asynchronously.
///
/// # Examples
///
/// ```
/// use handlebars::Handlebars;
/// use serde_json::json;
///
/// # tokio_test::block_on(async {
/// let mut hbs = Handlebars::new();
/// hbs.register_template_string("index", "Hello {{name}}!")?;
///
/// let data = json!({"name": "World"});
/// let html = hbs.render("index", &data)?;
/// assert_eq!(html, "Hello World!");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub async fn render_template() -> Result<String, Error> {
    // Implementation
}
```

### Examples with Setup

```rust
/// Create a user in the database.
///
/// # Examples
///
/// ```
/// use armature_auth::User;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # // Hidden setup code
/// # let db = setup_test_db()?;
/// let user = User::create("alice@example.com", "password123")?;
/// assert_eq!(user.email, "alice@example.com");
/// # Ok(())
/// # }
/// #
/// # fn setup_test_db() -> Result<Database, Box<dyn std::error::Error>> {
/// #     Ok(Database::new())
/// # }
/// ```
pub fn create_user(email: &str, password: &str) -> Result<User, Error> {
    // Implementation
}
```

## Doc Test Attributes

### `no_run` - Compile but don't run

Use for examples that require external resources:

```rust
/// Start the server.
///
/// # Examples
///
/// ```no_run
/// use armature_framework::prelude::*;
///
/// #[module()]
/// #[derive(Default)]
/// struct AppModule;
///
/// #[tokio::main]
/// async fn main() {
///     let app = Application::create::<AppModule>().await;
///     app.listen(3000).await.unwrap();
/// }
/// ```
pub async fn start_server() {}
```

### `ignore` - Skip completely

Use for examples that are placeholders or pseudo-code:

```rust
/// Complex algorithm (simplified).
///
/// # Examples
///
/// ```ignore
/// // This is pseudo-code
/// let result = complex_algorithm(data);
/// ```
pub fn complex_algorithm(data: &[u8]) -> Vec<u8> {
    vec![]
}
```

### `should_panic` - Expect panic

Use for error condition examples:

```rust
/// Divide two numbers (panics on zero).
///
/// # Examples
///
/// ```should_panic
/// use armature_core::utils::divide;
///
/// // This will panic
/// divide(10, 0);
/// ```
pub fn divide(a: i32, b: i32) -> i32 {
    if b == 0 {
        panic!("Division by zero");
    }
    a / b
}
```

### `compile_fail` - Expect compile error

Use to show incorrect usage:

```rust
/// Type-safe ID wrapper.
///
/// This example shows incorrect usage:
///
/// ```compile_fail
/// use armature_core::UserId;
///
/// // This won't compile (UserId != OrderId)
/// let user_id: UserId = OrderId::new(123);
/// ```
pub struct UserId(u64);
```

## Hidden Lines

Use `#` to hide setup/teardown code:

```rust
/// Query the database.
///
/// # Examples
///
/// ```
/// use armature_cache::Cache;
///
/// # tokio_test::block_on(async {
/// # let cache = Cache::new_memory();
/// let value: Option<String> = cache.get("key").await?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub async fn query_cache() {}
```

Hidden lines are compiled and run but not shown in documentation.

## Testing Standards

### Module-Level Documentation

Every module should have an example:

```rust
//! Authentication module for JWT and OAuth2.
//!
//! # Example
//!
//! ```
//! use armature_auth::{JwtManager, JwtConfig};
//!
//! let config = JwtConfig::new("secret_key");
//! let manager = JwtManager::new(config)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod jwt;
pub mod oauth2;
```

### Public API Documentation

Every public function/struct should have:

1. **Description** - What it does
2. **Examples** - How to use it
3. **Errors** - What can go wrong (if applicable)
4. **Panics** - When it panics (if applicable)

```rust
/// Create a new HTTP response.
///
/// # Examples
///
/// ```
/// use armature_core::HttpResponse;
///
/// let response = HttpResponse::ok()
///     .with_header("Content-Type", "application/json")
///     .with_body(b"{}".to_vec());
///
/// assert_eq!(response.status, 200);
/// ```
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn with_json<T: Serialize>(self, data: &T) -> Result<Self, Error> {
    // Implementation
}
```

### Test Coverage Goals

- **100%** of public APIs have documentation
- **90%+** of public APIs have runnable examples
- **All** crate-level docs have examples
- **All** doc examples compile and run

## Running Doc Tests

### All Workspace Members

```bash
# Run all doc tests
cargo test --doc --all

# With all features
cargo test --doc --all --all-features

# Specific crate
cargo test --doc -p armature-core
```

### Using the Script

```bash
# Run doc tests for all members
./scripts/test-docs.sh
```

### In GitHub Actions

```yaml
- name: Run documentation tests
  run: cargo test --doc --all --all-features
```

## Common Patterns

### Result Types

```rust
/// # Examples
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let result = fallible_operation()?;
/// assert_eq!(result, "success");
/// # Ok(())
/// # }
/// ```
pub fn fallible_operation() -> Result<String, Error> {
    Ok("success".to_string())
}
```

### Async Functions

```rust
/// # Examples
///
/// ```
/// # tokio_test::block_on(async {
/// let data = fetch_data().await?;
/// assert!(!data.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub async fn fetch_data() -> Result<Vec<u8>, Error> {
    Ok(vec![1, 2, 3])
}
```

### With Dependencies

```rust
/// # Examples
///
/// ```
/// use armature_core::{Application, HttpRequest, HttpResponse};
/// use armature_macro::get;
///
/// #[get("/hello")]
/// async fn hello(_req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok().with_body(b"Hello!".to_vec()))
/// }
/// ```
```

## Troubleshooting

### "cannot find type X in this scope"

**Problem:** Missing import in example.

**Solution:** Add all required imports:

```rust
/// ```
/// use armature_core::HttpRequest;  // ✅ Add this
/// use armature_core::HttpResponse; // ✅ And this
///
/// let response = HttpResponse::ok();
/// ```
```

### "async block yields a value but never gets executed"

**Problem:** Async code without runtime.

**Solution:** Use `tokio_test::block_on`:

```rust
/// ```
/// # tokio_test::block_on(async {  // ✅ Add this
/// let result = async_function().await?;
/// # Ok::<(), Box<dyn std::error::Error>>(())  // ✅ And this
/// # });  // ✅ And this
/// ```
```

### "error: cannot borrow as mutable"

**Problem:** Example doesn't show mutable binding.

**Solution:** Show the correct usage:

```rust
/// ```
/// let mut config = Config::new();  // ✅ Show mut
/// config.set_option("value");
/// ```
```

## Best Practices

### DO

✅ Test every public API
✅ Show realistic examples
✅ Include error handling
✅ Hide boilerplate with `#`
✅ Use `no_run` for resource-intensive examples
✅ Keep examples simple and focused

### DON'T

❌ Use `ignore` for real code
❌ Write examples that can break
❌ Omit necessary imports
❌ Show only happy path
❌ Make examples too complex

## Coverage Report

Check doc test coverage:

```bash
# Run with verbose output
cargo test --doc --all -- --nocapture

# Count doc tests
cargo test --doc --all 2>&1 | grep "test result"

# Generate coverage report
cargo tarpaulin --doc --all
```

## CI/CD Integration

### GitHub Actions Workflow

```yaml
name: Documentation Tests

on: [push, pull_request]

jobs:
  doc-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Run doc tests
        run: cargo test --doc --all --all-features

      - name: Check doc coverage
        run: ./scripts/test-docs.sh
```

## Summary

**Key Principles:**

1. **Every public API has an example**
2. **Examples are tested automatically**
3. **Hidden lines keep examples clean**
4. **Attributes handle edge cases**
5. **Documentation is code quality**

**Testing is Documentation! Document with Tests!** 📚✅


