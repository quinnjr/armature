//! Script runner — loads a Rhai script, builds the application, and starts the server.

use crate::bindings::register_app_api;
use crate::builder;
use crate::error::{AppError, Result};
use crate::types::ScriptApp;
use armature_core::{Application, Container};
use armature_rhai::register_armature_api;
use rhai::{Engine, Scope};
use std::path::Path;
use std::sync::Arc;
use tracing::info;

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

    let engine = Arc::new(engine);
    let ast = Arc::new(ast);

    // 5. Fire on_module_init hooks
    builder::fire_init_hooks(&app_def.module, &engine, &ast);

    // 6. Fire on_bootstrap hook
    if let Some(ref hook) = app_def.on_bootstrap {
        info!("Firing on_bootstrap hook");
        let _ = hook.call::<()>(&engine, &ast, ());
    }

    // 7. Build the router from the module tree
    let router = builder::build_router(&app_def, Arc::clone(&engine), Arc::clone(&ast))?;

    // 8. Create and start the application
    let container = Container::new();
    let application = Application::new(container, router);

    info!(port = port, host = %host, "Starting Rhai application");
    application.listen(port).await.map_err(AppError::Core)?;

    // 9. Fire shutdown hooks (after server stops)
    if let Some(ref hook) = app_def.on_shutdown {
        info!("Firing on_shutdown hook");
        let _ = hook.call::<()>(&engine, &ast, ());
    }
    builder::fire_destroy_hooks(&app_def.module, &engine, &ast);

    Ok(())
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
