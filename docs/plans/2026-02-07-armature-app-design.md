# armature-app: Full Application Building in Rhai

## Overview

A new crate that lets users define complete Armature applications in Rhai scripts — modules, controllers, services, guards, middleware, lifecycle hooks — with zero Rust code required. Invoked via `armature run app.rhai`.

## Rhai API Surface

The API mirrors the Rust decorator-based patterns using a registration/builder style.

### Services (mirrors `#[injectable]`)

```rhai
let user_service = service("UserService");
user_service.fn("get_users", || {
    [#{ id: 1, name: "Alice" }, #{ id: 2, name: "Bob" }]
});
user_service.fn("get_user", |id| {
    #{ id: id, name: "User " + id }
});
```

### Controllers and Routes (mirrors `#[controller]`, `#[get]`, `#[post]`, etc.)

```rhai
let users = controller("/api/users");

users.get("/", |req, ctx| {
    let svc = ctx.service("UserService");
    Response::ok().json(svc.get_users())
});

users.get("/:id", |req, ctx| {
    let id = req.param("id");
    Response::ok().json(ctx.service("UserService").get_user(id))
});

users.post("/", |req, ctx| {
    Response::created().json(req.body_json())
});
```

Handlers receive `req` (the request binding from armature-rhai) and `ctx` (DI context). `ctx.service("Name")` is the Rhai equivalent of struct field injection.

### Guards (mirrors `impl Guard`)

```rhai
let auth = guard("AuthGuard");
auth.can_activate(|ctx| {
    let token = ctx.request.header("Authorization");
    token != () && token.starts_with("Bearer ")
});

users.use_guard(auth);
```

Guards return `true`/`false`. `false` sends 403 Forbidden.

### Middleware (mirrors `impl Middleware`)

```rhai
let logging = middleware("RequestLogger");
logging.before(|req| {
    log_info(`${req.method} ${req.path}`);
    req
});
logging.after(|req, res| {
    log_info(`${res.status}`);
    res
});

users.use_middleware(logging);
```

`before` can short-circuit by returning a Response instead of a request. `after` can modify the response.

### Modules (mirrors `#[module(...)]`)

```rhai
let auth_module = create_module("AuthModule");
auth_module.providers([auth_service]);
auth_module.guards([auth]);

let app_module = create_module("AppModule");
app_module.providers([user_service]);
app_module.controllers([users]);
app_module.imports([auth_module]);
```

Module resolution is depth-first, same as Rust.

### Lifecycle Hooks

```rhai
app_module.on_module_init(|| {
    log_info("AppModule initialized");
});
app_module.on_module_destroy(|| {
    log_info("AppModule shutting down");
});

app.on_bootstrap(|| {
    log_info("Application bootstrapped");
});
app.on_shutdown(|| {
    log_info("Application shutting down");
});
```

Execution order matches Rust: init -> bootstrap -> [running] -> shutdown -> destroy.

### Application Bootstrap

```rhai
let app = Application::create(app_module);
app.listen(3000);
```

### Multi-file Imports

```rhai
import "services/user_service" as user_svc;
import "controllers/user_controller" as user_ctrl;

let app_module = create_module("AppModule");
app_module.providers([user_svc::service]);
app_module.controllers([user_ctrl::controller]);
```

Imports resolve relative to the script's directory.

## Crate Structure

```
armature-app/
├── Cargo.toml
└── src/
    ├── lib.rs          # public API + re-exports
    ├── bindings.rs     # registers service(), controller(), module(), guard(),
    │                   # middleware(), Application type into the Rhai engine
    ├── builder.rs      # converts Rhai objects into armature-core types
    ├── runner.rs       # loads .rhai file, executes, starts server
    ├── types.rs        # ScriptService, ScriptController, ScriptModule,
    │                   # ScriptGuard, ScriptMiddleware wrapper types
    └── error.rs        # error types
```

## Translation: Rhai to Core

| Rhai concept | Core type produced |
|---|---|
| `service("Name")` + `.fn()` | Registered in `Container` as callable service map |
| `controller("/path")` + `.get()` etc. | Routes in `Router` with script-backed handlers |
| `guard("Name")` + `.can_activate()` | `Arc<dyn Guard>` wrapping Rhai closure |
| `middleware("Name")` + `.before()`/`.after()` | `Arc<dyn Middleware>` wrapping Rhai closures |
| `create_module("Name")` + providers/controllers/imports | Recursive module resolution (depth-first) |
| `Application::create(module)` | Full bootstrap: modules -> container -> routes -> lifecycle |
| `app.listen(port)` | armature-core TCP listener with assembled router |

Key: HTTP server, routing dispatch, and connection handling run in native Rust. Rhai is invoked only at handler/guard/middleware call boundaries.

Services are stored as `Arc<RhaiServiceInstance>` (a map of method name to compiled AST). Calling `ctx.service("UserService").get_users()` looks up the AST and evaluates it.

## CLI Integration

```
armature run app.rhai              # run from file
armature run app.rhai --port 8080  # override port
armature run app.rhai --watch      # hot reload (uses existing ScriptWatcher)
```

Single addition to `armature-cli`.
