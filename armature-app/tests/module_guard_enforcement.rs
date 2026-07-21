//! Regression test for the Warning finding: modules accept guards via
//! `guards([...])` (mirroring NestJS module-scoped guards, stored on
//! `ScriptModule.guards`), but `build_router` only ever applied
//! controller-level `ctrl.guards` to route handlers — `module.guards` was
//! collected nowhere and never enforced. A module guard silently protected
//! nothing: every request sailed through regardless of what the guard
//! returned.
//!
//! `armature-app/src/builder.rs:46`

#[path = "support/mod.rs"]
mod support;

use armature_core::HttpRequest;

const DENIED_SCRIPT: &str = r#"
    let deny_all = guard("DenyAll");
    deny_all.can_activate(|req| {
        false
    });

    let ctrl = controller("/secure");
    ctrl.get("/", |req, ctx| {
        ok().body("should not be reached")
    });

    let app_module = create_module("AppModule");
    app_module.controllers([ctrl]);
    app_module.guards([deny_all]);

    let app = create_app(app_module);
    app.listen(3000);
"#;

#[tokio::test]
async fn module_level_guard_blocks_requests_to_its_controllers() {
    let router = support::build_router_from_script(DENIED_SCRIPT);

    let request = HttpRequest::new("GET".to_string(), "/secure".to_string());
    let response = router
        .route(request)
        .await
        .expect("router should dispatch to the matched route (the guard runs inside the handler, not at routing time)");

    assert_eq!(
        response.status,
        403,
        "a module-level guard returning false must reject the request before the handler body \
         runs; body was: {}",
        String::from_utf8_lossy(response.body_ref())
    );
    let body = String::from_utf8_lossy(response.body_ref());
    assert!(
        !body.contains("should not be reached"),
        "handler body must not have executed; got: {body}"
    );
}

/// Module guards run *ahead of* controller guards (prepended, not
/// appended) — a controller-level guard that would allow the request must
/// not override a module-level guard that denies it.
#[tokio::test]
async fn module_guard_runs_ahead_of_a_permissive_controller_guard() {
    let script = r#"
        let deny_all = guard("DenyAll");
        deny_all.can_activate(|req| { false });

        let allow_all = guard("AllowAll");
        allow_all.can_activate(|req| { true });

        let ctrl = controller("/secure");
        ctrl.use_guard(allow_all);
        ctrl.get("/", |req, ctx| {
            ok().body("should not be reached")
        });

        let app_module = create_module("AppModule");
        app_module.controllers([ctrl]);
        app_module.guards([deny_all]);

        let app = create_app(app_module);
        app.listen(3000);
    "#;
    let router = support::build_router_from_script(script);

    let request = HttpRequest::new("GET".to_string(), "/secure".to_string());
    let response = router.route(request).await.expect("router should dispatch");

    assert_eq!(
        response.status, 403,
        "the module's DenyAll guard must run (and reject) before the controller's AllowAll guard"
    );
}

/// Sanity check: a controller in a module with no `guards([...])` at all
/// is unaffected — the fix must not start injecting guards out of
/// nowhere.
#[tokio::test]
async fn controller_without_a_module_guard_is_unaffected() {
    let script = r#"
        let ctrl = controller("/open");
        ctrl.get("/", |req, ctx| {
            ok().body("ok")
        });

        let app_module = create_module("AppModule");
        app_module.controllers([ctrl]);

        let app = create_app(app_module);
        app.listen(3000);
    "#;
    let router = support::build_router_from_script(script);

    let request = HttpRequest::new("GET".to_string(), "/open".to_string());
    let response = router.route(request).await.expect("router should dispatch");

    assert_eq!(response.status, 200);
    assert_eq!(String::from_utf8_lossy(response.body_ref()), "ok");
}

/// A module guard that allows the request lets it through to the handler,
/// same as a permissive controller guard would.
#[tokio::test]
async fn module_guard_that_allows_lets_the_request_through() {
    let script = r#"
        let allow_all = guard("AllowAll");
        allow_all.can_activate(|req| { true });

        let ctrl = controller("/secure");
        ctrl.get("/", |req, ctx| {
            ok().body("welcome")
        });

        let app_module = create_module("AppModule");
        app_module.controllers([ctrl]);
        app_module.guards([allow_all]);

        let app = create_app(app_module);
        app.listen(3000);
    "#;
    let router = support::build_router_from_script(script);

    let request = HttpRequest::new("GET".to_string(), "/secure".to_string());
    let response = router.route(request).await.expect("router should dispatch");

    assert_eq!(response.status, 200);
    assert_eq!(String::from_utf8_lossy(response.body_ref()), "welcome");
}
