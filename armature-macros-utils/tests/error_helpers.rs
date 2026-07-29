//! Behavioral tests for `bail!` and `ensure!`.
//!
//! Against the old code every `bail!`/`ensure!` produced a `BadRequest` with a
//! hard-coded message; the documented error-kind and format-args forms did not
//! parse. These tests pin the fixed behavior: the caller's message is
//! surfaced, format args are interpolated, and the error-kind ident selects
//! the matching `Error` variant.

use armature_core::Error;
use armature_macros_utils::{bail, ensure};

fn bail_simple() -> Result<(), Error> {
    bail!("User not found");
}

fn bail_with_kind_and_format(id: i64) -> Result<(), Error> {
    bail!(NotFound, "User {} not found", id);
}

#[test]
fn bail_defaults_to_bad_request_with_message() {
    match bail_simple() {
        Err(Error::BadRequest(m)) => assert_eq!(m, "User not found"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn bail_selects_kind_and_interpolates_format_args() {
    match bail_with_kind_and_format(42) {
        Err(Error::NotFound(m)) => assert_eq!(m, "User 42 not found"),
        other => panic!("unexpected: {other:?}"),
    }
}

fn ensure_default(cond: bool) -> Result<(), Error> {
    ensure!(cond, "User account is inactive");
    Ok(())
}

fn ensure_with_kind(age: i64) -> Result<(), Error> {
    ensure!(age >= 18, BadRequest, "Must be 18 or older");
    Ok(())
}

fn ensure_with_kind_and_format(id: i64) -> Result<(), Error> {
    ensure!(false, NotFound, "record {} missing", id);
    Ok(())
}

#[test]
fn ensure_passes_when_condition_true() {
    assert!(ensure_default(true).is_ok());
    assert!(ensure_with_kind(21).is_ok());
}

#[test]
fn ensure_surfaces_message_on_failure() {
    match ensure_default(false) {
        Err(Error::BadRequest(m)) => assert_eq!(m, "User account is inactive"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn ensure_selects_kind() {
    match ensure_with_kind(10) {
        Err(Error::BadRequest(m)) => assert_eq!(m, "Must be 18 or older"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn ensure_selects_kind_and_formats() {
    match ensure_with_kind_and_format(7) {
        Err(Error::NotFound(m)) => assert_eq!(m, "record 7 missing"),
        other => panic!("unexpected: {other:?}"),
    }
}
