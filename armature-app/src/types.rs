//! Rhai-side wrapper types for defining Armature applications.
//!
//! These types are registered with the Rhai engine and used in scripts
//! to define services, controllers, guards, middleware, and modules.

use rhai::{Dynamic, EvalAltResult, FnPtr};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Cast every element of `items` to `T`, or return a Rhai error naming the
/// offending index and its actual type. Used by `ScriptModule`'s
/// `providers([...])`/`controllers([...])`/`guards([...])`/`imports([...])`
/// setters so a typo (e.g. passing a non-`ScriptService` into `providers`)
/// surfaces to the script author instead of being silently dropped.
fn cast_all<T: Clone + Send + Sync + 'static>(
    setter_name: &str,
    expected: &str,
    items: Vec<Dynamic>,
) -> Result<Vec<T>, Box<EvalAltResult>> {
    items
        .into_iter()
        .enumerate()
        .map(|(i, d)| {
            let actual = d.type_name().to_string();
            d.try_cast::<T>().ok_or_else(|| {
                Box::<EvalAltResult>::from(format!(
                    "{setter_name}([...]) element {i} has type `{actual}`, expected {expected}"
                ))
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ScriptService
// ---------------------------------------------------------------------------

/// A service defined in Rhai, holding named method closures.
///
/// Mirrors `#[injectable]` from Rust.
#[derive(Debug, Clone)]
pub struct ScriptService {
    pub name: String,
    pub methods: BTreeMap<String, FnPtr>,
}

impl ScriptService {
    pub fn new(name: String) -> Self {
        Self {
            name,
            methods: BTreeMap::new(),
        }
    }

    /// Register a method on this service.
    pub fn define(&mut self, name: String, handler: FnPtr) {
        self.methods.insert(name, handler);
    }
}

// ---------------------------------------------------------------------------
// ScriptRoute
// ---------------------------------------------------------------------------

/// A single route defined in a controller.
#[derive(Debug, Clone)]
pub struct ScriptRoute {
    pub method: String,
    pub path: String,
    pub handler: FnPtr,
}

// ---------------------------------------------------------------------------
// ScriptController
// ---------------------------------------------------------------------------

/// A controller defined in Rhai, holding routes and optional guards/middleware.
///
/// Mirrors `#[controller("/path")]` from Rust.
#[derive(Debug, Clone)]
pub struct ScriptController {
    pub base_path: String,
    pub routes: Vec<ScriptRoute>,
    pub guards: Vec<ScriptGuard>,
    pub middleware: Vec<ScriptMiddleware>,
}

impl ScriptController {
    pub fn new(base_path: String) -> Self {
        Self {
            base_path,
            routes: Vec::new(),
            guards: Vec::new(),
            middleware: Vec::new(),
        }
    }

    /// Add a route for the given HTTP method.
    fn add_route(&mut self, method: &str, path: String, handler: FnPtr) {
        let full_path = if path == "/" && self.base_path.ends_with('/') {
            self.base_path.trim_end_matches('/').to_string()
        } else if path == "/" {
            self.base_path.clone()
        } else {
            format!("{}{}", self.base_path, path)
        };
        self.routes.push(ScriptRoute {
            method: method.to_string(),
            path: full_path,
            handler,
        });
    }

    pub fn get(&mut self, path: String, handler: FnPtr) {
        self.add_route("GET", path, handler);
    }

    pub fn post(&mut self, path: String, handler: FnPtr) {
        self.add_route("POST", path, handler);
    }

    pub fn put(&mut self, path: String, handler: FnPtr) {
        self.add_route("PUT", path, handler);
    }

    pub fn delete(&mut self, path: String, handler: FnPtr) {
        self.add_route("DELETE", path, handler);
    }

    pub fn patch(&mut self, path: String, handler: FnPtr) {
        self.add_route("PATCH", path, handler);
    }

    pub fn use_guard(&mut self, guard: ScriptGuard) {
        self.guards.push(guard);
    }

    pub fn use_middleware(&mut self, mw: ScriptMiddleware) {
        self.middleware.push(mw);
    }
}

// ---------------------------------------------------------------------------
// ScriptGuard
// ---------------------------------------------------------------------------

/// A guard defined in Rhai.
///
/// Mirrors `impl Guard` from Rust. Returns true to allow, false to reject (403).
#[derive(Debug, Clone)]
pub struct ScriptGuard {
    pub name: String,
    pub handler: Option<FnPtr>,
}

impl ScriptGuard {
    pub fn new(name: String) -> Self {
        Self {
            name,
            handler: None,
        }
    }

    pub fn can_activate(&mut self, handler: FnPtr) {
        self.handler = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// ScriptMiddleware
// ---------------------------------------------------------------------------

/// Middleware defined in Rhai with optional before/after hooks.
///
/// Mirrors `impl Middleware` from Rust.
#[derive(Debug, Clone)]
pub struct ScriptMiddleware {
    pub name: String,
    pub before_fn: Option<FnPtr>,
    pub after_fn: Option<FnPtr>,
}

impl ScriptMiddleware {
    pub fn new(name: String) -> Self {
        Self {
            name,
            before_fn: None,
            after_fn: None,
        }
    }

    pub fn before(&mut self, handler: FnPtr) {
        self.before_fn = Some(handler);
    }

    pub fn after(&mut self, handler: FnPtr) {
        self.after_fn = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// ScriptModule
// ---------------------------------------------------------------------------

/// A module defined in Rhai, grouping providers, controllers, and imports.
///
/// Mirrors `#[module(...)]` from Rust.
#[derive(Debug, Clone)]
pub struct ScriptModule {
    pub name: String,
    pub providers: Vec<ScriptService>,
    pub controllers: Vec<ScriptController>,
    pub guards: Vec<ScriptGuard>,
    pub imports: Vec<ScriptModule>,
    pub on_init: Option<FnPtr>,
    pub on_destroy: Option<FnPtr>,
}

impl ScriptModule {
    pub fn new(name: String) -> Self {
        Self {
            name,
            providers: Vec::new(),
            controllers: Vec::new(),
            guards: Vec::new(),
            imports: Vec::new(),
            on_init: None,
            on_destroy: None,
        }
    }

    pub fn set_providers(&mut self, providers: Vec<Dynamic>) -> Result<(), Box<EvalAltResult>> {
        self.providers = cast_all(
            "providers",
            "a ScriptService (from service(\"Name\"))",
            providers,
        )?;
        Ok(())
    }

    pub fn set_controllers(&mut self, controllers: Vec<Dynamic>) -> Result<(), Box<EvalAltResult>> {
        self.controllers = cast_all(
            "controllers",
            "a ScriptController (from controller(\"/path\"))",
            controllers,
        )?;
        Ok(())
    }

    pub fn set_guards(&mut self, guards: Vec<Dynamic>) -> Result<(), Box<EvalAltResult>> {
        self.guards = cast_all("guards", "a ScriptGuard (from guard(\"Name\"))", guards)?;
        Ok(())
    }

    pub fn set_imports(&mut self, imports: Vec<Dynamic>) -> Result<(), Box<EvalAltResult>> {
        self.imports = cast_all(
            "imports",
            "a ScriptModule (from create_module(\"Name\"))",
            imports,
        )?;
        Ok(())
    }

    pub fn on_module_init(&mut self, handler: FnPtr) {
        self.on_init = Some(handler);
    }

    pub fn on_module_destroy(&mut self, handler: FnPtr) {
        self.on_destroy = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// ScriptApp
// ---------------------------------------------------------------------------

/// The assembled application, produced by `Application::create(module)`.
///
/// Holds the resolved module tree and listen configuration.
#[derive(Debug, Clone)]
pub struct ScriptApp {
    pub module: ScriptModule,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub on_bootstrap: Option<FnPtr>,
    pub on_shutdown: Option<FnPtr>,
}

impl ScriptApp {
    pub fn new(module: ScriptModule) -> Self {
        Self {
            module,
            port: None,
            host: None,
            on_bootstrap: None,
            on_shutdown: None,
        }
    }

    pub fn listen(&mut self, port: i64) {
        self.port = Some(port as u16);
    }

    pub fn listen_host(&mut self, port: i64, host: String) {
        self.port = Some(port as u16);
        self.host = Some(host);
    }

    pub fn on_bootstrap(&mut self, handler: FnPtr) {
        self.on_bootstrap = Some(handler);
    }

    pub fn on_shutdown(&mut self, handler: FnPtr) {
        self.on_shutdown = Some(handler);
    }
}

// ---------------------------------------------------------------------------
// ServiceContext
// ---------------------------------------------------------------------------

/// DI context passed to handler closures as the `ctx` argument.
///
/// Provides `ctx.invoke("ServiceName", "methodName", ...args)` for invoking
/// service methods from within route handlers. (Not `ctx.call(...)`: `call`
/// is Rhai's own reserved keyword for function-pointer invocation and
/// cannot be used as a custom method name — see `bindings.rs`'s
/// `register_service_context_api` for the full explanation.)
#[derive(Debug, Clone)]
pub struct ServiceContext {
    pub services: Arc<BTreeMap<String, BTreeMap<String, FnPtr>>>,
}

impl ServiceContext {
    /// Build a ServiceContext from a flat list of ScriptServices.
    pub fn from_services(services: &[ScriptService]) -> Self {
        let mut map = BTreeMap::new();
        for svc in services {
            map.insert(svc.name.clone(), svc.methods.clone());
        }
        Self {
            services: Arc::new(map),
        }
    }

    /// Look up a method FnPtr.
    pub fn get_method(&self, service: &str, method: &str) -> std::result::Result<&FnPtr, String> {
        let methods = self
            .services
            .get(service)
            .ok_or_else(|| format!("Service not found: {}", service))?;
        methods
            .get(method)
            .ok_or_else(|| format!("Method `{}` not found on service `{}`", method, service))
    }
}
