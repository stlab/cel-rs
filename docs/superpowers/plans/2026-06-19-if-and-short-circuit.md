# if Expression and Short-Circuit &&/|| Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Rust-style `if`/`else`/`else if` expressions and true short-circuit `&&`/`||` to the CEL parser using `DynSegment::new_fragment()` and `join2()`.

**Architecture:** The parser builds lazy sub-segments (fragments) via `context.new_fragment()`, uses a context-swap to redirect parsing into the fragment, then calls `context.join2(true_branch, false_branch)`. At runtime `join2` pops a `bool` from the stack and executes exactly one fragment. `&&`/`||` no longer go through the op-table; they are pure parse-time constructs. `if` is handled as a keyword inside the `Identifier` arm of `is_primary_expression`.

**Tech Stack:** Rust, proc_macro2/syn (tokenisation), cel-runtime `DynSegment`/`RawSegment`.

## Global Constraints

- Every new/modified function must have a `///` doc comment in contract style (see CLAUDE.md).
- Tests derive from the contract and public interface only — not from implementation.
- `cargo clippy --workspace -- -D warnings` must produce zero warnings after every task.
- No new `wrapping_*` arithmetic; signed integer ops use `checked_*`.

---

### Task 1: Short-circuit `&&`

**Files:**
- Modify: `cel-parser/src/lib.rs` — rewrite `is_and_expression`
- Modify: `cel-parser/src/op_table.rs` — remove `LOGICAL_AND_SIGNATURES`, its `BUILTINS` entry, and `test_logical_and`

**Interfaces:**
- Produces: `is_and_expression(&mut self) -> Result<bool>` — same signature, new semantics (short-circuits when LHS is `false`)

- [ ] **Step 1: Write failing tests**

Add inside `mod tests` at the bottom of `cel-parser/src/lib.rs`:

```rust
#[test]
fn and_short_circuits_on_false() {
    // Without short-circuit the RHS executes and division-by-zero errors.
    // With short-circuit the RHS fragment is skipped, returning false directly.
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("false && (1i32 / 0i32 == 0i32)")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), false);
}

#[test]
fn and_evaluates_rhs_when_lhs_true() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("true && false")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), false);
}

#[test]
fn and_chained_short_circuits() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("false && false && false")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), false);
}
```

```rust
#[test]
fn and_lhs_type_error() {
    // LHS is i32, not bool — join2 must reject it at parse time.
    let mut parser = CELParser::new(OpLookup::new());
    assert!(parser.parse_str("1i32 && true").is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test --workspace and_short_circuits_on_false
```

Expected: FAIL — `and_short_circuits_on_false` errors with division-by-zero instead of returning `Ok(false)`. The other two correctness tests and the type-error test may already pass or already fail for the right reasons; the important one is `and_short_circuits_on_false`.

- [ ] **Step 3: Rewrite `is_and_expression` in `cel-parser/src/lib.rs`**

Find and replace the entire `is_and_expression` function (currently begins with `/// \`and_expression = ...`). The new version removes `start_span` and replaces the `op_lookup.lookup` call with a context-swap + `join2`:

```rust
/// `and_expression = comparison_expression { "&&" comparison_expression }.`
fn is_and_expression(&mut self) -> Result<bool> {
    if self.is_comparison_expression()? {
        while self.is_punctuation("&&") {
            let mut rhs_fragment = self.context.new_fragment();
            std::mem::swap(&mut self.context, &mut rhs_fragment);
            if !self.is_comparison_expression()? {
                return Err(self.error_at("expected comparison_expression"));
            }
            std::mem::swap(&mut self.context, &mut rhs_fragment);
            let mut bypass_fragment = self.context.new_fragment();
            bypass_fragment.just(false);
            self.context.join2(rhs_fragment, bypass_fragment)?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
```

- [ ] **Step 4: Remove `&&` from `cel-parser/src/op_table.rs`**

Remove the static array (including its comment):

```rust
// Logical AND signatures
static LOGICAL_AND_SIGNATURES: &[OpSignature] =
    &[sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a && b))];
```

Remove its entry from the `BUILTINS` phf map:

```rust
    "&&" => LOGICAL_AND_SIGNATURES,
```

Remove the entire `test_logical_and` test function:

```rust
#[test]
fn test_logical_and() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(true);
    segment.just(false);
    lookup.lookup("&&", &mut segment, 2, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<bool>()?, false);
    Ok(())
}
```

