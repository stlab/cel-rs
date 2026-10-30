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
///
/// # Examples
///
/// ```
/// assert!(ez_adam::validation::validate_cel_expression("width / height").is_ok());
/// assert!(ez_adam::validation::validate_cel_expression("width /").is_err());
/// ```
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
    /// Arithmetic drafts validate without requiring variable bindings.
    fn accepts_a_simple_arithmetic_expression() {
        assert!(validate_cel_expression("width_pixels / height_pixels").is_ok());
    }

    #[test]
    /// Standard-library calls validate syntactically.
    fn accepts_a_call_to_a_cel_std_function() {
        assert!(validate_cel_expression("clamp(width_pixels, 0, 100)").is_ok());
    }

    #[test]
    /// An incomplete binary expression returns a parse error.
    fn rejects_malformed_syntax() {
        assert!(validate_cel_expression("width_pixels / ").is_err());
    }

    #[test]
    /// Empty input is not a CEL expression.
    fn rejects_an_empty_string() {
        assert!(validate_cel_expression("").is_err());
    }
}
