//! Error types for pm-lang.
//!
//! pm-lang reuses [`cel_parser::ParseError`] directly; no new error type is introduced.
//! All parse errors carry a [`proc_macro2::Span`] for source-location diagnostics.
//!
//! [`ParseError`] is re-exported from the crate root as [`crate::ParseError`].
