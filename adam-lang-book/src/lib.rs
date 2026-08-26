//! # The adam-lang Programming Language
//!
//! A tutorial and reference manual for **adam-lang**, the declarative language for describing
//! property models — sheets of cells linked by multi-way constraints — implemented by the
//! [`adam_lang`] crate and executed by [`adam_rs`].
//!
//! This book follows the shape of Kernighan & Ritchie's *The C Programming Language*: a
//! tutorial introduction you can read start to finish, followed by chapters that go back over
//! the same ground in more detail, followed by a terse reference manual for looking things up.
//! Every adam-lang source fragment shown in a fenced ` ```text ` block is exactly what you'd
//! type into a `.adm2` file; every fenced ` ```rust ` block is a real, compiled, and tested
//! example, driving adam-lang through its host embedding API (`cargo test --doc` runs all of
//! them).
//!
//! ## What this book does not cover
//!
//! adam-lang's expressions — the right-hand side of `:=`, a cell's initializer, a filter's
//! body, a `conditional`'s match subject — are [CEL](https://github.com/google/cel-spec)
//! expressions, parsed by the separate `cel-parser` crate and evaluated by `cel-runtime`. CEL's
//! own grammar, operators, literals, casts, and control-flow expressions (`if`/`else`,
//! closures, ranges) are **not** documented here; see `cel-parser`'s own crate documentation.
//! This book documents only what adam-lang adds on top of CEL: `sheet`, `cell`, `relationship`,
//! `conditional`, `out`, `require`, and `filter` — the declarative shell that turns a set of CEL
//! expressions into a live, bidirectional constraint graph.
//!
//! Likewise, `min`, `max`, `clamp`, `round`, and every other function callable from inside an
//! expression come from a function library installed into the parser's [`cel_parser::OpLookup`]
//! (this book's own examples install `cel-std`, exactly as the `begin` application does) — they
//! are not part of the adam-lang language itself, and adam-lang defines none of its own.
//!
//! ## How the book is organized
//!
//! - [`tutorial`] — Chapter 1: a guided walk through every construct, building up one small
//!   sheet at a time.
//! - [`cells`] — Chapter 2: `cell` declarations, types, and defaults, in full.
//! - [`expressions`] — Chapter 3: how an expression's cell dependencies are deduced, and the
//!   boundary between adam-lang and CEL.
//! - [`relationships`] — Chapter 4: `relationship` blocks and the constraint solver that picks
//!   one binding per relationship each round.
//! - [`conditionals`] — Chapter 5: `conditional` declarations.
//! - [`filters`] — Chapter 6: `filter` clauses — self-correcting cells.
//! - [`outputs`] — Chapter 7: `out` declarations and `require` blocks.
//! - [`style`] — Chapter 8: comments, doc comments, and the canonical formatting `adam fmt`
//!   produces.
//! - [`mod@reference`] — Appendix A: the reference manual — grammar, built-in types, error
//!   messages, and the host embedding API, gathered in one place for lookup rather than
//!   narrative reading.

pub mod cells;
pub mod conditionals;
pub mod expressions;
pub mod filters;
pub mod outputs;
pub mod reference;
pub mod relationships;
pub mod style;
pub mod support;
pub mod tutorial;
