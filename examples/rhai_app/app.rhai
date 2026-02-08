// =============================================================================
// Armature Rhai Application — Task Manager API
//
// A complete REST API built entirely in Rhai, demonstrating:
//   - Services with business logic
//   - Controllers with CRUD routes
//   - Guards for authentication
//   - Middleware for logging
//   - Module composition
//   - Lifecycle hooks
//
// Run:  armature run examples/rhai_app/app.rhai
// =============================================================================

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

// Simple in-memory "database" — returns static data for demonstration.
// In a real app you would call out to a database via a native binding.
let task_service = service("TaskService");

task_service.define("get_all", || {
    [
        #{ id: 1, title: "Learn Rust",           completed: true  },
        #{ id: 2, title: "Build with Armature",  completed: false },
        #{ id: 3, title: "Write Rhai scripts",   completed: false },
    ]
});

task_service.define("get_by_id", |id| {
    let tasks = [
        #{ id: 1, title: "Learn Rust",           completed: true  },
        #{ id: 2, title: "Build with Armature",  completed: false },
        #{ id: 3, title: "Write Rhai scripts",   completed: false },
    ];

    for task in tasks {
        if `${task.id}` == id {
            return task;
        }
    }

    // Not found — return unit
    ()
});

task_service.define("create", |title| {
    #{ id: 4, title: title, completed: false }
});

// A greeting service to show multi-service DI.
let greeting_service = service("GreetingService");

greeting_service.define("hello", |name| {
    `Hello, ${name}! Welcome to Armature.`
});

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

// A simple bearer-token guard.
let auth_guard = guard("AuthGuard");
auth_guard.can_activate(|req| {
    let token = req.header("authorization");

    if token == () {
        return false;
    }

    token.starts_with("Bearer ")
});

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

let request_logger = middleware("RequestLogger");

request_logger.before(|req| {
    log_info(`→ ${req.method} ${req.path}`);
    // Return the request unchanged to continue the chain.
    req
});

request_logger.after(|req, status| {
    log_info(`← ${req.method} ${req.path} — ${status}`);
});

// ---------------------------------------------------------------------------
// Controllers
// ---------------------------------------------------------------------------

// ---- Health controller (public, no auth) ----

let health = controller("/health");

health.get("/", |req, ctx| {
    ok().json(#{
        status: "ok",
        uptime: "running",
    })
});

// ---- Task controller (protected by auth guard) ----

let tasks = controller("/api/tasks");
tasks.use_guard(auth_guard);
tasks.use_middleware(request_logger);

// GET /api/tasks — list all tasks
tasks.get("/", |req, ctx| {
    let all = ctx.invoke("TaskService", "get_all");
    ok().json(all)
});

// GET /api/tasks/:id — get a single task
tasks.get("/:id", |req, ctx| {
    let id = req.param("id");
    let task = ctx.invoke("TaskService", "get_by_id", id);

    if task == () {
        not_found().json(#{ error: `Task ${id} not found` })
    } else {
        ok().json(task)
    }
});

// POST /api/tasks — create a task
tasks.post("/", |req, ctx| {
    let body = req.body_json();
    let title = body["title"];

    if title == () {
        return bad_request().json(#{ error: "Missing field: title" });
    }

    let task = ctx.invoke("TaskService", "create", title);
    created().json(task)
});

// DELETE /api/tasks/:id — delete a task (stub)
tasks.delete("/:id", |req, ctx| {
    let id = req.param("id");
    log_info(`Deleted task ${id}`);
    no_content()
});

// ---- Greeting controller (public) ----

let greetings = controller("/api/greetings");

greetings.get("/:name", |req, ctx| {
    let name = req.param("name");
    let msg = ctx.invoke("GreetingService", "hello", name);
    ok().json(#{ message: msg })
});

// ---------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------

// Group the greeting feature into its own module.
let greeting_module = create_module("GreetingModule");
greeting_module.providers([greeting_service]);
greeting_module.controllers([greetings]);

greeting_module.on_module_init(|| {
    log_info("GreetingModule initialized");
});

// Root application module — imports the greeting module.
let app_module = create_module("AppModule");
app_module.providers([task_service]);
app_module.controllers([health, tasks]);
app_module.imports([greeting_module]);

app_module.on_module_init(|| {
    log_info("AppModule initialized");
});

app_module.on_module_destroy(|| {
    log_info("AppModule destroyed — goodbye!");
});

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

let app = create_app(app_module);

app.on_bootstrap(|| {
    log_info("=== Armature Rhai App started ===");
    log_info("  GET    /health              — health check");
    log_info("  GET    /api/tasks           — list tasks   (auth required)");
    log_info("  GET    /api/tasks/:id       — get task     (auth required)");
    log_info("  POST   /api/tasks           — create task  (auth required)");
    log_info("  DELETE /api/tasks/:id       — delete task  (auth required)");
    log_info("  GET    /api/greetings/:name — greet someone");
});

app.on_shutdown(|| {
    log_info("=== Armature Rhai App shutting down ===");
});

app.listen(3000);
