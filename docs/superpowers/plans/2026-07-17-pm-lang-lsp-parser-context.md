# pm-lang LSP — Parser Generalization (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generalize `cel-parser`'s `CELParser` into `Parser<C: ParserContext>` (monomorphized generics) so its recursive-descent grammar is written once and can drive different backends, with `DynSegmentContext` reproducing today's runtime-execution behavior exactly — a pure, behavior-preserving refactor that lays the foundation for the future AST-building context (a later, separate plan) that the language server, formatter, and eventual macro-compilation backend will all consume.

**Architecture:** A new `ParserContext` trait defines the primitive operations the CEL grammar needs (push a literal, apply a named operator, build/join branch fragments, build/index tuples). `DynSegmentContext` implements it by wrapping a `cel_runtime::DynSegment` one-for-one. `CELParser`'s struct and `impl` become generic over `C: ParserContext`; the grammar productions (`is_or_expression`, `is_additive_expression`, ...) call trait methods instead of touching `DynSegment` directly, but are otherwise character-for-character the same logic. Critically, `CELParser`'s three context-returning entry points (`parse_or_expression`, `parse_tokens`, `parse_str`) keep their exact original signatures (`Result<DynSegment>`) via a small `Parser<DynSegmentContext>`-specific `impl` block that unwraps the generic core's `Result<DynSegmentContext>` — this is what keeps `pm-lang` (which stores `DynSegment` directly in `Vec<DynSegment>`/`RefCell<DynSegment>`) compiling with zero changes.

**Tech Stack:** Rust, existing `cel-parser`/`cel-runtime` crates. No new dependencies.

## Global Constraints

- `cargo fmt --all` before every commit (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must pass (this plan
  never touches `begin`, so its two `begin`-specific clippy invocations aren't relevant here, but
  running them costs nothing and catches surprises).
- Never commit directly to `main`; this work happens on the current worktree branch.
- Doc comments follow the project's contract style (Summary / Preconditions / Postconditions /
  Complexity) — see `CLAUDE.md`.
- Every existing `cel-parser` test, and every existing test in every crate that depends on it
  (`pm-lang`, `cel-rs-macros`, `begin`), must keep passing **completely unchanged** — this plan
  introduces no new syntax and no behavior change, only an internal restructuring.

---

### Task 1: `ParserContext` trait and `DynSegmentContext`

**Files:**
- Create: `cel-parser/src/parser_context.rs`
- Modify: `cel-parser/src/lib.rs` (add two lines: `pub mod parser_context;` and a re-export —
  nothing else in this file changes in this task)

**Interfaces:**
- Produces (used by Task 2): `pub trait ParserContext: Sized` with methods `new_context() -> Self`,
  `new_fragment(&self) -> Self`, `push_literal<T: 'static + Clone>(&mut self, value: T)`,
  `apply_op(&mut self, op_lookup: &OpLookup, name: &str, arity: usize, start: Span, end: Span) -> crate::Result<()>`,
  `join2(&mut self, then_fragment: Self, else_fragment: Self) -> anyhow::Result<()>`,
  `make_tuple(&mut self, n: usize, ambient_start: usize)`,
  `peek_tuple_arity(&self) -> Option<usize>`, `tuple_index(&mut self, index: usize)`,
  `current_stack_offset(&self) -> usize`. `pub struct DynSegmentContext(pub(crate) DynSegment)`
  implementing it, plus `DynSegmentContext::into_inner(self) -> DynSegment` and
  `Deref`/`DerefMut` to `DynSegment`.

- [ ] **Step 1: Write the test module for `DynSegmentContext`**

