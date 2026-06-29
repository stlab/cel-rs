//! # pm-lang
//!
//! A DSL parser for property models. Parses a pm-lang source string and produces
//! a live [`property_model::Sheet`].
//!
//! # Example
//!
//! ```rust,no_run
//! use pm_lang::{PmParser, TypeRegistry};
//! use cel_parser::OpLookup;
//!
//! let mut parser = PmParser::new(TypeRegistry::new(), OpLookup::new());
//! let sheet = parser.parse_str(r#"
//!     sheet image_resize {
//!         cell width:  f64 = 1920.0;
//!         cell height: f64 = 1080.0;
//!         cell area:   f64;
//!     }
//! "#).unwrap();
//! ```

mod error;
mod parser;
pub mod type_registry;

pub use cel_parser::ParseError;
pub use parser::PmParser;
pub use type_registry::TypeRegistry;
