//! UI tests that `expression!` rejects invalid CEL at compile time.

/// Requires the compiler to reject incomplete, incompatible, and unbound CEL expressions.
#[test]
fn expression_compile_fail() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
