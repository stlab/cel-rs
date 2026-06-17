# ParseError Span Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce a `ParseError` type that carries a `proc_macro2::Span` directly so proc-macro diagnostics point to the offending token, while `CELError` remains `Send + Sync` for async/runtime use via `SourceSpan`.

**Architecture:** The parser module currently uses `CELError` (line/column only) everywhere, losing the opaque compiler span handle at the point of error creation. We introduce `ParseError` as the parser's native error type (carries `proc_macro2::Span`, not `Send + Sync`) and make `CELError` the runtime-storage type. A `From<ParseError> for CELError` conversion extracts the `SourceSpan` at the one boundary where errors cross to async code (`DynSegment::from_str`). The proc-macro calls `parse_tokens` and reads `e.span()` directly from the `ParseError`, eliminating the `Span::call_site()` regression.

**Tech Stack:** Rust, `proc_macro2`, `annotate-snippets`

## Global Constraints

- All public doc comments must follow contract style as described in CLAUDE.md.
- No heap allocations beyond what is already present; no new `Box<dyn Trait>`.
- `cargo clippy --workspace -- -D warnings` must pass after every commit.
- `cargo test --workspace` must pass after every commit.
- `cargo test --doc --workspace` must pass after every commit.

---

### Task 1: Add `ParseError` to `parser/error.rs` and export it

**Files:**
- Modify: `cel-runtime/src/parser/error.rs`
- Modify: `cel-runtime/src/parser/mod.rs` (export line only)
- Modify: `cel-runtime/src/lib.rs` (re-export line only)

**Interfaces:**
- Produces:
  - `pub struct ParseError` with `fn new(message: impl Into<String>, span: proc_macro2::Span) -> Self`
  - `fn message(&self) -> &str`
  - `fn span(&self) -> proc_macro2::Span`
  - `fn format_rustc_style(&self, source_code: &str, filename: &str, start_line: u32, renderer: &Renderer) -> String`
  - `impl Display for ParseError`
  - `impl std::error::Error for ParseError`
  - `impl From<ParseError> for CELError`
  - `CELError::with_proc_macro_span` removed

---

- [ ] **Step 1: Write failing tests for `ParseError`**

Add a `#[cfg(test)]` block at the bottom of `cel-runtime/src/parser/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use annotate_snippets::Renderer;
    use proc_macro2::Span;

    #[test]
    fn parse_error_message() {
        let e = ParseError::new("bad token", Span::call_site());
        assert_eq!(e.message(), "bad token");
    }

    #[test]
    fn parse_error_display() {
        let e = ParseError::new("bad token", Span::call_site());
        assert_eq!(e.to_string(), "bad token");
    }

    #[test]
    fn parse_error_into_cel_error() {
        let e = ParseError::new("bad token", Span::call_site());
        let cel: CELError = e.into();
        assert_eq!(cel.message(), "bad token");
    }

    #[test]
    fn parse_error_format_rustc_style() {
        let source = "10 + 20 30";
        // We can only use call_site() in unit tests; the caret position
        // won't be meaningful, but the message and file path must appear.
        let e = ParseError::new("unexpected token", Span::call_site());
        let formatted = e.format_rustc_style(source, "test.cel", 1, &Renderer::plain());
        assert!(formatted.contains("error: unexpected token"));
        assert!(formatted.contains("test.cel"));
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```
cargo test --workspace parser::error
```

Expected: compile error — `ParseError` does not exist yet.

- [ ] **Step 3: Implement `ParseError` in `cel-runtime/src/parser/error.rs`**

Add the following after the `CELError` impl block (before the `Display` impl). Also remove `CELError::with_proc_macro_span`.

```rust
/// A parse error carrying the original `proc_macro2::Span` of the offending token.
///
/// Used as the return type of all parser methods. Not `Send + Sync` because
/// `proc_macro2::Span` wraps a compiler-internal handle that is only valid on
/// the proc-macro thread. Convert to [`CELError`] via `From` when the error
/// must cross thread boundaries or be stored for async reporting.
#[derive(Clone, Debug)]
pub struct ParseError {
    message: String,
    span: proc_macro2::Span,
}

impl ParseError {
    /// Creates a new parse error with the given message and token span.
    pub fn new(message: impl Into<String>, span: proc_macro2::Span) -> Self {
        ParseError {
            message: message.into(),
            span,
        }
    }

    /// Returns the error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the `proc_macro2::Span` of the offending token.
    ///
    /// Use this span with `quote_spanned!` in proc-macro code to attach the
    /// `compile_error!` to the exact source location.
    pub fn span(&self) -> proc_macro2::Span {
        self.span
    }

