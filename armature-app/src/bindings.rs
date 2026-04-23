//! Register all armature-app types and constructors into the Rhai engine.

use crate::types::{
    ScriptApp, ScriptController, ScriptGuard, ScriptMiddleware, ScriptModule, ScriptService,
    ServiceContext,
};
use rhai::{Dynamic, Engine, EvalAltResult, NativeCallContext};

/// Register the full application-building API with the Rhai engine.
pub fn register_app_api(engine: &mut Engine) {
    register_service_api(engine);
    register_controller_api(engine);
    register_guard_api(engine);
    register_middleware_api(engine);
    register_module_api(engine);
    register_app_api_types(engine);
    register_service_context_api(engine);
}

// ---------------------------------------------------------------------------
// service()
// ---------------------------------------------------------------------------

fn register_service_api(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptService>("ScriptService");
    engine.register_fn("service", ScriptService::new);
    engine.register_fn("define", ScriptService::define);
}

// ---------------------------------------------------------------------------
// controller()
// ---------------------------------------------------------------------------

fn register_controller_api(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptController>("ScriptController");
    engine.register_fn("controller", ScriptController::new);
    engine.register_fn("get", ScriptController::get);
    engine.register_fn("post", ScriptController::post);
    engine.register_fn("put", ScriptController::put);
    engine.register_fn("delete", ScriptController::delete);
    engine.register_fn("patch", ScriptController::patch);
    engine.register_fn("use_guard", ScriptController::use_guard);
    engine.register_fn("use_middleware", ScriptController::use_middleware);
}

// ---------------------------------------------------------------------------
// guard()
// ---------------------------------------------------------------------------

fn register_guard_api(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptGuard>("ScriptGuard");
    engine.register_fn("guard", ScriptGuard::new);
    engine.register_fn("can_activate", ScriptGuard::can_activate);
}

// ---------------------------------------------------------------------------
// middleware()
// ---------------------------------------------------------------------------

fn register_middleware_api(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptMiddleware>("ScriptMiddleware");
    engine.register_fn("middleware", ScriptMiddleware::new);
    engine.register_fn("before", ScriptMiddleware::before);
    engine.register_fn("after", ScriptMiddleware::after);
}

// ---------------------------------------------------------------------------
// create_module()
// ---------------------------------------------------------------------------

fn register_module_api(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptModule>("ScriptModule");
    // Note: `module` is a reserved keyword in Rhai, so we use `create_module`.
    engine.register_fn("create_module", ScriptModule::new);

    // Accept arrays of Dynamic and downcast internally
    engine.register_fn("providers", ScriptModule::set_providers);
    engine.register_fn("controllers", ScriptModule::set_controllers);
    engine.register_fn("guards", ScriptModule::set_guards);
    engine.register_fn("imports", ScriptModule::set_imports);

    engine.register_fn("on_module_init", ScriptModule::on_module_init);
    engine.register_fn("on_module_destroy", ScriptModule::on_module_destroy);
}

// ---------------------------------------------------------------------------
// Application::create() and app.listen()
// ---------------------------------------------------------------------------

fn register_app_api_types(engine: &mut Engine) {
    engine.register_type_with_name::<ScriptApp>("ScriptApp");

    // Application::create(module) — a static-style constructor
    engine.register_fn("create_app", ScriptApp::new);

    engine.register_fn("listen", ScriptApp::listen);
    engine.register_fn("listen_host", ScriptApp::listen_host);
    engine.register_fn("on_bootstrap", ScriptApp::on_bootstrap);
    engine.register_fn("on_shutdown", ScriptApp::on_shutdown);
}

// ---------------------------------------------------------------------------
// ServiceContext — ctx.call("Service", "method", ...args)
// ---------------------------------------------------------------------------

fn register_service_context_api(engine: &mut Engine) {
    engine.register_type_with_name::<ServiceContext>("ServiceContext");

    // 0 extra args: ctx.call("Service", "method")
    engine.register_fn(
        "invoke",
        |context: NativeCallContext,
         ctx: &mut ServiceContext,
         svc: String,
         method: String|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let fn_ptr = ctx
                .get_method(&svc, &method)
                .map_err(|e| Box::new(EvalAltResult::from(e)))?;
            fn_ptr.call_within_context::<Dynamic>(&context, ())
        },
    );

    // 1 extra arg: ctx.call("Service", "method", arg1)
    engine.register_fn(
        "invoke",
        |context: NativeCallContext,
         ctx: &mut ServiceContext,
         svc: String,
         method: String,
         arg1: Dynamic|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let fn_ptr = ctx
                .get_method(&svc, &method)
                .map_err(|e| Box::new(EvalAltResult::from(e)))?;
            fn_ptr.call_within_context::<Dynamic>(&context, (arg1,))
        },
    );

    // 2 extra args: ctx.call("Service", "method", arg1, arg2)
    engine.register_fn(
        "invoke",
        |context: NativeCallContext,
         ctx: &mut ServiceContext,
         svc: String,
         method: String,
         arg1: Dynamic,
         arg2: Dynamic|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let fn_ptr = ctx
                .get_method(&svc, &method)
                .map_err(|e| Box::new(EvalAltResult::from(e)))?;
            fn_ptr.call_within_context::<Dynamic>(&context, (arg1, arg2))
        },
    );

    // 3 extra args: ctx.call("Service", "method", arg1, arg2, arg3)
    engine.register_fn(
        "invoke",
        |context: NativeCallContext,
         ctx: &mut ServiceContext,
         svc: String,
         method: String,
         arg1: Dynamic,
         arg2: Dynamic,
         arg3: Dynamic|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let fn_ptr = ctx
                .get_method(&svc, &method)
                .map_err(|e| Box::new(EvalAltResult::from(e)))?;
            fn_ptr.call_within_context::<Dynamic>(&context, (arg1, arg2, arg3))
        },
    );
}
