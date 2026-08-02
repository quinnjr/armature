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

// ---------------------------------------------------------------------------
// Every documented size form must scale by its unit. `512kb` and `1.5mb` are
// suffixed *literals*, not identifiers — dropping the suffix used to make
// `#[body_limit(512kb)]` a 512-*byte* limit and `#[body_limit(1gb)]` a 1-byte
// one, entirely silently.
// ---------------------------------------------------------------------------

#[body_limit(512kb)]
async fn kb_suffix(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    Ok(HttpResponse::ok())
}

#[body_limit("10mb")]
async fn mb_string(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    Ok(HttpResponse::ok())
}

#[body_limit(1.5mb)]
async fn float_suffix(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    Ok(HttpResponse::ok())
}

#[body_limit(1gb)]
async fn gb_suffix(request: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &request;
    Ok(HttpResponse::ok())
}

/// A body of exactly `limit` bytes must be accepted, and one byte more rejected.
async fn assert_limit<F, Fut>(handler: F, limit: usize)
where
    F: Fn(HttpRequest) -> Fut,
    Fut: std::future::Future<Output = Result<HttpResponse, Error>>,
{
    let ok = handler(request_with_body(vec![0u8; limit])).await;
    assert!(
        ok.is_ok(),
        "a body of exactly {limit} bytes must be allowed"
    );

    let err = handler(request_with_body(vec![0u8; limit + 1]))
        .await
        .expect_err("a body one byte over the limit must be rejected");
    assert!(matches!(err, Error::PayloadTooLarge(_)));
}

#[tokio::test]
async fn body_limit_honors_documented_size_forms() {
    assert_limit(kb_suffix, 512 * 1024).await;
    assert_limit(mb_string, 10 * 1024 * 1024).await;
    assert_limit(float_suffix, 1024 * 1024 + 512 * 1024).await;

    // Probing the 1 GiB boundary would mean allocating two 1 GiB bodies, so
    // this asserts only that a modest body is accepted — which is exactly what
    // the old suffix-dropping behavior got wrong (it read `1gb` as a *1 byte*
    // limit and rejected everything).
    assert!(
        gb_suffix(request_with_body(vec![0u8; 4096])).await.is_ok(),
        "#[body_limit(1gb)] must not reject a 4 KiB body"
    );
}
