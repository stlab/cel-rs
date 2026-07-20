//! # pm-lsp
//!
//! A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) server
//! for `pm-lang`, built on [`lsp-server`](https://docs.rs/lsp-server) + `lsp-types`. Surfaces
//! `pm-lang`'s recovered syntax errors and [`pm_lang::check_sheet`]'s type diagnostics as
//! `textDocument/publishDiagnostics`.
//!
//! # Example
//!
//! ```rust,no_run
//! fn main() -> anyhow::Result<()> {
//!     pm_lsp::run()
//! }
//! ```

pub mod diagnostics;
mod dispatch;

pub use dispatch::{run, serve};
