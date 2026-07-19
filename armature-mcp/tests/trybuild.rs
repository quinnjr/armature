#[test]
fn register_macro_compiles_on_modern_rust() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/register_macro_compiles.rs");
}

#[test]
fn mcp_attribute_compiles() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/mcp_attribute_compiles.rs");
}
