//! # pm-lang
//!
//! A DSL parser for property models. Parses a pm-lang source string and produces
//! a live [`ParsedSheet`] (sheet plus cell names in declaration order).
//!
//! # Example
//!
//! ```rust,no_run
//! use pm_lang::{PmParser, TypeRegistry};
//! use cel_parser::OpLookup;
//!
//! let mut parser = PmParser::new(TypeRegistry::new(), OpLookup::new());
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
mod parser;
mod token_cursor;
mod trivia;
pub mod type_registry;
mod typecheck;

// pm-lang reuses cel_parser::ParseError directly; no new error type is introduced.
// All parse errors carry a proc_macro2::Span for source-location diagnostics.
pub use ast_parser::PmAstParser;
pub use cel_parser::ParseError;
pub use parser::{ParsedSheet, PmParser};
pub use trivia::attach_trivia;
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
