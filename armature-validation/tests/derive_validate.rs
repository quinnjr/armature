//! Behavior tests for `#[derive(Validate)]`.
//!
//! These fail to even compile against the pre-derive code (the derive did not
//! exist), so they double as the regression guard for the Critical task.

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

#[test]
fn derived_struct_accepts_valid_input() {
    let user = CreateUser {
        username: "john_doe".to_string(),
        email: "john@example.com".to_string(),
        password: "supersecret".to_string(),
        age: 30,
    };
    assert!(user.validate().is_ok());
}

#[test]
fn derived_struct_rejects_invalid_input() {
    let user = CreateUser {
        username: "jo".to_string(),        // too short
        email: "not-an-email".to_string(), // bad email
        password: "short".to_string(),     // too short
        age: 5,                            // below range
    };
    let errors = user.validate().unwrap_err();
    // one error per failing field
    let fields: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
    assert!(
        fields.contains(&"username"),
        "missing username error: {fields:?}"
    );
    assert!(fields.contains(&"email"), "missing email error: {fields:?}");
    assert!(
        fields.contains(&"password"),
        "missing password error: {fields:?}"
    );
    assert!(fields.contains(&"age"), "missing age error: {fields:?}");
}

#[test]
fn length_counts_characters_not_bytes() {
    #[derive(Validate)]
    struct Named {
        #[validate(length(min = 3, max = 5))]
        name: String,
    }
    // 4 emoji = 4 chars (ok for max=5) but 16 bytes.
    let ok = Named {
        name: "😀😀😀😀".to_string(),
    };
    assert!(ok.validate().is_ok());
    // 2 chars < min 3
    let bad = Named {
        name: "😀😀".to_string(),
    };
    assert!(bad.validate().is_err());
}

#[test]
fn regex_and_url_and_required() {
    #[derive(Validate)]
    struct Doc {
        #[validate(required)]
        title: String,
        #[validate(url)]
        homepage: String,
        #[validate(regex(pattern = "^[A-Z]"))]
        code: String,
    }
    let ok = Doc {
        title: "Hello".to_string(),
        homepage: "https://example.com".to_string(),
        code: "Xray".to_string(),
    };
    assert!(ok.validate().is_ok());

    let bad = Doc {
        title: "   ".to_string(),
        homepage: "not-a-url".to_string(),
        code: "lower".to_string(),
    };
    let errors = bad.validate().unwrap_err();
    assert_eq!(errors.len(), 3);
}

fn no_spaces(value: &str) -> Result<(), ValidationError> {
    if value.contains(' ') {
        Err(ValidationError::new("username", "No spaces allowed"))
    } else {
        Ok(())
    }
}

#[test]
fn custom_validator_is_invoked() {
    #[derive(Validate)]
    struct Account {
        #[validate(custom = "no_spaces")]
        username: String,
    }
    assert!(
        Account {
            username: "clean".to_string()
        }
        .validate()
        .is_ok()
    );
    assert!(
        Account {
            username: "has space".to_string()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn nested_validation_recurses() {
    #[derive(Validate)]
    struct Address {
        #[validate(required)]
        city: String,
        #[validate(length(min = 2, max = 2))]
        state: String,
    }

    #[derive(Validate)]
    struct Profile {
        #[validate(email)]
        email: String,
        #[validate]
        address: Address,
    }

    let ok = Profile {
        email: "a@b.com".to_string(),
        address: Address {
            city: "Portland".to_string(),
            state: "OR".to_string(),
        },
    };
    assert!(ok.validate().is_ok());

    // Nested failure surfaces through the parent.
    let bad = Profile {
        email: "a@b.com".to_string(),
        address: Address {
            city: "".to_string(),        // required fails
            state: "Oregon".to_string(), // length != 2
        },
    };
    let errors = bad.validate().unwrap_err();
    let fields: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
    assert!(fields.contains(&"city"), "{fields:?}");
    assert!(fields.contains(&"state"), "{fields:?}");
}
