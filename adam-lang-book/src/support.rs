//! Support code shared by this book's own runnable examples.
//!
//! Nothing here is part of adam-lang itself — it exists only so the doctests scattered through
//! this crate don't each repeat the same handful of setup lines.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;

/// Builds an [`AdamParser`] with the CEL standard library (`cel-std`) installed, exactly as
/// `begin` wires one up. Every runnable example in this book starts from this function, so
/// `min`, `max`, `clamp`, `round`, and the rest of `cel-std`'s functions are always available —
/// this book does not document those functions itself; see `cel-std`'s own crate documentation.
#[must_use]
pub fn parser() -> AdamParser {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    AdamParser::new(TypeRegistry::new(), lookup)
}
