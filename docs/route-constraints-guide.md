# Route Constraints Guide

Comprehensive guide to validating route parameters with Route Constraints in Armature.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Built-in Constraints](#built-in-constraints)
- [Basic Usage](#basic-usage)
- [Custom Constraints](#custom-constraints)
- [Combining Constraints](#combining-constraints)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Examples](#examples)
- [Summary](#summary)

---

## Overview

Route Constraints validate path parameters at the routing level, before handlers are called. This provides:

- **Early validation** - Fail fast before business logic
- **Better error messages** - Clear validation errors
- **Type safety** - Ensure parameters match expected types
- **Clean handlers** - No validation code in handlers

---

## Features

- ✅ Built-in constraints (Int, UUID, Email, etc.)
- ✅ Custom constraint creation
- ✅ Composable constraints
- ✅ Clear error messages
- ✅ Type-safe validation
- ✅ No runtime overhead (validation only on match)

---

## Built-in Constraints

### IntConstraint

Validates that a parameter is a valid signed integer.

```rust
use armature_core::*;

let constraint = IntConstraint;

assert!(constraint.validate("123").is_ok());
assert!(constraint.validate("-456").is_ok());
assert!(constraint.validate("abc").is_err());
```

### UIntConstraint

Validates that a parameter is a valid unsigned integer (≥ 0).

```rust
let constraint = UIntConstraint;

assert!(constraint.validate("123").is_ok());
assert!(constraint.validate("0").is_ok());
assert!(constraint.validate("-1").is_err());
```

### UuidConstraint

Validates that a parameter is a valid UUID (8-4-4-4-12 format).

```rust
let constraint = UuidConstraint;

assert!(constraint.validate("550e8400-e29b-41d4-a716-446655440000").is_ok());
assert!(constraint.validate("not-a-uuid").is_err());
```

### EmailConstraint

Validates that a parameter is a valid email address.

```rust
let constraint = EmailConstraint;

assert!(constraint.validate("user@example.com").is_ok());
assert!(constraint.validate("invalid-email").is_err());
```

### AlphaConstraint

Validates that a parameter contains only letters (a-z, A-Z).

```rust
let constraint = AlphaConstraint;

assert!(constraint.validate("hello").is_ok());
assert!(constraint.validate("WORLD").is_ok());
assert!(constraint.validate("hello123").is_err());
```

### AlphaNumConstraint

Validates that a parameter contains only letters and numbers.

```rust
let constraint = AlphaNumConstraint;

assert!(constraint.validate("user123").is_ok());
assert!(constraint.validate("ABC").is_ok());
assert!(constraint.validate("user-123").is_err());
```

### LengthConstraint

Validates that a parameter has a specific length or length range.

```rust
// Between 3 and 20 characters
let constraint = LengthConstraint::new(Some(3), Some(20));
assert!(constraint.validate("hello").is_ok());
assert!(constraint.validate("hi").is_err());

// At least 5 characters
let min_constraint = LengthConstraint::min(5);

// At most 10 characters
let max_constraint = LengthConstraint::max(10);

// Exactly 5 characters
let exact_constraint = LengthConstraint::exact(5);
```

### RangeConstraint

Validates that a number is within a specific range.

```rust
// Between 1 and 100
let constraint = RangeConstraint::new(Some(1), Some(100));
assert!(constraint.validate("50").is_ok());
assert!(constraint.validate("0").is_err());

// At least 0
let min_constraint = RangeConstraint::min(0);

// At most 1000
let max_constraint = RangeConstraint::max(1000);
```

### EnumConstraint

Validates that a parameter is one of a set of allowed values.

```rust
let constraint = EnumConstraint::new(vec![
    "active".to_string(),
    "inactive".to_string(),
    "pending".to_string(),
]);

assert!(constraint.validate("active").is_ok());
assert!(constraint.validate("unknown").is_err());
```

### RegexConstraint

Validates that a parameter matches a regular expression pattern.

```rust
// Only lowercase letters
let constraint = RegexConstraint::new(r"^[a-z]+$", "lowercase letters").unwrap();
assert!(constraint.validate("hello").is_ok());
assert!(constraint.validate("HELLO").is_err());
```

---

## Basic Usage

### Single Constraint

```rust
use armature_core::*;

// Create route constraints
let constraints = RouteConstraints::new()
    .add("id", Box::new(IntConstraint));

// Validate parameters
let mut params = std::collections::HashMap::new();
params.insert("id".to_string(), "123".to_string());

assert!(constraints.validate(&params).is_ok());
```

### Multiple Constraints

```rust
let constraints = RouteConstraints::new()
    .add("id", Box::new(UIntConstraint))
    .add("uuid", Box::new(UuidConstraint))
    .add("email", Box::new(EmailConstraint));
```

### With Route

```rust
use armature_core::*;
use std::sync::Arc;

let constraints = RouteConstraints::new()
    .add("id", Box::new(IntConstraint))
    .add("name", Box::new(AlphaConstraint));

let route = Route {
    method: HttpMethod::GET,
    path: "/users/:id/:name".to_string(),
    handler: Arc::new(|req| {
        Box::pin(async move {
            // Parameters are already validated!
            let id = req.path_params.get("id").unwrap();
            let name = req.path_params.get("name").unwrap();

            Ok(HttpResponse::ok())
        })
    }),
    constraints: Some(constraints),
};
```

---

## Custom Constraints

Implement the `RouteConstraint` trait:

```rust
use armature_core::*;

/// Custom constraint for US ZIP codes
struct ZipCodeConstraint;

impl RouteConstraint for ZipCodeConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if value.len() == 5 && value.chars().all(|c| c.is_numeric()) {
            Ok(())
        } else {
            Err(format!("'{}' is not a valid ZIP code", value))
        }
    }

    fn description(&self) -> &str {
        "5-digit ZIP code"
    }
}

// Use it
let constraints = RouteConstraints::new()
    .add("zip", Box::new(ZipCodeConstraint));
```

---

## Best Practices

### 1. Validate Early

```rust
// ✅ GOOD - Validate at route level
let constraints = RouteConstraints::new()
    .add("id", Box::new(IntConstraint));

let route = Route {
    constraints: Some(constraints),
    ..route
};

// Handler is only called if validation passes
async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id: i32 = req.path_params.get("id").unwrap().parse().unwrap();
    // Safe to unwrap - already validated
}
```

### 2. Use Appropriate Constraints

```rust
// ✅ GOOD - Specific constraints
let constraints = RouteConstraints::new()
    .add("id", Box::new(UIntConstraint))  // IDs are always positive
    .add("uuid", Box::new(UuidConstraint))  // UUIDs have specific format
    .add("email", Box::new(EmailConstraint));  // Emails have rules
```

### 3. Provide Clear Error Messages

```rust
impl RouteConstraint for CustomConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if !is_valid(value) {
            Err(format!(
                "'{}' must be a valid product code (format: ABC-1234)",
                value
            ))
        } else {
            Ok(())
        }
    }
}
```

---

## API Reference

### Built-in Constraints

| Constraint | Constructor | Description |
|------------|-------------|-------------|
| `IntConstraint` | Unit struct | Signed integer |
| `UIntConstraint` | Unit struct | Unsigned integer |
| `FloatConstraint` | Unit struct | Floating point |
| `AlphaConstraint` | Unit struct | Letters only |
| `AlphaNumConstraint` | Unit struct | Letters and numbers |
| `UuidConstraint` | Unit struct | UUID format |
| `EmailConstraint` | Unit struct | Email format |
| `LengthConstraint` | `new(min, max)` | String length |
| `RangeConstraint` | `new(min, max)` | Number range |
| `EnumConstraint` | `new(values)` | Enum values |
| `RegexConstraint` | `new(pattern, desc)` | Regex match |

---

## Summary

**Key Points:**

1. **Validate early** with route constraints
2. **Use built-in constraints** for common cases
3. **Create custom constraints** for domain-specific validation
4. **Provide clear error messages** for users
5. **Combine with extractors** for type safety
6. **Fail fast** before handler execution

**Quick Reference:**

```rust
// Create constraints
let constraints = RouteConstraints::new()
    .add("id", Box::new(UIntConstraint))
    .add("uuid", Box::new(UuidConstraint))
    .add("status", Box::new(EnumConstraint::new(vec![
        "active".to_string(),
        "inactive".to_string(),
    ])));

// Add to route
let route = Route {
    method: HttpMethod::GET,
    path: "/users/:id/:uuid/:status".to_string(),
    handler: my_handler,
    constraints: Some(constraints),
};
```

**Benefits:**

- 🚀 **Early validation** - Fail fast
- 🛡️ **Type safety** - Ensure valid parameters
- 📝 **Clear errors** - Better error messages
- 🧹 **Clean handlers** - No validation code
- 🔧 **Maintainable** - Centralized validation
- 🎯 **Focused** - Handlers focus on business logic

