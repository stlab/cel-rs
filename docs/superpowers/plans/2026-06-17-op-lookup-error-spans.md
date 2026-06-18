# Op-Lookup Error Spans Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give op-lookup errors expression-scoped spans by storing start and end spans separately in `ParseError`, eliminating `Span::join()` entirely (it is not yet stable).

**Architecture:** Three sequential changes: (1) `ParseError` gains an optional `end_span` field and a `new_range()` constructor; (2) `OpLookup::lookup()` gains `start`/`end` span parameters and uses `new_range()` — all call sites in `op_table.rs` and `mod.rs` updated atomically; (3) `cel-rs-macros` emits a second `compile_error!()` pointing to the expression end when `end_span` is `Some`.

**Tech Stack:** Rust stable, `proc_macro2`, `annotate-snippets`, `anyhow`, `quote`

**Starting state:** Branch `sean-parent/parser-actions` at commit `afe6581`. `CELParser` already has `last_span: Span`, `advance()` updating it, and `peek_span() -> Option<Span>`. Productions already capture `start_span = self.peek_span()`. The `join_spans()` helper (lines 111-113 of `mod.rs`) will be removed in Task 2.

## Global Constraints

- All public doc comments follow contract style: `# Panics` / `# Errors` / `# Safety` for preconditions; `/// - Complexity:` bullet when not O(1); `# Examples` for public APIs.
- `cargo clippy --workspace -- -D warnings` must pass after every commit.
- `cargo test --workspace` and `cargo test --doc --workspace` must pass after every commit.
- No `Span::join()` anywhere — `proc_macro::Span::join()` is not yet stable.
- TDD: write the failing test first, confirm it fails, then implement.

---

### Task 1: Add two-span support to `ParseError`

**Files:**
- Modify: `cel-runtime/src/parser/error.rs`

**Interfaces:**
- Produces:
  - `ParseError::new_range(message: impl Into<String>, start: proc_macro2::Span, end: proc_macro2::Span) -> Self`
  - `ParseError::end_span(&self) -> Option<proc_macro2::Span>`
  - `ParseError::new()` unchanged; `ParseError::span()` returns start span in all cases.
  - `From<ParseError> for CELError` merges both spans into `SourceSpan { start: e.span.start(), end: e.end_span.unwrap_or(e.span).end() }`.
  - `ParseError::format_rustc_style()` uses the merged span for the byte-range annotation.

---

- [ ] **Step 1: Write the failing tests**

  In the `#[cfg(test)] mod tests` block at the bottom of `cel-runtime/src/parser/error.rs`, add after the existing tests:

  ```rust
  #[test]
  fn parse_error_new_range_has_end_span() {
      let start = Span::call_site();
      let end = Span::call_site();
      let e = ParseError::new_range("type mismatch", start, end);
      assert_eq!(e.message(), "type mismatch");
      assert!(e.end_span().is_some());
  }

  #[test]
  fn parse_error_new_range_cel_error_merges_spans() {
      let start = Span::call_site();
      let end = Span::call_site();
      let e = ParseError::new_range("type mismatch", start, end);
      let cel: CELError = e.into();
      assert_eq!(cel.message(), "type mismatch");
  }

  #[test]
  fn parse_error_new_has_no_end_span() {
      let e = ParseError::new("bad token", Span::call_site());
      assert!(e.end_span().is_none());
  }
  ```

- [ ] **Step 2: Confirm the tests fail**

  ```
  cargo test --workspace parse_error_new_range
  cargo test --workspace parse_error_new_has_no_end_span
  ```

  Expected: compile error — `new_range` and `end_span` not found.

