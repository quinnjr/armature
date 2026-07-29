//! Shared test helpers for `armature-app` integration tests.
//!
//! This file is *not* a standalone test binary — it lives under
//! `tests/support/` (a subdirectory, not `tests/support.rs`) precisely so
//! Cargo does not treat it as its own test target. Each test file that
//! needs these helpers pulls it in with:
//!
//! ```ignore
//! #[path = "support/mod.rs"]
//! mod support;
//! ```
//!
//! Not every test file uses every helper here (this file is compiled fresh
//! into each test binary that includes it), so unused-function warnings
//! are expected and suppressed at the module level rather than per-binary.

#![allow(dead_code)]

use armature_app::bindings::register_app_api;
use armature_app::builder;
use armature_app::types::ScriptApp;
use armature_core::Router;
use armature_rhai::register_armature_api;
use rhai::{Engine, Scope};
use std::sync::Arc;

/// Build a fresh Rhai engine with the same bindings `armature_app::runner`
/// wires up (armature-rhai's HTTP bindings + armature-app's application
/// bindings).
pub fn new_engine() -> Engine {
    let mut engine = Engine::new();
    register_armature_api(&mut engine);
    register_app_api(&mut engine);
    engine
}

/// Compile and run a script, returning the `ScriptApp` it defined (via
/// `create_app(module)`), or a `String` error describing what went wrong
/// (compile error, runtime error, or "no ScriptApp defined").
///
/// This mirrors `armature_app::runner::run`'s steps 1-4 without ever
/// starting a network listener, so tests can assert on script-level
/// failures (e.g. a reserved keyword, a type-mismatched setter) directly.
pub fn try_build_app(script: &str) -> Result<ScriptApp, String> {
    let engine = new_engine();

    let ast = engine.compile(script).map_err(|e| e.to_string())?;

    let mut scope = Scope::new();
    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| e.to_string())?;

    scope
        .iter()
        .find_map(|(_, _, value)| value.try_cast::<ScriptApp>())
        .ok_or_else(|| "script did not define an application via create_app(module)".to_string())
}

/// Compile, run, and build a full `armature_core::Router` from a script —
/// the same pipeline `armature_app::runner::run` uses, minus the actual
/// network listen. Panics with a descriptive message on any failure (this
/// is the "happy path" helper for tests that then dispatch requests
/// against the returned router).
pub fn build_router_from_script(script: &str) -> Router {
    let engine = new_engine();

    let ast = engine
        .compile(script)
        .unwrap_or_else(|e| panic!("script should compile: {e}"));

    let mut scope = Scope::new();
    engine
        .run_ast_with_scope(&mut scope, &ast)
        .unwrap_or_else(|e| panic!("script should run without error: {e}"));

    let app_def = scope
        .iter()
        .find_map(|(_, _, value)| value.try_cast::<ScriptApp>())
        .expect("script should define an application via create_app(module)");

    builder::build_router(&app_def, Arc::new(engine), Arc::new(ast))
        .unwrap_or_else(|e| panic!("router should build from the ScriptApp: {e}"))
}
