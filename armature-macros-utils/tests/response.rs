//! Behavioral tests for the response macros: `json!`, `html!`, `text!`, `redirect!`.
//!
//! Every documented form is exercised, including the optional leading status
//! (numeric literal or `ok`/`permanent`/`temporary` alias). Against the old
//! code the multi-argument forms did not even parse and every response was
//! hard-coded to 200 (or 302 for redirect), so these tests are the regression
//! guard for the "status argument is ignored" bug.

use armature_core::HttpResponse;
use armature_macros_utils::{html, json, redirect, text};
use serde_json::json as sjson;

#[test]
fn json_defaults_to_200() {
    let r: HttpResponse = json!(sjson!({ "message": "hi" })).unwrap();
    assert_eq!(r.status, 200);
    let v: serde_json::Value = serde_json::from_slice(r.body_ref()).unwrap();
    assert_eq!(v["message"], "hi");
}

#[test]
fn json_honours_explicit_numeric_status() {
    let r = json!(201, sjson!({ "id": 1 })).unwrap();
    assert_eq!(r.status, 201);
}

#[test]
fn json_honours_ok_alias() {
    let r = json!(ok, sjson!({ "id": 1 })).unwrap();
    assert_eq!(r.status, 200);
}

#[test]
fn html_defaults_to_200_with_content_type() {
    let r = html!("<h1>Welcome</h1>").unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(
        r.headers.get("Content-Type").unwrap(),
        "text/html; charset=utf-8"
    );
    assert_eq!(r.body_string(), "<h1>Welcome</h1>");
}

#[test]
fn html_honours_explicit_status() {
    let r = html!(404, "<h1>Not Found</h1>").unwrap();
    assert_eq!(r.status, 404);
    assert_eq!(r.body_string(), "<h1>Not Found</h1>");
}

#[test]
fn text_defaults_to_200_with_content_type() {
    let r = text!("Hello, world!").unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(
        r.headers.get("Content-Type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(r.body_string(), "Hello, world!");
}

#[test]
fn text_honours_ok_alias() {
    let r = text!(ok, "done").unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body_string(), "done");
}

#[test]
fn redirect_defaults_to_302() {
    let r = redirect!("/home").unwrap();
    assert_eq!(r.status, 302);
    assert_eq!(r.headers.get("Location").unwrap(), "/home");
}

#[test]
fn redirect_honours_numeric_status() {
    let r = redirect!(301, "/new-location").unwrap();
    assert_eq!(r.status, 301);
    assert_eq!(r.headers.get("Location").unwrap(), "/new-location");
}

#[test]
fn redirect_honours_permanent_alias() {
    let r = redirect!(permanent, "/perm").unwrap();
    assert_eq!(r.status, 301);
}

#[test]
fn redirect_honours_temporary_alias() {
    let r = redirect!(temporary, "/temp").unwrap();
    assert_eq!(r.status, 302);
    assert_eq!(r.headers.get("Location").unwrap(), "/temp");
}
