//! Behavioral tests for the `#[catch]` exception-filter decorator.
//!
//! These use the decorator exactly as documented in
//! `armature_core::exception_filter` and drive the generated filter through the
//! `ExceptionFilter` trait.

use armature_core::exception_filter::{ExceptionContext, ExceptionFilter};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::catch;

fn ctx() -> ExceptionContext {
    ExceptionContext::from_request(HttpRequest::new("GET".to_string(), "/x".to_string()))
}

// Catch specific error types.
#[catch(NotFound, RouteNotFound)]
async fn handle_not_found(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::not_found()
}

#[tokio::test]
async fn typed_filter_reports_its_types_and_catches() {
    let filter = HandleNotFoundExceptionFilter::new();
    assert_eq!(filter.handles(), Some(vec!["NotFound", "RouteNotFound"]));
    assert_eq!(filter.priority(), 0);

    let resp = filter.catch(&Error::NotFound("x".to_string()), &ctx()).await;
    assert_eq!(resp.expect("must handle").status, 404);
}

// Catch-all filter (no error types).
#[catch]
async fn handle_all(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::internal_server_error()
}

#[tokio::test]
async fn catch_all_handles_everything() {
    let filter = HandleAllExceptionFilter::new();
    assert_eq!(filter.handles(), None);

    let resp = filter
        .catch(&Error::Internal("boom".to_string()), &ctx())
        .await;
    assert_eq!(resp.expect("must handle").status, 500);
}

// Priority is honored.
#[catch(Validation, priority = 100)]
async fn handle_validation(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::new(422)
}

#[tokio::test]
async fn priority_is_applied() {
    let filter = HandleValidationExceptionFilter::new();
    assert_eq!(filter.priority(), 100);
    assert_eq!(filter.handles(), Some(vec!["Validation"]));

    let resp = filter
        .catch(&Error::Validation("bad".to_string()), &ctx())
        .await;
    assert_eq!(resp.expect("must handle").status, 422);
}

// The generated factory function returns the filter instance.
#[tokio::test]
async fn factory_function_constructs_filter() {
    let filter = handle_not_found();
    assert_eq!(filter.name(), "HandleNotFoundExceptionFilter");
}
