//! Regression test for the Critical finding: the service-invocation API
//! passed to route handlers as `ctx` was documented as `ctx.call(...)`
//! (bindings.rs doc comments, the `ServiceContext` doc in types.rs, and
//! the crate Quick Start in lib.rs), but the four dispatch closures were
//! only ever registered under the name `invoke`. Every documented handler
//! calling `ctx.call(...)` failed at runtime with a missing-function
//! error.
//!
//! `armature-app/src/bindings.rs:110`
//!
//! IMPORTANT — this is *not* fixed by registering a function named "call":
//! `call` is Rhai's own reserved keyword for invoking a stored function
//! pointer (`rhai::engine::KEYWORD_FN_PTR_CALL`). Both `ctx.call(...)`
//! (method-call style) and `call(ctx, ...)` (plain-call style) are
//! intercepted *unconditionally* by Rhai's own dispatch
//! (`rhai::func::call::{make_method_call, make_function_call}`) before any
//! user-registered function of that name is ever consulted — confirmed
//! empirically below and by reading the pinned rhai 1.25.1 source. So the
//! real fix (matching the audit finding's own documented fallback: "or
//! update every doc/comment/example to say invoke") is: `invoke` is the
//! one true, working name, and every doc reference now says
//! `ctx.invoke(...)` instead of `ctx.call(...)`.

#[path = "support/mod.rs"]
mod support;

use armature_core::HttpRequest;

const SCRIPT: &str = r#"
    let user_service = service("UserService");
    user_service.define("get_users", || {
        [#{ id: 1, name: "Alice" }, #{ id: 2, name: "Bob" }]
    });

    let users = controller("/api/users");
    users.get("/", |req, ctx| {
        let data = ctx.invoke("UserService", "get_users");
        ok().json(data)
    });

    let app_module = create_module("AppModule");
    app_module.providers([user_service]);
    app_module.controllers([users]);

    let app = create_app(app_module);
    app.listen(3000);
"#;

#[tokio::test]
async fn ctx_invoke_resolves_and_invokes_the_service_method() {
    let router = support::build_router_from_script(SCRIPT);

    let request = HttpRequest::new("GET", "/api/users".to_string());
    let response = router.route(request).await.unwrap_or_else(|e| {
        panic!(
            "ctx.invoke(\"UserService\", \"get_users\") should resolve to a matched route, got \
             router error: {e}"
        )
    });

    assert_eq!(response.status, 200, "handler should not have errored");
    let body = String::from_utf8_lossy(response.body_ref());
    assert!(body.contains("Alice"), "response body was: {body}");
    assert!(body.contains("Bob"), "response body was: {body}");
}

/// `ctx.invoke` with an extra argument (the 2-arg dispatch overload) also
/// resolves, not just the 0-arg form.
#[tokio::test]
async fn ctx_invoke_with_an_argument_resolves() {
    let script = r#"
        let greeter = service("Greeter");
        greeter.define("greet", |name| {
            "Hello, " + name + "!"
        });

        let ctrl = controller("/greet");
        ctrl.get("/", |req, ctx| {
            let msg = ctx.invoke("Greeter", "greet", "World");
            ok().body(msg)
        });

        let app_module = create_module("AppModule");
        app_module.providers([greeter]);
        app_module.controllers([ctrl]);

        let app = create_app(app_module);
        app.listen(3000);
    "#;
    let router = support::build_router_from_script(script);

    let request = HttpRequest::new("GET", "/greet".to_string());
    let response = router.route(request).await.expect("route should dispatch");

    assert_eq!(response.status, 200);
    assert_eq!(
        String::from_utf8_lossy(response.body_ref()),
        "Hello, World!"
    );
}

/// Documents *why* the doc fix landed on `invoke` rather than `call`:
/// `ctx.call(...)` is unconditionally intercepted by Rhai's own
/// `KEYWORD_FN_PTR_CALL` dispatch (it tries to treat the first string
/// argument as a function pointer) before it can ever reach a
/// user-registered function — so it can never be made to work as
/// documented, regardless of what armature-app registers.
#[tokio::test]
async fn ctx_call_cannot_be_made_to_work_it_is_a_reserved_rhai_keyword() {
    let script = SCRIPT.replace("ctx.invoke(", "ctx.call(");
    let router = support::build_router_from_script(&script);

    let request = HttpRequest::new("GET", "/api/users".to_string());
    let response = router
        .route(request)
        .await
        .expect("router should still dispatch to the matched route");

    // The handler itself errors (500) because Rhai's reserved `.call(...)`
    // keyword tries to treat "UserService" as a function pointer.
    assert_eq!(
        response.status, 500,
        "ctx.call(...) must NOT resolve — if this starts passing, Rhai's reserved-keyword \
         behavior has changed and the doc fix should be revisited"
    );
    let body = String::from_utf8_lossy(response.body_ref());
    assert!(
        body.contains("expecting Fn") || body.contains("Fn"),
        "expected Rhai's reserved fn-pointer-call error, got: {body}"
    );
}
