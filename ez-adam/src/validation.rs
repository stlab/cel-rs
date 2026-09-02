//! Validation of user-entered CEL expression text (relationship-group
//! formulas, restrict expressions) against `cel-parser`'s grammar.

use cel_parser::{AstContext, OpLookup, ParseError, Parser};

/// Checks that `text` is a syntactically valid CEL expression, with
/// `cel-std`'s functions (e.g. `clamp`, `min`, `max`) in scope.
///
/// # Errors
///
/// Returns the underlying [`ParseError`] if `text` does not parse as a CEL
/// expression.
///
/// - Complexity: O(n) in `text.len()` (it lexes and parses `text`).
pub fn validate_cel_expression(text: &str) -> Result<(), ParseError> {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = Parser::<AstContext>::new(lookup);
    parser.parse_str_ast(text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_simple_arithmetic_expression() {
        assert!(validate_cel_expression("width_pixels / height_pixels").is_ok());
    }

    #[test]
    fn accepts_a_call_to_a_cel_std_function() {
        assert!(validate_cel_expression("clamp(width_pixels, 0, 100)").is_ok());
    }

    #[test]
    fn rejects_malformed_syntax() {
        assert!(validate_cel_expression("width_pixels / ").is_err());
    }

    #[test]
    fn rejects_an_empty_string() {
        assert!(validate_cel_expression("").is_err());
    }
}
