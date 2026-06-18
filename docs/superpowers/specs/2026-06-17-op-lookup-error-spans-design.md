# Op-Lookup Error Spans Design

**Date:** 2026-06-17
**Status:** Approved (revised)

## Goal

Replace `error_at()` at all op-lookup failure sites with expression-scoped spans: from the first token of the production to the last token consumed before the lookup. For example, `"Hello" + 32.0 + "World"` should report:

At runtime (annotate-snippets, contiguous underline):
```
error: operation error: Operation '+' not found for types [alloc::string::String, f64]
 --> test.cel:1:1
  |
1 | "Hello" + 32.0 + "World"
  | ^^^^^^^^^^^^^^
```

At compile time (two `compile_error!()` diagnostics, stable Rust):
```
error: operation error: Operation '+' not found for types [alloc::string::String, f64]
 --> src/main.rs:3:5
  |
3 |     expression!("Hello" + 32.0)
  |                  ^^^^^^^
error: expression continues here
 --> src/main.rs:3:16
  |
3 |     expression!("Hello" + 32.0)
  |                           ^^^^
```

The same spans will later be bound to the operation in `DynSegment` for runtime error reporting (out of scope for this change).

## Background

The parser currently calls `error_at()` at every op-lookup failure. `error_at()` points to the *next unconsumed token*, which is wrong for type mismatch errors — it implicates the wrong token rather than the expression that produced the mismatched types.

`ParseError` already carries a `proc_macro2::Span`. The missing pieces are: (1) tracking the span of the last consumed token, (2) a second constructor on `ParseError` for expression-range errors, and (3) passing both the start and end spans into `lookup()`.

