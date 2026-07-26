// trybuild pass case: every documented `#[validate(...)]` rule must expand and
// compile.

use armature_validation::{Validate, ValidationError};

fn not_reserved(value: &str) -> Result<(), ValidationError> {
    if value == "admin" {
        Err(ValidationError::new("username", "reserved"))
    } else {
        Ok(())
    }
}

#[derive(Validate)]
struct Inner {
    #[validate(required)]
    label: String,
}

#[derive(Validate)]
struct CreateUser {
    #[validate(length(min = 3, max = 50))]
    #[validate(custom = "not_reserved")]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(url)]
    website: String,

    #[validate(length(min = 8))]
    password: String,

    #[validate(range(min = 13, max = 120))]
    age: u8,

    #[validate(range(min = 0.0, max = 1.0))]
    score: f64,

    #[validate(regex(pattern = "^[A-Z]"))]
    code: String,

    #[validate]
    inner: Inner,
}

fn main() {
    let user = CreateUser {
        username: "john".to_string(),
        email: "john@example.com".to_string(),
        website: "https://example.com".to_string(),
        password: "supersecret".to_string(),
        age: 42,
        score: 0.5,
        code: "Xyz".to_string(),
        inner: Inner {
            label: "ok".to_string(),
        },
    };
    assert!(user.validate().is_ok());
}
