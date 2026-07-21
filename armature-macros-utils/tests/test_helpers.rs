//! Behavioral tests for `test_request!`, `assert_json!`, and `assert_status!`.
//!
//! Against the old code `test_request!` discarded its input and always returned
//! `GET /`, while `assert_json!`/`assert_status!` evaluated their argument and
//! asserted nothing (false-green). The `#[should_panic]` tests below are the
//! regression guard for the hollow assert macros: they can only panic once the
//! macros perform a real comparison.

use armature_core::HttpResponse;
use armature_macros_utils::{assert_json, assert_status, test_request};

#[test]
fn test_request_builds_get() {
    let req = test_request!(GET "/users");
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/users");
    assert!(req.body_ref().is_empty());
}

#[test]
fn test_request_builds_post_with_body() {
    let req = test_request!(POST "/users", serde_json::json!({ "name": "Alice" }));
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/users");
    let v: serde_json::Value = serde_json::from_slice(req.body_ref()).unwrap();
    assert_eq!(v["name"], "Alice");
    assert_eq!(
        req.headers.get("Content-Type").unwrap(),
        "application/json"
    );
}

#[test]
fn test_request_builds_with_headers() {
    let req = test_request!(GET "/users/123", headers: { "Authorization": "Bearer token" });
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/users/123");
    // HeaderMap lookup is case-insensitive.
    assert_eq!(
        req.headers.get("authorization").unwrap(),
        "Bearer token"
    );
}

#[test]
fn test_request_builds_with_body_and_headers() {
    let req = test_request!(
        POST "/login",
        serde_json::json!({ "user": "bob" }),
        headers: { "X-Trace": "abc", "Accept": "application/json" }
    );
    assert_eq!(req.method, "POST");
    let v: serde_json::Value = serde_json::from_slice(req.body_ref()).unwrap();
    assert_eq!(v["user"], "bob");
    assert_eq!(req.headers.get("X-Trace").unwrap(), "abc");
    assert_eq!(req.headers.get("Accept").unwrap(), "application/json");
}

fn sample_response() -> HttpResponse {
    HttpResponse::new(200)
        .with_json(&serde_json::json!({ "id": 1, "name": "Alice" }))
        .unwrap()
}

#[test]
fn assert_status_matches_numeric() {
    let resp = sample_response();
    assert_status!(resp, 200);
}

#[test]
fn assert_status_matches_ok_alias() {
    let resp = sample_response();
    assert_status!(resp, ok);
}

#[test]
#[should_panic(expected = "status")]
fn assert_status_panics_on_mismatch() {
    let resp = sample_response();
    assert_status!(resp, 404);
}

#[test]
fn assert_json_matches_object_literal() {
    let resp = sample_response();
    assert_json!(resp, { "id": 1, "name": "Alice" });
}

#[test]
#[should_panic(expected = "assert_json!: response body did not match expected JSON")]
fn assert_json_panics_on_mismatch() {
    let resp = sample_response();
    assert_json!(resp, { "id": 2, "name": "Bob" });
}