- [ ] **Step 3: Add `end_span` field and update the struct**

  Replace the `ParseError` struct definition and the opening of its `impl` block. Current:

  ```rust
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
  ```

  Replace with:

  ```rust
  /// A parse error carrying the original `proc_macro2::Span` of the offending token or
  /// expression start, plus an optional end span for range errors.
  ///
  /// Used as the return type of all parser methods. Not `Send + Sync` because
  /// `proc_macro2::Span` wraps a compiler-internal handle that is only valid on
  /// the proc-macro thread. Convert to [`CELError`] via `From` when the error
  /// must cross thread boundaries or be stored for async reporting.
  #[derive(Clone, Debug)]
  pub struct ParseError {
      message: String,
      span: proc_macro2::Span,
      end_span: Option<proc_macro2::Span>,
  }

  impl ParseError {
      /// Creates a new parse error with the given message and token span.
      pub fn new(message: impl Into<String>, span: proc_macro2::Span) -> Self {
          ParseError {
              message: message.into(),
              span,
              end_span: None,
          }
      }

      /// Creates a parse error spanning a sub-expression from `start` to `end`.
      ///
      /// Use this for op-lookup failures where the error implicates a full
      /// sub-expression rather than a single token. At runtime the two spans
      /// are merged into a contiguous underline; at compile time two separate
      /// `compile_error!()` diagnostics are emitted.
      pub fn new_range(
          message: impl Into<String>,
          start: proc_macro2::Span,
          end: proc_macro2::Span,
      ) -> Self {
          ParseError {
              message: message.into(),
              span: start,
              end_span: Some(end),
          }
      }
  ```

- [ ] **Step 4: Add `end_span()` accessor**

  After the existing `span()` method, add:

  ```rust
  /// Returns the end span of this error, or `None` for single-point errors.
  ///
  /// `Some` for errors created with [`new_range`](Self::new_range);
  /// `None` for errors created with [`new`](Self::new).
  pub fn end_span(&self) -> Option<proc_macro2::Span> {
      self.end_span
  }
  ```

- [ ] **Step 5: Update `ParseError::format_rustc_style()` to use merged span**

  Replace:

  ```rust
      let source_span = SourceSpan::from_proc_macro2(self.span);
      let byte_range = span_to_byte_range(source_code, source_span);
  ```

  With:

  ```rust
      let source_span = SourceSpan {
          start: self.span.start(),
          end: self.end_span.unwrap_or(self.span).end(),
      };
      let byte_range = span_to_byte_range(source_code, source_span);
  ```

- [ ] **Step 6: Update `From<ParseError> for CELError` to merge spans**

  Replace:

  ```rust
  impl From<ParseError> for CELError {
      fn from(e: ParseError) -> Self {
          CELError::new(e.message, SourceSpan::from_proc_macro2(e.span))
      }
  }
  ```

  With:

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

- [ ] **Step 7: Run tests and clippy**

  ```
  cargo test --workspace
  cargo test --doc --workspace
  cargo clippy --workspace -- -D warnings
  ```

  Expected: all pass.

- [ ] **Step 8: Commit**

  ```
  git add cel-runtime/src/parser/error.rs
  git commit -m "Add ParseError::new_range() and end_span field for expression-range errors"
  ```

---

### Task 2: Update `OpLookup::lookup()` to two spans and fix all call sites

**Files:**
- Modify: `cel-runtime/src/parser/op_table.rs`
- Modify: `cel-runtime/src/parser/mod.rs`

Both files must be committed together — the signature change and call-site updates are mutually required for compilation.

**Interfaces:**
- Consumes: `ParseError::new_range()` from Task 1.
- Produces:
  - `OpLookup::lookup(&self, name: &str, segment: &mut DynSegment, num_operands: usize, start: proc_macro2::Span, end: proc_macro2::Span) -> std::result::Result<(), super::ParseError>`
  - All productions pass `(start_span.expect("..."), self.last_span)` as the two span args.
  - `join_spans()` helper removed from `mod.rs`.
  - `op_type_mismatch_error_spans_full_expression` test updated to check `end_span()`.

---

- [ ] **Step 1: Write the failing test in `op_table.rs`**

  In the `#[cfg(test)] mod tests` block of `cel-runtime/src/parser/op_table.rs`, add after `lookup_not_found_error_carries_span`:

  ```rust
  #[test]
  fn lookup_not_found_error_has_range() {
      let lookup = OpLookup::new();
      let mut segment = DynSegment::new::<()>();
      segment.just(10u32);
      segment.just(20.0f64);
      let err = lookup
          .lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())
          .unwrap_err();
      assert!(
          err.end_span().is_some(),
          "op-lookup errors should carry an end span"
      );
  }
  ```

- [ ] **Step 2: Confirm the test fails to compile**

  ```
  cargo test --workspace lookup_not_found_error_has_range
  ```

  Expected: compile error — `lookup` called with 5 arguments, 4 expected.