    /// Formats this error in rustc diagnostic style with source context.
    ///
    /// Identical contract to [`CELError::format_rustc_style`]; prefer calling
    /// this directly on a `ParseError` rather than converting to `CELError`
    /// first when you have the source text at hand.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use annotate_snippets::Renderer;
    /// use cel_runtime::parser::CELParser;
    /// use cel_runtime::OpLookup;
    ///
    /// let line = line!() + 1;
    /// let source = "10 + 20 30";
    /// let mut parser = CELParser::new(OpLookup::new());
    /// if let Err(e) = parser.parse_str(source) {
    ///     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
    /// }
    /// ```
    pub fn format_rustc_style(
        &self,
        source_code: &str,
        filename: &str,
        start_line: u32,
        renderer: &Renderer,
    ) -> String {
        let source_span = SourceSpan::from_proc_macro2(self.span);
        let byte_range = span_to_byte_range(source_code, source_span);
        let report = [
            Group::with_title(Level::ERROR.primary_title(self.message.as_str())).element(
                Snippet::source(source_code)
                    .path(filename)
                    .line_start(start_line as usize)
                    .annotation(AnnotationKind::Primary.span(byte_range)),
            ),
        ];
        renderer.render(&report)
    }
}

impl From<ParseError> for CELError {
    /// Converts a [`ParseError`] to a [`CELError`] by extracting the
    /// [`SourceSpan`] from the token span.
    ///
    /// Use this at async/runtime boundaries where `Send + Sync` is required.
    fn from(e: ParseError) -> Self {
        CELError::new(e.message, SourceSpan::from_proc_macro2(e.span))
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}
```

Remove the `CELError::with_proc_macro_span` method from the `CELError` impl block (it is replaced by `ParseError::new`).

- [ ] **Step 4: Export `ParseError` from `parser/mod.rs`**

In `cel-runtime/src/parser/mod.rs`, change line 90:

```rust
// before
pub use error::{CELError, SourceSpan};

// after
pub use error::{CELError, ParseError, SourceSpan};
```

- [ ] **Step 5: Re-export `ParseError` from `cel-runtime/src/lib.rs`**

Add `ParseError` alongside the existing re-exports:

```rust
// before
pub use parser::CELError;
pub use parser::CELParser;
pub use parser::op_table::OpLookup;

// after
pub use parser::CELError;
pub use parser::CELParser;
pub use parser::ParseError;
pub use parser::op_table::OpLookup;
```

- [ ] **Step 6: Run tests to confirm they pass**

```
cargo test --workspace parser::error
```

Expected: all `ParseError` tests pass; no regressions.

- [ ] **Step 7: Commit**

```
git add cel-runtime/src/parser/error.rs cel-runtime/src/parser/mod.rs cel-runtime/src/lib.rs
git commit -m "Add ParseError carrying proc_macro2::Span; From<ParseError> for CELError"
```

---

### Task 2: Migrate parser internals to return `ParseError`

**Files:**
- Modify: `cel-runtime/src/parser/mod.rs` (all internal error construction + `Result` alias)
- Modify: `cel-runtime/src/lib.rs` (`DynSegment::from_str`)

**Interfaces:**
- Consumes: `ParseError::new`, `CELError::from(ParseError)` (from Task 1)
- Produces:
  - `pub type Result<T> = std::result::Result<T, ParseError>`
  - `CELParser::error_at` returns `ParseError`
  - `CELParser::parse_str` returns `parser::Result<DynSegment>` (i.e., `Result<DynSegment, ParseError>`)
  - `CELParser::parse_tokens` returns `parser::Result<DynSegment>` (i.e., `Result<DynSegment, ParseError>`)
  - `CELParser::is_expression` returns `parser::Result<bool>` (i.e., `Result<bool, ParseError>`)
  - `DynSegment::from_str` `Err` type remains `CELError` (unchanged externally), converts internally

---

- [ ] **Step 1: Change the `Result<T>` alias in `parser/mod.rs`**

```rust
// before
pub type Result<T> = std::result::Result<T, CELError>;

// after
pub type Result<T> = std::result::Result<T, ParseError>;
```

After this change the file will not compile until all construction sites are updated. Proceed through steps 2–4 before running tests.

- [ ] **Step 2: Update `error_at` to return `ParseError`**

In `CELParser::error_at` (around line 390):

```rust
// before
fn error_at(&mut self, message: &str) -> CELError {
    let span = match self.peek_token() {
        Some(token) => {
            use lex_lexer::HasSpan;
            token.span()
        }
        None => Span::call_site(),
    };
    CELError::new(message, SourceSpan::from_proc_macro2(span))
}

// after
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
```

- [ ] **Step 3: Replace `CELError::with_proc_macro_span` in `push_literal`**

`push_literal` contains ~18 calls of the form:
```rust
CELError::with_proc_macro_span(format!("..."), token.span())
```

Replace every occurrence with:
```rust
ParseError::new(format!("..."), token.span())
```

Also update the lex-error mapping in `parse_str`:

```rust
// before
.map_err(|e| CELError::with_proc_macro_span(format!("lex: {}", e), e.span()))?;

// after
.map_err(|e| ParseError::new(format!("lex: {}", e), e.span()))?;
```

- [ ] **Step 4: Update `DynSegment::from_str` in `cel-runtime/src/lib.rs` to convert the error**

```rust
// before
impl std::str::FromStr for DynSegment {
    type Err = parser::CELError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut parser = parser::CELParser::new(OpLookup::new());
        parser.parse_str(s)
    }
}

