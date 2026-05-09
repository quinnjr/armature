//! Integration tests for common Armature workflows.
//!
//! These tests verify that the most common use cases work correctly.

#![allow(clippy::get_first)]
#![allow(clippy::unnecessary_get_then_check)]

use armature_core::*;

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
    assert_eq!(response.body, b"<h1>Hello</h1>".to_vec());

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
    let mut request = HttpRequest::new("GET".to_string(), "/api/users/123".to_string());
    request
        .path_params
        .insert("id".to_string(), "123".to_string());
    request
        .query_params
        .insert("format".to_string(), "json".to_string());
    request
        .headers
        .insert("Content-Type".to_string(), "application/json".to_string());
    request.body = b"{\"name\":\"John\"}".to_vec();

    // Test param helper
    assert_eq!(request.param("id"), Some(&"123".to_string()));
    assert_eq!(request.param("unknown"), None);

    // Test query helper
    assert_eq!(request.query("format"), Some(&"json".to_string()));
    assert_eq!(request.query("unknown"), None);

    // Test json deserialization
    #[derive(serde::Deserialize)]
    struct UserInput {
        name: String,
    }
    let user: UserInput = request.json().unwrap();
    assert_eq!(user.name, "John");
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

    // Test constant backoff
    let backoff = BackoffStrategy::Constant(Duration::from_millis(500));
    match backoff {
        BackoffStrategy::Constant(d) => assert_eq!(d.as_millis(), 500),
        _ => panic!("Expected constant backoff"),
    }
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
