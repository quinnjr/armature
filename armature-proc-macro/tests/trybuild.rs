//! Compile-pass coverage: every newly exported decorator must compile when
//! used as documented.

#[test]
fn decorators_compile() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/catch_pass.rs");
    t.pass("tests/ui/guard_pass.rs");
    t.pass("tests/ui/middleware_pass.rs");
}
