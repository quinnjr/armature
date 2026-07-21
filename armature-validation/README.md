# armature-validation

Request validation for the Armature framework.

## Features

- **Derive Macro** - `#[derive(Validate)]` for structs
- **Built-in Rules** - Email, URL, length, range, regex, etc.
- **Custom Validators** - Create your own validation rules
- **Nested Validation** - Validate nested structs
- **Error Messages** - Customizable error messages

## Installation

```toml
[dependencies]
armature-validation = "0.1"
```

## Quick Start

```rust
use armature_validation::{Validate, ValidationError};

#[derive(Validate)]
struct CreateUser {
    #[validate(length(min = 3, max = 50))]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(length(min = 8))]
    password: String,

    #[validate(range(min = 13, max = 120))]
    age: u8,
}

fn create_user(data: CreateUser) -> Result<(), Vec<ValidationError>> {
    data.validate()?;
    // Process valid data
    Ok(())
}
```

## Available Validators

| Validator | Description |
|-----------|-------------|
| `email` | Valid email address |
| `url` | Valid URL |
| `length(min, max)` | String length bounds |
| `range(min, max)` | Numeric range |
| `regex(pattern)` | Regex match |
| `required` | Non-empty value |
| `custom(fn)` | Custom function |

## Custom Validators

```rust
fn validate_username(username: &str) -> Result<(), ValidationError> {
    if username.contains(' ') {
        return Err(ValidationError::new("username", "No spaces allowed"));
    }
    Ok(())
}

#[derive(Validate)]
struct User {
    #[validate(custom = "validate_username")]
    username: String,
}
```

## License

MIT OR Apache-2.0

