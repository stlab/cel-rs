//! # adam-lsp
//!
//! A [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) server
//! for `adam-lang`, built on [`lsp-server`](https://docs.rs/lsp-server) + `lsp-types`. Surfaces
//! `adam-lang`'s recovered syntax and nonexecuting type diagnostics as
//! `textDocument/publishDiagnostics`.
//!
//! # Explicit modules
//!
//! Initialize with `initializationOptions.moduleSources`, an array of records with
//! string fields `path` (an explicit [`adam_lang::ModulePath`]), `uri` (the absolute
//! source URI), and `text` (the complete source). Both paths and URIs must be unique.
//! Include importing roots as well as definitions:
//!
//! ```json
//! {"moduleSources":[
//!   {"path":"constraints","uri":"file:///repo/constraints.adm2","text":"sheet ordered(a: f64, b: f64) {}"},
//!   {"path":"app","uri":"file:///repo/app.adm2","text":"use constraints::ordered; sheet app { cell x: f64; cell y: f64; relationship ordered(a: x, b: y); }"}
//! ]}
//! ```
//!
//! Send the same complete object in `workspace/didChangeConfiguration.settings`
//! to replace the snapshot. `{"moduleSourcesError":"read failure"}` blocks checking
//! with visible configuration diagnostics until a valid snapshot arrives; it never
//! substitutes empty source text. Invalid snapshots retain the last valid ownership
//! mapping and report errors. An empty `moduleSources` array explicitly removes it.
//!
//! Open-document text overrides the registered revision by exact URI, including
//! through configuration refreshes. Closing restores the supplied revision. Changes
//! conservatively recheck every registered definition and republish every supplied
//! and open URI, clearing removed-file diagnostics. Labels retain real file owners
//! and become cross-file related information with UTF-16 ranges. Unregistered open
//! files use the source-only convenience checker with the same standard/Euclid
//! libraries. The server does no filesystem loading, discovery, root instantiation
//! or automatic language migration.
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
mod modules;

pub use dispatch::{run, serve};
