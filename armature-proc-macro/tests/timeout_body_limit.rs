//! `#[timeout]` / `#[body_limit]` must use the handler's real request
//! parameter name, not a hard-coded `req`.

use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{body_limit, timeout};
use std::collections::HashMap;

fn request_with_body(body: Vec<u8>) -> HttpRequest {
    HttpRequest::from_parts(
        "POST",
        "/".to_string(),
        HashMap::new(),
        body,
        HashMap::new(),
        HashMap::new(),
    )
}

// Parameter deliberately named `request`, not `req`.
#[timeout(ms = 20)]
async fn quick(request: HttpRequest) -> Result<HttpResponse, Error> {
    // Reference the parameter so an unused-variable lint cannot hide a bug.
    let _ = request.method.clone();
    Ok(HttpResponse::ok())
}

#[timeout(ms = 20)]
async fn slow(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    Ok(HttpResponse::ok())
}

#[body_limit(8)]
async fn upload(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn timeout_allows_fast_handler() {
    let resp = quick(request_with_body(vec![])).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn timeout_rejects_slow_handler() {
    let err = slow(request_with_body(vec![])).await.unwrap_err();
    assert!(matches!(err, Error::RequestTimeout(_)));
}

#[tokio::test]
async fn body_limit_allows_small_body() {
    let resp = upload(request_with_body(vec![0u8; 4])).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn body_limit_rejects_large_body() {
    let err = upload(request_with_body(vec![0u8; 32])).await.unwrap_err();
    assert!(matches!(err, Error::PayloadTooLarge(_)));
}
