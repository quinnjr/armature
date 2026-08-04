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
use armature_core::{HttpMethod, Route, RouteParamsExt, Router};

/// Arbitrary path-matching scenario.
#[derive(Debug, Arbitrary)]
struct FuzzPathParams {
    /// Method to match against the registered routes.
    method: FuzzMethod,
    /// Path to match against the router.
    path: String,
    /// Concrete segments substituted into the registered patterns to build
    /// targets whose capture is known in advance.
    segments: Vec<String>,
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

/// Every parameter name the patterns in `build_router` declare.
///
/// `/**` captures under the name `*`: the router strips one leading `*` and
/// substitutes `*` for the empty remainder.
/// Every parameter name the patterns above declare.
///
/// Kept in sync by hand, so it is worth naming where each comes from:
/// `id`/`user_id`/`post_id` from the `/users/...` routes, `resource` from the
/// `/api/v1/:resource/:id` trio, `org`/`repo`/`branch`/`path` from the tree
/// route, and `*` from the bare catch-all.
const DECLARED_NAMES: [&str; 9] = [
    "id", "user_id", "post_id", "resource", "path", "org", "repo", "branch", "*",
];

/// The most parameters any registered pattern declares, from
/// `/:org/:repo/tree/:branch/*path`.
const MAX_DECLARED_PARAMS: usize = 4;

/// Whether a fuzzer-chosen string can stand in for exactly one path segment.
///
/// It must survive segment splitting as a single non-empty segment (no `/`,
/// not empty) and must not be truncated by the query-string split that
/// `match_route` performs before matching (no `?`).
fn is_plain_segment(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains('?')
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
        // Only the registered patterns can produce captures, and only one of
        // them can win, so the names come from a closed set and cannot
        // outnumber the largest pattern's declarations. A capture named
        // anything else means a name leaked across routes or was interned from
        // the request rather than the pattern.
        assert!(
            params.len() <= MAX_DECLARED_PARAMS,
            "captured {} parameters, more than any registered pattern declares: {params:?}",
            params.len()
        );
        for (name, value) in &params {
            assert!(
                DECLARED_NAMES.contains(name),
                "captured an undeclared parameter name {name:?}"
            );
            // Captured values are raw `Bytes` now, so parsing one is exactly
            // what a handler does: decode first, and a non-UTF-8 capture simply
            // has no numeric interpretation.
            if let Ok(s) = std::str::from_utf8(value) {
                let _ = s.parse::<i32>();
                let _ = s.parse::<u64>();
                let _ = s.parse::<f64>();
            }
        }
    }

    // `/**` is registered for GET and a catch-all consumes zero or more
    // segments, so GET matching is total: every target reaches that route if
    // nothing registered earlier claims it first.
    assert!(
        router.match_route("GET", &data.path).is_some(),
        "GET must always fall through to /**: {:?}",
        data.path
    );

    // `match_route` splits the target at the first `?` before matching, so
    // appending a query string can change neither what matched nor what was
    // captured - including when the target already carries one, since the
    // prefix before the first `?` is unchanged either way.
    let base = router.match_route(method.as_str(), &data.path).map(|(_, p)| p);
    let queried = router
        .match_route(method.as_str(), &format!("{}?a=1", data.path))
        .map(|(_, p)| p);
    assert_eq!(
        base, queried,
        "a query string must not participate in matching"
    );

    // TRACE has no method slot and no registered route, so it is unroutable by
    // construction: neither the tree nor the fallback scan can answer it.
    assert!(
        router.match_route("TRACE", &data.path).is_none(),
        "TRACE is not a registered method and must never match"
    );

    // Targets built by substituting concrete segments into the registered
    // patterns, where the expected capture is known ahead of time. Only
    // segments that survive splitting intact are usable, and the list is capped
    // so a probe path stays a reasonable length.
    let plain: Vec<&str> = data
        .segments
        .iter()
        .map(String::as_str)
        .filter(|s| is_plain_segment(s))
        .take(8)
        .collect();

