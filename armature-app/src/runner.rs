//! Script runner — loads a Rhai script, builds the application, and starts the server.

use crate::bindings::register_app_api;
use crate::builder;
use crate::error::{AppError, Result};
use crate::types::ScriptApp;
use armature_core::{Application, Container};
use armature_rhai::register_armature_api;
use rhai::{AST, Engine, Scope};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info};

/// Configuration for the script runner.
#[derive(Default)]
pub struct RunConfig {
    /// Override port (takes precedence over script-defined port).
    pub port: Option<u16>,
    /// Override host.
    pub host: Option<String>,
}

/// Load and run a Rhai application script.
///
/// 1. Creates a Rhai engine with all armature-app + armature-rhai bindings
/// 2. Compiles and executes the script
/// 3. Extracts the ScriptApp from the scope
/// 4. Builds an armature-core Application from it
/// 5. Starts the HTTP server
pub async fn run(script_path: &Path, config: RunConfig) -> Result<()> {
    // 1. Create engine
    let mut engine = create_engine();

    // 2. Compile
    let script_dir = script_path.parent().unwrap_or_else(|| Path::new("."));
    let source =
        tokio::fs::read_to_string(script_path)
            .await
            .map_err(|_| AppError::ScriptNotFound {
                path: script_path.to_path_buf(),
            })?;

    // Set module resolver to load imports relative to script directory
    let resolver = rhai::module_resolvers::FileModuleResolver::new_with_path(script_dir);
    engine.set_module_resolver(resolver);

    let ast = engine.compile(&source).map_err(|e| AppError::Compilation {
        path: script_path.to_path_buf(),
        message: e.to_string(),
    })?;

    // 3. Execute script to build up the app structure
    let mut scope = Scope::new();
    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| AppError::Runtime {
            message: e.to_string(),
        })?;

    // 4. Extract the ScriptApp from scope
    let app_def = extract_app(&scope)?;

    let port = config.port.or(app_def.port).ok_or(AppError::NoPort)?;
    let host = config
        .host
        .or(app_def.host.clone())
        .unwrap_or_else(|| "0.0.0.0".to_string());
    // Resolve the host knob into a real bind address up front (fail fast,
    // before any lifecycle hooks run) instead of computing it and then
    // discarding it in favor of a hardcoded 0.0.0.0 bind.
    let bind_addr = resolve_bind_addr(&host, port)?;

    let engine = Arc::new(engine);
    let ast = Arc::new(ast);

    // 5. Fire on_module_init hooks
    builder::fire_init_hooks(&app_def.module, &engine, &ast);

    // 6. Fire on_bootstrap hook — a failure here is logged AND aborts
    // startup (unlike on_shutdown, where the server has already run).
    fire_bootstrap_hook(&app_def.on_bootstrap, &engine, &ast)?;

    // 7. Build the router from the module tree
    let router = builder::build_router(&app_def, Arc::clone(&engine), Arc::clone(&ast))?;

    // 8. Create and start the application
    let container = Container::new();
    let application = Application::new(container, router);

    info!(address = %bind_addr, "Starting Rhai application");
    application
        .listen_on(bind_addr)
        .await
        .map_err(AppError::Core)?;

    // 9. Fire shutdown hooks (after server stops)
    fire_shutdown_hook(&app_def.on_shutdown, &engine, &ast);
    builder::fire_destroy_hooks(&app_def.module, &engine, &ast);

    Ok(())
}

/// Fire the `on_bootstrap` hook, if any.
///
/// Mirrors `builder::fire_init_hooks`'s logging, but — unlike module init
/// hooks — a failing bootstrap hook also aborts startup: the caller must
/// propagate the returned `Err` rather than start the server anyway with
/// no diagnostic.
fn fire_bootstrap_hook(hook: &Option<rhai::FnPtr>, engine: &Engine, ast: &AST) -> Result<()> {
    if let Some(hook) = hook {
        info!("Firing on_bootstrap hook");
        if let Err(e) = hook.call::<()>(engine, ast, ()) {
            error!(error = %e, "on_bootstrap hook failed; aborting startup");
            return Err(AppError::Runtime {
                message: format!("on_bootstrap hook failed: {e}"),
            });
        }
    }
    Ok(())
}

/// Fire the `on_shutdown` hook, if any, logging (but not propagating) a
/// failure — matching `builder::fire_destroy_hooks`. The server has
/// already stopped running by this point, so there is nothing left to
/// abort; swallowing the return value entirely (as the old code did) just
/// meant a failing shutdown hook left no diagnostic at all.
fn fire_shutdown_hook(hook: &Option<rhai::FnPtr>, engine: &Engine, ast: &AST) {
    if let Some(hook) = hook {
        info!("Firing on_shutdown hook");
        if let Err(e) = hook.call::<()>(engine, ast, ()) {
            error!(error = %e, "on_shutdown hook failed");
        }
    }
}

/// Resolve the configured host/port into a bindable [`SocketAddr`].
///
/// Returns [`AppError::InvalidHost`] instead of silently falling back to a
/// default interface when `host` isn't a valid IP address literal. Note
/// that DNS hostnames (e.g. `"localhost"`) are deliberately *not* resolved
/// here — callers must supply a literal IP address.
fn resolve_bind_addr(host: &str, port: u16) -> Result<SocketAddr> {
    let ip: std::net::IpAddr = host.parse().map_err(|_| AppError::InvalidHost {
        host: host.to_string(),
    })?;
    Ok(SocketAddr::new(ip, port))
}

