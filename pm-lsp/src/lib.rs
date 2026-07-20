//! # pm-lsp
//!
//! A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) server
//! for `pm-lang`, built on [`lsp-server`](https://docs.rs/lsp-server) + `lsp-types`. Surfaces
//! `pm-lang`'s recovered syntax errors and [`pm_lang::check_sheet`]'s type diagnostics as
//! `textDocument/publishDiagnostics`.
//!
//! # Example
//!
//! ```rust,ignore
//! fn main() -> anyhow::Result<()> {
//!     pm_lsp::run()
//! }
//! ```

pub mod diagnostics;

// `run`/`serve` are added in Task 2; `lib.rs` only declares `diagnostics` for now so this
// module compiles and its tests can run standalone.
