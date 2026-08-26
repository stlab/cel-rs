//! # Chapter 8: Program Style
//!
//! ## 8.1 Comments
//!
//! `//` starts a line comment; `/* ... */` a block comment — exactly the same two forms C,
//! Rust, and CEL all share:
//!
//! ```text
//! // a whole-line comment
//! cell width: i32 = 1920; // a trailing comment
//! /* a block comment, on one line or several */
//! ```
//!
//! ## 8.2 Doc comments
//!
//! `///` immediately before a `cell`, `relationship`, `conditional`, or `out` declaration, and
//! `//!` immediately before the `sheet` keyword itself, are doc comments — recovered by the
//! language server and the formatter, and otherwise inert (they carry no meaning to
//! `propagate()`):
//!
//! ```text
//! //! Describes a simple resize dialog.
//! sheet image_resize {
//!     /// The image's width in pixels, before any resampling.
//!     cell original_width_pixels: i32 = 1600;
//! }
//! ```
//!
//! ## 8.3 Canonical formatting
//!
//! adam-lang ships its own formatter (`adam fmt`, backed by [`adam_lang::format_sheet`]) with
//! one canonical layout: 4-space indentation, opening braces on the same line as the keyword
//! that introduces them, and no space before a declaration's closing `;`. Given input that
//! doesn't already follow this layout, formatting normalizes it:
//!
//! ```rust
//! let mut ast_parser = adam_lang::AdamAstParser::new();
//! let sheet = ast_parser
//!     .parse_str("sheet s{cell x:i32=1;cell y:i32=2;}")
//!     .unwrap();
//! assert!(sheet.errors.is_empty());
//!
//! let formatted = adam_lang::format_sheet(&sheet);
//! assert_eq!(
//!     formatted,
//!     "sheet s {\n    cell x: i32 = 1;\n    cell y: i32 = 2;\n}\n"
//! );
//! ```
//!
//! A formatter run is expected to be **idempotent** — formatting already-canonical source
//! reproduces it unchanged — and preserves every comment, doc comment, and blank line exactly
//! where it appeared, including a file-header comment before `sheet` itself and a trailing
//! comment before a block's own closing `}`. `adam-lsp`'s `textDocument/formatting` handler
//! refuses to format a sheet with any recorded syntax error rather than guess at intent —
//! [`adam_lang::format_sheet`]'s own precondition is that `sheet.errors` is empty.
//!
//! A `conditional` branch's trailing `,` (the grammar allows one after each branch's closing
//! `}`) is always omitted by the formatter, even though the parser still accepts it on input —
//! so canonical adam-lang source never has one.