/// Create a Rhai engine with all application-building and HTTP bindings.
fn create_engine() -> Engine {
    let mut engine = Engine::new();

    // Armature HTTP bindings (Request, Response, helpers)
    register_armature_api(&mut engine);

    // App-building bindings (service, controller, module, etc.)
    register_app_api(&mut engine);

    // Sane defaults for script execution limits
    engine.set_max_operations(1_000_000);
    engine.set_max_call_levels(64);
    engine.set_max_string_size(1024 * 1024);
    engine.set_max_array_size(10_000);
    engine.set_max_map_size(10_000);

    engine
}

/// Extract the ScriptApp from the scope after script execution.
///
/// Looks for a variable of type ScriptApp in the scope.
fn extract_app(scope: &Scope) -> Result<ScriptApp> {
    for (_, _, value) in scope.iter() {
        if value.is::<ScriptApp>() {
            return Ok(value.cast::<ScriptApp>());
        }
    }
    Err(AppError::NoApplication)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::FnPtr;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    // -- resolve_bind_addr ---------------------------------------------
    //
    // Regression coverage for the Warning finding: `host` was computed
    // from config/script but then discarded — `Application::listen(port)`
    // always bound `0.0.0.0` regardless. `resolve_bind_addr` is the new
    // function `run()` now uses to build the real bind address; it did
    // not exist at all in the pre-fix code (the pre-fix `run()` just
    // called `application.listen(port)`, never touching `host` past the
    // log line).

    #[test]
    fn resolve_bind_addr_parses_ipv4_host() {
        let addr = resolve_bind_addr("127.0.0.1", 3000).expect("valid IPv4 host should resolve");
        assert_eq!(
            addr,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 3000)
        );
    }

    #[test]
    fn resolve_bind_addr_parses_ipv6_host() {
        let addr = resolve_bind_addr("::1", 8080).expect("valid IPv6 host should resolve");
        assert_eq!(addr, SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8080));
    }

    #[test]
    fn resolve_bind_addr_parses_unspecified_host() {
        let addr = resolve_bind_addr("0.0.0.0", 80).expect("0.0.0.0 should resolve");
        assert_eq!(addr, SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 80));
    }

    #[test]
    fn resolve_bind_addr_rejects_unparseable_host_with_a_clear_error() {
        let err = resolve_bind_addr("not-an-ip", 3000)
            .expect_err("garbage host must not silently resolve to some default interface");
        match err {
            AppError::InvalidHost { host } => assert_eq!(host, "not-an-ip"),
            other => panic!("expected AppError::InvalidHost, got: {other:?}"),
        }
    }

    #[test]
    fn resolve_bind_addr_rejects_bare_hostnames_dns_is_not_resolved() {
        // "localhost" is a hostname, not an IP literal — armature-app does
        // not perform DNS resolution, so this must error rather than
        // silently falling back to some default interface.
        let err = resolve_bind_addr("localhost", 3000)
            .expect_err("bare hostnames are not IP literals and must not silently succeed");
        assert!(matches!(err, AppError::InvalidHost { .. }));
    }

    // -- fire_bootstrap_hook / fire_shutdown_hook -----------------------
    //
    // Regression coverage for the Info finding: both hooks used to invoke
    // via `let _ = hook.call(...)`, discarding the error unconditionally.
    // `fire_bootstrap_hook` did not exist as a separate function in the
    // pre-fix code (the swallow was inlined directly in `run()`), so this
    // is new, function-doesn't-exist-yet RED coverage; the behavioral
    // contract (bootstrap aborts, shutdown doesn't) is what's under test.

    /// Compile a script whose final expression is a `FnPtr` (via Rhai's
    /// builtin `Fn("name")`) pointing at a function defined earlier in the
    /// same script, so the returned `FnPtr` is callable against `ast`.
    fn compile_hook(engine: &Engine, script: &str) -> (AST, FnPtr) {
        let ast = engine.compile(script).expect("hook script should compile");
        let hook: FnPtr = engine
            .eval_ast(&ast)
            .expect("hook script should evaluate to a function pointer");
        (ast, hook)
    }

    #[test]
    fn fire_bootstrap_hook_aborts_startup_by_returning_err_when_the_hook_fails() {
        let engine = create_engine();
        let (ast, hook) = compile_hook(
            &engine,
            r#"
                fn boom() { throw "bootstrap exploded"; }
                Fn("boom")
            "#,
        );

        let result = fire_bootstrap_hook(&Some(hook), &engine, &ast);
        let err = result.expect_err(
            "a failing on_bootstrap hook must abort startup by returning Err, not be swallowed",
        );
        assert!(
            err.to_string().contains("bootstrap exploded"),
            "error should carry the hook's own error message, got: {err}"
        );
    }

    #[test]
    fn fire_bootstrap_hook_is_a_noop_when_absent() {
        let engine = create_engine();
        let ast = engine.compile("()").unwrap();
        assert!(fire_bootstrap_hook(&None, &engine, &ast).is_ok());
    }

    #[test]
    fn fire_bootstrap_hook_succeeds_when_the_hook_succeeds() {
        let engine = create_engine();
        let (ast, hook) = compile_hook(
            &engine,
            r#"
                fn noop() { }
                Fn("noop")
            "#,
        );
        assert!(fire_bootstrap_hook(&Some(hook), &engine, &ast).is_ok());
    }

    #[test]
    fn fire_shutdown_hook_does_not_panic_or_propagate_when_the_hook_fails() {
        let engine = create_engine();
        let (ast, hook) = compile_hook(
            &engine,
            r#"
                fn boom() { throw "shutdown exploded"; }
                Fn("boom")
            "#,
        );

        // Must not panic. The return type is `()` — there is nothing to
        // propagate to, since the server has already stopped by the time
        // shutdown hooks run; the fix is that the failure is now logged
        // via `error!` instead of being discarded with `let _ = ...`.
        fire_shutdown_hook(&Some(hook), &engine, &ast);
    }
}
