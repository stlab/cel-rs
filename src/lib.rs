//! cel-rs provides a stack-based runtime for developing domain specific languages, including
//! concatenative languages to describe concurrent processes.
//!
//! This crate exposes two main components:
//!
//! - **cel-runtime**: The core stack-based runtime and CEL parser
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
//! use cel_rs::cel_runtime::{CELParser, OpLookup};
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let input = TokenStream::from_str("10").unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//! parser.set_tokens(input.into_iter());
//! let result = parser.is_expression();
//! assert!(result.is_ok());
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

pub use cel_rs_macros;
pub use cel_runtime;

// Re-export commonly used items for convenience
/// Re-exports from the cel-runtime crate for convenient access.
pub mod runtime {
    pub use cel_runtime::*;
}

/// Re-exports for the CEL parser (part of cel-runtime).
pub mod parser {
    pub use cel_runtime::parser::{CELParser, op_table::OpLookup};
}

/// Re-exports from the cel-rs-macros crate for convenient access.
pub mod macros {
    pub use cel_rs_macros::*;
}
#[cfg(all(test, feature = "playground"))]
mod playground {
    struct Experiment {
        a: u32,
    }

    struct Segment<T, F: Fn(&mut T) -> String> {
        context: T,
        f: F,
    }

    impl<T, F: Fn(&mut T) -> String> Segment<T, F> {
        fn new(context: T, f: F) -> Self {
            Self { context, f }
        }

        fn call(&mut self) -> String {
            (self.f)(&mut self.context)
        }
    }

    #[test]
    fn context_segment() {
        let experiment = Experiment { a: 1 };
        let mut segment = Segment::new(experiment, |e| e.a.to_string());
        println!("{}", segment.call());
    }

    /*
    #[test]
    fn expression_macro_error() {
        let result = cel_rs_macros::expression! {
            "Hello" + "World" + 32.0;
        };
    }

    #[test]
    fn expression_macro_error2() {
        let result = cel_rs_macros::expression! {
            "Hello" + "World" + 32.0
        };
    }
    */
}
