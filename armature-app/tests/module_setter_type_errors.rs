//! Regression test for the Info finding: `ScriptModule::set_providers` /
//! `set_controllers` / `set_guards` / `set_imports` used
//! `filter_map(try_cast)` over the incoming Dynamic array, so any element
//! of the wrong type (e.g. a typo passing a `ScriptService` into
//! `guards([...])`) was silently dropped with no error surfaced to the
//! script author.
//!
//! `armature-app/src/types.rs:208`

#[path = "support/mod.rs"]
mod support;

#[tokio::test]
async fn providers_with_a_wrong_typed_element_errors_instead_of_silently_dropping_it() {
    let script = r#"
        let ctrl = controller("/x");
        let app_module = create_module("AppModule");
        app_module.providers([ctrl]); // wrong type: a ScriptController, not a ScriptService
        let app = create_app(app_module);
        app.listen(3000);
    "#;

    let err = support::try_build_app(script)
        .expect_err("providers([wrong-type]) must error, not silently drop the element");
    assert!(
        err.contains("providers"),
        "error should name the offending setter, got: {err}"
    );
}

#[tokio::test]
async fn controllers_with_a_wrong_typed_element_errors_instead_of_silently_dropping_it() {
    let script = r#"
        let svc = service("Svc");
        let app_module = create_module("AppModule");
        app_module.controllers([svc]); // wrong type: a ScriptService, not a ScriptController
        let app = create_app(app_module);
        app.listen(3000);
    "#;

    let err = support::try_build_app(script)
        .expect_err("controllers([wrong-type]) must error, not silently drop the element");
    assert!(
        err.contains("controllers"),
        "error should name the offending setter, got: {err}"
    );
}

#[tokio::test]
async fn guards_with_a_wrong_typed_element_errors_instead_of_silently_dropping_it() {
    let script = r#"
        let svc = service("Svc");
        let app_module = create_module("AppModule");
        app_module.guards([svc]); // wrong type: a ScriptService, not a ScriptGuard
        let app = create_app(app_module);
        app.listen(3000);
    "#;

    let err = support::try_build_app(script)
        .expect_err("guards([wrong-type]) must error, not silently drop the element");
    assert!(
        err.contains("guards"),
        "error should name the offending setter, got: {err}"
    );
}

#[tokio::test]
async fn imports_with_a_wrong_typed_element_errors_instead_of_silently_dropping_it() {
    let script = r#"
        let svc = service("Svc");
        let app_module = create_module("AppModule");
        app_module.imports([svc]); // wrong type: a ScriptService, not a ScriptModule
        let app = create_app(app_module);
        app.listen(3000);
    "#;

    let err = support::try_build_app(script)
        .expect_err("imports([wrong-type]) must error, not silently drop the element");
    assert!(
        err.contains("imports"),
        "error should name the offending setter, got: {err}"
    );
}

/// Sanity check: correctly-typed elements are unaffected by the fix.
#[tokio::test]
async fn correctly_typed_providers_still_work() {
    let script = r#"
        let svc = service("Svc");
        let app_module = create_module("AppModule");
        app_module.providers([svc]);
        let app = create_app(app_module);
        app.listen(3000);
    "#;

    support::try_build_app(script).expect("correctly-typed providers([...]) must still succeed");
}
