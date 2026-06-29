//! pm-lang parser — filled in Tasks 4–6.

use cel_parser::OpLookup;
use property_model::Sheet;

use crate::ParseError;
use crate::TypeRegistry;

/// A parser result type aliasing [`cel_parser::ParseError`] as the error.
pub type Result<T> = std::result::Result<T, ParseError>;

/// Parses pm-lang source strings into live [`Sheet`]s.
pub struct PmParser {
    #[allow(dead_code)]
    pub(crate) types: TypeRegistry,
    #[allow(dead_code)]
    pub(crate) cel: cel_parser::CELParser,
}

impl PmParser {
    /// Creates a parser with the given type registry and operation lookup.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use pm_lang::{PmParser, TypeRegistry};
    /// use cel_parser::OpLookup;
    ///
    /// let parser = PmParser::new(TypeRegistry::new(), OpLookup::new());
    /// ```
    pub fn new(types: TypeRegistry, op_lookup: OpLookup) -> Self {
        PmParser {
            types,
            cel: cel_parser::CELParser::new(op_lookup),
        }
    }

    /// Parses a pm-lang source string into a live [`Sheet`].
    ///
    /// # Errors
    ///
    /// Returns `Err` on any syntax error, unknown type name, type mismatch, or undeclared
    /// cell name.
    pub fn parse_str(&mut self, _source: &str) -> Result<Sheet> {
        Err(ParseError::new(
            "not implemented",
            proc_macro2::Span::call_site(),
        ))
    }
}
