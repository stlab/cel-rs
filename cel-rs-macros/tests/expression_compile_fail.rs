//! UI tests that `expression!` rejects invalid CEL at compile time.

#[test]
fn expression_compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
