//! Behavioral tests for the `#[use_guard]` / `#[guard]` decorators.

use armature_core::guard::{Guard, GuardContext};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{guard, use_guard};
use std::collections::HashMap;

#[derive(Default)]
struct AllowGuard;

#[async_trait::async_trait]
impl Guard for AllowGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(true)
    }
}

#[derive(Default)]
struct DenyGuard;

#[async_trait::async_trait]
impl Guard for DenyGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(false)
    }
}

/// A guard constructed with an argument, exercising the instance-expression form.
struct FlagGuard {
    allow: bool,
}

impl FlagGuard {
    fn new(allow: bool) -> Self {
        Self { allow }
    }
}

#[async_trait::async_trait]
impl Guard for FlagGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(self.allow)
    }
}

fn request() -> HttpRequest {
    HttpRequest::from_parts(
        "GET".to_string(),
        "/protected".to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}

// Type-based guard via `#[use_guard]`. The handler parameter is named `req`
// and referenced in the body: the wrapper must preserve that binding.
#[use_guard(AllowGuard)]
async fn allowed(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = req.method.clone();
    Ok(HttpResponse::ok())
}

#[use_guard(DenyGuard)]
async fn denied(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn passing_guard_runs_handler() {
    let resp = allowed(request()).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn failing_guard_blocks_handler() {
    let err = denied(request()).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)));
}

// Instance-based guard via `#[guard(expr)]`.
#[guard(FlagGuard::new(true))]
async fn allowed_instance(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[guard(FlagGuard::new(false))]
async fn denied_instance(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[tokio::test]
async fn instance_guard_runs_handler() {
    let resp = allowed_instance(request()).await.unwrap();
    assert_eq!(resp.status, 200);
}

#[tokio::test]
async fn instance_guard_can_block() {
    let err = denied_instance(request()).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)));
}
