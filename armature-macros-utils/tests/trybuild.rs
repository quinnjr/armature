//! Compile-pass coverage for every documented macro form.
//!
//! Each fixture under `tests/ui/pass/` uses the documented syntax of a group
//! of macros and must compile cleanly. This is the guard that the documented
//! forms actually parse and expand — the class of bug (multi-arg forms that
//! never parsed) that made this crate the worst in the workspace.

#[test]
fn documented_forms_compile() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/pass/*.rs");
}
