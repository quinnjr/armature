//! Integration tests for common Armature workflows.
//!
//! These tests verify that the most common use cases work correctly.

#![allow(clippy::get_first)]
#![allow(clippy::unnecessary_get_then_check)]

use armature_core::*;
use bytes::Bytes;

// =============================================================================
// HTTP Response Tests
// =============================================================================

#[test]
fn test_http_response_convenience_methods() {
    // Test JSON response shorthand
    let response = HttpResponse::json(&serde_json::json!({"message": "hello"})).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"application/json".to_string())
    );

    // Test HTML response
    let response = HttpResponse::html("<h1>Hello</h1>");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"text/html; charset=utf-8".to_string())
    );
    assert_eq!(response.body, Bytes::from_static(b"<h1>Hello</h1>"));

    // Test text response
    let response = HttpResponse::text("Hello, World!");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"text/plain; charset=utf-8".to_string())
    );

    // Test redirect
    let response = HttpResponse::redirect("https://example.com");
    assert_eq!(response.status, 302);
    assert_eq!(
        response.headers.get("Location"),
        Some(&"https://example.com".to_string())
    );

    // Test permanent redirect
    let response = HttpResponse::redirect_permanent("https://example.com");
    assert_eq!(response.status, 301);

    // Test status code helpers
    assert_eq!(HttpResponse::unauthorized().status, 401);
    assert_eq!(HttpResponse::forbidden().status, 403);
    assert_eq!(HttpResponse::conflict().status, 409);
    assert_eq!(HttpResponse::service_unavailable().status, 503);
    assert_eq!(HttpResponse::accepted().status, 202);
    assert_eq!(HttpResponse::empty().status, 204);

    // Test fluent builder methods
    let response = HttpResponse::ok()
        .content_type("application/xml")
        .cache_control("max-age=3600")
        .with_body(b"<xml/>".to_vec());
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"application/xml".to_string())
    );
    assert_eq!(
        response.headers.get("Cache-Control"),
        Some(&"max-age=3600".to_string())
    );

    // Test no_cache
    let response = HttpResponse::ok().no_cache();
    assert!(
        response
            .headers
            .get("Cache-Control")
            .unwrap()
            .contains("no-store")
    );

    // Test cookie — armature-core 0.2.3 introduced multi-cookie support,
    // which moved Set-Cookie out of the headers map and into a dedicated
    // `cookies` Vec so multiple cookies can be emitted on one response.
    let response = HttpResponse::ok().cookie("session", "abc123; HttpOnly");
    assert_eq!(response.cookies.len(), 1);
    assert!(response.cookies[0].starts_with("session=abc123"));

    // Test status checks
    let ok = HttpResponse::ok();
    assert!(ok.is_success());
    assert!(!ok.is_redirect());
    assert!(!ok.is_client_error());
    assert!(!ok.is_server_error());

    let redirect = HttpResponse::redirect("/");
    assert!(!redirect.is_success());
    assert!(redirect.is_redirect());

    let not_found = HttpResponse::not_found();
    assert!(not_found.is_client_error());

    let error = HttpResponse::internal_server_error();
    assert!(error.is_server_error());
}

// =============================================================================
// Container Tests
// =============================================================================

#[test]
fn test_container_convenience_methods() {
    let container = Container::new();

    // Test register and resolve
    #[derive(Clone, Default)]
    struct Config {
        debug: bool,
    }

    container.register(Config { debug: true });

    // Test require (should not panic)
    let config = container.require::<Config>();
    assert!(config.debug);

    // Test get_or_default
    #[derive(Clone, Default)]
    struct OtherConfig {
        timeout: u32,
    }

    let other = container.get_or_default::<OtherConfig>();
    assert_eq!(other.timeout, 0); // Default value

    // Test register_if_missing
    assert!(!container.register_if_missing(Config { debug: false }));
    assert!(container.require::<Config>().debug); // Still true

    #[derive(Clone)]
    struct NewService;
    assert!(container.register_if_missing(NewService));
    assert!(container.has::<NewService>());
}

// =============================================================================
// Error Tests
// =============================================================================

