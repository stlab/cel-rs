//! Support crate backing *The Adam Programming Language* — the tutorial and reference
//! manual for [`adam_lang`] rendered from `book-src/` via [mdBook](https://rust-lang.github.io/mdBook/).
//!
//! This crate holds no prose. Every runnable example in the book lives as a standalone `.adm2`
//! file under `book-src/examples/<chapter>/`, pulled directly into a chapter's markdown with
//! mdBook's `{{#include examples/<chapter>/<name>.adm2}}` directive. The matching test in
//! `tests/<chapter>.rs` loads that same file with `include_str!` and asserts on its behavior,
//! verified by `cargo test -p adam-lang-book`. These assertions check the documented language
//! behavior; the staged browser build separately checks live rendering.
//! [`support::parser`] supplies the shared host registrations.
//!
//! Build the book itself with `mdbook build` (or `mdbook serve`) from this directory.

pub mod support;
