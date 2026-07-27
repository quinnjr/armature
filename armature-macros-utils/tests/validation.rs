//! Behavioral tests for `validate!`, `validate_required!`, and `validate_email!`.
//!
//! Against the old code the caller-supplied message was discarded (always
//! "Validation failed"), `validate_required!` accepted only a single field,
//! and the custom-function form did not parse. These tests pin the fixed
//! behavior: real messages, per-field required checks that name the missing
//! field, and the three-argument custom-validator form.

use armature_core::Error;
use armature_macros_utils::{validate, validate_email, validate_required};

fn check_age(age: i64) -> Result<(), Error> {
    validate!(age >= 18, "Must be 18 or older");
    Ok(())
}

#[test]
fn validate_condition_passes() {
    assert!(check_age(21).is_ok());
}

#[test]
fn validate_surfaces_caller_message() {
    match check_age(10) {
        Err(Error::Validation(m)) => assert_eq!(m, "Must be 18 or older"),
        other => panic!("unexpected: {other:?}"),
    }
}

fn is_valid_email(value: &str) -> bool {
    value.contains('@')
}

fn check_email_field(email: &str) -> Result<(), Error> {
    validate!(email, is_valid_email, "Invalid email format");
    Ok(())
}

#[test]
fn validate_custom_function_form() {
    assert!(check_email_field("a@b.com").is_ok());
    match check_email_field("nope") {
        Err(Error::Validation(m)) => assert_eq!(m, "Invalid email format"),
        other => panic!("unexpected: {other:?}"),
    }
}

fn require_all(
    name: Option<String>,
    email: Option<String>,
    password: Option<String>,
) -> Result<(), Error> {
    validate_required!(name, email, password);
    Ok(())
}

#[test]
fn validate_required_passes_when_all_present() {
    assert!(require_all(Some("a".into()), Some("b".into()), Some("c".into())).is_ok());
}

#[test]
fn validate_required_names_the_missing_field() {
    match require_all(Some("a".into()), None, Some("c".into())) {
        Err(Error::Validation(m)) => assert!(m.contains("email"), "message was: {m}"),
        other => panic!("unexpected: {other:?}"),
    }
}

fn check_email(value: &str) -> Result<(), Error> {
    validate_email!(value);
    Ok(())
}

#[test]
fn validate_email_accepts_valid() {
    assert!(check_email("user@example.com").is_ok());
}

#[test]
fn validate_email_rejects_invalid() {
    match check_email("not-an-email") {
        Err(Error::Validation(_)) => {}
        other => panic!("unexpected: {other:?}"),
    }
}
