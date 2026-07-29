//! `trybuild` gate for `#[derive(Validate)]`. Ensures the documented derive
//! syntax expands and compiles cleanly.

#[test]
fn derive_ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/derive_pass.rs");
}
