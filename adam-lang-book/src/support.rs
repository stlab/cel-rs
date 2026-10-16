//! Support code shared by this book's own runnable examples.
//!
//! Nothing here is part of adam-lang itself — it exists only so the doctests scattered through
//! this crate don't each repeat the same handful of setup lines.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;

/// Builds an [`AdamParser`] with `cel-std` and `cel-euclid` installed, exactly as
/// `begin` wires one up. Every runnable example in this book starts from this function, so
/// `min`, `max`, `clamp`, `round`, and the rest of `cel-std`'s functions are always available —
/// this book does not document those functions itself; see `cel-std`'s own crate documentation.
///
/// - Complexity: O(s²) in the number of installed signatures.
///
/// # Examples
/// ```
/// let mut parser = adam_lang_book::support::parser();
/// assert!(parser.parse_str("sheet s { cell p: Point2D; }").is_ok());
/// ```
#[must_use]
pub fn parser() -> AdamParser {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    cel_euclid::install(&mut lookup);
    let mut types = TypeRegistry::new();
    cel_euclid::register_adam_types(&mut types);
    AdamParser::new(types, lookup)
}
