//! Integration tests for every macro exported by `armature-macros`.
//!
//! `#[macro_export]`-ed `macro_rules!` macros are meant to be consumed from
//! external crates, so this file exercises them the same way a downstream
//! user would (`use armature_macros::prelude::*;`, `armature_macros::routes!`),
//! which is also the most faithful way to catch expansion/compile regressions
//! like the one this suite was written to catch (`routes!` did not compile
//! at all before this suite existed).

use armature_core::{Error, HttpMethod, HttpRequest, HttpResponse};
use armature_macros::prelude::*;
use armature_macros::routes;

fn json_body(resp: &HttpResponse) -> serde_json::Value {
    serde_json::from_slice(&resp.body_bytes()).expect("response body should be valid JSON")
}

fn request_with(
    path_params: &[(&str, &str)],
    query_params: &[(&str, &str)],
    headers: &[(&str, &str)],
) -> HttpRequest {
    let mut req = HttpRequest::new("GET", "/".to_string());
    for (k, v) in path_params {
        req.path_params.insert((*k).to_string(), (*v).to_string());
    }
    for (k, v) in query_params {
        req.push_query_param(*k, *v);
    }
    for (k, v) in headers {
        req.headers.insert(*k, *v);
    }
    req
}

// ---------------------------------------------------------------------
// Response macros: json_response!, ok_json!, created_json!, bad_request!,
// not_found!, internal_error!
// ---------------------------------------------------------------------

#[test]
fn json_response_sets_given_status_and_body() {
    let resp = json_response!(202, serde_json::json!({ "status": "processing" })).unwrap();
    assert_eq!(resp.status, 202);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "status": "processing" })
    );
}

#[test]
fn ok_json_returns_200_with_serialized_body() {
    let resp = ok_json!(serde_json::json!({ "message": "Success", "id": 123 })).unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "message": "Success", "id": 123 })
    );
}

#[test]
fn created_json_returns_201_with_serialized_body() {
    let resp = created_json!(serde_json::json!({ "id": 42 })).unwrap();
    assert_eq!(resp.status, 201);
    assert_eq!(json_body(&resp), serde_json::json!({ "id": 42 }));
}

#[test]
fn bad_request_returns_400_with_error_envelope() {
    let resp = bad_request!("Invalid input").unwrap();
    assert_eq!(resp.status, 400);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "error": "Invalid input", "status": 400 })
    );
}

#[test]
fn bad_request_supports_format_args() {
    let field = "email";
    let resp = bad_request!("Field '{}' is invalid", field).unwrap();
    assert_eq!(resp.status, 400);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "error": "Field 'email' is invalid", "status": 400 })
    );
}

#[test]
fn not_found_returns_404_with_error_envelope() {
    let user_id = 7;
    let resp = not_found!("User {} not found", user_id).unwrap();
    assert_eq!(resp.status, 404);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "error": "User 7 not found", "status": 404 })
    );
}

#[test]
fn internal_error_returns_500_with_error_envelope() {
    let resp = internal_error!("Database connection failed").unwrap();
    assert_eq!(resp.status, 500);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({ "error": "Database connection failed", "status": 500 })
    );
}

// ---------------------------------------------------------------------
// Parameter extraction: path_param!, path_params!, query_param!, header!
// ---------------------------------------------------------------------

fn extract_id(req: &HttpRequest) -> Result<i64, Error> {
    let id: i64 = path_param!(req, "id")?;
    Ok(id)
}

#[test]
fn path_param_parses_present_value() {
    let req = request_with(&[("id", "42")], &[], &[]);
    assert_eq!(extract_id(&req).unwrap(), 42);
}