#[test]
fn test_error_convenience_methods() {
    // Test convenience constructors
    let err = Error::bad_request("Invalid input");
    assert_eq!(err.status_code(), 400);

    let err = Error::unauthorized("No token");
    assert_eq!(err.status_code(), 401);

    let err = Error::forbidden("Access denied");
    assert_eq!(err.status_code(), 403);

    let err = Error::not_found("User not found");
    assert_eq!(err.status_code(), 404);

    let err = Error::conflict("Resource already exists");
    assert_eq!(err.status_code(), 409);

    let err = Error::internal("Something went wrong");
    assert_eq!(err.status_code(), 500);

    let err = Error::validation("Email is required");
    assert_eq!(err.status_code(), 400);

    let err = Error::timeout("Request took too long");
    assert_eq!(err.status_code(), 408);

    let err = Error::rate_limited("Too many requests");
    assert_eq!(err.status_code(), 429);

    let err = Error::unavailable("Under maintenance");
    assert_eq!(err.status_code(), 503);
}

#[test]
fn test_error_help_messages() {
    let err = Error::ProviderNotFound("MyService".to_string());
    assert!(err.help().is_some());
    assert!(err.help().unwrap().contains("register"));

    let err = Error::RouteNotFound("/api/users".to_string());
    assert!(err.help().is_some());
    assert!(err.help().unwrap().contains("controller"));

    let err = Error::Deserialization("invalid JSON".to_string());
    assert!(err.help().is_some());
    assert!(err.help().unwrap().contains("JSON"));

    let err = Error::Unauthorized("Invalid token".to_string());
    assert!(err.help().is_some());
    assert!(err.help().unwrap().contains("Authorization"));

    let err = Error::TooManyRequests("Rate limit exceeded".to_string());
    assert!(err.help().is_some());
    assert!(err.help().unwrap().contains("retry"));

    // Not all errors have help
    let err = Error::Internal("Unknown error".to_string());
    assert!(err.help().is_none());
}

// =============================================================================
// HTTP Request Tests
// =============================================================================

#[test]
fn test_http_request_helpers() {
    let mut request = HttpRequest::new("GET", "/api/users/123".to_string());
    request.push_param("id", "123");
    request.push_query_param("format", "json");
    request
        .headers
        .insert("Content-Type", "application/json".to_string());
    request.body = Bytes::from_static(b"{\"name\":\"John\"}");

    // Test param helper
    assert_eq!(request.param("id"), Some("123"));
    assert_eq!(request.param("unknown"), None);

    // Test query helper
    assert_eq!(request.query_param("format"), Some("json"));
    assert_eq!(request.query_param("unknown"), None);

    // Test json deserialization
    #[derive(serde::Deserialize)]
    struct UserInput {
        name: String,
    }
    let user: UserInput = request.json().unwrap();
    assert_eq!(user.name, "John");
}

// =============================================================================
// Router Round-Trip
// =============================================================================

/// A request actually dispatched through a `Router`.
///
/// Every other test in this file constructs a type and asserts on the value it
/// just built. This one registers a route, sends a real request through
/// `Router::route`, and asserts on what the handler saw and what came back -
/// path-param capture, query parsing on a target that carries a query string,
/// status, and 404 for an unregistered path.
#[tokio::test]
async fn test_router_dispatches_a_request_end_to_end() {
    async fn show_user(req: HttpRequest) -> Result<HttpResponse, Error> {
        // The handler reads what routing captured and what the target carried.
        let id = req.param("id").unwrap_or("<none>").to_owned();
        let verbose = req.query_param("verbose").unwrap_or("<none>").to_owned();
        Ok(HttpResponse::ok().with_body(format!("{id}|{verbose}").into_bytes()))
    }

    let mut router = Router::new();
    router.add_route(Route::new(HttpMethod::GET, "/users/:id", show_user));

    // A query string must not affect matching, must not be captured into the
    // path param, and must be readable through `query_param`.
    let response = router
        .route(HttpRequest::new("GET", "/users/1?verbose=true"))
        .await
        .expect("GET /users/1?verbose=true must dispatch");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, Bytes::from_static(b"1|true"));

    // Same route, no query string.
    let response = router
        .route(HttpRequest::new("GET", "/users/42"))
        .await
        .expect("GET /users/42 must dispatch");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, Bytes::from_static(b"42|<none>"));

    // An unregistered path is a routing error, not a 200.
    assert!(
        router
            .route(HttpRequest::new("GET", "/nope"))
            .await
            .is_err(),
        "an unregistered path must not dispatch"
    );
}

