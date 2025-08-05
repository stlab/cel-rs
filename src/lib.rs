//! cel-rs provides a stack-based runtime for developing domain specific languages, including
//! concatenative languages to describe concurrent processes.
//!
//! This crate exposes three main components:
//!
//! - **cel-runtime**: The core stack-based runtime for developing domain specific languages
//! - **cel-parser**: A recursive descent parser for CEL expressions
//! - **cel-rs-macros**: Procedural macros for CEL expressions
//!
//! # Examples
//!
//! ## Using the Runtime
//!
//! ```rust
//! use cel_rs::cel_runtime::*;
//!
//! // Create a segment that takes a u32 and &str as arguments
//! let segment = Segment::<(u32, &str)>::new()
//!     .op1r(|s| {
//!         let r = s.parse::<u32>()?;
//!         Ok(r)
//!     })
//!     .op2(|a, b| a + b)
//!     .op1(|r| r.to_string());
//! assert_eq!(segment.call((1u32, "2")).unwrap(), "3");
//! ```
//!
//! ## Using the Parser
//!
//! ```rust
//! use cel_rs::cel_parser::CELParser;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let input = TokenStream::from_str("10 + 20").unwrap();
//! let mut parser = CELParser::new(input.into_iter());
//! assert!(parser.is_expression());
//! ```
//!
//! ## Using the Macros
//!
//! ```rust
//! use cel_rs::cel_rs_macros::expression;
//!
//! expression! {
//!     54 + 25 * (11 + 6 * 6)
//! };
//! ```

pub use cel_parser;
pub use cel_rs_macros;
pub use cel_runtime;

// Re-export commonly used items for convenience
pub mod runtime {
    pub use cel_runtime::*;
}

pub mod parser {
    pub use cel_parser::*;
}

pub mod macros {
    pub use cel_rs_macros::*;
}
