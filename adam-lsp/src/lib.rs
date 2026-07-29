//! # adam-lsp
//!
//! A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) server
//! for `adam-lang`, built on [`lsp-server`](https://docs.rs/lsp-server) + `lsp-types`. Surfaces
//! `adam-lang`'s recovered syntax errors and [`adam_lang::check_sheet`]'s type diagnostics as
//! `textDocument/publishDiagnostics`.
//!
//! # Example
//!
//! ```rust,no_run
//! fn main() -> anyhow::Result<()> {
//!     adam_lsp::run()
//! }
//! ```

pub mod diagnostics;
mod dispatch;

pub use dispatch::{run, serve};
