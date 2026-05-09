# Testing Coverage Guide

Complete guide to testing practices, coverage measurement, and quality standards for Armature applications.

## Table of Contents

- [Overview](#overview)
- [Testing Standards](#testing-standards)
- [Running Tests](#running-tests)
- [Measuring Coverage](#measuring-coverage)
- [Types of Tests](#types-of-tests)
- [Writing Quality Tests](#writing-quality-tests)
- [Coverage Goals](#coverage-goals)
- [Testing Utilities](#testing-utilities)
- [Best Practices](#best-practices)
- [CI/CD Integration](#cicd-integration)
- [Examples](#examples)

## Overview

Armature follows rigorous testing standards to ensure reliability, maintainability, and confidence in code changes. This guide covers testing practices, coverage measurement, and quality standards.

### Testing Philosophy

- **High Coverage**: Target 85% code coverage across the codebase
- **Quality over Quantity**: Meaningful tests that verify behavior
- **Fast Feedback**: Quick test execution for rapid development
- **Test Pyramid**: Balanced mix of unit, integration, and E2E tests
- **Behavior Testing**: Test what code does, not how it does it

## Testing Standards

### Coverage Targets

| Component | Target | Priority |
|-----------|--------|----------|
| Core Library | 90%+ | Critical |
| Submodules | 85%+ | High |
| Examples | 50%+ | Medium |
| Documentation Examples | 100% | High |

### Quality Metrics

- ✅ All public APIs have tests
- ✅ All error paths covered
- ✅ Edge cases tested
- ✅ No flaky tests
- ✅ Fast execution (< 30s for full suite)
- ✅ Clear, descriptive test names
- ✅ Isolated tests (no shared state)

## Running Tests

### Basic Commands

```bash
# Run all tests
cargo test

# Run tests for specific package
cargo test --package armature-core

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run tests in parallel (default)
cargo test

# Run tests serially (for debugging)
cargo test -- --test-threads=1
```

### Running by Type

```bash
# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test '*'

# Doc tests only
cargo test --doc

# Specific module tests
cargo test --package armature-core --lib container
```

### Workspace Testing

```bash
# All packages
cargo test --workspace

# All packages with all features
cargo test --workspace --all-features

# Exclude examples
cargo test --workspace --lib --bins
```

## Measuring Coverage

### Using Tarpaulin (Recommended)

Install tarpaulin:

```bash
cargo install cargo-tarpaulin
```

Generate coverage report:

```bash
# HTML report
cargo tarpaulin --out Html --output-dir coverage

# Terminal output
cargo tarpaulin --out Stdout

# XML for CI
cargo tarpaulin --out Xml

# Workspace coverage
cargo tarpaulin --workspace --all-features

# Exclude tests directory
cargo tarpaulin --workspace --exclude-files 'tests/*'
```

View HTML report:

```bash
open coverage/index.html  # macOS
xdg-open coverage/index.html  # Linux
```

### Using llvm-cov

Install llvm-cov:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
```

Generate coverage:

```bash
# HTML report
cargo llvm-cov --html --open

# Workspace coverage
cargo llvm-cov --workspace --all-features

# JSON report
cargo llvm-cov --json --output-path coverage.json

# Text summary
cargo llvm-cov --summary-only
```

### Coverage Configuration

Create `.tarpaulin.toml`:

```toml
[build]
workspace = true

[report]
out = ["Html", "Xml"]
output-dir = "coverage"

[coverage]
exclude-files = [
    "tests/*",
    "examples/*",
    "target/*",
]

[run]
all-features = true
```

## Types of Tests

### 1. Unit Tests

Test individual functions, methods, and modules in isolation.

**Location**: `src/` files with `#[cfg(test)]` modules

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation() {
        let user = User::new("Alice");
        assert_eq!(user.name, "Alice");
    }

    #[test]
    fn test_validation_failure() {
        let result = User::validate("");
        assert!(result.is_err());
    }

    #[test]
    #[should_panic(expected = "Invalid ID")]
    fn test_panic_on_invalid_id() {
        User::from_id(-1);
    }
}
```

### 2. Integration Tests

Test multiple components working together.

**Location**: `tests/` directory

```rust
// tests/api_integration.rs
use armature_framework::prelude::*;
use armature_testing::*;

#[tokio::test]
async fn test_full_request_flow() {
    let app = TestAppBuilder::new()
        .add_module::<AppModule>()
        .build();

    let client = app.client();

    let response = client.get("/users/1").await;

    assert_http_ok(&response);
    assert_json_contains(&response, "name", "Alice");
}
```

### 3. Documentation Tests

Test examples in documentation comments.

```rust
/// Create a new user.
///
/// # Examples
///
/// ```
/// use armature_framework::User;
///
/// let user = User::new("Alice");
/// assert_eq!(user.name, "Alice");
/// ```
pub fn new(name: &str) -> Self {
    // ...
}
```

Run doc tests:

```bash
cargo test --doc
```

### 4. Benchmark Tests (Optional)

Measure performance.

```rust
#[cfg(test)]
mod benches {
    use super::*;
    use std::time::Instant;

    #[test]
    fn benchmark_user_creation() {
        let start = Instant::now();

        for _ in 0..10_000 {
            let _ = User::new("Alice");
        }

        let duration = start.elapsed();
        assert!(duration.as_millis() < 100, "Too slow: {:?}", duration);
    }
}
```

## Writing Quality Tests

### Test Naming

Use descriptive names that explain what is being tested:

**Good**:
```rust
#[test]
fn test_user_login_succeeds_with_valid_credentials() { }

#[test]
fn test_user_login_fails_with_invalid_password() { }

#[test]
fn test_cache_expires_after_ttl() { }
```

**Bad**:
```rust
#[test]
fn test1() { }

#[test]
fn test_login() { }  // Too vague

#[test]
fn test_cache() { }  // What about cache?
```

### AAA Pattern

Structure tests with Arrange-Act-Assert:

```rust
#[test]
fn test_user_service_creates_user() {
    // Arrange
    let service = UserService::new();
    let user_data = UserData {
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };

    // Act
    let user = service.create_user(user_data).unwrap();

    // Assert
    assert_eq!(user.name, "Alice");
    assert_eq!(user.email, "alice@example.com");
    assert!(user.id > 0);
}
```

### Test Isolation

Each test should be independent:

**Good**:
```rust
#[test]
fn test_user_creation() {
    let db = create_test_db();  // Fresh DB for this test
    let service = UserService::new(db);

    let user = service.create_user("Alice").unwrap();
    assert_eq!(user.name, "Alice");
}

#[test]
fn test_user_deletion() {
    let db = create_test_db();  // Fresh DB for this test
    let service = UserService::new(db);

    let user = service.create_user("Bob").unwrap();
    service.delete_user(user.id).unwrap();
    assert!(service.get_user(user.id).is_none());
}
```

**Bad**:
```rust
// ❌ Shared state between tests
static mut GLOBAL_USER: Option<User> = None;

#[test]
fn test_1_create() {
    unsafe { GLOBAL_USER = Some(create_user()); }
}

#[test]
fn test_2_update() {  // Depends on test_1
    unsafe { update_user(GLOBAL_USER.as_ref().unwrap()); }
}
```

### Testing Async Code

Use `#[tokio::test]` for async tests:

```rust
use tokio;

#[tokio::test]
async fn test_async_operation() {
    let service = AsyncService::new();

    let result = service.fetch_data().await;

    assert!(result.is_ok());
}

#[tokio::test]
async fn test_concurrent_requests() {
    let service = Arc::new(AsyncService::new());

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let service = Arc::clone(&service);
            tokio::spawn(async move {
                service.process(i).await
            })
        })
        .collect();

    for handle in handles {
        assert!(handle.await.unwrap().is_ok());
    }
}
```

### Testing Error Cases

Always test error paths:

```rust
#[test]
fn test_user_validation_empty_name() {
    let result = User::validate("");
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().to_string(),
        "Name cannot be empty"
    );
}

#[test]
fn test_database_connection_failure() {
    let result = Database::connect("invalid://url");
    match result {
        Err(DatabaseError::ConnectionFailed(_)) => {}, // ✅ Expected
        _ => panic!("Expected ConnectionFailed error"),
    }
}

#[test]
#[should_panic(expected = "User not found")]
fn test_panic_on_missing_user() {
    let service = UserService::new();
    service.get_user_or_panic(999);  // Non-existent ID
}
```

## Coverage Goals

### Minimum Requirements

**Before Merging PR:**
- ✅ Overall coverage: 85%+
- ✅ New code coverage: 90%+
- ✅ Critical paths: 100%
- ✅ All public APIs tested
- ✅ No decrease in existing coverage

### Priority Areas (100% Coverage)

1. **Error Handling**
   - All error types
   - Error conversions
   - Error messages

2. **Public APIs**
   - All public functions
   - All public methods
   - All traits

3. **Security-Critical Code**
   - Authentication
   - Authorization
   - Encryption/Decryption
   - Input validation

4. **Data Integrity**
   - Database operations
   - Cache operations
   - File I/O

### Acceptable Lower Coverage

- **Private implementation details**: 70%+
- **Examples**: 50%+
- **Generated code**: Excluded
- **External integrations**: Mocked

## Testing Utilities

Armature provides comprehensive testing utilities in `armature-testing`.

### Test Application Builder

```rust
use armature_testing::*;

#[tokio::test]
async fn test_with_test_app() {
    let app = TestAppBuilder::new()
        .add_module::<UserModule>()
        .add_module::<AuthModule>()
        .build();

    let client = app.client();

    let response = client.get("/users").await;
    assert_http_ok(&response);
}
```

### Mock Services

```rust
use armature_testing::*;

#[test]
fn test_with_mock_service() {
    let mock_db = MockService::new("Database")
        .with_response("query", Ok("data"))
        .with_response("save", Err("Connection failed"));

    let service = UserService::new(mock_db);

    // Test with mocked responses
    assert!(service.get_data().is_ok());
    assert!(service.save_data().is_err());
}
```

### Spy Pattern

```rust
use armature_testing::*;

#[test]
fn test_service_interaction() {
    let real_service = UserService::new();
    let spy = Spy::new(real_service);

    // Use the service
    spy.inner().create_user("Alice");
    spy.inner().create_user("Bob");

    // Verify interactions
    assert_eq!(spy.call_count(), 2);
    assert!(spy.was_called("create_user"));
}
```

### Assertions

```rust
use armature_testing::*;

#[tokio::test]
async fn test_assertions() {
    let response = client.get("/api/user/1").await;

    // HTTP assertions
    assert_http_ok(&response);
    assert_http_status(&response, 200);

    // JSON assertions
    assert_json_contains(&response, "name", "Alice");
    assert_json_path(&response, "$.user.email", "alice@example.com");

    // Header assertions
    assert_header(&response, "Content-Type", "application/json");
}
```

## Best Practices

### 1. Test One Thing

Each test should verify one specific behavior:

**Good**:
```rust
#[test]
fn test_user_name_validation_accepts_valid_names() {
    assert!(User::validate_name("Alice").is_ok());
}

#[test]
fn test_user_name_validation_rejects_empty_names() {
    assert!(User::validate_name("").is_err());
}

#[test]
fn test_user_name_validation_rejects_too_long_names() {
    let long_name = "a".repeat(300);
    assert!(User::validate_name(&long_name).is_err());
}
```

**Bad**:
```rust
#[test]
fn test_user_validation() {
    // ❌ Testing multiple behaviors
    assert!(User::validate_name("Alice").is_ok());
    assert!(User::validate_name("").is_err());
    assert!(User::validate_email("alice@example.com").is_ok());
    assert!(User::validate_age(25).is_ok());
}
```

### 2. Use Helper Functions

Extract common setup into helper functions:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_user(name: &str) -> User {
        User {
            id: 1,
            name: name.to_string(),
            email: format!("{}@example.com", name.to_lowercase()),
            created_at: Utc::now(),
        }
    }

    fn create_test_db() -> TestDatabase {
        TestDatabase::new().with_migrations()
    }

    #[test]
    fn test_user_operations() {
        let db = create_test_db();
        let user = create_test_user("Alice");

        // Test logic...
    }
}
```

### 3. Test Edge Cases

Don't just test the happy path:

```rust
#[test]
fn test_pagination_edge_cases() {
    let service = UserService::new();

    // Empty list
    assert_eq!(service.paginate(0, 10).len(), 0);

    // First page
    assert_eq!(service.paginate(1, 10).len(), 10);

    // Last page (partial)
    assert_eq!(service.paginate(5, 10).len(), 7);

    // Beyond last page
    assert_eq!(service.paginate(100, 10).len(), 0);

    // Invalid page size
    assert!(service.paginate(1, 0).is_empty());
    assert!(service.paginate(1, -1).is_empty());
}
```

### 4. Avoid Test Interdependence

Tests should not depend on execution order:

**Good**:
```rust
#[test]
fn test_a() {
    let data = setup_test_data();
    // Test with data
}

#[test]
fn test_b() {
    let data = setup_test_data();  // Independent setup
    // Test with data
}
```

**Bad**:
```rust
static mut SHARED_STATE: Option<Data> = None;

#[test]
fn test_a() {
    unsafe { SHARED_STATE = Some(create_data()); }
}

#[test]
fn test_b() {
    // ❌ Depends on test_a running first
    unsafe { use_data(SHARED_STATE.as_ref().unwrap()); }
}
```

### 5. Use Descriptive Assertions

Make failures easy to understand:

**Good**:
```rust
assert_eq!(
    user.age, 25,
    "User age should be 25, but was {}",
    user.age
);

assert!(
    result.is_ok(),
    "Expected successful result, got error: {:?}",
    result.err()
);
```

**Bad**:
```rust
assert!(user.age == 25);  // ❌ No context on failure
assert!(result.is_ok());   // ❌ No error information
```

### 6. Clean Up Resources

Ensure tests clean up after themselves:

```rust
#[test]
fn test_file_operations() {
    let temp_file = "test_output.txt";

    // Test code...
    write_file(temp_file, "data");

    // Clean up
    let _ = std::fs::remove_file(temp_file);
}

// Better: Use RAII
#[test]
fn test_file_operations_safe() {
    let _temp_file = TempFile::new("test_output.txt");  // Auto-cleanup on drop

    // Test code...
}
```

## CI/CD Integration

### GitHub Actions

Create `.github/workflows/test.yml`:

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v3

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          override: true

      - name: Run tests
        run: cargo test --workspace --all-features

      - name: Install tarpaulin
        run: cargo install cargo-tarpaulin

      - name: Generate coverage
        run: cargo tarpaulin --workspace --all-features --out Xml

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v3
        with:
          files: ./cobertura.xml
          fail_ci_if_error: true

      - name: Check coverage threshold
        run: |
          coverage=$(cargo tarpaulin --workspace --all-features --out Stdout | grep -oP '\d+\.\d+(?=%)')
          if (( $(echo "$coverage < 85.0" | bc -l) )); then
            echo "Coverage $coverage% is below 85% threshold"
            exit 1
          fi
```

### Coverage Badges

Add to README.md:

```markdown
[![codecov](https://codecov.io/gh/username/armature/branch/main/graph/badge.svg)](https://codecov.io/gh/username/armature)
```

### Pre-commit Hook

Create `.git/hooks/pre-commit`:

```bash
#!/bin/bash
set -e

echo "Running tests..."
cargo test --workspace

echo "Checking coverage..."
cargo tarpaulin --workspace --all-features | grep -q '85\.'

echo "✅ All checks passed"
```

## Examples

### Complete Test Module

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Helper functions
    fn setup() -> TestEnvironment {
        TestEnvironment::new()
    }

    fn create_test_user(name: &str) -> User {
        User::new(name)
    }

    // Unit tests
    #[test]
    fn test_user_creation() {
        let user = create_test_user("Alice");
        assert_eq!(user.name, "Alice");
    }

    #[test]
    fn test_user_validation_empty_name() {
        let result = User::validate("");
        assert!(result.is_err());
    }

    // Async tests
    #[tokio::test]
    async fn test_async_user_fetch() {
        let service = UserService::new();
        let user = service.fetch_user(1).await.unwrap();
        assert_eq!(user.id, 1);
    }

    // Error tests
    #[test]
    #[should_panic(expected = "Invalid ID")]
    fn test_panic_on_invalid_id() {
        User::from_id(-1);
    }

    // Integration tests
    #[tokio::test]
    async fn test_full_user_flow() {
        let env = setup();

        // Create
        let user = env.service.create_user("Alice").await.unwrap();

        // Read
        let fetched = env.service.get_user(user.id).await.unwrap();
        assert_eq!(fetched.name, "Alice");

        // Update
        env.service.update_user(user.id, "Alice Updated").await.unwrap();

        // Delete
        env.service.delete_user(user.id).await.unwrap();
        assert!(env.service.get_user(user.id).await.is_none());
    }
}
```

## Summary

**Key Takeaways:**

1. ✅ **Target 85% coverage** across the codebase
2. ✅ **Write meaningful tests** that verify behavior
3. ✅ **Test error paths** as thoroughly as happy paths
4. ✅ **Keep tests isolated** and independent
5. ✅ **Use descriptive names** and clear assertions
6. ✅ **Measure coverage** regularly with tarpaulin or llvm-cov
7. ✅ **Integrate testing** into CI/CD pipeline
8. ✅ **Follow the test pyramid**: Many unit tests, some integration tests, few E2E tests

**Testing Checklist:**

- [ ] All public APIs have tests
- [ ] All error cases covered
- [ ] Edge cases tested
- [ ] Async code properly tested
- [ ] No test interdependence
- [ ] Clear, descriptive test names
- [ ] Coverage meets 85% threshold
- [ ] CI/CD pipeline includes tests
- [ ] Documentation examples compile
- [ ] No flaky tests

**Resources:**

- [Rust Testing Book](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [cargo-tarpaulin](https://github.com/xd009642/tarpaulin)
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)
- [armature-testing](../armature-testing/) - Built-in testing utilities

Quality testing is not about achieving 100% coverage—it's about having confidence that your code works correctly in all scenarios. Focus on testing behavior, edge cases, and error handling to build robust, reliable applications.