- [ ] **Step 3: Update `OpLookup::lookup()` signature and doc comment in `op_table.rs`**

  Replace the doc comment and function signature. Current:

  ```rust
  /// Looks up and applies an operation, binding the expression span to the registered op.
  ///
  /// Searches scopes in LIFO order, then falls back to built-in operations.
  ///
  /// # Errors
  ///
  /// Returns a [`super::ParseError`] carrying `span` if no scope or built-in handles the
  /// request, or if a scope itself returns an error.
  ///
  /// - Complexity: O(k) in the number of registered scopes, plus O(s) for the built-in
  ///   signature scan where s is the number of signatures for the operator.
  pub fn lookup(
      &self,
      name: &str,
      segment: &mut DynSegment,
      num_operands: usize,
      span: proc_macro2::Span,
  ) -> std::result::Result<(), super::ParseError> {
  ```

  Replace with:

  ```rust
  /// Looks up and applies an operation, attaching the expression span to any error.
  ///
  /// Searches scopes in LIFO order, then falls back to built-in operations.
  ///
  /// # Errors
  ///
  /// Returns a [`super::ParseError`] spanning `start..=end` if no scope or built-in
  /// handles the request, or if a scope itself returns an error.
  ///
  /// - Complexity: O(k) in the number of registered scopes, plus O(s) for the built-in
  ///   signature scan where s is the number of signatures for the operator.
  pub fn lookup(
      &self,
      name: &str,
      segment: &mut DynSegment,
      num_operands: usize,
      start: proc_macro2::Span,
      end: proc_macro2::Span,
  ) -> std::result::Result<(), super::ParseError> {
  ```

- [ ] **Step 4: Update the `ParseError` constructions inside `lookup()`**

  Inside the body, replace every `super::ParseError::new(msg, span)` with `super::ParseError::new_range(msg, start, end)`. There are three sites (scope-error path, builtin-error path, not-found path).

- [ ] **Step 5: Update the doc example on `OpLookup`**

  Find the doc example that calls `lookup`. Replace:

  ```rust
  /// lookup.lookup("+", &mut segment, 2, proc_macro2::Span::call_site()).unwrap();
  ```

  With:

  ```rust
  /// lookup.lookup("+", &mut segment, 2, proc_macro2::Span::call_site(), proc_macro2::Span::call_site()).unwrap();
  ```

