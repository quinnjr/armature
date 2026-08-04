//! Fuzz target for route registration and matching.
//!
//! Unlike `path_params.rs`, which fuzzes paths against a fixed, realistic set
//! of routes, this target lets the fuzzer choose the *patterns* too, so
//! `Router::add_route` itself (pattern compilation, segment splitting, wildcard
//! placement) is exercised alongside `Router::match_route`.

#![no_main]

use std::collections::HashSet;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::error::Error;
use armature_core::http::{HttpRequest, HttpResponse};
use armature_core::param_intern;
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

/// The segment substituted for a parameter when building a path that a pattern
/// is known to describe.
const PLACEHOLDER: &str = "x";

/// The parameter names a pattern declares, in this crate's own spelling.
///
/// Mirrors the router's own reading of a pattern: `:name` and `*name`, with a
/// bare `*` named `*`. Patterns written directly in `matchit`'s brace syntax
/// are deliberately not covered here - see the `braces` guard below.
fn declared_names(pattern: &str) -> Vec<&str> {
    pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .filter_map(|segment| {
            segment.strip_prefix(':').or_else(|| {
                segment
                    .strip_prefix('*')
                    .map(|name| if name.is_empty() { "*" } else { name })
            })
        })
        .collect()
}

/// Build a request path the pattern matches by construction: static segments
/// verbatim, one placeholder segment per parameter, and one segment for a
/// catch-all, which is where pattern validation ends.
fn concrete_path(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    for segment in pattern.split('/').filter(|segment| !segment.is_empty()) {
        out.push('/');
        if segment.starts_with(':') {
            out.push_str(PLACEHOLDER);
        } else if segment.starts_with('*') {
            out.push_str(PLACEHOLDER);
            // Anything after a catch-all is unreachable for the matcher too.
            break;
        } else {
            out.push_str(segment);
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

fuzz_target!(|data: FuzzRouting| {
    // Limit route count to prevent OOM
    let max_routes = 100;
    let routes: Vec<_> = data.routes.into_iter().take(max_routes).collect();

    // Create router
    let mut router = Router::new();

    // Register routes - should handle arbitrary patterns. The patterns that
    // actually made it in are kept so the invariants below can be stated
    // against the router's real contents rather than the fuzzer's wish list.
    let mut registered: Vec<(FuzzMethod, String)> = Vec::new();
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
            pattern.clone(),
            dummy_handler,
        ));
        registered.push((route.method, pattern));
    }

    // A path built by substituting concrete segments into a registered pattern
    // must match *something*. Which route answers it is deliberately not
    // asserted: registration order and the route tree's specificity rules both
    // bear on that, and a later pattern is free to be shadowed by an earlier
    // one. The claim is only that the router never 404s a path one of its own
    // routes describes.
    for (method, pattern) in &registered {
        if pattern.contains('?') {
            // `match_route` truncates the target at the first `?` before
            // matching, so a static segment containing one describes a path the
            // matcher can never be handed intact.
            continue;
        }
        let target = concrete_path(pattern);
        assert!(
            router
                .match_route(method.as_http_method().as_str(), &target)
                .is_some(),
            "{target:?} was built from the registered pattern {pattern:?} and must match"
        );
    }

    // Every name a pattern declares, and the widest declaration any single
    // pattern makes. Both are only sound while no pattern uses `matchit`'s
    // brace syntax: `{name}` reaches the route tree untranslated, so it is a
    // parameter to the matcher while being invisible to the reading above.
    // Such patterns are still registered and matched - they just do not get to
    // constrain the name set.
    // Either brace disqualifies the name set, not just `{`. `matchit` collapses
    // an escaped `}}` during normalization, so a pattern carrying only closing
    // braces — `/}}` is the shortest — can still hand back a capture whose name
    // nothing in `declared_names` ever saw.
    let braces = registered
        .iter()
        .any(|(_, pattern)| pattern.contains('{') || pattern.contains('}'));
    // Both spellings of a parameter name are admitted. `translate_pattern`
    // rewrites `:name` to `{name}`, so a name that itself begins with `*` lands
    // as `{*name}` — `matchit`'s catch-all spelling — and the tree captures it
    // with the star stripped, while the router pairs captures against
    // `param_names`, which read the star from the original pattern. Which of
    // the two comes back depends on whether the route reached the tree or the
    // fallback scan. Asserting either one specifically would be asserting a
    // detail of that routing decision rather than the property this check is
    // for, which is that a captured name came from a pattern at all and not
    // from the request.
    let declared: HashSet<&str> = registered
        .iter()
        .flat_map(|(_, pattern)| declared_names(pattern))
        .flat_map(|name| [name, name.strip_prefix('*').unwrap_or(name)])
        // The interner is hard-capped, and every capture past the cap comes
        // back under this sentinel instead of its own name. That is the
        // framework reporting exhaustion, not a name leaking in from the
        // request, so it belongs in the admitted set rather than tripping the
        // assertion this set exists for.
        .chain(std::iter::once(param_intern::OVERFLOW_NAME))
        .collect();
    let max_declared = registered
        .iter()
        .map(|(_, pattern)| declared_names(pattern).len())
        .max()
        .unwrap_or(0);

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

        // Exactly one registered pattern answers a match, so every captured
        // name has to be one that pattern declared, and no match can carry more
        // captures than the widest pattern declares. A name from outside that
        // set would mean it came from the request rather than the route.
        if !braces
            && let Some((_, params)) = router.match_route(method.as_http_method().as_str(), &path)
        {
            assert!(
                params.len() <= max_declared,
                "{path:?} captured {} parameters, more than any registered \
                 pattern declares: {params:?}",
                params.len()
            );
            for (name, _) in &params {
                assert!(
                    declared.contains(*name),
                    "{path:?} captured {name:?}, which no registered pattern declares"
                );
            }
        }

        // The target is split at the first `?` before matching, so a query
        // string can change neither what matched nor what was captured.
        let base = router
            .match_route(method.as_http_method().as_str(), &path)
            .map(|(_, params)| params);
        let queried = router
            .match_route(method.as_http_method().as_str(), &format!("{path}?a=1"))
            .map(|(_, params)| params);
        assert_eq!(
            base, queried,
            "a query string must not participate in matching {path:?}"
        );
    }
});