- [ ] **Step 5: Run tests and clippy**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: all tests pass, zero warnings.

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/lib.rs cel-parser/src/op_table.rs
git commit -m "feat: short-circuit && using new_fragment and join2"
```

---

### Task 2: Short-circuit `||`

**Files:**
- Modify: `cel-parser/src/lib.rs` — rewrite `is_or_expression`
- Modify: `cel-parser/src/op_table.rs` — remove `LOGICAL_OR_SIGNATURES` and its `BUILTINS` entry

**Interfaces:**
- Produces: `is_or_expression(&mut self) -> Result<bool>` — same signature, short-circuits when LHS is `true`

- [ ] **Step 1: Write failing tests**

Add inside `mod tests` in `cel-parser/src/lib.rs`:

```rust
#[test]
fn or_short_circuits_on_true() {
    // Without short-circuit the RHS executes and division-by-zero errors.
    // With short-circuit the RHS fragment is skipped, returning true directly.
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("true || (1i32 / 0i32 == 0i32)")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), true);
}

#[test]
fn or_evaluates_rhs_when_lhs_false() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("false || true")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), true);
}

#[test]
fn or_chained() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("true || false || false")
        .expect("should parse");
    assert_eq!(segment.call0::<bool>().unwrap(), true);
}
```

- [ ] **Step 2: Run test to verify it fails**

```
cargo test --workspace or_short_circuits_on_true
```

Expected: FAIL — errors with division-by-zero instead of returning `Ok(true)`.

- [ ] **Step 3: Rewrite `is_or_expression` in `cel-parser/src/lib.rs`**

Find and replace the entire `is_or_expression` function. The new version removes `start_span` and replaces the `op_lookup.lookup` call. Note the argument order to `join2`: `bypass_fragment` (the `true` shortcut) is fragment_0 (the true-branch), `rhs_fragment` is fragment_1 (the false-branch).

```rust
/// `or_expression = and_expression { "||" and_expression }.`
fn is_or_expression(&mut self) -> Result<bool> {
    if self.is_and_expression()? {
        while self.is_punctuation("||") {
            let mut rhs_fragment = self.context.new_fragment();
            std::mem::swap(&mut self.context, &mut rhs_fragment);
            if !self.is_and_expression()? {
                return Err(self.error_at("expected and_expression"));
            }
            std::mem::swap(&mut self.context, &mut rhs_fragment);
            let mut bypass_fragment = self.context.new_fragment();
            bypass_fragment.just(true);
            self.context.join2(bypass_fragment, rhs_fragment)?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
```

- [ ] **Step 4: Remove `||` from `cel-parser/src/op_table.rs`**

Remove the static array (including its comment):

```rust
// Logical OR signatures
static LOGICAL_OR_SIGNATURES: &[OpSignature] =
    &[sig!(TYPE_BOOL, 2, |seg| seg.op2(|a: bool, b: bool| a || b))];
```

Remove its entry from the `BUILTINS` phf map:

```rust
    "||" => LOGICAL_OR_SIGNATURES,
```

There is no `test_logical_or` in op_table — no additional test removal needed.

- [ ] **Step 5: Run tests and clippy**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: all tests pass, zero warnings.

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/lib.rs cel-parser/src/op_table.rs
git commit -m "feat: short-circuit || using new_fragment and join2"
```

---

### Task 3: `if`/`else`/`else if` expression

**Files:**
- Modify: `cel-parser/src/lib.rs` — add `is_keyword`, `is_if_expression`; extend `is_primary_expression`; update module-level grammar comment

**Interfaces:**
- Produces:
  - `is_keyword(&mut self, keyword: &str) -> bool` — helper, consumes matching identifier
  - `is_if_expression(&mut self) -> Result<bool>` — called after `if` is already consumed

- [ ] **Step 1: Write failing tests**

Add inside `mod tests` in `cel-parser/src/lib.rs`:

```rust
#[test]
fn if_true_branch_selected() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if true { 1i32 } else { 2i32 }")
        .expect("should parse");
    assert_eq!(segment.call0::<i32>().unwrap(), 1);
}

#[test]
fn if_false_branch_selected() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if false { 1i32 } else { 2i32 }")
        .expect("should parse");
    assert_eq!(segment.call0::<i32>().unwrap(), 2);
}

#[test]
fn if_else_if_first_branch() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if true { 1i32 } else if false { 2i32 } else { 3i32 }")
        .expect("should parse");
    assert_eq!(segment.call0::<i32>().unwrap(), 1);
}

#[test]
fn if_else_if_middle_branch() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if false { 1i32 } else if true { 2i32 } else { 3i32 }")
        .expect("should parse");
    assert_eq!(segment.call0::<i32>().unwrap(), 2);
}

#[test]
fn if_else_if_last_branch() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if false { 1i32 } else if false { 2i32 } else { 3i32 }")
        .expect("should parse");
    assert_eq!(segment.call0::<i32>().unwrap(), 3);
}

#[test]
fn if_omitted_else_unit_branch() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if true { () }")
        .expect("should parse");
    segment.call0::<()>().expect("should execute");
}

#[test]
fn if_omitted_else_rejects_non_unit_then() {
    // then-branch returns i32, implicit else returns () — types must match.
    let mut parser = CELParser::new(OpLookup::new());
    assert!(parser.parse_str("if false { 1i32 }").is_err());
}

#[test]
fn if_branch_type_mismatch_is_error() {
    let mut parser = CELParser::new(OpLookup::new());
    assert!(parser.parse_str("if true { 1i32 } else { true }").is_err());
}

#[test]
fn if_missing_open_brace_is_error() {
    let mut parser = CELParser::new(OpLookup::new());
    assert!(parser.parse_str("if true 1i32 } else { 2i32 }").is_err());
}

#[test]
fn if_missing_else_after_brace_is_fine() {
    // Omitting else is allowed; result type must be ().
    let mut parser = CELParser::new(OpLookup::new());
    let mut segment = parser
        .parse_str("if false { () }")
        .expect("should parse");
    segment.call0::<()>().expect("should execute");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```
cargo test --workspace if_true_branch_selected if_false_branch_selected
```

Expected: FAIL — parse errors because `if` is treated as an unknown identifier.

- [ ] **Step 3: Support `()` as a unit expression in `is_primary_expression`**

In `cel-parser/src/lib.rs`, find the `OpenDelim { Parenthesis }` arm of `is_primary_expression`. It currently reads:

```rust
Some(Token::OpenDelim {
    delimiter: Delimiter::Parenthesis,
    ..
}) => {
    self.advance();
    if !self.is_or_expression()? {
        return Err(self.error_at("expected expression"));
    }
    match self.peek_token() {
        Some(Token::CloseDelim {
            delimiter: Delimiter::Parenthesis,
            ..
        }) => {
            self.advance(); // consume CloseDelim
            Ok(true)
        }
        _ => Err(self.error_at("expected closing parenthesis")),
    }
}
```

Replace it with:

```rust
Some(Token::OpenDelim {
    delimiter: Delimiter::Parenthesis,
    ..
}) => {
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
        self.context.just(());
        return Ok(true);
    }
    if !self.is_or_expression()? {
        return Err(self.error_at("expected expression"));
    }
    match self.peek_token() {
        Some(Token::CloseDelim {
            delimiter: Delimiter::Parenthesis,
            ..
        }) => {
            self.advance();
            Ok(true)
        }
        _ => Err(self.error_at("expected closing parenthesis")),
    }
}
```

- [ ] **Step 4: Add `is_keyword` helper to `CELParser` in `cel-parser/src/lib.rs`**

Add this method to the `impl CELParser` block, adjacent to `is_punctuation`:

```rust
/// Consumes and returns `true` if the next token is an identifier matching `keyword`.
fn is_keyword(&mut self, keyword: &str) -> bool {
    match self.peek_token() {
        Some(Token::Identifier(ident)) if ident.to_string() == keyword => {
            self.advance();
            true
        }
        _ => false,
    }
}
```

- [ ] **Step 5: Add `is_if_expression` method to `CELParser` in `cel-parser/src/lib.rs`**

Add this method to the `impl CELParser` block. Place it after `is_primary_expression`:

```rust
/// `if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].`
///
/// - Precondition: The `if` keyword has already been consumed by the caller.
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
        fragment.just(());
        fragment
    };
    self.context.join2(then_fragment, else_fragment)?;
    Ok(true)
}
```

- [ ] **Step 6: Extend `is_primary_expression` to dispatch `"if"` to `is_if_expression`**

In `cel-parser/src/lib.rs`, find the `Token::Identifier(ident)` arm of `is_primary_expression`. It currently reads:

```rust
Some(Token::Identifier(ident)) => {
    let ident_name = ident.to_string();
    let ident_span = ident.span();
    self.advance();

    self.op_lookup
        .lookup(&ident_name, &mut self.context, 0, ident_span, ident_span)?;

    Ok(true)
}
```

Replace it with:

```rust
Some(Token::Identifier(ident)) => {
    let ident_name = ident.to_string();
    let ident_span = ident.span();
    self.advance();

    if ident_name == "if" {
        return self.is_if_expression();
    }

    self.op_lookup
        .lookup(&ident_name, &mut self.context, 0, ident_span, ident_span)?;

    Ok(true)
}
```

- [ ] **Step 7: Update the module-level grammar comment in `cel-parser/src/lib.rs`**

Find the `//! primary_expression = ...` line in the module doc comment (around line 29) and replace it with:

```rust
//! primary_expression = literal | identifier | "(" or_expression ")" | if_expression.
//! if_expression = "if" or_expression "{" or_expression "}" [ "else" ( "{" or_expression "}" | if_expression ) ].
```

- [ ] **Step 8: Run all tests and clippy**

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

Expected: all tests pass, zero warnings.

- [ ] **Step 9: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "feat: add if/else/else-if expression with optional else"
```
