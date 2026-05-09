# Documentation Testing

Comprehensive documentation testing for the Armature framework.

## Overview

All code examples in documentation are tested automatically using Rust's built-in doc test feature. This ensures that all examples compile correctly and run as expected.

## Documentation Test Coverage

### Total Documentation Tests: 25

| Package | Doc Tests | Status |
|---------|-----------|--------|
| armature-jwt | 5 | ✅ Pass |
| armature-cache | 5 | ✅ Pass |
| armature-config | 3 | ✅ Pass |
| armature-validation | 3 | ✅ Pass |
| armature-cron | 3 | ✅ Pass |
| armature-queue | 2 | ✅ Pass |
| armature-core | 1 | ✅ Pass |
| armature-security | 1 | ✅ Pass |
| armature-openapi | 1 | ✅ Pass |
| armature-opentelemetry | 1 | ✅ Pass |

### Ignored Tests: 4

Some tests use `ignore` attribute for:
- Examples requiring external dependencies
- Examples with complex setup requirements
- HTTPS examples requiring certificates

## Running Documentation Tests

### Test All Documentation

```bash
cargo test --doc --all
```

### Test Specific Package

```bash
cargo test --doc --package armature-jwt
cargo test --doc --package armature-config
cargo test --doc --package armature-validation
```

### Show Documentation Test Output

```bash
cargo test --doc --all -- --nocapture
```

## Documentation Examples by Module

### JWT Authentication (`armature-jwt`)

**5 doc tests**

Examples cover:
- Basic JWT token creation and verification
- Custom claims with user data
- Token signing and verification
- Standard claims usage
- JwtManager API methods

```rust
use armature_jwt::{JwtConfig, JwtManager, StandardClaims};

let config = JwtConfig::new("secret".to_string());
let manager = JwtManager::new(config)?;

let claims = StandardClaims::new()
    .with_subject("user123".to_string())
    .with_expiration(3600);

let token = manager.sign(&claims)?;
let decoded: StandardClaims = manager.verify(&token)?;
```

### Configuration Management (`armature-config`)

**3 doc tests**

Examples cover:
- Basic configuration management
- Environment variable loading
- Type conversions (string, int, float, bool)
- Nested configuration keys

```rust
use armature_config::ConfigManager;

let manager = ConfigManager::new();
manager.set("app.name", "MyApp")?;
manager.set("app.port", 3000i64)?;

let name: String = manager.get("app.name")?;
let port: i64 = manager.get("app.port")?;
```

### Validation Framework (`armature-validation`)

**3 doc tests**

Examples cover:
- Basic validation with built-in validators
- Validation rules builder pattern
- Number validation (min, max, range, positive)
- String validation (email, length, format)

```rust
use armature_validation::{Validate, ValidationError, NotEmpty, IsEmail};

impl Validate for UserInput {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if let Err(e) = NotEmpty::validate(&self.name, "name") {
            errors.push(e);
        }
        if let Err(e) = IsEmail::validate(&self.email, "email") {
            errors.push(e);
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}
```

### Cache Management (`armature-cache`)

**5 doc tests**

Examples cover:
- Redis cache initialization
- Basic get/set operations
- TTL management
- Namespaced caching
- Cache manager usage

### Cron Scheduling (`armature-cron`)

**3 doc tests**

Examples cover:
- Cron expression parsing
- Job scheduling
- Scheduler management

### Queue System (`armature-queue`)

**2 doc tests**

Examples cover:
- Job queue creation
- Job enqueuing
- Worker management

### Security Middleware (`armature-security`)

**1 doc test**

Example covers:
- SecurityMiddleware configuration
- HSTS, CSP, and other security headers
- Default vs. custom security settings

## Documentation Quality Standards

All documentation examples must:

### ✅ Compile Successfully
- All examples must compile without errors
- Use proper imports and type annotations
- Include necessary error handling

### ✅ Be Self-Contained
- Include all necessary imports
- Provide complete, runnable code
- Use `# fn main() -> Result<(), Box<dyn std::error::Error>>` for error handling

### ✅ Demonstrate Real Usage
- Show realistic use cases
- Include proper error handling
- Demonstrate best practices

### ✅ Be Tested Automatically
- Run as part of CI/CD pipeline
- Fail build if examples don't compile
- Verified on every commit

