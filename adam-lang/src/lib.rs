//! # adam-lang
//!
//! A DSL parser for property models. Parses an adam-lang source string and produces
//! a live [`ParsedSheet`] (sheet plus cell names in declaration order).
//!
//! # Grammar
//!
//! ```text
//! sheet              = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item         = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl
//!                        | source_decl).
//! cell_decl          = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
//! cell_filter        = "filter" identifier ":" or_expression.
//! cell_type_init     = (":" type_expr ["=" or_expression]) | ("=" or_expression).
//! source_decl        = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
//! type_expr          = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
//! relationship_decl  = "relationship" "{" { binding } "}".
//! binding            = binding_target ":=" or_expression ";".
//! binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".
//! conditional_decl   = "conditional" or_expression "{" { conditional_branch } "}".
//! conditional_branch = (literal | "_") "=>" "{" { relationship_decl } "}" [ "," ].
//! out_decl           = "out" identifier [":" type_expr] ":=" or_expression
//!                        [ cell_filter ] [ require_block ] ";".
//! require_block      = "require" "{" { requirement } "}".
//! requirement        = identifier ":" or_expression ";".
//! ```
//!
//! The design spec for `cell_decl` also calls for an optional trailing `":=" or_expression`
//! clause, not shown above because **this crate does not yet implement it** — see
//! `docs/superpowers/specs/2026-08-19-adam-lang-syntax-design.md`'s "Explicitly out of scope"
//! section; it's deferred pending a forward-reference/hoisting decision. Only `cell_decl`'s
//! `"=" or_expression` one-time initializer and its optional `cell_filter`/`require_block`
//! clauses are implemented today.
//!
//! `or_expression` and its descendants (`literal`, `identifier`, and the rest of the
//! CEL expression grammar) are defined by `cel_parser` — see that crate's own
//! [`# Grammar`](../cel_parser/index.html#grammar) section.
//!
//! A `cell_filter`'s `or_expression` names no explicit parameter list: `_` always refers to the
//! candidate value being conformed (of the filtered cell's own declared type), and every other
//! identifier that names an already-declared cell is a dependency, deduced exactly as a
//! `relationship` binding's/`out` declaration's/`conditional`'s own `or_expression` deduces its
//! inputs — see [`ast::CellFilter`]. `_` is reserved inside a filter expression only.
//!
//! # Example
//!
//! `AdamParser::new` takes an [`OpLookup`](cel_parser::OpLookup) instance. See
//! [`OpLookup::push_library_scope`](cel_parser::OpLookup::push_library_scope) for how to
//! install one (e.g. `cel-std`) before parsing.
//!
//! ```rust
//! use adam_lang::{AdamParser, TypeRegistry};
//! use cel_parser::OpLookup;
//!
//! let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
//! let parsed = parser.parse_str(r#"
//!     sheet image_resize {
//!         cell width:  f64 = 1920.0;
//!         cell height: f64 = 1080.0;
//!         cell area:   f64;
//!     }
//! "#).unwrap();
//! ```

pub mod ast;
mod ast_parser;
mod fmt;
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;
mod typecheck;

// adam-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::AdamAstParser;
pub use cel_parser::ParseError;
pub use fmt::{FormatSourceError, format_sheet, format_source};
pub use parser::{AdamParser, ParsedSheet};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
