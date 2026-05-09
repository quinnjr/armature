//! Converts Rhai-defined application objects into armature-core types.
//!
//! This module takes a [`ScriptApp`] (produced by executing a Rhai script)
//! and assembles a real [`armature_core::Application`] with a populated
//! [`Router`] and [`Container`].

use crate::error::{AppError, Result};
use crate::types::{
    ScriptApp, ScriptController, ScriptGuard, ScriptMiddleware, ScriptModule, ScriptRoute,
    ScriptService, ServiceContext,
};
use armature_core::{
    HttpMethod, HttpRequest, HttpResponse, Route, Router, handler::from_legacy_handler,
    routing::HandlerFn,
};
use armature_rhai::{RequestBinding, ResponseBinding};
use rhai::{AST, Dynamic, Engine, FnPtr};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Build an armature-core Router from a ScriptApp's module tree.
///
/// Recursively resolves imports, collects all services, and creates
/// native handler closures that bridge Rhai FnPtrs to async HTTP handlers.
pub fn build_router(app: &ScriptApp, engine: Arc<Engine>, ast: Arc<AST>) -> Result<Router> {
    // 1. Flatten all services from the module tree
    let services = collect_services(&app.module);
    let service_ctx = Arc::new(ServiceContext::from_services(&services));
    debug!(
        service_count = services.len(),
        "Collected services from module tree"
    );

    // 2. Flatten all controllers
    let controllers = collect_controllers(&app.module);
    debug!(
        controller_count = controllers.len(),
        "Collected controllers from module tree"
    );

    // 3. Build router
    let mut router = Router::new();

    for ctrl in &controllers {
        for route in &ctrl.routes {
            let handler = make_handler(
                route,
                &ctrl.guards,
                &ctrl.middleware,
                Arc::clone(&engine),
                Arc::clone(&ast),
                Arc::clone(&service_ctx),
            );

            let method = parse_method(&route.method)?;
            let boxed = from_legacy_handler(handler);
            router.add_route(Route {
                method,
                path: route.path.clone(),
                handler: boxed,
                constraints: None,
            });

            info!(method = %route.method, path = %route.path, "Registered route");
        }
    }

    Ok(router)
}

/// Fire lifecycle init hooks from the module tree.
pub fn fire_init_hooks(module: &ScriptModule, engine: &Engine, ast: &AST) {
    // Depth-first: imports first
    for imp in &module.imports {
        fire_init_hooks(imp, engine, ast);
    }

    if let Some(ref hook) = module.on_init {
        debug!(module = %module.name, "Firing on_module_init");
        if let Err(e) = hook.call::<()>(engine, ast, ()) {
            error!(module = %module.name, error = %e, "on_module_init failed");
        }
    }
}