// =============================================================================
// Circuit Breaker Tests
// =============================================================================

#[test]
fn test_circuit_breaker_basic() {
    use armature_core::resilience::{CircuitBreaker, CircuitBreakerConfig, CircuitState};

    let config = CircuitBreakerConfig::default();

    let cb = CircuitBreaker::new(config);
    assert_eq!(cb.state(), CircuitState::Closed);

    // Record failures to trip the circuit
    for _ in 0..5 {
        cb.record_failure();
    }

    // After enough failures, circuit should be open
    assert_eq!(cb.state(), CircuitState::Open);
}

// =============================================================================
// Retry Tests
// =============================================================================

#[test]
fn test_retry_config() {
    use armature_core::resilience::{BackoffStrategy, RetryConfig};
    use std::time::Duration;

    // Test exponential backoff
    let config = RetryConfig::default();
    assert!(config.max_attempts > 0);

    // Destructuring a `Constant` built one line earlier only asserts that
    // `match` works, so assert what the strategy actually computes instead:
    // a constant backoff returns the same delay for every attempt.
    let backoff = BackoffStrategy::Constant(Duration::from_millis(500));
    let delays: Vec<Duration> = (1..=4)
        .map(|attempt| backoff.delay_for_attempt(attempt))
        .collect();
    assert_eq!(delays, vec![Duration::from_millis(500); 4]);
}

// =============================================================================
// Bulkhead Tests
// =============================================================================

#[tokio::test]
async fn test_bulkhead_basic() {
    use armature_core::resilience::{Bulkhead, BulkheadConfig};

    let config = BulkheadConfig::new("test", 2);
    let bulkhead = Bulkhead::new(config);

    // Check initial capacity
    assert!(bulkhead.has_capacity());
    assert_eq!(bulkhead.available_permits(), 2);

    // Stats should show proper counts
    let stats = bulkhead.stats();
    assert_eq!(stats.name, "test");
    assert_eq!(stats.max_concurrent, 2);
}

/// A bulkhead that never has a permit taken never demonstrates it bounds
/// anything. Hold both permits of a size-2 bulkhead and assert the third
/// caller is refused rather than admitted.
#[tokio::test]
async fn test_bulkhead_refuses_calls_beyond_capacity() {
    use armature_core::resilience::{Bulkhead, BulkheadConfig, BulkheadError};
    use tokio::sync::oneshot;

    let bulkhead = Bulkhead::new(BulkheadConfig::new("saturate", 2));

    // Two in-flight calls, each parked on a channel we control, so both permits
    // are genuinely held while the third attempt is made.
    let mut releases = Vec::new();
    let mut holders = Vec::new();
    for _ in 0..2 {
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let (entered_tx, entered_rx) = oneshot::channel::<()>();
        let bulkhead = bulkhead.clone();

        holders.push(tokio::spawn(async move {
            bulkhead
                .call(|| async move {
                    let _ = entered_tx.send(());
                    let _ = release_rx.await;
                    Ok::<(), std::convert::Infallible>(())
                })
                .await
        }));
        entered_rx.await.expect("call must enter the bulkhead");
        releases.push(release_tx);
    }

    assert_eq!(bulkhead.available_permits(), 0);
    assert!(!bulkhead.has_capacity());

    // Third caller: no permit is free, so it is rejected outright.
    let rejected = bulkhead
        .try_call(|| async { Ok::<(), std::convert::Infallible>(()) })
        .await;
    assert!(
        matches!(rejected, Err(BulkheadError::Full)),
        "a third concurrent call must be refused, got {rejected:?}"
    );

    // Release both holders; capacity comes back.
    for release in releases {
        let _ = release.send(());
    }
    for holder in holders {
        holder
            .await
            .expect("holder task")
            .expect("call must succeed");
    }
    assert_eq!(bulkhead.available_permits(), 2);
    assert!(
        bulkhead
            .try_call(|| async { Ok::<(), std::convert::Infallible>(()) })
            .await
            .is_ok(),
        "a freed permit must be reusable"
    );
}