## Doc Test Attributes

### `no_run`
For examples that compile but shouldn't execute:
```rust
/// ```no_run
/// use armature_framework::prelude::*;
/// let app = Application::create::<AppModule>().await;
/// app.listen(3000).await.unwrap();
/// ```
```

### `ignore`
For examples that should be excluded from testing:
```rust
/// ```ignore
/// // This example requires external setup
/// ```
```

### `should_panic`
For examples demonstrating error conditions:
```rust
/// ```should_panic
/// panic!("This should fail");
/// ```
```

## Module-Level Documentation

### Format

```rust
//! # Module Name
//!
//! Brief module description.
//!
//! # Examples
//!
//! ```
//! use module_name::Feature;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let feature = Feature::new();
//! feature.do_something()?;
//! # Ok(())
//! # }
//! ```
```

### Best Practices

1. **Start with `//!` for module docs**
2. **Include at least one working example**
3. **Show the most common use case**
4. **Keep examples concise but complete**
5. **Test examples automatically**

## Method-Level Documentation

### Format

```rust
/// Brief description of what this method does.
///
/// # Arguments
///
/// * `arg1` - Description of arg1
/// * `arg2` - Description of arg2
///
/// # Returns
///
/// Description of return value.
///
/// # Errors
///
/// Description of possible errors.
///
/// # Examples
///
/// ```
/// use module_name::Type;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let instance = Type::new();
/// instance.method(arg1, arg2)?;
/// # Ok(())
/// # }
/// ```
pub fn method(&self, arg1: Type1, arg2: Type2) -> Result<ReturnType, Error> {
    // Implementation
}
```

## Testing Strategy

### Local Testing

Before committing:
```bash
# Test all documentation
cargo test --doc --all

# Test specific module
cargo test --doc --package armature-jwt

# Show all test output
cargo test --doc --all -- --nocapture
```

### CI/CD Integration

Documentation tests run as part of the CI pipeline:
```yaml
- name: Test Documentation
  run: cargo test --doc --all
```

### Pre-commit Hook

Add to `.git/hooks/pre-commit`:
```bash
#!/bin/sh
cargo test --doc --all || exit 1
```

## Continuous Improvement

### Goals

- ✅ 100% of public APIs have doc comments
- ✅ All modules have at least one code example
- ✅ All code examples compile and run
- ✅ Examples demonstrate real-world usage
- ⏳ Expand examples to cover edge cases
- ⏳ Add more advanced usage examples
- ⏳ Include troubleshooting examples

### Metrics

| Metric | Current | Goal |
|--------|---------|------|
| Doc test count | 25 | 50+ |
| Modules with examples | 10 | 20 |
| Example pass rate | 100% | 100% |
| Public API coverage | ~80% | 100% |

## Common Patterns

### Error Handling in Examples

```rust
/// # Examples
///
/// ```
/// use module::Type;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let result = Type::try_new()?;
/// # Ok(())
/// # }
/// ```
```

### Hidden Setup Code

Use `#` to hide setup code:
```rust
/// ```
/// # use module::Type;
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let value = Type::new();
/// // User sees this
/// value.do_something()?;
/// # Ok(())
/// # }
/// ```
```

### Testing Private Items

Private items aren't tested by default:
```rust
#[cfg(test)]
mod tests {
    /// This will be tested
    /// ```
    /// assert!(true);
    /// ```
    fn private_function() {}
}
```

## Troubleshooting

### Example Doesn't Compile

1. Check imports are correct
2. Verify types match
3. Add error handling if needed
4. Use `# fn main() -> Result<...>` wrapper

### Example Compiles But Fails

1. Check assertion logic
2. Verify example data is valid
3. Ensure dependencies are available
4. Consider using `no_run` if appropriate

### Example Takes Too Long

1. Use `no_run` for long-running examples
2. Simplify the example
3. Mock expensive operations
4. Use timeouts if necessary

## Summary

Documentation testing ensures that all code examples in the Armature framework:

- ✅ **Compile correctly** - No broken examples
- ✅ **Run successfully** - Examples actually work
- ✅ **Stay up-to-date** - Tests fail when APIs change
- ✅ **Serve as integration tests** - Real usage verification
- ✅ **Provide confidence** - Users can trust the documentation

**Total: 25 passing documentation tests across 10 packages** 🎉