#[test]
fn path_param_errors_when_missing() {
    let req = request_with(&[], &[], &[]);
    match extract_id(&req) {
        Err(Error::BadRequest(msg)) => assert!(msg.contains("Missing path parameter"), "{msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn path_param_errors_when_unparseable() {
    let req = request_with(&[("id", "not-a-number")], &[], &[]);
    match extract_id(&req) {
        Err(Error::BadRequest(msg)) => assert!(msg.contains("Invalid path parameter"), "{msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

fn extract_user_and_post(req: &HttpRequest) -> Result<(i64, i64), Error> {
    // `path_params!`'s own expansion already resolves to the parsed tuple
    // (its internal `?`s are the terminal operation per field), unlike
    // `path_param!` (singular), whose expansion is `Result<T, Error>`.
    let (user_id, post_id) = path_params!(req, "user_id": i64, "post_id": i64);
    Ok((user_id, post_id))
}

#[test]
fn path_params_extracts_multiple_values() {
    let req = request_with(&[("user_id", "1"), ("post_id", "2")], &[], &[]);
    assert_eq!(extract_user_and_post(&req).unwrap(), (1, 2));
}

#[test]
fn path_params_errors_when_any_missing() {
    let req = request_with(&[("user_id", "1")], &[], &[]);
    match extract_user_and_post(&req) {
        Err(Error::BadRequest(msg)) => assert!(msg.contains("post_id"), "{msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn query_param_returns_some_when_present_and_parseable() {
    let req = request_with(&[], &[("page", "3")], &[]);
    let page: Option<u32> = query_param!(req, "page");
    assert_eq!(page, Some(3));
}

#[test]
fn query_param_returns_none_when_missing() {
    let req = request_with(&[], &[], &[]);
    let page: Option<u32> = query_param!(req, "page");
    assert_eq!(page, None);
}

#[test]
fn query_param_returns_none_when_unparseable() {
    let req = request_with(&[], &[("page", "abc")], &[]);
    let page: Option<u32> = query_param!(req, "page");
    assert_eq!(page, None);
}

#[test]
fn header_hit_returns_value() {
    let req = request_with(&[], &[], &[("Content-Type", "application/json")]);
    let content_type = header!(req, "Content-Type").unwrap();
    assert_eq!(content_type, "application/json");
}

#[test]
fn header_miss_returns_bad_request_error() {
    let req = request_with(&[], &[], &[]);
    match header!(req, "Authorization") {
        Err(Error::BadRequest(msg)) => assert!(msg.contains("Missing header"), "{msg}"),
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

#[test]
fn header_optional_fallback_matches_documented_form() {
    let req = request_with(&[], &[], &[]);
    // Matches the corrected `header!` doc form (src/lib.rs): the macro
    // yields `Result<&String, Error>`, so the fallback must be `&String`,
    // not a `&str` literal (`.unwrap_or("text/plain")` is a type mismatch).
    let default = "text/plain".to_string();
    let content_type = header!(req, "Content-Type").unwrap_or(&default);
    assert_eq!(content_type, "text/plain");
}

// ---------------------------------------------------------------------
// Validation / guard macros: validation_error!, guard!
// ---------------------------------------------------------------------

fn validate_age(age: u32) -> Result<(), Error> {
    if age < 18 {
        return validation_error!("Age must be at least {}", 18);
    }
    Ok(())
}

#[test]
fn validation_error_short_circuits_with_validation_variant() {
    match validate_age(10) {
        Err(Error::Validation(msg)) => assert_eq!(msg, "Age must be at least 18"),
        other => panic!("expected Validation error, got {other:?}"),
    }
}

#[test]
fn validation_error_not_triggered_when_valid() {
    assert!(validate_age(21).is_ok());
}

struct User {
    is_admin: bool,
}

fn require_admin(user: &User) -> Result<(), Error> {
    guard!(user.is_admin, "Admin access required");
    Ok(())
}

#[test]
fn guard_passes_when_condition_true() {
    let user = User { is_admin: true };
    assert!(require_admin(&user).is_ok());
}

#[test]
fn guard_returns_forbidden_when_condition_false() {
    let user = User { is_admin: false };
    match require_admin(&user) {
        Err(Error::Forbidden(msg)) => assert_eq!(msg, "Admin access required"),
        other => panic!("expected Forbidden error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// json_object!
// ---------------------------------------------------------------------

#[test]
fn json_object_builds_expected_map() {
    let id = 1;
    let name = "Ada";
    let active = true;
    let built = json_object! {
        "id" => id,
        "name" => name,
        "active" => active,
    };
    assert_eq!(
        built,
        serde_json::json!({ "id": 1, "name": "Ada", "active": true })
    );
}

// ---------------------------------------------------------------------
// paginated_response!
// ---------------------------------------------------------------------

#[test]
fn paginated_response_reports_correct_fields_without_use_after_move() {
    // `users` is an owned Vec, matching the README's documented call form
    // (`paginated_response!(users, page, total)`). This guards that `$data`
    // is bound once (`__armature_paginated_data`) and reused for both the
    // serialized `data` field and the `per_page` length, so a side-effecting
    // `$data` expression is evaluated only once and `per_page`/`len()` stay
    // in sync with the serialized payload.
    let users = vec!["alice".to_string(), "bob".to_string(), "carol".to_string()];
    let expected_data = users.clone();
    let page = 2u32;
    let total = 42u64;

    let resp = paginated_response!(users, page, total).unwrap();

    assert_eq!(resp.status, 200);
    assert_eq!(
        json_body(&resp),
        serde_json::json!({
            "data": expected_data,
            "pagination": {
                "page": page,
                "total": total,
                "per_page": expected_data.len(),
            }
        })
    );
}

// ---------------------------------------------------------------------
// log_error!
// ---------------------------------------------------------------------

fn failing_operation() -> Result<(), Error> {
    let db_err = "connection refused";
    log_error!("Failed to connect to database: {}", db_err)
}

#[test]
fn log_error_logs_and_returns_internal_error() {
    match failing_operation() {
        Err(Error::Internal(msg)) => {
            assert_eq!(msg, "Failed to connect to database: connection refused")
        }
        other => panic!("expected Internal error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// routes!
// ---------------------------------------------------------------------

async fn list_users(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}

async fn create_user(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::created())
}

async fn get_user(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}

#[test]
fn routes_macro_expands_and_builds_route_vec() {
    // Regression test for the `routes!` macro: it used to build a `Route`
    // struct literal with `handler: Arc::new($handler)`, which does not
    // compile because `Route::handler` is a `BoxedHandler` (constructible
    // only via `BoxedHandler::new`/`Route::new`), not an `Arc<H>`. If this
    // test compiles at all, the fix is verified; the assertions further
    // check that the expansion wires up method/path correctly per entry.
    let built = routes! {
        GET "/users" => list_users,
        POST "/users" => create_user,
        GET "/users/:id" => get_user,
    };

    assert_eq!(built.len(), 3);

    assert_eq!(built[0].method, HttpMethod::GET);
    assert_eq!(built[0].path, "/users");

    assert_eq!(built[1].method, HttpMethod::POST);
    assert_eq!(built[1].path, "/users");

    assert_eq!(built[2].method, HttpMethod::GET);
    assert_eq!(built[2].path, "/users/:id");
}

#[test]
fn routes_macro_supports_trailing_comma_and_single_entry() {
    let built = routes! {
        GET "/health" => list_users,
    };
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].method, HttpMethod::GET);
    assert_eq!(built[0].path, "/health");
}
