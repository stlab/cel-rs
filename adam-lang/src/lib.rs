//! # adam-lang
//!
//! A DSL parser for property models. Parses an adam-lang source string and produces
//! a live [`ParsedSheet`] (sheet plus cell names in declaration order).
//!
//! # Grammar
//!
//! ```text
//! sheet              = "sheet" identifier "{" { sheet_item } "}".
//! sheet_item         = cell_decl | relationship_decl | conditional_decl | out_decl.
//! cell_decl          = "cell" identifier cell_type_init ";".
//! cell_type_init     = (":" type_expr [ "=" or_expression ]) | ("=" or_expression).
//! type_expr          = identifier | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
//! relationship_decl  = "relationship" [ identifier ] "{" { method_decl } "}".
//! conditional_decl   = "conditional" or_expression "{" { conditional_branch } [ default_branch ] "}".
//! conditional_branch = or_expression "=>" "{" { relationship_decl } "}" [ "," ].
//! default_branch     = "_"   "=>" "{" { relationship_decl } "}" [ "," ].
//! method_decl        = "method" cell_list "->" cell_list method_body.
//! out_decl           = "out" identifier [ ":" type_expr ] "{" out_method { condition_decl } "}".
//! out_method         = "method" cell_list method_body.
//! condition_decl     = "condition" identifier cell_list "{" or_expression "}".
//! cell_list          = "[" identifier { "," identifier } "]".
//! method_body        = "{" or_expression "}".
//! ```
//!
//! `or_expression` and its descendants (`literal`, `identifier`, and the rest of the
//! CEL expression grammar) are defined by `cel_parser` — see that crate's own
//! [`# Grammar`](../cel_parser/index.html#grammar) section.
//!
//! # Example
//!
//! ```rust,no_run
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
pub use fmt::format_sheet;
pub use parser::{AdamParser, ParsedSheet};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
