//! Regression test for the Info finding: the crate's Quick Start doc
//! example (`armature-app/src/lib.rs`) called `module("AppModule")` to
//! create a module, but the module constructor is registered as
//! `create_module` because `module` is a reserved keyword in Rhai
//! (bindings.rs:73-74). Copy-pasting the documented example failed to
//! compile.
//!
//! `armature-app/src/lib.rs:26`
//!
//! This test extracts the literal ```rhai fenced code block from
//! `src/lib.rs`'s crate-level doc comment at test time and feeds it
//! through the exact same engine/bindings `armature_app::runner::run`
//! uses, so it will catch this specific regression again if the doc ever
//! drifts from what actually compiles and runs.
//!
//! It also guards a second doc fix made alongside this one: the example
//! originally called `ctx.call(...)`, which is *not* achievable at all —
//! `call` is Rhai's own reserved function-pointer-invocation keyword (see
//! `bindings.rs`'s `register_service_context_api` doc for the full
//! explanation) — so the doc now says `ctx.invoke(...)`, the one name that
//! actually works.

#[path = "support/mod.rs"]
mod support;

use armature_core::HttpRequest;

fn extract_rhai_fence(source: &str) -> String {
    let start_marker = "```rhai\n";
    let start = source
        .find(start_marker)
        .expect("lib.rs doc comment should contain a ```rhai fenced block")
        + start_marker.len();
    let end = source[start..]
        .find("```")
        .expect("```rhai fence should be closed");
    let fenced = &source[start..start + end];

    // Doc-comment lines are prefixed with `//! ` (or bare `//!` on blank
    // lines) — strip that to get back real Rhai source.
    fenced
        .lines()
        .map(|line| {
            line.strip_prefix("//! ")
                .or_else(|| line.strip_prefix("//!"))
                .unwrap_or(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extracted_quick_start_script() -> String {
    let lib_rs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read src/lib.rs");
    extract_rhai_fence(&lib_rs)
}

#[test]
fn quick_start_example_compiles_as_documented() {
    let script = extracted_quick_start_script();

    assert!(
        script.contains("create_module"),
        "sanity check: the extracted Quick Start block should mention create_module; extracted:\n{script}"
    );
    assert!(
        !script.contains("= module(\"AppModule\")"),
        "the Quick Start must not use the reserved keyword `module` as a function call; \
         extracted:\n{script}"
    );
    assert!(
        !script.contains("let data = ctx.call("),
        "the Quick Start must not actually invoke ctx.call(...) — `call` is a reserved Rhai \
         keyword and can never dispatch to a custom method (an explanatory comment mentioning \
         the name is fine); extracted:\n{script}"
    );
    assert!(
        script.contains("ctx.invoke("),
        "sanity check: the extracted Quick Start block should use ctx.invoke(...); extracted:\n{script}"
    );

    // It must actually compile through the same bindings
    // `armature_app::runner::run` registers.
    let engine = support::new_engine();
    engine.compile(&script).unwrap_or_else(|e| {
        panic!("Quick Start example must compile as documented: {e}\n\nscript:\n{script}")
    });
}

/// Stronger than compiling: the whole documented example must actually
/// build a working router and serve the request it describes.
#[tokio::test]
async fn quick_start_example_runs_end_to_end() {
    let script = extracted_quick_start_script();
    let router = support::build_router_from_script(&script);

    let request = HttpRequest::new("GET", "/api/users".to_string());
    let response = router
        .route(request)
        .await
        .expect("the documented GET /api/users route should be registered and dispatch");

    assert_eq!(
        response.status,
        200,
        "the documented example's handler must not error; body: {}",
        String::from_utf8_lossy(response.body_ref())
    );
    let body = String::from_utf8_lossy(response.body_ref());
    assert!(body.contains("Alice"), "response body was: {body}");
    assert!(body.contains("Bob"), "response body was: {body}");
}
