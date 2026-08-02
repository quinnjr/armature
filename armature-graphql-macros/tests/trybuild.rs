//! Compile-pass coverage: `#[resolver]` must compile when used as documented
//! on a realistic query + mutation + subscription impl block, composed via
//! `MergedObject`/`MergedSubscription`.
//!
//! Deliberately no compile-fail cases here (fragile, rustc-version-dependent
//! stderr matching) — the unit tests in `src/resolver.rs` already give
//! robust, version-independent coverage of the rejection paths by asserting
//! on `syn::Error` message content directly.

#[test]
fn resolver_compiles() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/resolver_pass.rs");
}
