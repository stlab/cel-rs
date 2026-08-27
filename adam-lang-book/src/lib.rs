//! Support crate backing *The Adam Programming Language* — the tutorial and reference
//! manual for [`adam_lang`] rendered from `book-src/` via [mdBook](https://rust-lang.github.io/mdBook/).
//!
//! This crate holds no prose. Every runnable example in the book lives in `tests/*.rs` as an
//! ordinary `#[test]` function, verified by `cargo test -p adam-lang-book`, and pulled into a
//! chapter's markdown with mdBook's `{{#include tests/chapter.rs:anchor}}` directive — so a
//! chapter can never drift from code that actually compiles and passes. [`support::parser`]
//! is the one piece of shared setup those tests build on.
//!
//! Build the book itself with `mdbook build` (or `mdbook serve`) from this directory.

pub mod support;
