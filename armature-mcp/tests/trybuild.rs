#[test]
fn register_macro_compiles_on_modern_rust() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/register_macro_compiles.rs");
}