`proc_macro::Span::join()` is not yet stable (tracking issue [#54725](https://github.com/rust-lang/rust/issues/54725)), so joining spans is not available on stable Rust. The design avoids `join()` entirely by storing both spans separately and combining them at the reporting layer.

## Design

### 1. `ParseError` — new constructor (`parser/error.rs`)

Add an `end_span: Option<proc_macro2::Span>` field and a `new_range` constructor:

```rust
pub struct ParseError {
    message: String,
    span: proc_macro2::Span,          // start of expression (or sole span)
    end_span: Option<proc_macro2::Span>, // end of expression, if range error
}

impl ParseError {
    // existing — unchanged
    pub fn new(message: impl Into<String>, span: proc_macro2::Span) -> Self

    // new — for op-lookup failures that span a full sub-expression
    pub fn new_range(
        message: impl Into<String>,
        start: proc_macro2::Span,
        end: proc_macro2::Span,
    ) -> Self

    // existing — unchanged
    pub fn message(&self) -> &str
    pub fn span(&self) -> proc_macro2::Span

    // new accessor
    pub fn end_span(&self) -> Option<proc_macro2::Span>
}
```

`format_rustc_style` on `ParseError` merges the two spans into one `SourceSpan` (start from `self.span.start()`, end from `self.end_span.unwrap_or(self.span).end()`) and renders a single annotation.

### 2. `CELError` / `SourceSpan` — runtime conversion (`parser/error.rs`)

`SourceSpan` is unchanged (it already stores `start: LineColumn` and `end: LineColumn`).

`From<ParseError> for CELError` is updated to merge the two spans into one `SourceSpan`:

```rust
impl From<ParseError> for CELError {
    fn from(e: ParseError) -> Self {
        let source_span = SourceSpan {
            start: e.span.start(),
            end: e.end_span.unwrap_or(e.span).end(),
        };
        CELError::new(e.message, source_span)
    }
}
```

This produces a contiguous underline from the expression start to the expression end without needing `Span::join()`.

### 3. `OpLookup::lookup()` — new signature (`parser/op_table.rs`)

```rust
/// Looks up and applies an operation, attaching expression span to any error.
///
/// Searches scopes in LIFO order, then falls back to built-in operations.
///
/// # Errors
///
/// Returns a [`ParseError`] spanning `start..=end` if no scope or built-in
/// handles the request, or if a scope itself returns an error.
///
/// - Complexity: O(k) in the number of registered scopes, plus O(s) for built-in
///   signature scan where s is the number of signatures for the operator.
pub fn lookup(
    &self,
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    start: proc_macro2::Span,
    end: proc_macro2::Span,
) -> std::result::Result<(), super::ParseError>
```

`BuiltinScope` and `ScopeFn` keep their `anyhow::Result` return types. Conversion to `ParseError` occurs only at the `OpLookup::lookup()` boundary:

```rust
// Scope returns error:
Err(e) => return Err(ParseError::new_range(format!("operation error: {}", e), start, end))

// No match found:
Err(ParseError::new_range(
    format!("operation error: Operation '{}' not found for types [{}]", name, type_names),
    start,
    end,
))
```

### 4. `CELParser` — span tracking (`parser/mod.rs`)

#### New field

```rust
last_span: Span,  // span of the most recently consumed token
```

Initialized to `Span::call_site()` in `CELParser::new()` and reset to `Span::call_site()` in `set_tokens()`. Only meaningful after at least one token has been consumed.

#### `advance()` — records span of consumed token

```rust
/// Advances past the current token, recording its span in `last_span`.
///
/// # Panics
///
/// Panics if no token stream has been set or if there is no current token.
fn advance(&mut self) {
    use lex_lexer::HasSpan;
    self.last_span = self.tokens.as_mut().expect("tokens set")
        .next().expect("token required to advance").span();
}
```

#### `peek_span()` — returns `Option<Span>`

```rust
/// Returns the span of the next token without consuming it, or `None` if exhausted.
fn peek_span(&mut self) -> Option<Span> {
    self.peek_token().map(|token| {
        use lex_lexer::HasSpan;
        token.span()
    })
}
```

#### Production entry and lookup sites

At the entry of every production that calls `op_lookup.lookup()`:

```rust
let start_span = self.peek_span();
```

At each lookup call, replacing the old error pattern:

```rust
let start = start_span.expect("production has token at start");
self.op_lookup.lookup(op_name, &mut self.context, arity, start, self.last_span)?;
```

`start_span.expect()` is safe because the lookup is only reached after a successful sub-production parse, which guarantees a token existed at entry.

No `Span::join()` is called anywhere.

**Affected productions:** `is_or_expression`, `is_and_expression`, `is_comparison_expression`, `is_bitwise_or_expression`, `is_bitwise_xor_expression`, `is_bitwise_and_expression`, `is_bitwise_shift_expression`, `is_additive_expression`, `is_multiplicative_expression`, `is_unary_expression`, `is_postfix_expression`.

**`error_at()` is unchanged** — it continues to serve syntax errors (unexpected token, missing RHS, unmatched parenthesis).

#### Span semantics per production class

| Production | `start_span` | `self.last_span` at lookup | Error spans |
|---|---|---|---|
| Binary loop (`+`, `\|\|`, etc.) | First token of LHS | Last token of RHS | `lhs op rhs` (accumulates across iterations) |
| Comparison (`==`, `<`, etc.) | First token of LHS | Last token of RHS | `lhs op rhs` |
| Unary (`-`, `!`) | Operator token | Last token of operand | `-expr` or `!expr` |
| Postfix call (`()`) | Identifier token | Closing `)` | `f(args)` |

For chained binary ops (`a + b + c`) where the first succeeds and second fails, `start_span` is still the start of the first operand (`a`), so the start span correctly covers from `a`. The end span is the last token of `c`.

### 5. `cel-rs-macros` — compile-time error emission (`cel-rs-macros/src/lib.rs`)

When `ParseError` has only `span` (single-span errors like "Undefined identifier"), emit one `compile_error!()` as before.

When `ParseError` has both `span` and `end_span` (range errors from `lookup()`), emit two `compile_error!()` items:

```rust
let msg = e.message();
let start = e.span();
let mut tokens = quote_spanned! { start => compile_error!(#msg) };
if let Some(end) = e.end_span() {
    tokens.extend(quote_spanned! { end => compile_error!("expression continues here") });
}
tokens
```

## Error message consolidation

The old `"call: "` prefix in `is_postfix_expression` is replaced by `"operation error: "` for consistency, since `lookup()` now constructs the `ParseError` directly.

The `call_undefined_call_op` test assertion is updated from `starts_with("call:")` to `starts_with("operation error:")`.

## Current code state

The branch `sean-parent/parser-actions` currently has partial implementation that used `Span::join()` and `unwrap_or` fallbacks. All those commits need to be replaced by this design. The implementation plan starts from commit `811df34`.

## Test changes

### `parser/error.rs`

- Tests for `ParseError::new_range`: verify `span()` returns start, `end_span()` returns `Some(end)`.
- Test for `From<ParseError> for CELError` with a range error: verify `CELError::span()` covers from start to end.

### `parser/op_table.rs`

- All existing `lookup(name, segment, arity, span)` calls gain a second span argument: `lookup(name, segment, arity, Span::call_site(), Span::call_site())`.
- New test: `lookup_not_found_error_has_range` — verifies `lookup()` failure returns a `ParseError` whose `end_span()` is `Some`.

### `parser/mod.rs`

- `call_undefined_call_op`: update assertion from `starts_with("call:")` to `starts_with("operation error:")`.
- New test `op_type_mismatch_error_spans_full_expression`: parse `"Hello" + 32.0`, assert error `end_span()` is `Some` and its `end().column` covers the end of `32.0`.

## Non-goals

- Storing the expression span on the `DynSegment` operation for runtime error reporting is deferred to a follow-on task.
- No changes to `error_at()` or syntax-error spans.
- No changes to the `ScopeFn` signature.
- `proc_macro::Span::join()` is not used anywhere; this design is fully stable.
