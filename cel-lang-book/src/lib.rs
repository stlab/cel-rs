//! Support crate for *The CEL Language* book.
//!
//! This crate exists so the static mdBook can point at a real Rust package
//! whose examples compile against [`cel_parser`] and [`cel_runtime`]. The
//! first release of the book is intentionally static: it documents the
//! implementation in this repository and compiles checked snippets, but it
//! does not ship a live evaluator or any custom mdBook preprocessor.
//!
//! Build the book with `mdbook build cel-lang-book`, serve it with
//! `mdbook serve cel-lang-book`, and compile the crate's tests with
//! `cargo test -p cel-lang-book`.