/// Fire lifecycle destroy hooks from the module tree (reverse order).
pub fn fire_destroy_hooks(module: &ScriptModule, engine: &Engine, ast: &AST) {
    if let Some(ref hook) = module.on_destroy {
        debug!(module = %module.name, "Firing on_module_destroy");
        if let Err(e) = hook.call::<()>(engine, ast, ()) {
            error!(module = %module.name, error = %e, "on_module_destroy failed");
        }
    }

    // Reverse: imports after self
    for imp in module.imports.iter().rev() {
        fire_destroy_hooks(imp, engine, ast);
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Recursively collect all services from a module tree (depth-first).
fn collect_services(module: &ScriptModule) -> Vec<ScriptService> {
    let mut out = Vec::new();
    for imp in &module.imports {
        out.extend(collect_services(imp));
    }
    out.extend(module.providers.iter().cloned());
    out
}

/// Recursively collect all controllers from a module tree (depth-first).
fn collect_controllers(module: &ScriptModule) -> Vec<ScriptController> {
    let mut out = Vec::new();
    for imp in &module.imports {
        out.extend(collect_controllers(imp));
    }
    out.extend(module.controllers.iter().cloned());
    out
}

/// Create a legacy HandlerFn that bridges a Rhai FnPtr to an async handler.
fn make_handler(
    route: &ScriptRoute,
    guards: &[ScriptGuard],
    middleware: &[ScriptMiddleware],
    engine: Arc<Engine>,
    ast: Arc<AST>,
    service_ctx: Arc<ServiceContext>,
) -> HandlerFn {
    let handler_fn = route.handler.clone();
    let guards: Vec<ScriptGuard> = guards.to_vec();
    let before_mw: Vec<(String, FnPtr)> = middleware
        .iter()
        .filter_map(|m| m.before_fn.as_ref().map(|f| (m.name.clone(), f.clone())))
        .collect();
    let after_mw: Vec<(String, FnPtr)> = middleware
        .iter()
        .filter_map(|m| m.after_fn.as_ref().map(|f| (m.name.clone(), f.clone())))
        .collect();

    Arc::new(
        move |req: HttpRequest| -> Pin<
            Box<
                dyn Future<Output = std::result::Result<HttpResponse, armature_core::Error>> + Send,
            >,
        > {
            let engine = Arc::clone(&engine);
            let ast = Arc::clone(&ast);
            let ctx = (*service_ctx).clone();
            let handler_fn = handler_fn.clone();
            let guards = guards.clone();
            let before_mw = before_mw.clone();
            let after_mw = after_mw.clone();

            Box::pin(async move {
                // Run on a blocking thread since Rhai is synchronous
                tokio::task::spawn_blocking(move || {
                    // 1. Run guards
                    for guard in &guards {
                        if let Some(ref check) = guard.handler {
                            let req_binding = RequestBinding::from_request(&req);
                            match check.call::<bool>(&engine, &ast, (req_binding,)) {
                                Ok(true) => {}
                                Ok(false) => {
                                    return Ok(HttpResponse::new(403).with_body(
                                        format!("Forbidden: guard `{}` rejected", guard.name)
                                            .into_bytes(),
                                    ));
                                }
                                Err(e) => {
                                    error!(guard = %guard.name, error = %e, "Guard error");
                                    return Ok(HttpResponse::new(403)
                                        .with_body(format!("Forbidden: {}", e).into_bytes()));
                                }
                            }
                        }
                    }

                    // 2. Run before middleware
                    let current_req = req;
                    for (name, mw_fn) in &before_mw {
                        let req_binding = RequestBinding::from_request(&current_req);
                        match mw_fn.call::<Dynamic>(&engine, &ast, (req_binding,)) {
                            Ok(val) => {
                                // If middleware returns a ResponseBinding, short-circuit
                                if val.is::<ResponseBinding>() {
                                    let resp: ResponseBinding = val.cast();
                                    return Ok(resp.into_http_response());
                                }
                                // Otherwise continue (middleware may have logged, etc.)
                            }
                            Err(e) => {
                                error!(middleware = %name, error = %e, "Before middleware error");
                            }
                        }
                    }

                    // 3. Call the handler
                    let req_binding = RequestBinding::from_request(&current_req);
                    let result = handler_fn.call::<Dynamic>(&engine, &ast, (req_binding, ctx));

                    let response = match result {
                        Ok(val) => dynamic_to_response(val),
                        Err(e) => {
                            error!(error = %e, "Handler error");
                            HttpResponse::new(500)
                                .with_body(format!("Internal Server Error: {}", e).into_bytes())
                        }
                    };

                    // 4. Run after middleware
                    let mut final_response = response;
                    for (name, mw_fn) in &after_mw {
                        let req_binding = RequestBinding::from_request(&current_req);
                        let resp_status = Dynamic::from(final_response.status as i64);
                        match mw_fn.call::<Dynamic>(&engine, &ast, (req_binding, resp_status)) {
                            Ok(val) => {
                                if val.is::<ResponseBinding>() {
                                    let resp: ResponseBinding = val.cast();
                                    final_response = resp.into_http_response();
                                }
                            }
                            Err(e) => {
                                error!(middleware = %name, error = %e, "After middleware error");
                            }
                        }
                    }

                    Ok(final_response)
                })
                .await
                .map_err(|e| armature_core::Error::Internal(format!("Task join error: {}", e)))?
            })
        },
    )
}

/// Convert a Rhai Dynamic value into an HttpResponse.
fn dynamic_to_response(val: Dynamic) -> HttpResponse {
    // ResponseBinding — use directly
    if val.is::<ResponseBinding>() {
        let resp: ResponseBinding = val.cast();
        return resp.into_http_response();
    }

    // String — return as text/plain
    if val.is_string() {
        let text: String = val.cast();
        let mut resp = HttpResponse::new(200);
        resp.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        return resp.with_body(text.into_bytes());
    }

    // Map or Array — return as JSON
    if (val.is_map() || val.is_array())
        && let Ok(json) = serde_json::to_string(&rhai_to_json(val))
    {
        let mut resp = HttpResponse::new(200);
        resp.headers
            .insert("content-type".to_string(), "application/json".to_string());
        return resp.with_body(json.into_bytes());
    }

    // Unit or unknown — empty 200
    HttpResponse::new(200)
}

/// Convert a Rhai Dynamic to a serde_json::Value.
fn rhai_to_json(val: Dynamic) -> serde_json::Value {
    if val.is_unit() {
        serde_json::Value::Null
    } else if val.is_bool() {
        serde_json::Value::Bool(val.as_bool().unwrap())
    } else if val.is_int() {
        serde_json::Value::Number(val.as_int().unwrap().into())
    } else if val.is_float() {
        serde_json::Number::from_f64(val.as_float().unwrap())
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    } else if val.is_string() {
        serde_json::Value::String(val.into_string().unwrap())
    } else if val.is_array() {
        let arr: rhai::Array = val.cast();
        serde_json::Value::Array(arr.into_iter().map(rhai_to_json).collect())
    } else if val.is_map() {
        let map: rhai::Map = val.cast();
        let obj: serde_json::Map<String, serde_json::Value> = map
            .into_iter()
            .map(|(k, v)| (k.to_string(), rhai_to_json(v)))
            .collect();
        serde_json::Value::Object(obj)
    } else {
        serde_json::Value::String(format!("{:?}", val))
    }
}

fn parse_method(method: &str) -> Result<HttpMethod> {
    match method {
        "GET" => Ok(HttpMethod::GET),
        "POST" => Ok(HttpMethod::POST),
        "PUT" => Ok(HttpMethod::PUT),
        "DELETE" => Ok(HttpMethod::DELETE),
        "PATCH" => Ok(HttpMethod::PATCH),
        "OPTIONS" => Ok(HttpMethod::OPTIONS),
        "HEAD" => Ok(HttpMethod::HEAD),
        _ => Err(AppError::Builder {
            message: format!("Unknown HTTP method: {}", method),
        }),
    }
}
