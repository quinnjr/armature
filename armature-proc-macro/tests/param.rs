//! Behavioral tests for the `Param` derive.
//!
//! Two documented modes must both work:
//! - single-value types via `FromStr` (`from_param` / `from_param_opt`)
//! - multi-field structs where each named field is extracted from the
//!   same-named path parameter via `FromStr` (`from_request`)

use armature_core::HttpRequest;
use armature_proc_macro::Param;
use std::collections::HashMap;

fn request_with_params(pairs: &[(&str, &str)]) -> HttpRequest {
    let mut path_params = HashMap::new();
    for (k, v) in pairs {
        path_params.insert(k.to_string(), v.to_string());
    }
    HttpRequest::from_parts(
        "GET".to_string(),
        "/".to_string(),
        HashMap::new(),
        vec![],
        path_params,
        HashMap::new(),
    )
}

// Single-value mode (must keep working).
#[derive(Param)]
struct UserId(u32);

impl std::str::FromStr for UserId {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(UserId(s.parse()?))
    }
}

#[test]
fn single_value_from_param() {
    let req = request_with_params(&[("id", "42")]);
    let id = UserId::from_param(&req, "id").expect("must parse");
    assert_eq!(id.0, 42);
}

#[test]
fn single_value_from_param_missing() {
    let req = request_with_params(&[]);
    assert!(UserId::from_param(&req, "id").is_err());
    assert!(UserId::from_param_opt(&req, "id").is_none());
}

// Multi-field mode (documented example that never worked).
#[derive(Param)]
struct UserPostParams {
    user_id: u32,
    post_id: u32,
}

#[test]
fn multi_field_from_request() {
    let req = request_with_params(&[("user_id", "7"), ("post_id", "99")]);
    let params = UserPostParams::from_request(&req).expect("must extract both params");
    assert_eq!(params.user_id, 7);
    assert_eq!(params.post_id, 99);
}

#[test]
fn multi_field_missing_param_errors() {
    let req = request_with_params(&[("user_id", "7")]);
    assert!(UserPostParams::from_request(&req).is_err());
}

#[test]
fn multi_field_bad_value_errors() {
    let req = request_with_params(&[("user_id", "7"), ("post_id", "not-a-number")]);
    assert!(UserPostParams::from_request(&req).is_err());
}