    // `/users/:id` is the first route registered, so nothing can shadow it: a
    // two-segment `/users/<x>` target must capture exactly `id = x`.
    if let Some(id) = plain.first() {
        let target = format!("/users/{id}");
        let (_, params) = router
            .match_route("GET", &target)
            .unwrap_or_else(|| panic!("{target:?} must match /users/:id"));
        assert_eq!(params.len(), 1, "{target:?} captured {params:?}");
        assert_eq!(params[0].0, "id", "{target:?} captured {params:?}");
        assert_eq!(
            &params[0].1[..],
            id.as_bytes(),
            "{target:?} must capture the segment verbatim"
        );
    }

    // `/users/:id` matches exactly two segments, so it cannot claim a
    // four-segment target; `/users/:user_id/posts/:post_id` is the first that
    // can, and both captures are pinned by position in the pattern.
    if plain.len() >= 2 {
        let (user, post) = (plain[0], plain[1]);
        let target = format!("/users/{user}/posts/{post}");
        let (_, params) = router
            .match_route("GET", &target)
            .unwrap_or_else(|| panic!("{target:?} must match /users/:user_id/posts/:post_id"));
        assert_eq!(params.len(), 2, "{target:?} captured {params:?}");
        assert_eq!(
            params.get_bytes("user_id").map(|v| &v[..]),
            Some(user.as_bytes()),
            "{target:?} captured {params:?}"
        );
        assert_eq!(
            params.get_bytes("post_id").map(|v| &v[..]),
            Some(post.as_bytes()),
            "{target:?} captured {params:?}"
        );
    }

    // `/api/v1/*path` is registered before the later catch-alls, and neither
    // `/users/...` route can claim a target whose first segment is `api`, so
    // the capture is the remainder verbatim - substituting it back into the
    // pattern reconstructs the target.
    //
    // At least one remaining segment is required. A catch-all consuming zero
    // segments is served by the fallback scan rather than the route tree, and
    // which of two overlapping catch-alls answers it then depends on how the
    // tree and the scan divide the work, which is not a promise this target
    // should hold the router to.
    if !plain.is_empty() {
        let rest = plain.join("/");
        let target = format!("/api/v1/{rest}");
        let (_, params) = router
            .match_route("GET", &target)
            .unwrap_or_else(|| panic!("{target:?} must match /api/v1/*path"));
        assert_eq!(params.len(), 1, "{target:?} captured {params:?}");
        assert_eq!(params[0].0, "path", "{target:?} captured {params:?}");
        assert_eq!(
            &params[0].1[..],
            rest.as_bytes(),
            "the catch-all must capture the remainder verbatim, so that \
             /api/v1/ + capture reconstructs the target"
        );
    }

    // `/:org/:repo/tree/:branch/*path` mixes leading parameters with a
    // catch-all. Its first segment is a parameter, so every earlier route with a
    // static first segment is a potential claimant; excluding those three
    // prefixes leaves this pattern as the only one that can answer, without the
    // property having to depend on how the route tree backtracks out of a static
    // branch that turns out not to match.
    if plain.len() >= 4 && !matches!(plain[0], "api" | "files" | "users") {
        let (org, repo, branch) = (plain[0], plain[1], plain[2]);
        let rest = plain[3..].join("/");
        let target = format!("/{org}/{repo}/tree/{branch}/{rest}");
        let (_, params) = router
            .match_route("GET", &target)
            .unwrap_or_else(|| panic!("{target:?} must match /:org/:repo/tree/:branch/*path"));
        assert_eq!(params.len(), 4, "{target:?} captured {params:?}");
        for (name, expected) in [
            ("org", org),
            ("repo", repo),
            ("branch", branch),
            ("path", rest.as_str()),
        ] {
            assert_eq!(
                params.get_bytes(name).map(|v| &v[..]),
                Some(expected.as_bytes()),
                "{target:?} captured {params:?}"
            );
        }
    }

    // Edge cases against the same real matcher. These stay unasserted: `/*`,
    // `/**` and `//users//123//` as *targets* are literal segments, and which
    // catch-all answers them is exactly the tree-versus-scan question left
    // alone above.
    for edge_path in ["", "/", "/*", "//users//123//", "/**"] {
        let _ = router.match_route(method.as_str(), edge_path);
    }
    for edge_method in ["", "GET", "get", "TRACE"] {
        let _ = router.match_route(edge_method, &data.path);
    }
});