- [ ] **Step 6: Update all test call sites in `op_table.rs`**

  Every `lookup.lookup(name, &mut segment, arity, Span::call_site())` call in the test module gets a second `Span::call_site()` argument. The full list:

  - `test_addition_u32`: `lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_subtraction_i32`: `lookup.lookup("-", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_arithmetic_overflow`: `lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_division_by_zero`: `lookup.lookup("/", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_modulo_by_zero`: `lookup.lookup("%", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_multiplication_f64`: `lookup.lookup("*", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_comparison_less_than`: `lookup.lookup("<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_logical_and`: `lookup.lookup("&&", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_bitwise_and`: `lookup.lookup("&", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_unary_negation`: `lookup.lookup("-", &mut segment, 1, Span::call_site(), Span::call_site())?;`
  - `test_logical_not`: `lookup.lookup("!", &mut segment, 1, Span::call_site(), Span::call_site())?;`
  - `test_unregistered_operation`: `let result = lookup.lookup("unknown_op", &mut segment, 2, Span::call_site(), Span::call_site());`
  - `test_custom_scope`: `lookup.lookup("double", &mut segment, 1, Span::call_site(), Span::call_site())?;`
  - `test_scope_override` (two calls): both `lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_left_shift_u64`: `lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_right_shift_i32`: `lookup.lookup(">>", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_shift_overflow`: `lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_shift_i32_rhs`: `lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_shift_negative_rhs_errors`: `lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_shift_wide_rhs_overflow_errors`: `lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `test_shift_rejects_float_rhs`: `assert!(lookup.lookup("<<", &mut segment, 2, Span::call_site(), Span::call_site()).is_err());`
  - `test_scope_pop` (two calls): both `lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site())?;`
  - `lookup_not_found_error_carries_span`: update to `lookup.lookup("+", &mut segment, 2, Span::call_site(), Span::call_site()).unwrap_err();`

- [ ] **Step 7: Remove `join_spans()` from `mod.rs`**

  Delete the free function (approximately lines 111–113):

  ```rust
  fn join_spans(start: Span, end: Span) -> Span {
      start.join(end).unwrap_or(start)
  }
  ```

- [ ] **Step 8: Update all `lookup()` call sites in `mod.rs`**

  Two patterns to replace throughout all 11 productions:

  **Pattern A** — inline `let expr_span`:

  Find (example from `is_or_expression`):
  ```rust
  let expr_span = join_spans(start_span.expect("production has token at start"), self.last_span);
  self.op_lookup.lookup("||", &mut self.context, 2, expr_span)?;
  ```

  Replace with:
  ```rust
  self.op_lookup.lookup(
      "||",
      &mut self.context,
      2,
      start_span.expect("production has token at start"),
      self.last_span,
  )?;
  ```

  **Pattern B** — multi-line `join_spans` (in `loop` productions):

  Find (example from `is_additive_expression`):
  ```rust
  let expr_span = join_spans(
      start_span.expect("production has token at start"),
      self.last_span,
  );
  self.op_lookup.lookup(op_name, &mut self.context, 2, expr_span)?;
  ```

  Replace with:
  ```rust
  self.op_lookup.lookup(
      op_name,
      &mut self.context,
      2,
      start_span.expect("production has token at start"),
      self.last_span,
  )?;
  ```

  Apply to all productions. After replacement, confirm `join_spans` appears nowhere in `mod.rs`.

- [ ] **Step 9: Update the identifier lookup in `is_primary_expression`**

  Find:
  ```rust
  self.op_lookup
      .lookup(&ident_name, &mut self.context, 0, ident_span)
      .map_err(|_| {
          ParseError::new(format!("Undefined identifier: {ident_name}"), ident_span)
      })?;
  ```

  Replace with (`ident_span` for both `start` and `end`; the `map_err` overrides the error):
  ```rust
  self.op_lookup
      .lookup(&ident_name, &mut self.context, 0, ident_span, ident_span)
      .map_err(|_| {
          ParseError::new(format!("Undefined identifier: {ident_name}"), ident_span)
      })?;
  ```

- [ ] **Step 10: Update `op_type_mismatch_error_spans_full_expression` test**

  Find the span-checking portion of this test. Current:
  ```rust
  let end_col = err.span().end().column;
  assert!(
      end_col >= 14,
      "span should reach the end of 32.0 (expected end.column >= 14, got {})",
      end_col
  );
  ```

  Replace with:
  ```rust
  // err.span() is the start ("Hello", col 0–7); end_span() is the end (32.0, end col 14).
  let end_span = err.end_span().expect("op-lookup errors carry an end span");
  assert!(
      end_span.end().column >= 14,
      "end span should reach the end of 32.0 (expected end.column >= 14, got {})",
      end_span.end().column
  );
  ```

- [ ] **Step 11: Run tests and clippy**

  ```
  cargo test --workspace
  cargo test --doc --workspace
  cargo clippy --workspace -- -D warnings
  ```

  Expected: all pass.

- [ ] **Step 12: Commit**

  ```
  git add cel-runtime/src/parser/op_table.rs cel-runtime/src/parser/mod.rs
  git commit -m "Update lookup() to two-span signature; remove join_spans()"
  ```

---

### Task 3: Emit second `compile_error!()` for range errors in `cel-rs-macros`

**Files:**
- Modify: `cel-rs-macros/src/lib.rs`

**Interfaces:**
- Consumes: `ParseError::end_span() -> Option<proc_macro2::Span>` from Task 1.
- Produces: When `e.end_span()` is `Some`, the `expression!` macro emits a second `compile_error!("expression continues here")` at the end span, in addition to the main error at the start span.

---

- [ ] **Step 1: Update the `expression!` macro error arm**

  Replace the `Err` arm:

  Current:
  ```rust
  Err(e) => {
      let msg_lit = Literal::string(e.message());
      quote_spanned!(e.span() => compile_error!(#msg_lit)).into()
  }
  ```

  Replace with:
  ```rust
  Err(e) => {
      let msg_lit = Literal::string(e.message());
      let mut tokens = quote_spanned!(e.span() => compile_error!(#msg_lit));
      if let Some(end) = e.end_span() {
          let end_lit = Literal::string("expression continues here");
          tokens.extend(quote_spanned!(end => compile_error!(#end_lit)));
      }
      tokens.into()
  }
  ```

- [ ] **Step 2: Run tests and clippy**

  ```
  cargo test --workspace
  cargo test --doc --workspace
  cargo clippy --workspace -- -D warnings
  ```

  Expected: all pass.

- [ ] **Step 3: Commit**

  ```
  git add cel-rs-macros/src/lib.rs
  git commit -m "Emit second compile_error!() for expression end span in expression! macro"
  ```
