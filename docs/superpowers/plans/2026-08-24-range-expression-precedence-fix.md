# Range Expression Precedence Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `range_expression`'s precedence to match Rust's own (`..`/`..=` bind looser than `||`, not tighter than comparisons), decouple `expression`'s grammar from "and nothing else follows" so adam-lang's entry points can reach it without an end-of-stream requirement, and route adam-lang through the corrected, range-aware `expression` production instead of the narrower `or_expression`.

**Architecture:** Three sequential, independently-testable changes to code already merged in this same PR (`docs/superpowers/plans/2026-08-24-cel-range-syntax.md`, Task 5): (1) relocate `range_expression` from between `comparison_expression`/`bitwise_or_expression` to sit above `or_expression`, with `or_expression` operands; (2) split `expression`'s "match the grammar" behavior from its "and require end-of-stream" behavior, moving the latter to `parse_tokens_ctx` and a renamed `parse_expression`/`parse_expression_ctx`/`parse_expression_ast` family (renamed from `parse_or_expression*`); (3) point adam-lang's two CEL entry points at the renamed, range-aware `parse_expression`/`parse_expression_ast` instead of the old `parse_or_expression`/`parse_or_expression_ast`.

**Tech Stack:** Rust, `cel-parser`'s hand-written recursive-descent parser, `adam-lang`'s two parsers (`AdamParser` for `Sheet` construction, `AdamAstParser` for the LSP/formatter's AST).

**Spec:** `docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md` (§2, "Range Grammar" — corrected in place; read the corrected version, not history, before starting).

## Global Constraints

- `cargo test --workspace`, `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, and `cargo fmt --all -- --check` must all stay clean after every task.
- Do not touch `is_closure_expression`'s or `tuple_or_group`'s calls to `is_or_expression` — closure bodies and tuple elements keep parsing `or_expression` (not the new range-aware `expression`) in this plan; that's a deliberate scope boundary, not an oversight (see each task's "Out of Scope" note).
- Do not attempt to make adam-lang actually *use* a range value (registering `Range<T>` etc. as adam-lang cell types, wiring filters) — that's the separate, later deduced-filter-args/`FilterKind` plan the spec's §1/§3/§4 describe. This plan only fixes precedence and makes range syntax grammar-reachable from adam-lang's entry points.
- This branch (`sean_parent/cel-range-syntax`) already has PR #144 open against `worktree-sean_parent+adam-filter-range-slider`; these tasks land as further commits on the same branch/PR, not a new one.

---

### Task 1: Fix `range_expression`'s precedence — operands become `or_expression`, sitting above it

**Files:**

- Modify: `cel-parser/src/lib.rs` (`is_comparison_expression`, `is_range_expression`, the crate-level `//!` grammar summary)
- Test: `cel-parser/src/lib.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes: nothing new — `is_or_expression`, `is_bitwise_or_expression`, `self.context.apply_op`, and the six op-table names (`"range"`, `"range_inclusive"`, `"range_from"`, `"range_to"`, `"range_to_inclusive"`, `"range_full"`) all already exist, unchanged, from the prior plan.
- Produces: `is_range_expression` is now called from `is_expression` (not `is_comparison_expression`), and its own internal calls are `is_or_expression` (not `is_bitwise_or_expression`). `is_expression`'s own public signature, behavior, and EOS-checking are unchanged in this task — only what it calls, and therefore what grammar it accepts, changes. Consumed by Task 2, which changes `is_expression`'s EOS behavior next.

- [ ] **Step 1: Write a failing test proving range endpoints now absorb a comparison**

```rust
#[test]
fn range_endpoint_absorbs_a_trailing_comparison_confirming_or_expression_operands() {
    // Confirms range operands are `or_expression`, not `bitwise_or_expression`: in
    // `1i32..5i32 == true`, `5i32 == true` must group together as the range's right
    // endpoint's own `or_expression` and fail *there* (`i32` vs `bool`) — proving the
    // endpoint absorbed the whole comparison, rather than `..` grabbing only `5i32` and
    // `==` applying afterward to an already-built `Range`.
    let mut parser = CELParser::new(OpLookup::new());
    let err = parser.parse_str("1i32..5i32 == true").unwrap_err();
    assert!(
        err.message().starts_with("no operation"),
        "expected a 'no operation `==`' error from inside the range's right endpoint, got: {}",
        err.message()
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p cel-parser range_endpoint_absorbs_a_trailing_comparison_confirming_or_expression_operands`
Expected: FAIL. Today, `is_comparison_expression` calls `is_range_expression` (operands `bitwise_or_expression`) *before* checking for `==`, so `1i32..5i32` builds a `Range<i32>` first, and `==` then applies to `(Range<i32>, bool)` — a different, currently-unverified error shape (`Range<i32>`'s type name may not even format cleanly, since it isn't one of `op_table.rs`'s registered primitive `TYPE_IDS` names) — the test may fail with a different message, a panic, or by not failing to parse at the point this test expects. Note what actually happens in your report; don't guess without running it.

- [ ] **Step 3: Revert `is_comparison_expression` to call `is_bitwise_or_expression` directly**

Replace the current `is_comparison_expression` (which calls `is_range_expression` — the version from the prior, now-superseded PR task) with the pre-range-syntax original:

```rust
/// `comparison_expression = bitwise_or_expression
///     [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].`
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
```

- [ ] **Step 4: Rewrite `is_range_expression` to use `or_expression` operands, and call it from `is_expression` instead**

Replace `is_range_expression`'s doc comment and body (every `self.is_bitwise_or_expression()?` call becomes `self.is_or_expression()?`; the doc comment's grammar and rationale are restated in terms of `or_expression`):

```rust
/// `range_expression = ( or_expression [ ".." [ or_expression ] | "..=" or_expression ] )
///                   | ( ".." [ or_expression ] )
///                   | ( "..=" or_expression ) .`
///
/// Left-factored so every alternative is chosen by one concrete leading token rather
/// than by first deciding whether an optional `or_expression` is present: the three
/// alternatives start with `or_expression`'s own FIRST set, the literal `".."`, or the
/// literal `"..="` respectively — pairwise disjoint (`..`/`..=` can never be the first
/// token of an `or_expression`), so picking among them needs exactly one token, and none
/// of them opens with a bracketed, possibly-empty non-terminal.
///
/// `..`'s right operand is optional wherever it appears (covering, across the three
/// alternatives, `Range`/`RangeFrom`/`RangeTo`/`RangeFull`); `..=`'s right operand is
/// never optional (covering `RangeInclusive`/`RangeToInclusive`) — there is no
/// inclusive-from-only range in Rust, and no such form is registered in the op-table for
/// it to dispatch to, so a bare `..=`, or a left operand followed by `..=` and nothing
/// after, is a parse error, not a valid empty match.
///
/// Operands are `or_expression` — matching Rust's own precedence, where `..`/`..=` bind
/// *looser* than `||` (and everything below it: `&&`, comparisons, bitwise ops,
/// arithmetic). So `1 + 2..3 * 4` still groups as `(1 + 2)..(3 * 4)` (arithmetic is well
/// inside `or_expression`'s own chain), and `a == b..c == d` groups the *whole*
/// comparisons as the two endpoints: `(a == b)..(c == d)`, not `a == (b..c) == d`.
fn is_range_expression(&mut self) -> Result<bool> {
    let start_span = self.peek_span();

    if self.is_punctuation("..=") {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected or_expression"));
        }
        self.context.apply_op(
            &self.op_lookup,
            "range_to_inclusive",
            1,
            start_span.expect("production has token at start"),
            self.last_span,
        )?;
        return Ok(true);
    }

    if self.is_punctuation("..") {
        if self.is_or_expression()? {
            self.context.apply_op(
                &self.op_lookup,
                "range_to",
                1,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
        } else {
            self.context.apply_op(
                &self.op_lookup,
                "range_full",
                0,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
        }
        return Ok(true);
    }

    if self.is_or_expression()? {
        if self.is_punctuation("..=") {
            if !self.is_or_expression()? {
                return Err(self.error_at("expected or_expression"));
            }
            self.context.apply_op(
                &self.op_lookup,
                "range_inclusive",
                2,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
        } else if self.is_punctuation("..") {
            if self.is_or_expression()? {
                self.context.apply_op(
                    &self.op_lookup,
                    "range",
                    2,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            } else {
                self.context.apply_op(
                    &self.op_lookup,
                    "range_from",
                    1,
                    start_span.expect("production has token at start"),
                    self.last_span,
                )?;
            }
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
```

Then change `is_expression` to call it (only the one line inside the `if` changes — its own EOS-checking body is untouched in this task):

```rust
/// `expression = range_expression ?eos?.`
pub fn is_expression(&mut self) -> Result<bool> {
    if !self.is_range_expression()? {
        return Ok(false);
    }
    if self.peek_token().is_some() {
        return Err(self.error_at("unexpected token"));
    }
    Ok(true)
}
```

- [ ] **Step 5: Update the crate-level `//!` grammar summary**

```text
expression = range_expression ?eos?.
range_expression = ( or_expression [ ".." [ or_expression ] | "..=" or_expression ] )
                  | ( ".." [ or_expression ] )
                  | ( "..=" or_expression ) .
or_expression = and_expression { "||" and_expression }.
and_expression = comparison_expression { "&&" comparison_expression }.
comparison_expression = bitwise_or_expression
    [ ("==" | "!=" | "<" | ">" | "<=" | ">=") bitwise_or_expression ].
bitwise_or_expression = bitwise_xor_expression { "|" bitwise_xor_expression }.
```

(everything from `bitwise_xor_expression` downward is unchanged — only the `expression`/`range_expression`/`comparison_expression` lines move/change.)

- [ ] **Step 6: Run the new test, then the full `range`/`comparison`/`chained` regression set**

Run: `cargo test -p cel-parser range_endpoint_absorbs_a_trailing_comparison_confirming_or_expression_operands`
Expected: PASS.

Run: `cargo test -p cel-parser range`
Expected: PASS — this also re-runs the prior plan's nine end-to-end range tests (`range_expression_constructs_a_range`, `range_inclusive_expression_constructs_a_range_inclusive`, `range_from_expression_constructs_a_range_from`, `range_to_expression_constructs_a_range_to`, `range_to_inclusive_expression_constructs_a_range_to_inclusive`, `range_full_expression_constructs_a_range_full`, `range_endpoints_are_full_bitwise_or_expressions`, `chained_ranges_are_a_parse_error`, `range_to_inclusive_without_a_right_operand_is_a_parse_error`) unchanged — they use only literal endpoints, so their expected results don't change, only which production underneath now handles them. If any of these nine fail, that's a regression to fix before continuing, not an expected/acceptable change.

- [ ] **Step 7: Run the full workspace suite, fmt check, and clippy**

Run: `cargo test --workspace`
Expected: PASS, no regressions.

Run: `cargo fmt --all -- --check`
Expected: clean (or run `cargo fmt --all` and include the diff in this commit).

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "fix(cel-parser): range_expression binds looser than or_expression, matching Rust"
```

---

### Task 2: Decouple `expression`'s grammar from end-of-stream

**Files:**

- Modify: `cel-parser/src/lib.rs` (`is_expression`, `parse_tokens_ctx`, `parse_or_expression_ctx`/`parse_or_expression`/`parse_or_expression_ast` → renamed, the two crate-doc examples, the crate-level `//!` grammar summary's `# Note` section)
- Test: `cel-parser/src/lib.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes: `is_range_expression` (Task 1, unchanged in this task).
- Produces: `is_expression(&mut self) -> Result<bool>` now checks only the grammar, no end-of-stream. `parse_expression_ctx(&mut self) -> Result<C>` (renamed from `parse_or_expression_ctx`), `parse_expression(&mut self) -> Result<DynSegment>` (renamed from `parse_or_expression`, on `impl Parser<DynSegmentContext>`), and `parse_expression_ast(&mut self) -> Result<Expr>` (renamed from `parse_or_expression_ast`, on `impl Parser<AstContext>`) are the new no-EOS, range-aware, top-level entry points — consumed by Task 3.

**Out of scope:** `parse_or_expression_ctx`/`parse_or_expression`/`parse_or_expression_ast` are *renamed*, not duplicated — after this task nothing in the codebase parses a bare, non-range-aware `or_expression` as a standalone top-level production anymore (closures and tuple elements still call `is_or_expression` directly mid-grammar, unaffected — see Task 1's constraints).

- [ ] **Step 1: Write failing tests for the renamed `parse_expression` family**

These simply rename two existing tests (currently named after `parse_or_expression`) to their new names, keeping the same bodies — write them under the new names first so Step 2 shows they fail only because the new names don't exist yet, then delete the old-named versions in Step 4:

```rust
#[test]
fn parse_expression_stops_before_comma() -> anyhow::Result<()> {
    use lex_lexer::LexLexer;
    let stream: proc_macro2::TokenStream = "10i32 + 20i32, 5i32".parse().unwrap();
    let mut parser = CELParser::new(OpLookup::new());
    parser.set_lex_tokens(LexLexer::new(stream.into_iter()).peekable());
    let mut seg = parser
        .parse_expression()
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let result: i32 = seg.call0()?;
    assert_eq!(result, 30);
    let remaining: Vec<_> = parser.take_lex_tokens().expect("tokens present").collect();
    // The comma and "5i32" should remain unconsumed.
    assert_eq!(
        remaining.len(),
        2,
        "expected 2 remaining tokens (comma and 5i32)"
    );
    Ok(())
}

#[test]
fn parse_expression_on_empty_input_returns_error() {
    use lex_lexer::LexLexer;
    let stream: proc_macro2::TokenStream = "".parse().unwrap();
    let mut parser = CELParser::new(OpLookup::new());
    parser.set_lex_tokens(LexLexer::new(stream.into_iter()).peekable());
    let result = parser.parse_expression();
    assert!(result.is_err(), "expected Err for empty input");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p cel-parser parse_expression_stops_before_comma parse_expression_on_empty_input_returns_error`
Expected: FAIL to compile — `parse_expression` doesn't exist yet (only `parse_or_expression` does).

- [ ] **Step 3: Split EOS out of `is_expression`, add it to `parse_tokens_ctx`, and rename the `parse_or_expression*` family**

```rust
/// `expression = range_expression.`
pub fn is_expression(&mut self) -> Result<bool> {
    self.is_range_expression()
}
```

Find `parse_or_expression_ctx` (in the generic `impl<C: ParserContext> Parser<C>` block) and rename it, updating its body and doc comment:

```rust
/// Parses one `expression` from the current token stream and returns the built context.
///
/// Unlike [`parse_str_ctx`](Self::parse_str_ctx), this method does not require
/// end-of-stream, allowing adam-lang to parse an expression embedded within a larger token
/// stream.
///
/// # Errors
///
/// Returns an error if the input does not contain a valid `expression`.
///
/// - Complexity: O(n) in the number of tokens in the expression.
pub fn parse_expression_ctx(&mut self) -> Result<C> {
    if !self.is_expression()? {
        return Err(self.error_at("expression expected"));
    }
    Ok(std::mem::replace(&mut self.context, C::new_context()))
}
```

Find `parse_or_expression` (in `impl Parser<DynSegmentContext>`) and rename it:

```rust
pub fn parse_expression(&mut self) -> Result<DynSegment> {
    self.parse_expression_ctx()
        .map(DynSegmentContext::into_inner)
}
```

Find `parse_or_expression_ast` (in `impl Parser<AstContext>`) and rename it:

```rust
/// Parses one `expression` from the current token stream and returns the built [`Expr`].
///
/// Unlike [`parse_str_ast`](Self::parse_str_ast), this method does not require
/// end-of-stream, allowing adam-lang to parse an expression embedded within a larger token
/// stream.
///
/// # Errors
///
/// Returns an error if the input does not contain a valid `expression`.
///
/// - Complexity: O(n) in the number of tokens in the expression.
pub fn parse_expression_ast(&mut self) -> Result<Expr> {
    self.parse_expression_ctx().map(AstContext::into_expr)
}
```

Add the end-of-stream check directly into `parse_tokens_ctx` (this is what used to live inside `is_expression`):

```rust
pub fn parse_tokens_ctx(&mut self, tokens: TokenStreamIter) -> Result<C> {
    self.set_tokens(tokens);
    if !self.is_expression()? {
        return Err(self.error_at("expression expected"));
    }
    if self.peek_token().is_some() {
        return Err(self.error_at("unexpected token"));
    }
    Ok(std::mem::replace(&mut self.context, C::new_context()))
}
```

- [ ] **Step 4: Rename the two existing `parse_or_expression*`-named tests' old versions away**

Delete the pre-existing `parse_or_expression_stops_before_comma` and `parse_or_expression_on_empty_input_returns_error` test functions entirely (their bodies are now duplicated, under the new names, from Step 1) — don't leave both old and new versions testing the same renamed method under two names.

- [ ] **Step 5: Update the two crate-doc examples to use `parse_tokens` instead of `set_tokens` + `is_expression`**

`is_expression()` no longer enforces end-of-stream, so these two examples (one in the crate-level `//!` doc block near the top of the file, one on the `Parser` struct's own `///` doc block — both demonstrate the exact same thing) must switch to `parse_tokens`, which now carries that behavior:

```rust
//! let input = TokenStream::from_str("10").unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//! let result = parser.parse_tokens(input.into_iter());
//! assert!(result.is_ok());
```

```rust
//! let input = TokenStream::from_str(source).unwrap();
//! let mut parser = CELParser::new(OpLookup::new());
//!
//! if let Err(e) = parser.parse_tokens(input.into_iter()) {
//!     // Format error starting at line 1
//!     println!("{}", e.format_rustc_style(source, file!(), line, &Renderer::plain()));
//! }
```

Apply the identical change to both copies (drop the `parser.set_tokens(input.into_iter());` line, and replace `parser.is_expression()` with `parser.parse_tokens(input.into_iter())`).

- [ ] **Step 6: Update the crate-level grammar summary's `# Note` section**

Replace:

```text
# Note

`?eos?` denotes end of stream.
```

with:

```text
# Note

`?eos?` denotes end of stream. `expression` above is the bare grammar production and does
not by itself require end-of-stream — [`Parser::is_expression`] only checks the grammar.
[`Parser::parse_tokens_ctx`] (and the `parse_tokens`/`parse_str`/`parse_tokens_ast`/
`parse_str_ast` convenience wrappers built on it) additionally require end-of-stream, for
parsing a whole, self-contained token stream (e.g. `cel-rs-macros`'s `expression!`
proc-macro, which must reject a macro body with anything left over).
[`Parser::parse_expression_ctx`] (and its `parse_expression`/`parse_expression_ast`
wrappers) do not, for parsing an expression embedded in a larger token stream — this is
what adam-lang's entry points use.
```

- [ ] **Step 7: Run the renamed tests, then the full workspace suite, fmt check, and clippy**

Run: `cargo test -p cel-parser parse_expression_stops_before_comma parse_expression_on_empty_input_returns_error`
Expected: PASS.

Run: `cargo test --doc -p cel-parser`
Expected: PASS — confirms the two rewritten doc examples still compile and run correctly.

Run: `cargo test --workspace`
Expected: PASS, no regressions (in particular, `incomplete_expression` and the other `parse_str`-based "unexpected token" tests must still pass — they now get that behavior from `parse_tokens_ctx`'s own check instead of `is_expression`'s, with identical observable behavior).

Run: `cargo fmt --all -- --check`
Expected: clean.

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "refactor(cel-parser): decouple expression's grammar from end-of-stream"
```

---

### Task 3: adam-lang routes its CEL entry points through the range-aware `expression`

**Files:**

- Modify: `adam-lang/src/parser.rs` (`parse_cel_or_expression` → renamed, 4 call sites)
- Modify: `adam-lang/src/ast_parser.rs` (`parse_cel_or_expression` → renamed, 7 call sites)
- Test: `adam-lang/src/parser.rs`, `adam-lang/src/ast_parser.rs` (existing `#[cfg(test)] mod tests` in each)

**Interfaces:**

- Consumes: `Parser::<DynSegmentContext>::parse_expression(&mut self) -> Result<DynSegment>` and `Parser::<AstContext>::parse_expression_ast(&mut self) -> Result<cel_parser::Expr>` (Task 2).
- Produces: `AdamParser::parse_cel_expression` (private, `adam-lang/src/parser.rs`) and `AdamAstParser::parse_cel_expression` (private, `adam-lang/src/ast_parser.rs`) — same signatures as the `parse_cel_or_expression` methods they replace, just renamed and delegating to the new CEL-parser methods. No later task depends on these; this is the plan's last task.

**Out of scope:** making adam-lang actually *use* a parsed range value (a cell of type `Range<i32>`, a filter clamping to `lo..=hi`) is not this task's job — `Range<T>` etc. aren't registered adam-lang types, so a range-producing expression still fails, cleanly, at semantic type-inference (see Step 1's second test) exactly as any other unregistered-type expression already does. That failure is expected and correct for this plan; making it *succeed* is the later filter/`FilterKind` plan's job.

- [ ] **Step 1: Write two failing tests — one on each parser**

In `adam-lang/src/ast_parser.rs`'s test module, add (mirroring the existing `sheet s { relationship { x := a; } }`-style tests already in that file):

```rust
#[test]
fn range_syntax_is_reachable_from_a_relationship_binding() {
    let sheet = AdamAstParser::new()
        .parse_str("sheet s { relationship { x := 1..5; } }")
        .unwrap();
    let ast::SheetItem::Relationship(rel) = &sheet.items[0] else {
        panic!("expected a Relationship item, got {:?}", sheet.items[0]);
    };
    assert!(
        matches!(&rel.bindings[0].body, cel_parser::Expr::Op { name, .. } if name == "range"),
        "expected the binding body to be a range Expr::Op, got {:?}",
        rel.bindings[0].body
    );
}
```

In `adam-lang/src/parser.rs`'s test module, add (using the existing `parser()` helper already defined there):

```rust
#[test]
fn parse_cell_range_initializer_fails_cleanly_at_type_inference_not_grammar() {
    // `Range<i32>` isn't a registered adam-lang type — this must still fail today, but only
    // at `eval_segment_boxed`'s existing "cannot infer a type" check, proving the CEL-level
    // range parsing itself succeeded (rather than failing as "unexpected token" or similar
    // at the grammar level, which would indicate the entry-point swap didn't take effect).
    let result = parser().parse_str("sheet s { cell x = 1i32..5i32; }");
    assert!(result.is_err());
    let err = result.err().expect("expected Err");
    assert_eq!(
        err.message(),
        "cannot infer a type for this expression; register a type name for it or add an \
         explicit `: type_expr` annotation"
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p adam-lang range_syntax_is_reachable_from_a_relationship_binding parse_cell_range_initializer_fails_cleanly_at_type_inference_not_grammar`
Expected: FAIL. Today, both parsers call `parse_or_expression`/`parse_or_expression_ast`, which parse only `or_expression` — `1..5`'s leading `1` parses fine as a bare `or_expression`, but the following `..5` is then unconsumed, left as a "remaining token" the caller's own subsequent parsing (expecting `;` or `}`) rejects with a different error than either test expects. Note the actual failure in your report.

- [ ] **Step 3: Rename `parse_cel_or_expression` to `parse_cel_expression` in both files**

In `adam-lang/src/parser.rs`:

```rust
/// Delegates one `expression` to CELParser, sharing the token stream.
fn parse_cel_expression(&mut self, ctx: &mut ParseContext) -> Result<DynSegment> {
    let tokens = ctx.cursor.take_tokens().expect("tokens present");
    self.cel.set_lex_tokens(tokens);
    let result = self.cel.parse_expression();
    ctx.cursor
        .set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
    result
}
```

Update all four call sites in that file — each is the exact text `self.parse_cel_or_expression(ctx)`, becoming `self.parse_cel_expression(ctx)`, at (as of this writing) lines 228, 310, 681, and 819. Search the file for `parse_cel_or_expression` afterward to confirm zero remaining occurrences besides the renamed definition itself.

In `adam-lang/src/ast_parser.rs`:

```rust
/// Delegates one `expression` to `cel_parser::Parser<AstContext>`, sharing the token
/// stream (the same take/set-tokens handoff `crate::AdamParser` uses for the `DynSegment`
/// path).
fn parse_cel_expression(&mut self, cursor: &mut TokenCursor) -> Result<cel_parser::Expr> {
    let tokens = cursor.take_tokens().expect("tokens present");
    self.cel.set_lex_tokens(tokens);
    let result = self.cel.parse_expression_ast();
    cursor.set_tokens(self.cel.take_lex_tokens().expect("tokens set"));
    result
}
```

Update all seven call sites in that file the same way — each is the exact text
`self.parse_cel_or_expression(cursor)`, becoming `self.parse_cel_expression(cursor)`, at
(as of this writing) lines 191, 197, 246, 351, 371, 464, and 507. Search the file for
`parse_cel_or_expression` afterward to confirm zero remaining occurrences besides the
renamed definition itself.

- [ ] **Step 4: Run the two new tests, then the full adam-lang and workspace suites**

Run: `cargo test -p adam-lang range_syntax_is_reachable_from_a_relationship_binding parse_cell_range_initializer_fails_cleanly_at_type_inference_not_grammar`
Expected: PASS.

Run: `cargo test -p adam-lang`
Expected: PASS — every existing adam-lang test parses ordinary (non-range) expressions identically through `expression` as it did through `or_expression`, since `range_expression`'s fallback case (no `..`/`..=` present) is exactly `or_expression`.

Run: `cargo test --workspace`
Expected: PASS, no regressions (also re-checks `adam-lsp` and `begin`, which depend on `adam-lang`).

Run: `cargo fmt --all -- --check`
Expected: clean.

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add adam-lang/src/parser.rs adam-lang/src/ast_parser.rs
git commit -m "refactor(adam-lang): route CEL entry points through range-aware expression"
```
