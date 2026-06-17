# Op-Lookup Error Spans Design

**Date:** 2026-06-17
**Status:** Approved

## Goal

Replace `error_at()` at all op-lookup failure sites with expression-scoped spans: from the first token of the production to the last token consumed before the lookup. For example, `"Hello" + 32.0 + "World"` should report:

```
error: operation error: Operation '+' not found for types [alloc::string::String, f64]
 --> test.cel:1:1
  |
1 | "Hello" + 32.0 + "World"
  | ^^^^^^^^^^^^^^
```

The same span will later be bound to the operation in `DynSegment` for runtime error reporting (out of scope for this change).

## Background

The parser currently calls `error_at()` at every op-lookup failure. `error_at()` points to the *next unconsumed token*, which is wrong for type mismatch errors — it implicates the wrong token rather than the expression that produced the mismatched types.

`ParseError` already carries a `proc_macro2::Span`, so the infrastructure for precise spans exists. The only missing pieces are (1) tracking the span of the last consumed token and (2) passing the production span into `lookup()`.

## Design

### 1. `CELParser` — span tracking (`parser/mod.rs`)

#### New field

```rust
last_span: Span,  // span of the most recently consumed token
```

Initialized to `Span::call_site()` in `CELParser::new()` and reset to `Span::call_site()` in `set_tokens()`. Only used after at least one token has been consumed.

#### `advance()` — extracts span from consumed token

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

`advance()` is always called after `peek_token()` has confirmed a token exists, so `expect("token required to advance")` is a valid assertion.

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

Returns `Option<Span>` rather than a fallback value: a caller that needs the span must be able to assert it exists via `expect()`.

#### Production entry and lookup sites

At the entry of every production that calls `op_lookup.lookup()`:

```rust
let start_span = self.peek_span();
```

At each lookup call, replacing the old `if ... let Err(e) = ... error_at(...)` pattern:

```rust
let expr_span = start_span.expect("production has token at start")
    .join(self.last_span)
    .expect("all tokens in a CEL expression are from the same source");
self.op_lookup.lookup(op_name, &mut self.context, arity, expr_span)?;
```

`start_span.expect()` is safe because the lookup is only reached after a successful sub-production parse, which guarantees a token existed at entry. `join().expect()` is safe because all tokens originate from a single `TokenStream`.

The `if self.context.stack_ids.len() >= N` guards are removed: they were artifacts of the `if let Err` pattern and are structurally always true when lookup is reached (operands were just parsed).

**Affected productions:** `is_or_expression`, `is_and_expression`, `is_comparison_expression`, `is_bitwise_or_expression`, `is_bitwise_xor_expression`, `is_bitwise_and_expression`, `is_bitwise_shift_expression`, `is_additive_expression`, `is_multiplicative_expression`, `is_unary_expression`, `is_postfix_expression`.

**`error_at()` is unchanged** — it continues to serve syntax errors (unexpected token, missing RHS, unmatched parenthesis) where the current token position is the correct diagnostic site.

#### Span semantics per production class

| Production | `start_span` | `last_span` at lookup | Error spans |
|---|---|---|---|
| Binary loop (`+`, `\|\|`, etc.) | First token of LHS | Last token of RHS | `lhs op rhs` (and accumulates across iterations) |
| Comparison (`==`, `<`, etc.) | First token of LHS | Last token of RHS | `lhs op rhs` |
| Unary (`-`, `!`) | Operator token | Last token of operand | `-expr` or `!expr` |
| Postfix call (`()`) | Identifier token | Closing `)` | `f(args)` |

For chained binary ops (`a + b + c`) where the first succeeds and second fails, `start_span` is still the start of the first operand (`a`), so the error spans the full `a + b + c`. This correctly shows all components that produced the mismatched types.

### 2. `OpLookup::lookup()` — new signature (`parser/op_table.rs`)

```rust
/// Looks up and applies an operation, binding the expression span to the registered op.
///
/// Searches scopes in LIFO order, then falls back to built-in operations.
///
/// # Errors
///
/// Returns a [`ParseError`] carrying `span` if no scope or built-in handles the request,
/// or if a scope itself returns an error.
///
/// - Complexity: O(k) in the number of registered scopes, plus O(s) for built-in
///   signature scan where s is the number of signatures for the operator.
pub fn lookup(
    &self,
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    span: proc_macro2::Span,
) -> std::result::Result<(), ParseError>
```

`BuiltinScope::lookup()` and `ScopeFn` keep their `anyhow::Result` return types. The conversion to `ParseError` occurs only at the `OpLookup::lookup()` boundary:

```rust
// Scope returns error:
Err(e) => return Err(ParseError::new(format!("operation error: {}", e), span))

// No match found:
Err(ParseError::new(
    format!("operation error: Operation '{}' not found for types [{}]", name, type_names),
    span,
))
```

`use super::ParseError` is added to `op_table.rs`.

### 3. Error message consolidation

The `"call: "` prefix used in `is_postfix_expression` is replaced by `"operation error: "` for consistency, since `lookup()` now constructs the `ParseError` directly and context-specific prefixes would require re-wrapping. The `call_undefined_call_op` test assertion is updated to check for `"operation error: "`.

## Test changes

### `parser/op_table.rs`

- All `lookup(name, segment, arity)` calls gain `Span::call_site()` as a fourth argument.
- Test functions returning `anyhow::Result<()>`: in non-proc-macro (fallback) mode, `proc_macro2::Span` is `Send + Sync + 'static`, so `ParseError` satisfies `anyhow::Error`'s `From<E: Error + Send + Sync + 'static>` bound and `?` continues to work unchanged.

### `parser/mod.rs`

- `call_undefined_call_op`: update `starts_with("call:")` assertion to `starts_with("operation error:")`.
- New tests verifying that op-lookup failure spans cover the full expression, not just the operator token. For example: a test parsing `"Hello" + 32.0` that fails, then asserts the error span covers both the string and float literals.

## Non-goals

- Storing the expression span on the `DynSegment` operation for runtime error reporting is deferred to a follow-on task. The `span` parameter passed to `lookup()` is used for error messages on failure; on success it is not yet forwarded to `DynSegment`.
- No changes to `error_at()` or syntax-error spans.
- No changes to the `ScopeFn` signature.
