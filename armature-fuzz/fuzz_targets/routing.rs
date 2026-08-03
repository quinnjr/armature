//! Fuzz target for route registration and matching.
//!
//! Unlike `path_params.rs`, which fuzzes paths against a fixed, realistic set
//! of routes, this target lets the fuzzer choose the *patterns* too, so
//! `Router::add_route` itself (pattern compilation, segment splitting, wildcard
//! placement) is exercised alongside `Router::match_route`.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::error::Error;
use armature_core::http::{HttpRequest, HttpResponse};
use armature_core::{HttpMethod, Route, Router};

/// Arbitrary routing scenario for fuzzing.
#[derive(Debug, Arbitrary)]
struct FuzzRouting {
    /// Routes to register
    routes: Vec<FuzzRoute>,
    /// Paths to match against
    match_paths: Vec<(FuzzMethod, String)>,
}

#[derive(Debug, Arbitrary)]
struct FuzzRoute {
    method: FuzzMethod,
    pattern: String,
}

#[derive(Debug, Arbitrary, Clone, Copy)]
enum FuzzMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
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
            FuzzMethod::Query => HttpMethod::QUERY,
        }
    }
}

// Dummy handler for route registration - never invoked, since `match_route`
// only performs matching and does not dispatch.
async fn dummy_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}

fuzz_target!(|data: FuzzRouting| {
    // Limit route count to prevent OOM
    let max_routes = 100;
    let routes: Vec<_> = data.routes.into_iter().take(max_routes).collect();

    // Create router
    let mut router = Router::new();

    // Register routes - should handle arbitrary patterns
    for route in &routes {
        // Normalize pattern to start with /
        let pattern = if route.pattern.starts_with('/') {
            route.pattern.clone()
        } else {
            format!("/{}", route.pattern)
        };

        // Skip extremely long patterns
        if pattern.len() > 1000 {
            continue;
        }

        router.add_route(Route::new(
            route.method.as_http_method(),
            pattern,
            dummy_handler,
        ));
    }

    // Match paths - should handle arbitrary input
    for (method, path) in &data.match_paths {
        // Normalize path
        let path = if path.starts_with('/') {
            path.clone()
        } else {
            format!("/{}", path)
        };

        // Skip extremely long paths
        if path.len() > 10000 {
            continue;
        }

        // Matching must not panic, and any captured params must be readable.
        if let Some((_, params)) = router.match_route(method.as_http_method().as_str(), &path) {
            for (name, value) in &params {
                let _ = name.len();
                let _ = value.len();
            }
        }
    }
});
