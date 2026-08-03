//! Fuzz target for route matching / path parameter extraction.
//!
//! `armature_core::route_params::{match_path_zero_alloc, CompiledPattern}`
//! and `armature_core::simd_parser::extract_path_params` have zero
//! production callers: `grep -rln "route_params::" armature-core/src`
//! returns only `route_params.rs` itself, and `extract_path_params` is
//! self-labeled "Legacy" in its own doc comment. The real routing hot path
//! is `armature_core::routing::Router::route`, which matches paths via its
//! own private `match_path`/`split_segments` functions - never anything in
//! `route_params` or `extract_path_params`.
//!
//! `Router::route` itself needs a full async handler dispatch to fuzz
//! directly, but it delegates all of its path matching to the public,
//! synchronous `Router::match_route`, which runs the exact same
//! `match_path`/`split_segments` matching logic without executing a
//! handler. This target builds a real `Router` registered with a set of
//! representative route patterns and fuzzes `Router::match_route`, i.e. the
//! genuine production path-matching hot path.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::error::Error;
use armature_core::http::{HttpRequest, HttpResponse};
use armature_core::{HttpMethod, Route, Router};

/// Arbitrary path-matching scenario.
#[derive(Debug, Arbitrary)]
struct FuzzPathParams {
    /// Method to match against the registered routes.
    method: FuzzMethod,
    /// Path to match against the router.
    path: String,
}

#[derive(Debug, Arbitrary, Clone, Copy)]
enum FuzzMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Query,
}

impl FuzzMethod {
    fn as_http_method(self) -> HttpMethod {
        match self {
            FuzzMethod::Get => HttpMethod::GET,
            FuzzMethod::Post => HttpMethod::POST,
            FuzzMethod::Put => HttpMethod::PUT,
            FuzzMethod::Delete => HttpMethod::DELETE,
            FuzzMethod::Patch => HttpMethod::PATCH,
            FuzzMethod::Head => HttpMethod::HEAD,
            FuzzMethod::Options => HttpMethod::OPTIONS,
            FuzzMethod::Query => HttpMethod::QUERY,
        }
    }
}

// Dummy handler for route registration - never actually invoked, since
// `match_route` only performs matching and does not dispatch.
async fn dummy_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}

/// Build a router with route patterns representative of real applications
/// (single params, multiple params, and catch-alls), across multiple
/// methods so method-mismatch handling is exercised too.
fn build_router() -> Router {
    let mut router = Router::new();
    for (method, pattern) in [
        (HttpMethod::GET, "/users/:id"),
        (HttpMethod::GET, "/users/:user_id/posts/:post_id"),
        (HttpMethod::GET, "/api/v1/*path"),
        (HttpMethod::GET, "/files/*path"),
        (HttpMethod::GET, "/:org/:repo/tree/:branch/*path"),
        (HttpMethod::GET, "/**"),
        (HttpMethod::POST, "/users/:id"),
        (HttpMethod::PUT, "/api/v1/:resource/:id"),
        (HttpMethod::DELETE, "/api/v1/:resource/:id"),
        (HttpMethod::PATCH, "/api/v1/:resource/:id"),
    ] {
        router.add_route(Route::new(method, pattern, dummy_handler));
    }
    router
}

fuzz_target!(|data: FuzzPathParams| {
    if data.path.len() > 10000 {
        return;
    }

    let router = build_router();
    let method = data.method.as_http_method();

    // Real production matcher: `Router::match_route` is the same
    // `match_path`/`split_segments` logic that `Router::route` uses on the
    // live request-handling hot path, minus the async handler dispatch.
    if let Some((_, params)) = router.match_route(method.as_str(), &data.path) {
        let _ = params.len();
        for (k, v) in &params {
            let _ = k.len();
            let _ = v.is_empty();
            // Captured values are raw `Bytes` now, so parsing one is exactly
            // what a handler does: decode first, and a non-UTF-8 capture simply
            // has no numeric interpretation.
            if let Ok(s) = std::str::from_utf8(v) {
                let _ = s.parse::<i32>();
                let _ = s.parse::<u64>();
                let _ = s.parse::<f64>();
            }
        }
    }

    // Edge cases against the same real matcher.
    for edge_path in ["", "/", "/*", "//users//123//", "/**"] {
        let _ = router.match_route(method.as_str(), edge_path);
    }
    for edge_method in ["", "GET", "get", "TRACE"] {
        let _ = router.match_route(edge_method, &data.path);
    }
});
