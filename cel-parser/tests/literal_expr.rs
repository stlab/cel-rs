//! Verifies retained literal conversion without reparsing or execution.

/// Preserves concrete widths and rejects malformed literal suffixes.
#[test]
fn retained_literal_conversion_uses_shared_validation() {
    let token: syn::Lit = syn::parse_str("12u16").unwrap();
    assert!(matches!(
        cel_parser::literal_expr(&token).unwrap(),
        cel_parser::Expr::Literal {
            value: cel_parser::Literal::U16(12),
            ..
        }
    ));
    let token: syn::Lit = syn::parse_str("12unknown").unwrap();
    assert!(cel_parser::literal_expr(&token).is_err());
}