// after
impl std::str::FromStr for DynSegment {
    type Err = parser::CELError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut parser = parser::CELParser::new(OpLookup::new());
        parser.parse_str(s).map_err(parser::CELError::from)
    }
}
```

- [ ] **Step 5: Fix the debug print in `print_error_formatting` test**

In `parser/mod.rs` around line 1041, `err.span()` now returns `proc_macro2::Span` (method calls) instead of `SourceSpan` (field access):

```rust
// before
eprintln!(
    "DEBUG: span.start.line = {}, span.start.column = {}",
    err.span().start.line,
    err.span().start.column
);

// after
eprintln!(
    "DEBUG: span.start.line = {}, span.start.column = {}",
    err.span().start().line,
    err.span().start().column
);
```

- [ ] **Step 6: Update module-level doc comment in `parser/mod.rs`**

Change line 9 from:
```rust
//! Parse errors are returned as [`CELError`], which carries a message and source span for diagnostics.
```
to:
```rust
//! Parse errors are returned as [`ParseError`], which carries a `proc_macro2::Span` for precise diagnostics.
//! Convert to [`CELError`] (via `From`) when the error must be stored or sent across thread boundaries.
```

Also update the two doc examples on `CELParser` struct (around line 274 and 80) that call `e.format_rustc_style`. They use `parser.is_expression()` returning `Err(e)` — these still compile unchanged because `ParseError` has `format_rustc_style` with the same signature.

- [ ] **Step 7: Run tests**

```
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace -- -D warnings
```

Expected: all pass.

- [ ] **Step 8: Commit**

```
git add cel-runtime/src/parser/mod.rs cel-runtime/src/lib.rs
git commit -m "Migrate parser to return ParseError; convert to CELError at FromStr boundary"
```

---

### Task 3: Fix proc-macro to use `ParseError::span()`

**Files:**
- Modify: `cel-rs-macros/src/lib.rs`

**Interfaces:**
- Consumes: `CELParser::parse_tokens` returning `parser::Result<DynSegment>` (i.e., `Result<DynSegment, ParseError>`) from Task 2; `ParseError::span()` returning `proc_macro2::Span` from Task 1
- Produces: `compile_error!` emitted at the span of the offending token, not `call_site()`

---

- [ ] **Step 1: Update imports in `cel-rs-macros/src/lib.rs`**

```rust
// before
use cel_runtime::{CELError, CELParser, OpLookup};
use proc_macro::TokenStream as ProcMacroTokenStream;
use proc_macro2::{Literal, TokenStream};
use quote::quote_spanned;

// after
use cel_runtime::{CELParser, OpLookup};
use proc_macro::TokenStream as ProcMacroTokenStream;
use proc_macro2::{Literal, TokenStream};
use quote::quote_spanned;
```

(`CELError` is no longer used in this file; `ParseError` is inferred from the return type of `parse_tokens`.)

- [ ] **Step 2: Rewrite the `expression` proc-macro body**

```rust
// before
#[proc_macro]
pub fn expression(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    let input = TokenStream::from(input);
    let mut parser = CELParser::new(OpLookup::new());
    parser.set_tokens(input.into_iter());
    match parser.is_expression() {
        Ok(true) => ProcMacroTokenStream::new(),
        Ok(false) => {
            let e = CELError::new(
                "Expected expression",
                cel_runtime::parser::SourceSpan::default(),
            );
            let msg_lit = Literal::string(&e.to_string());
            quote_spanned!(proc_macro2::Span::call_site() => compile_error!(#msg_lit)).into()
        }
        Err(e) => {
            let msg_lit = Literal::string(&e.to_string());
            quote_spanned!(proc_macro2::Span::call_site() => compile_error!(#msg_lit)).into()
        }
    }
}

// after
#[proc_macro]
pub fn expression(input: ProcMacroTokenStream) -> ProcMacroTokenStream {
    let input = TokenStream::from(input);
    let mut parser = CELParser::new(OpLookup::new());
    match parser.parse_tokens(input.into_iter()) {
        Ok(_) => ProcMacroTokenStream::new(),
        Err(e) => {
            let msg_lit = Literal::string(e.message());
            quote_spanned!(e.span() => compile_error!(#msg_lit)).into()
        }
    }
}
```

`parse_tokens` handles the empty-input (`Ok(false)`) case internally by calling `error_at("expression expected")`, so there is no longer an `Ok(false)` arm to handle. The `compile_error!` is now attached to the span of the actual offending token rather than `call_site()`.

- [ ] **Step 3: Run full test suite including doc tests**

```
cargo test --workspace
cargo test --doc --workspace
cargo clippy --workspace -- -D warnings
```

Expected: all pass.

- [ ] **Step 4: Commit**

```
git add cel-rs-macros/src/lib.rs
git commit -m "Fix proc-macro diagnostics: use ParseError::span() for compile_error! location"
```