Create `cel-parser/src/parser_context.rs` with only this content (the trait and struct it
references don't exist yet — this is the intended failing state):

```rust
//! `ParserContext`: the pluggable target a CEL grammar production emits into.
//!
//! The recursive-descent grammar in `lib.rs` is generic over `C: ParserContext` so the same
//! grammar can drive different backends without duplicating it. [`DynSegmentContext`] is the
//! first implementation: it reproduces exactly what `CELParser` did before this trait existed,
//! wrapping a [`DynSegment`] one-for-one. A future AST-building context (for the language
//! server, formatter, and eventual macro-compilation backend) is expected to be the second.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op_table::OpLookup;
    use proc_macro2::Span;

    #[test]
    fn new_context_is_empty_and_ready_for_literals() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32);
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 10);
    }

    #[test]
    fn apply_op_dispatches_builtin_addition() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32);
        ctx.push_literal(20i32);
        let lookup = OpLookup::new();
        ctx.apply_op(&lookup, "+", 2, Span::call_site(), Span::call_site())
            .unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 30);
    }

    #[test]
    fn apply_op_propagates_lookup_error() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(10i32);
        ctx.push_literal("hi".to_string());
        let lookup = OpLookup::new();
        let err = ctx
            .apply_op(&lookup, "+", 2, Span::call_site(), Span::call_site())
            .expect_err("mismatched operand types must fail");
        assert!(err.message().starts_with("no operation"));
    }

    #[test]
    fn make_tuple_and_tuple_index_roundtrip() {
        let mut ctx = DynSegmentContext::new_context();
        let ambient_start = ctx.current_stack_offset();
        ctx.push_literal(1i32);
        ctx.push_literal(2i32);
        ctx.make_tuple(2, ambient_start);
        assert_eq!(ctx.peek_tuple_arity(), Some(2));
        ctx.tuple_index(1);
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn peek_tuple_arity_is_none_for_non_tuple() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(5i32);
        assert_eq!(ctx.peek_tuple_arity(), None);
    }

    #[test]
    fn join2_selects_then_fragment_when_condition_true() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(true);
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32);
        let mut else_fragment = ctx.new_fragment();
        else_fragment.push_literal(2i32);
        ctx.join2(then_fragment, else_fragment).unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 1);
    }

    #[test]
    fn join2_selects_else_fragment_when_condition_false() {
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(false);
        let mut then_fragment = ctx.new_fragment();
        then_fragment.push_literal(1i32);
        let mut else_fragment = ctx.new_fragment();
        else_fragment.push_literal(2i32);
        ctx.join2(then_fragment, else_fragment).unwrap();
        assert_eq!(ctx.into_inner().call0::<i32>().unwrap(), 2);
    }

    #[test]
    fn deref_gives_transparent_access_to_dyn_segment_methods() {
        // Proves DynSegmentContext doesn't need `.into_inner()` for read-only DynSegment
        // methods not part of ParserContext itself (e.g. peek_output_type_id).
        let mut ctx = DynSegmentContext::new_context();
        ctx.push_literal(7i32);
        assert_eq!(ctx.peek_output_type_id(), Some(std::any::TypeId::of::<i32>()));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cel-parser parser_context`
Expected: compile errors — `cannot find function \`new_context\` in this scope` (and similarly for
every other `DynSegmentContext`/`ParserContext` reference), since neither exists yet.

- [ ] **Step 3: Implement `ParserContext` and `DynSegmentContext`**

Add this content **above** the `#[cfg(test)] mod tests { ... }` block already in
`cel-parser/src/parser_context.rs` (the module doc comment at the top of the file from Step 1
stays where it is):

```rust
use cel_runtime::DynSegment;
use proc_macro2::Span;

use crate::op_table::OpLookup;

/// The pluggable target a grammar production emits into.
///
/// Each method mirrors one operation the grammar in `lib.rs` needs. Implementations decide what
/// "emitting" means: [`DynSegmentContext`] executes immediately into a stack machine; a future
/// AST-building context would instead record a tree node.
pub trait ParserContext: Sized {
    /// Creates a fresh, empty context with no operations recorded yet.
    fn new_context() -> Self;

    /// Creates an empty fragment for building an alternate branch (one side of a
    /// short-circuiting `||`/`&&`, or an `if`/`else` branch), independent of `self`.
    ///
    /// - Precondition: `self` matches whatever precondition the implementation's equivalent of
    ///   `DynSegment::new_fragment` requires (for `DynSegmentContext`, a condition value already
    ///   present).
    fn new_fragment(&self) -> Self;

    /// Pushes a literal value.
    fn push_literal<T: 'static + Clone>(&mut self, value: T);

    /// Applies a named operator or zero-arity identifier lookup, using `op_lookup` to resolve it
    /// against whatever this context currently holds.
    ///
    /// # Errors
    ///
    /// Returns `Err` if `op_lookup` cannot resolve `name` for `arity` operands.
    fn apply_op(
        &mut self,
        op_lookup: &OpLookup,
        name: &str,
        arity: usize,
        start: Span,
        end: Span,
    ) -> crate::Result<()>;

    /// Joins two previously-built fragments into `self`, consuming a leading condition value
    /// already present on `self`. `then_fragment`'s contribution is used when the condition is
    /// `true`; `else_fragment`'s when `false`.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the fragments' produced types are incompatible.
    fn join2(&mut self, then_fragment: Self, else_fragment: Self) -> anyhow::Result<()>;

    /// Combines the last `n` emitted values into a single tuple value.
    fn make_tuple(&mut self, n: usize, ambient_start: usize);

    /// Returns the arity of the tuple currently on top, or `None` if the top value isn't a
    /// tuple.
    fn peek_tuple_arity(&self) -> Option<usize>;

    /// Replaces the tuple on top with its `index`-th element.
    ///
    /// - Precondition: `peek_tuple_arity()` returns `Some(arity)` with `index < arity`.
    fn tuple_index(&mut self, index: usize);

    /// Returns the current stack offset, used to compute tuple layouts.
    fn current_stack_offset(&self) -> usize;
}

/// [`ParserContext`] implementation that executes directly into a [`DynSegment`], reproducing
/// the runtime-execution behavior `CELParser` always had before this trait existed.
///
/// # Examples
///
/// ```rust
/// use cel_parser::parser_context::{DynSegmentContext, ParserContext};
///
/// let mut ctx = DynSegmentContext::new_context();
/// ctx.push_literal(10i32);
/// ```
pub struct DynSegmentContext(pub(crate) DynSegment);

impl DynSegmentContext {
    /// Returns the wrapped [`DynSegment`], consuming `self`.
    pub fn into_inner(self) -> DynSegment {
        self.0
    }
}

impl std::ops::Deref for DynSegmentContext {
    type Target = DynSegment;

    fn deref(&self) -> &DynSegment {
        &self.0
    }
}

impl std::ops::DerefMut for DynSegmentContext {
    fn deref_mut(&mut self) -> &mut DynSegment {
        &mut self.0
    }
}

impl ParserContext for DynSegmentContext {
    fn new_context() -> Self {
        DynSegmentContext(DynSegment::new::<()>())
    }

    fn new_fragment(&self) -> Self {
        DynSegmentContext(self.0.new_fragment())
    }

    fn push_literal<T: 'static + Clone>(&mut self, value: T) {
        self.0.just(value);
    }

    fn apply_op(
        &mut self,
        op_lookup: &OpLookup,
        name: &str,
        arity: usize,
        start: Span,
        end: Span,
    ) -> crate::Result<()> {
        op_lookup.lookup(name, &mut self.0, arity, start, end)
    }

    fn join2(&mut self, then_fragment: Self, else_fragment: Self) -> anyhow::Result<()> {
        self.0.join2(then_fragment.0, else_fragment.0)
    }

    fn make_tuple(&mut self, n: usize, ambient_start: usize) {
        self.0.make_tuple(n, ambient_start);
    }

    fn peek_tuple_arity(&self) -> Option<usize> {
        self.0.peek_tuple_arity()
    }

    fn tuple_index(&mut self, index: usize) {
        self.0.tuple_index(index);
    }

    fn current_stack_offset(&self) -> usize {
        self.0.current_stack_offset()
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser parser_context`
Expected: 8 tests pass (`new_context_is_empty_and_ready_for_literals`,
`apply_op_dispatches_builtin_addition`, `apply_op_propagates_lookup_error`,
`make_tuple_and_tuple_index_roundtrip`, `peek_tuple_arity_is_none_for_non_tuple`,
`join2_selects_then_fragment_when_condition_true`,
`join2_selects_else_fragment_when_condition_false`,
`deref_gives_transparent_access_to_dyn_segment_methods`).

- [ ] **Step 5: Wire the new module into `cel-parser/src/lib.rs`**

In `cel-parser/src/lib.rs`, find this line (near the top, in the module declarations):

```rust
pub mod op_table;
```

Add immediately after it:

```rust
pub mod parser_context;
```

Then find this line (in the `pub use` block just below the module declarations):

```rust
pub use op_table::OpLookup;
```

Add immediately after it:

```rust
pub use parser_context::{DynSegmentContext, ParserContext};
```

- [ ] **Step 6: Run the full existing `cel-parser` test suite to confirm zero regressions**

Run: `cargo test -p cel-parser`
Expected: every test that existed before this task still passes, plus the 8 new
`parser_context` tests — no failures, no changes to any existing test needed.

- [ ] **Step 7: Format and lint**

Run:
```bash
cargo fmt --all
cargo clippy -p cel-parser --all-targets -- -D warnings
```
Expected: `cargo fmt` makes no further changes beyond what it applies; `clippy` reports zero
warnings.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/parser_context.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add ParserContext trait and DynSegmentContext

Generalizes the target a CEL grammar production emits into, so the same
grammar can later drive an AST-building context (for the language server
and formatter) without duplicating it. DynSegmentContext reproduces
today's DynSegment-based execution exactly; no behavior change yet.
EOF
)"
```

---

### Task 2: Generalize `CELParser` into `Parser<C: ParserContext>`

**Files:**
- Modify: `cel-parser/src/lib.rs` (full rewrite of everything from the top of the file through
  the end of the `impl` blocks — i.e. everything *before* `#[cfg(test)] mod tests {` — using
  Task 1's trait; the `#[cfg(test)] mod tests { ... }` and `#[cfg(test)] mod playground { ... }`
  blocks at the end of the file are **not modified at all** and are the regression safety net for
  this task)

**Interfaces:**
- Consumes: `parser_context::{ParserContext, DynSegmentContext}` (Task 1).
- Produces: `pub struct Parser<C: ParserContext> { .. }`; `pub type CELParser = Parser<DynSegmentContext>;`
  — every existing public method on `CELParser` (`new`, `set_tokens`, `set_lex_tokens`,
  `take_lex_tokens`, `op_lookup_mut`, `is_expression`, `parse_or_expression`, `parse_tokens`,
  `parse_str`) keeps its **exact original signature**, so `pm-lang`, `cel-rs-macros`, and `begin`
  need zero changes. The generic `impl<C: ParserContext> Parser<C>` additionally exposes
  `parse_or_expression_ctx(&mut self) -> Result<C>`, `parse_tokens_ctx(&mut self, tokens: TokenStreamIter) -> Result<C>`,
  and `parse_str_ctx(&mut self, s: &str) -> Result<C>` for future contexts (not used by anything
  yet — no test targets these directly, they're exercised indirectly through every existing test
  that calls `parse_str`/`parse_tokens`/`parse_or_expression`, since the `DynSegmentContext`
  wrappers delegate straight to them).

**Why this preserves every existing test unchanged:** `pm-lang/src/parser.rs` declares
`fn parse_cel_or_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment>` and calls
`self.cel.parse_or_expression()` directly as that function's return value — if `parse_or_expression`
returned `Result<DynSegmentContext>` instead of `Result<DynSegment>`, this would be a type error in
`pm-lang`, a crate this task must not touch. The split below (generic `_ctx` methods returning
`Result<C>`, plus a `Parser<DynSegmentContext>`-specific `impl` block with the original names
returning `Result<DynSegment>`) is what avoids that.

- [ ] **Step 1: Replace everything before `#[cfg(test)]` in `cel-parser/src/lib.rs`**

Read the current file first to find exactly where `#[cfg(test)]` begins (search for
`#[cfg(test)]\nmod tests {`) — everything from line 1 up to (not including) that line is being
replaced. Replace it with:

```rust
//! A recursive descent parser for CEL (Common Expression Language) expressions.
//!
//! This crate provides a parser that can parse CEL expressions into executable segments.
//! The parser follows the CEL grammar specification and provides detailed error reporting
//! with source location information.
//!
//! # Error Handling
//!
//! Parse errors are returned as [`ParseError`], which carries a `proc_macro2::Span` for precise diagnostics.
//! Convert to [`CELError`] (via `From`) when the error must be stored or sent across thread boundaries.
//! All errors result from malformed input (syntax errors, type mismatches, undefined identifiers).
//!
//! # Grammar
//!
//! ```text
//! expression = or_expression ?eos?.
//! or_expression = and_expression { "||" and_expression }.
//! and_expression = comparison_expression { "&&" comparison_expression }.
//! comparison_expression = bitwise_or_expression
//!     [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
//! bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
//! bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.
//! bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.
//! bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.
//! additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.
//! multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.
//! unary_expression = (("-" | "!") unary_expression) | postfix_expression.
//! postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.
//! primary_expression = literal | identifier | tuple_or_group | if_expression.
//! tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".
//! if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].
//! parameter_list = [ or_expression { "," or_expression } ].
//! ```
//!
//! # Note
//!
//! `?eos?` denotes end of stream.
//!
//! # Examples
//!
//! ```rust
//! use cel_parser::{CELParser, OpLookup};
//!
//! let mut segment = CELParser::new(OpLookup::new()).parse_str("10u32 + 20u32 * 5u32").unwrap();
//! let result = segment.call0::<u32>();
//! assert!(result.is_ok());
//! assert_eq!(result.unwrap(), 110); // 10 + 20 * 5 = 10 + 100
//! ```
//!
//! ## Basic Usage
//!
//! ```rust
//! use cel_parser::CELParser;
//! use cel_parser::OpLookup;
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
//! ## Error Formatting
//!
//! ```rust
//! use annotate_snippets::Renderer;
//! use cel_parser::CELParser;
//! use cel_parser::OpLookup;
//! use proc_macro2::TokenStream;
//! use std::str::FromStr;
//!
//! let line = line!() + 1;
//! let source = r#"
//!   10 20
//! "#; // Invalid: missing operator
//! let input = TokenStream::from_str(source).unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//! parser.set_tokens(input.into_iter());
//!
//! if let Err(e) = parser.is_expression() {
//!     // Format error starting at line 1
//!     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
//! }
//! ```

mod error;
pub mod lex_lexer;
pub mod op_table;
pub mod parser_context;

pub use error::{CELError, FormatRustcStyle, ParseError, SourceSpan, SpanContext};
pub use op_table::OpLookup;
pub use parser_context::{DynSegmentContext, ParserContext};
pub use proc_macro2::LineColumn;

use lex_lexer::{LexLexer, Literal as CelLiteral, Token, TokenStreamIter};

use cel_runtime::DynSegment;
use proc_macro2::{Delimiter, Span, TokenStream};
use std::iter::Peekable;
use std::str::FromStr;

/// Parser result type.
pub type Result<T> = std::result::Result<T, ParseError>;

/// Pushes a literal value from `token` onto `output`.
///
/// # Errors
///
/// Returns `Err` if the literal type is unsupported or if a suffixed numeric
/// literal cannot be parsed.
fn push_literal_token<C: ParserContext>(output: &mut C, lit: CelLiteral) -> Result<()> {
    match lit {
        CelLiteral::Int(integer) => {
            match integer.suffix() {
                "" | "i32" => output.push_literal(integer.base10_parse::<i32>().map_err(|e| {
                    ParseError::new(
                        format!("invalid i32 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "u8" => output.push_literal(integer.base10_parse::<u8>().map_err(|e| {
                    ParseError::new(
                        format!("invalid u8 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "u16" => output.push_literal(integer.base10_parse::<u16>().map_err(|e| {
                    ParseError::new(
                        format!("invalid u16 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "u32" => output.push_literal(integer.base10_parse::<u32>().map_err(|e| {
                    ParseError::new(
                        format!("invalid u32 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "u64" => output.push_literal(integer.base10_parse::<u64>().map_err(|e| {
                    ParseError::new(
                        format!("invalid u64 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "u128" => output.push_literal(integer.base10_parse::<u128>().map_err(|e| {
                    ParseError::new(
                        format!("invalid u128 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "usize" => output.push_literal(integer.base10_parse::<usize>().map_err(|e| {
                    ParseError::new(
                        format!("invalid usize literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "i8" => output.push_literal(integer.base10_parse::<i8>().map_err(|e| {
                    ParseError::new(
                        format!("invalid i8 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "i16" => output.push_literal(integer.base10_parse::<i16>().map_err(|e| {
                    ParseError::new(
                        format!("invalid i16 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "i64" => output.push_literal(integer.base10_parse::<i64>().map_err(|e| {
                    ParseError::new(
                        format!("invalid i64 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "i128" => output.push_literal(integer.base10_parse::<i128>().map_err(|e| {
                    ParseError::new(
                        format!("invalid i128 literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                "isize" => output.push_literal(integer.base10_parse::<isize>().map_err(|e| {
                    ParseError::new(
                        format!("invalid isize literal `{integer}`: {e}"),
                        integer.span(),
                    )
                })?),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid integer literal suffix: `{suffix}`"),
                        integer.span(),
                    ));
                }
            };
        }
        CelLiteral::Float(float) => {
            match float.suffix() {
                "" | "f64" => output.push_literal(float.base10_parse::<f64>().map_err(|e| {
                    ParseError::new(format!("invalid f64 literal `{float}`: {e}"), float.span())
                })?),
                "f32" => output.push_literal(float.base10_parse::<f32>().map_err(|e| {
                    ParseError::new(format!("invalid f32 literal `{float}`: {e}"), float.span())
                })?),
                suffix => {
                    return Err(ParseError::new(
                        format!("invalid float literal suffix: `{suffix}`"),
                        float.span(),
                    ));
                }
            };
        }
        CelLiteral::Str(string) => {
            output.push_literal(string.value());
        }
        CelLiteral::Bool(lit_bool) => {
            output.push_literal(lit_bool.value);
        }
        CelLiteral::Char(ch) => {
            output.push_literal(ch.value());
        }
        CelLiteral::Byte(byte) => {
            output.push_literal(byte.value());
        }
        CelLiteral::ByteStr(byte_str) => {
            output.push_literal(byte_str.value());
        }
        CelLiteral::CStr(c_str) => {
            output.push_literal(c_str.value());
        }
        other => {
            return Err(ParseError::new(
                format!("unsupported literal: {other:?}"),
                other.span(),
            ));
        }
    }
    Ok(())
}

/// A recursive descent parser for expressions, generic over the [`ParserContext`] it emits
/// into.
///
/// [`CELParser`] is a type alias for `Parser<DynSegmentContext>` and remains the concrete type
/// most callers use; it behaves identically to how `CELParser` always has, before this type
/// became generic.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use cel_parser::OpLookup;
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let input = TokenStream::from_str("10").unwrap();
/// let mut parser = CELParser::new(OpLookup::new());
/// parser.set_tokens(input.into_iter());
/// let result = parser.is_expression();
/// assert!(result.is_ok());
/// ```
///
/// ## Error Formatting
///
/// ```rust
/// use annotate_snippets::Renderer;
/// use cel_parser::OpLookup;
/// use cel_parser::CELParser;
/// use proc_macro2::TokenStream;
/// use std::str::FromStr;
///
/// let line = line!() + 1;
/// let source = r#"
///   10 + 20 30
/// "#; // Invalid: missing operator
/// let input = TokenStream::from_str(source).unwrap();
/// let mut parser = CELParser::new(OpLookup::new());
/// parser.set_tokens(input.into_iter());
///
/// if let Err(e) = parser.is_expression() {
///     // Format error starting at line 1
///     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
/// }
/// ```
pub struct Parser<C: ParserContext> {
    tokens: Option<Peekable<LexLexer>>,
    context: C,
    op_lookup: OpLookup,
    last_span: Span,
}

/// A recursive descent parser that executes directly into a [`DynSegment`].
///
/// This is the parser every existing caller uses; behavior is unchanged from before [`Parser`]
/// became generic over [`ParserContext`].
pub type CELParser = Parser<DynSegmentContext>;

impl<C: ParserContext> Parser<C> {
    /// Creates a new CEL parser with the given operation lookup.
    ///
    /// No tokens are set at construction; use [`set_tokens`](Self::set_tokens),
    /// [`parse_tokens_ctx`](Self::parse_tokens_ctx), or [`parse_str_ctx`](Self::parse_str_ctx)
    /// to parse.
    ///
    /// # Arguments
    ///
    /// * `op_lookup` - Operation lookup for resolving operators and identifiers
    pub fn new(op_lookup: OpLookup) -> Self {
        Parser {
            tokens: None,
            context: C::new_context(),
            op_lookup,
            last_span: Span::call_site(),
        }
    }

    /// Sets the token stream for parsing, resetting internal state.
    ///
    /// Call before [`is_expression`](Self::is_expression) or use
    /// [`parse_tokens_ctx`](Self::parse_tokens_ctx) which sets tokens and parses in one step.
    pub fn set_tokens(&mut self, tokens: TokenStreamIter) {
        self.tokens = Some(LexLexer::new(tokens).peekable());
        self.context = C::new_context();
        self.last_span = Span::call_site();
    }

    /// Sets the token stream from an existing [`LexLexer`] iterator for inline expression parsing.
    ///
    /// Resets the context. Use together with [`parse_or_expression_ctx`](Self::parse_or_expression_ctx)
    /// and [`take_lex_tokens`](Self::take_lex_tokens) to share a token stream between pm-lang and
    /// [`CELParser`].
    pub fn set_lex_tokens(&mut self, tokens: std::iter::Peekable<lex_lexer::LexLexer>) {
        self.tokens = Some(tokens);
        self.context = C::new_context();
        self.last_span = Span::call_site();
    }

    /// Parses one `or_expression` from the current token stream and returns the built context.
    ///
    /// Unlike [`parse_str_ctx`](Self::parse_str_ctx), this method does not require
    /// end-of-stream, allowing pm-lang to parse an expression embedded within a larger token
    /// stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression_ctx(&mut self) -> Result<C> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expression expected"));
        }
        Ok(std::mem::replace(&mut self.context, C::new_context()))
    }

    /// Returns the remaining token stream after expression parsing.
    ///
    /// Call after [`parse_or_expression_ctx`](Self::parse_or_expression_ctx) to recover the
    /// shared [`LexLexer`] for continued pm-lang parsing.
    pub fn take_lex_tokens(&mut self) -> Option<std::iter::Peekable<lex_lexer::LexLexer>> {
        self.tokens.take()
    }

    /// Parses a token stream into a context value.
    ///
    /// Sets the token source, runs the expression grammar, and returns the context on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse_tokens_ctx(&mut self, tokens: TokenStreamIter) -> Result<C> {
        self.set_tokens(tokens);
        if !self.is_expression()? {
            return Err(self.error_at("expression expected"));
        }
        Ok(std::mem::replace(&mut self.context, C::new_context()))
    }

    /// Parses a string into a context value.
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens_ctx(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    pub fn parse_str_ctx(&mut self, s: &str) -> Result<C> {
        let input =
            TokenStream::from_str(s).map_err(|e| ParseError::new(e.to_string(), e.span()))?;
        self.parse_tokens_ctx(input.into_iter())
    }

    /// Returns a mutable reference to the operation lookup.
    ///
    /// This allows customization of the operations available during parsing,
    /// such as adding new scopes for custom operations or identifiers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_parser::op_table::OpLookup;
    /// use cel_parser::CELParser;
    /// use cel_runtime::DynSegment;
    /// use proc_macro2::TokenStream;
    /// use std::any::TypeId;
    /// use std::str::FromStr;
    ///
    /// let input = TokenStream::from_str("10 + 20").unwrap();
    /// let mut lookup = OpLookup::new();
    /// lookup.push_scope(|name, segment, num_operands, _span| {
    ///     let matches = {
    ///         let top = segment.peek_stack_infos(num_operands);
    ///         name == "+" && top.len() == 2 && top[0].type_id == TypeId::of::<i32>()
    ///     };
    ///     if matches {
    ///         segment.op2(|a: i32, b: i32| a + b + 1)?; // Custom addition
    ///         Ok(true)
    ///     } else {
    ///         Ok(false)
    ///     }
    /// });
    /// let mut parser = CELParser::new(lookup);
    /// parser.set_tokens(input.into_iter());
    /// ```
    pub fn op_lookup_mut(&mut self) -> &mut OpLookup {
        &mut self.op_lookup
    }

    /// Advances past the current token, recording its span in `last_span`.
    ///
    /// # Panics
    ///
    /// Panics if no token stream has been set or if there is no current token.
    fn advance(&mut self) {
        use lex_lexer::HasSpan;
        self.last_span = self
            .tokens
            .as_mut()
            .expect("tokens set")
            .next()
            .expect("token required to advance")
            .span();
    }

    /// Returns the span of the next token without consuming it, or `None` if exhausted.
    fn peek_span(&mut self) -> Option<Span> {
        self.peek_token().map(|token| {
            use lex_lexer::HasSpan;
            token.span()
        })
    }

    /// Peeks at the current token without consuming it.
    ///
    /// Returns `None` if there are no more tokens.
    fn peek_token(&mut self) -> Option<&Token> {
        self.tokens.as_mut().expect("tokens set").peek()
    }

    /// Builds a [`ParseError`] at the current token's span (or call_site if no token).
    fn error_at(&mut self, message: &str) -> ParseError {
        let span = match self.peek_token() {
            Some(token) => {
                use lex_lexer::HasSpan;
                token.span()
            }
            None => Span::call_site(),
        };
        ParseError::new(message, span)
    }

    /// Consumes and returns `true` if the next token is punctuation matching `target`.
    fn is_punctuation(&mut self, target: &str) -> bool {
        match self.peek_token() {
            Some(Token::Punct { op, .. }) if op == target => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    /// Consumes and returns `true` if the next token is an identifier matching `keyword`.
    fn is_keyword(&mut self, keyword: &str) -> bool {
        match self.peek_token() {
            Some(Token::Identifier(ident)) if ident == keyword => {
                self.advance();
                true
            }
            _ => false,
        }
    }

    /// `expression = or_expression <EOF>.`
    pub fn is_expression(&mut self) -> Result<bool> {
        if !self.is_or_expression()? {
            return Ok(false);
        }
        if self.peek_token().is_some() {
            return Err(self.error_at("unexpected token"));
        }
        Ok(true)
    }

    /// `or_expression = and_expression { "||" and_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `||`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_or_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_and_expression()? {
            while self.is_punctuation("||") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_and_expression()? {
                    return Err(self.error_at("expected and_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                let mut bypass_fragment = self.context.new_fragment();
                bypass_fragment.push_literal(true);
                self.context
                    .join2(bypass_fragment, rhs_fragment)
                    .map_err(|e| {
                        ParseError::new_range(
                            e.to_string(),
                            start_span.expect("production has token at start"),
                            self.last_span,
                        )
                    })?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `and_expression = comparison_expression { "&&" comparison_expression }.`
    ///
    /// # Errors
    ///
    /// Returns an error if the RHS is missing after `&&`, if the RHS does not
    /// produce a `bool`, or if any sub-expression returns an error.
    fn is_and_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_comparison_expression()? {
            while self.is_punctuation("&&") {
                let mut rhs_fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                if !self.is_comparison_expression()? {
                    return Err(self.error_at("expected comparison_expression"));
                }
                std::mem::swap(&mut self.context, &mut rhs_fragment);
                let mut bypass_fragment = self.context.new_fragment();
                bypass_fragment.push_literal(false);
                self.context
                    .join2(rhs_fragment, bypass_fragment)
                    .map_err(|e| {
                        ParseError::new_range(
                            e.to_string(),
                            start_span.expect("production has token at start"),
                            self.last_span,
                        )
                    })?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `comparison_expression = bitwise_or_expression [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].`
    fn is_comparison_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_or_expression()? {
            // Longer operators first: must check "==" before "=", "<=" before "<", etc.
            let op_name = if self.is_punctuation("==") {
                Some("==")
            } else if self.is_punctuation("!=") {
                Some("!=")
            } else if self.is_punctuation("<=") {
                Some("<=")
            } else if self.is_punctuation(">=") {
                Some(">=")
            } else if self.is_punctuation("<") {
                Some("<")
            } else if self.is_punctuation(">") {
                Some(">")
            } else {
                None
            };

            if let Some(op_name) = op_name {
                if !self.is_bitwise_or_expression()? {
                    return Err(self.error_at("expected bitwise_or_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    op_name,
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.`
    fn is_bitwise_or_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_xor_expression()? {
            while self.is_punctuation("|") {
                if !self.is_bitwise_xor_expression()? {
                    return Err(self.error_at("expected bitwise_xor_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "|",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_xor_expression = bitwise_and_expression { "^" bitwise_and_expression }.`
    fn is_bitwise_xor_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_and_expression()? {
            while self.is_punctuation("^") {
                if !self.is_bitwise_and_expression()? {
                    return Err(self.error_at("expected bitwise_and_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "^",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_and_expression = bitwise_shift_expression { "&" bitwise_shift_expression }.`
    fn is_bitwise_and_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_bitwise_shift_expression()? {
            while self.is_punctuation("&") {
                if !self.is_bitwise_shift_expression()? {
                    return Err(self.error_at("expected bitwise_shift_expression"));
                }
                self.context.apply_op(
                    &self.op_lookup,
                    "&",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `bitwise_shift_expression = additive_expression { ("<<" | ">>") additive_expression }.`
    fn is_bitwise_shift_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_additive_expression()? {
            loop {
                let op_name = if self.is_punctuation("<<") {
                    Some("<<")
                } else if self.is_punctuation(">>") {
                    Some(">>")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_additive_expression()? {
                        return Err(self.error_at("expected additive_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.`
    fn is_additive_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_multiplicative_expression()? {
            loop {
                let op_name = if self.is_punctuation("+") {
                    Some("+")
                } else if self.is_punctuation("-") {
                    Some("-")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_multiplicative_expression()? {
                        return Err(self.error_at("expected multiplicative_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `multiplicative_expression = unary_expression { ("*" | "/" | "%") unary_expression }.`
    fn is_multiplicative_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if self.is_unary_expression()? {
            loop {
                let op_name = if self.is_punctuation("*") {
                    Some("*")
                } else if self.is_punctuation("/") {
                    Some("/")
                } else if self.is_punctuation("%") {
                    Some("%")
                } else {
                    None
                };

                if let Some(op_name) = op_name {
                    if !self.is_unary_expression()? {
                        return Err(self.error_at("expected unary_expression"));
                    }
                    self.context.apply_op(
                        &self.op_lookup,
                        op_name,
                        2,
                        start_span.expect("production has token at start"),
                        self.last_span,
                    )?;
                } else {
                    break;
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// `unary_expression = (("-" | "!") unary_expression) | primary_expression.`
    fn is_unary_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        let op_name = if self.is_punctuation("-") {
            Some("-")
        } else if self.is_punctuation("!") {
            Some("!")
        } else {
            None
        };

        if let Some(op_name) = op_name {
            if !self.is_unary_expression()? {
                return Err(self.error_at("expected unary_expression"));
            }
            self.context.apply_op(
                &self.op_lookup,
                op_name,
                1,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
            Ok(true)
        } else {
            self.is_postfix_expression()
        }
    }

    /// `postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.`
    ///
    /// The repetition allows chained indices (`t.0.1`): each `"." unsuffixed_integer`
    /// is applied in turn to whatever value the previous step left on top of the
    /// stack. Source text like `.0.1` tokenizes as a single `.` followed by one
    /// float literal `0.1` (Rust's own lexer maximally munches the digits after
    /// the second `.`), so that case is detected and split back into its two
    /// integer indices — see the `Token::Literal(CelLiteral::Float(..))` arm below.
    fn is_postfix_expression(&mut self) -> Result<bool> {
        let start_span = self.peek_span();
        if !self.is_primary_expression()? {
            return Ok(false);
        }
        loop {
            if matches!(
                self.peek_token(),
                Some(Token::OpenDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance(); // consume "("
                let arg_count = self.parameter_list()?;
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Parenthesis,
                        ..
                    }) => {
                        self.advance(); // consume ")"
                    }
                    _ => return Err(self.error_at("expected closing parenthesis")),
                }
                // Stack order is [callee, arg1, arg2, ...]; lookup peeks top (arg_count + 1) entries.
                self.context.apply_op(
                    &self.op_lookup,
                    "()",
                    arg_count + 1,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            } else if self.is_punctuation(".") {
                match self.peek_token() {
                    Some(Token::Literal(CelLiteral::Int(integer))) => {
                        let integer = integer.clone();
                        if !integer.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        self.advance();
                        let index = integer.base10_parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{integer}`: {e}"))
                        })?;
                        self.apply_tuple_index(index)?;
                    }
                    Some(Token::Literal(CelLiteral::Float(float))) => {
                        let float = float.clone();
                        if !float.suffix().is_empty() {
                            return Err(self.error_at("tuple index must be an unsuffixed integer"));
                        }
                        // base10_digits() returns the decimal digits with underscores
                        // stripped and the suffix removed, e.g. "0.1" or "10.25" for
                        // ordinary decimal floats — splitting on '.' recovers the two
                        // chained integer indices. Scientific-notation floats (e.g.
                        // `1e2`) normalize to digits with no '.' at all; reject those
                        // as a parse error (checked before advancing, so the error
                        // span still points at the float token) rather than assuming
                        // a '.' is always present.
                        let digits = float.base10_digits();
                        let Some((first, second)) = digits.split_once('.') else {
                            return Err(self.error_at(
                                "tuple index chain must use decimal notation (e.g. `.0.1`)",
                            ));
                        };
                        self.advance();
                        let first_index = first.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{first}`: {e}"))
                        })?;
                        let second_index = second.parse::<usize>().map_err(|e| {
                            self.error_at(&format!("invalid tuple index `{second}`: {e}"))
                        })?;
                        self.apply_tuple_index(first_index)?;
                        self.apply_tuple_index(second_index)?;
                    }
                    _ => return Err(self.error_at("expected integer after '.'")),
                }
            } else {
                break;
            }
        }
        Ok(true)
    }

    /// Applies a single `.N` tuple-index operation to the value currently on
    /// top of the context, replacing it with element `index`.
    ///
    /// # Errors
    /// Returns an error if the top of stack isn't a tuple, or if `index` is
    /// out of range for its arity.
    fn apply_tuple_index(&mut self, index: usize) -> Result<()> {
        let arity = self
            .context
            .peek_tuple_arity()
            .ok_or_else(|| self.error_at("'.N' requires a tuple"))?;
        if index >= arity {
            return Err(self.error_at(&format!(
                "tuple index `{index}` out of range for tuple of arity {arity}"
            )));
        }
        self.context.tuple_index(index);
        Ok(())
    }

    /// `parameter_list = [ or_expression { "," or_expression } ].`
    ///
    /// Returns the argument count.
    fn parameter_list(&mut self) -> Result<usize> {
        let mut count = 0;
        if self.is_or_expression()? {
            count += 1;
            while self.is_punctuation(",") {
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression after comma"));
                }
                count += 1;
            }
        }
        Ok(count)
    }

    /// `primary_expression = literal | identifier | tuple_or_group | if_expression.`
    ///
    /// Dispatches to [`is_if_expression`](Self::is_if_expression) when the `if` keyword is seen,
    /// and to [`is_tuple_or_group`](Self::is_tuple_or_group) when `(` is seen.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A literal value cannot be parsed (e.g., integer out of range).
    /// - An identifier is not found in the op lookup table.
    /// - A tuple-or-group expression fails to parse.
    /// - An `if` expression fails to parse.
    fn is_primary_expression(&mut self) -> Result<bool> {
        match self.peek_token() {
            Some(Token::Literal(lit)) => {
                let lit_clone = lit.clone();
                self.advance();
                push_literal_token(&mut self.context, lit_clone)?;
                Ok(true)
            }
            Some(Token::Identifier(ident)) => {
                let ident_name = ident.to_string();
                let ident_span = ident.span();
                self.advance();

                if ident_name == "if" {
                    return self.is_if_expression();
                }

                self.context
                    .apply_op(&self.op_lookup, &ident_name, 0, ident_span, ident_span)?;

                Ok(true)
            }
            Some(Token::OpenDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            }) => self.is_tuple_or_group(),
            _ => Ok(false),
        }
    }

    /// `tuple_or_group = "(" [ or_expression ["," [ or_expression { "," or_expression } ]] ] ")".`
    ///
    /// `()` parses as unit, `(expr)` as grouping, `(expr,)` as a 1-tuple, and
    /// `(expr, expr, ...)` as an n-tuple.
    ///
    /// - Precondition: The next token is `Token::OpenDelim` with `Delimiter::Parenthesis`.
    ///
    /// # Errors
    ///
    /// Returns an error if the parenthesized expression or tuple literal is malformed, has a
    /// missing or misplaced comma, or is missing its closing `)`.
    fn is_tuple_or_group(&mut self) -> Result<bool> {
        self.advance();
        // Unit expression: ()
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            self.advance();
            self.context.push_literal(());
            return Ok(true);
        }
        let ambient_start = self.context.current_stack_offset();
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression"));
        }
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Grouping: exactly one expression, no comma.
            self.advance();
            return Ok(true);
        }
        if !self.is_punctuation(",") {
            return Err(self.error_at("expected ',' or closing parenthesis"));
        }
        let mut count = 1;
        if matches!(
            self.peek_token(),
            Some(Token::CloseDelim {
                delimiter: Delimiter::Parenthesis,
                ..
            })
        ) {
            // Single element + trailing comma: 1-tuple.
            self.advance();
            self.context.make_tuple(count, ambient_start);
            return Ok(true);
        }
        loop {
            if !self.is_or_expression()? {
                return Err(self.error_at("expected expression after ','"));
            }
            count += 1;
            if matches!(
                self.peek_token(),
                Some(Token::CloseDelim {
                    delimiter: Delimiter::Parenthesis,
                    ..
                })
            ) {
                self.advance();
                break;
            }
            if !self.is_punctuation(",") {
                return Err(self.error_at("expected ',' or closing parenthesis"));
            }
        }
        self.context.make_tuple(count, ambient_start);
        Ok(true)
    }

    /// `if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].`
    ///
    /// - Precondition: The `if` keyword has already been consumed by the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the condition is missing, if a `{` or `}` delimiter is missing,
    /// if the then-branch or else-branch expression is missing, or if the then and else
    /// branch types do not match (as detected by `join2`).
    ///
    /// - Postcondition: Returns `Ok(true)` on success; `Ok(false)` is never returned.
    fn is_if_expression(&mut self) -> Result<bool> {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected condition after `if`"));
        }
        match self.peek_token() {
            Some(Token::OpenDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `{` after if condition")),
        }
        let mut then_fragment = self.context.new_fragment();
        std::mem::swap(&mut self.context, &mut then_fragment);
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression in then-branch"));
        }
        std::mem::swap(&mut self.context, &mut then_fragment);
        match self.peek_token() {
            Some(Token::CloseDelim {
                delimiter: Delimiter::Brace,
                ..
            }) => {
                self.advance();
            }
            _ => return Err(self.error_at("expected `}` after then-branch")),
        }
        let else_fragment = if self.is_keyword("else") {
            if self.is_keyword("if") {
                // else if: recursively parse another if_expression
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                self.is_if_expression()?;
                std::mem::swap(&mut self.context, &mut fragment);
                fragment
            } else {
                // else { expr }
                match self.peek_token() {
                    Some(Token::OpenDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `{` or `if` after `else`")),
                }
                let mut fragment = self.context.new_fragment();
                std::mem::swap(&mut self.context, &mut fragment);
                if !self.is_or_expression()? {
                    return Err(self.error_at("expected expression in else-branch"));
                }
                std::mem::swap(&mut self.context, &mut fragment);
                match self.peek_token() {
                    Some(Token::CloseDelim {
                        delimiter: Delimiter::Brace,
                        ..
                    }) => {
                        self.advance();
                    }
                    _ => return Err(self.error_at("expected `}` after else-branch")),
                }
                fragment
            }
        } else {
            // Implicit else: () — then-branch must also return ()
            let mut fragment = self.context.new_fragment();
            fragment.push_literal(());
            fragment
        };
        self.context
            .join2(then_fragment, else_fragment)
            .map_err(|e| ParseError::new(e.to_string(), self.last_span))?;
        Ok(true)
    }
}

impl Parser<DynSegmentContext> {
    /// Parses one `or_expression` from the current token stream and returns the segment.
    ///
    /// Unlike [`parse_str`](Self::parse_str), this method does not require end-of-stream,
    /// allowing pm-lang to parse an expression embedded within a larger token stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid `or_expression`.
    ///
    /// - Complexity: O(n) in the number of tokens in the expression.
    pub fn parse_or_expression(&mut self) -> Result<DynSegment> {
        self.parse_or_expression_ctx()
            .map(DynSegmentContext::into_inner)
    }

    /// Parses a token stream into a [`DynSegment`].
    ///
    /// Sets the token source, runs the expression grammar, and returns the segment on success.
    ///
    /// # Errors
    ///
    /// Returns an error if the input does not contain a valid CEL expression.
    pub fn parse_tokens(&mut self, tokens: TokenStreamIter) -> Result<DynSegment> {
        self.parse_tokens_ctx(tokens).map(DynSegmentContext::into_inner)
    }

    /// Parses a string into a [`DynSegment`].
    ///
    /// Tokenizes the string then parses; equivalent to
    /// `parse_tokens(TokenStream::from_str(s)?.into_iter())`.
    ///
    /// # Errors
    ///
    /// Returns an error on lex failure or if the input does not contain a valid CEL expression.
    pub fn parse_str(&mut self, s: &str) -> Result<DynSegment> {
        self.parse_str_ctx(s).map(DynSegmentContext::into_inner)
    }
}
```

Leave everything from `#[cfg(test)]\nmod tests {` through the end of the file completely
untouched.

- [ ] **Step 2: Run the full `cel-parser` test suite**

Run: `cargo test -p cel-parser`
Expected: every test passes — the same tests that passed before this task, with no test code
changed at all. If anything fails, the most likely cause is a missed call-site conversion (e.g.
a leftover `.just(` that should be `.push_literal(`, or a leftover `self.op_lookup.lookup(...)`
that should be `self.context.apply_op(&self.op_lookup, ...)`) — compare against the exact
grammar production shown above for the failing test's expression form.

- [ ] **Step 3: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: every test in every crate passes, including `pm-lang`, `cel-rs-macros`, `property-model`,
and `begin` — none of these crates' source is touched by this task, so this step is purely a
safety-net confirmation that `CELParser`'s public API is unchanged.

- [ ] **Step 4: Lint and build checks**

Run:
```bash
cargo build --workspace
cargo clippy -p cel-parser --all-targets -- -D warnings
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: zero warnings from all three.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
refactor(cel-parser): generalize CELParser into Parser<C: ParserContext>

The grammar now emits into a ParserContext instead of touching DynSegment
directly, with DynSegmentContext reproducing today's execution exactly.
CELParser (= Parser<DynSegmentContext>) keeps its exact original public
API via a dedicated impl block, so no other crate needs any changes.
EOF
)"
```

---

## Self-Review

**Spec coverage:** This plan covers the `cel-parser` half of the design doc's Phase 1
("Generalize the parser" — `ParserContext`, `DynSegmentContext`, behavior-preserving). The
`pm-lang`-side analogous refactor (its own declaration-level grammar becoming
context-generic) is intentionally a separate follow-on plan — `pm-lang`'s refactor depends on
this one landing first, and bundling both into one plan would make a single review/execution
pass unreasonably large and risky. Phases 2–5 of the design doc (AST context, LSP diagnostics,
formatter, richer LSP features) are explicitly out of scope for this plan and will each get their
own plan once this one is merged.

**Placeholder scan:** No TBD/TODO; every step shows complete code; no step says "similar to
Task N" without the code.

**Type consistency:** `ParserContext`'s trait methods (`new_context`, `new_fragment`,
`push_literal`, `apply_op`, `join2`, `make_tuple`, `peek_tuple_arity`, `tuple_index`,
`current_stack_offset`) are named and typed identically between Task 1's trait definition and
every call site in Task 2's converted grammar. `Parser<C>`'s generic `_ctx`-suffixed methods
(`parse_or_expression_ctx`, `parse_tokens_ctx`, `parse_str_ctx`) and the `Parser<DynSegmentContext>`-specific
originals (`parse_or_expression`, `parse_tokens`, `parse_str`) are consistently named and typed
across both impl blocks in Task 2.

---

Plan complete and saved to `docs/superpowers/plans/2026-07-17-pm-lang-lsp-parser-context.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
