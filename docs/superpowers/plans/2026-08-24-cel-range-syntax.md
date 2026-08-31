# CEL Range Syntax Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Rust's six range-expression forms (`a..b`, `a..`, `..b`, `..`, `a..=b`, `..=b`) to CEL, constructing `std::ops::{Range, RangeFrom, RangeTo, RangeFull, RangeInclusive, RangeToInclusive}` values — construction only, no operations (no slicing, no membership test, no comparisons) on the resulting values in this pass.

**Architecture:** Three additive changes, no new abstractions:
1. `cel-parser`'s lexer combines the punctuation `.`+`.` into a two-char op and `.`+`.`+`=` into a three-char op, exactly the way `&`+`&` already combines into `&&`.
2. `cel-parser`'s op-table gets new (name, per-type-signature) entries — `"range"`, `"range_inclusive"`, `"range_from"`, `"range_to"`, `"range_to_inclusive"` (arity 2 or 1, one signature per numeric type, built with the existing `sig!` macro) and `"range_full"` (arity 0, via a small custom scope function mirroring the existing `round_scope`, since arity-0 built-ins can't go through the generic per-type `OpSignature` table).
3. `cel-parser`'s grammar gets one new production, `range_expression`, inserted between `comparison_expression` and `bitwise_or_expression`, dispatching to whichever of the six op-table names the parsed shape calls for.

No `cel-runtime` changes are needed: `DynSegment::op0`/`op1`/`op2` already push/pop any `'static` type generically (confirmed against `ADD_SIGNATURES`' `String`-producing signature), so `Range<T>` etc. work with zero new runtime plumbing. No `cel-parser::ast`/`ty.rs` changes are needed either: named operators already produce a generic `Expr::Op` AST node and are type-checked generically via `builtin_operand_types(name)`, so the six new op-table names are picked up automatically by both the AST-building parser context and the static type checker.

**Tech Stack:** Rust, `proc_macro2` (token stream), `cel-parser`'s hand-written recursive-descent parser.

**Spec:** [docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md](../specs/2026-08-22-filter-deduction-range-slider-design.md) — this plan implements only that spec's §2 (`RangeInclusive<T>`/`..=`), generalized to full six-form parity per follow-up discussion. §1/§3/§4 (deduced filter args, `FilterKind`, `begin` UI) are separate, later plans.

## Global Constraints

- No operations beyond construction: do not register `==`, comparisons, iteration, or membership (`contains`) for any range type in this pass.
- No `cel-runtime` changes — if a task seems to need one, stop and re-read the Architecture section above; it almost certainly doesn't.
- Every new numeric-type signature array must cover the same 14 numeric types `ADD_SIGNATURES` covers: `u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64` (not `bool`/`String` — ranges are numeric-only for now, matching the design spec).
- `cargo test --workspace` and `cargo clippy --workspace --exclude begin --all-targets -- -D warnings` must both stay clean after every task.

---

### Task 1: Grammar hygiene — left-factor `parameter_list`'s optionality to its call site

Unrelated to range syntax itself, but surfaced while editing this same grammar: `parameter_list`'s own production currently bakes in "zero or more" (`parameter_list = [ or_expression { "," or_expression } ]`), so its one caller invokes it unconditionally and relies on it to silently produce `0` for an empty `()`. Move the optionality to the call site instead — `parameter_list` becomes "one or more" (`or_expression { "," or_expression }`), and `postfix_expression`'s own grammar line changes from `"(" parameter_list ")"` to `"(" [ parameter_list ] ")"`, with `is_postfix_expression` deciding explicitly (by peeking for `)`) whether to call it at all. Same accepted language, same argument counts in every case exercised by the existing test suite (verified below); the only observable change is that a malformed call with a leading comma right after `(` (e.g. `f(,5)`, not covered by any existing test) now reports `"expected expression"` at the point of failure instead of a delayed `"expected closing parenthesis"`.

**Files:**

- Modify: `cel-parser/src/lib.rs` (top-of-file grammar summary comment, `parameter_list`, `is_postfix_expression`)
- Test: `cel-parser/src/lib.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes: nothing from later tasks — this is pre-existing grammar, independent of range syntax.
- Produces: `parameter_list(&mut self) -> Result<usize>` now requires at least one `or_expression` (errors otherwise); `is_postfix_expression` gains an explicit zero-args check before calling it. No later task depends on this — it's ordered first only because the user asked for it while the file was already open for grammar changes.

- [ ] **Step 1: Write a failing test for the new error message on a leading comma**

```rust
#[test]
fn call_leading_comma_reports_expected_expression_at_the_comma() {
    let mut lookup = OpLookup::new();
    lookup.push_scope(
        |name, segment, num_operands, _span| match (name, num_operands) {
            ("f", 0) => {
                segment.op0(|| 0i32);
                Ok(true)
            }
            _ => Ok(false),
        },
    );
    let mut parser = CELParser::new(lookup);
    let err = match parser.parse_str("f(,5)") {
        Err(e) => e,
        Ok(_) => panic!("expected parse error for leading comma"),
    };
    assert_eq!(err.message(), "expected expression");
}
```

- [ ] **Step 2: Run it to verify it fails (with today's message, not the new one)**

Run: `cargo test -p cel-parser call_leading_comma_reports_expected_expression_at_the_comma`
Expected: FAIL — today this input produces `"expected closing parenthesis"` (parameter_list currently swallows the empty case and the mismatch is only caught once the parser looks for `)`), not `"expected expression"`.

- [ ] **Step 3: Left-factor `parameter_list` and its call site**

```rust
/// `parameter_list = or_expression { "," or_expression }.`
///
/// Always parses at least one `or_expression` — callers that need to allow zero
/// arguments (`postfix_expression`'s `"(" [ parameter_list ] ")"`) check for that
/// possibility themselves before calling, rather than `parameter_list` swallowing it.
///
/// Returns the argument count.
///
/// # Errors
/// Returns an error if the first token can't start an `or_expression`, or if a comma
/// isn't followed by one.
fn parameter_list(&mut self) -> Result<usize> {
    if !self.is_or_expression()? {
        return Err(self.error_at("expected expression"));
    }
    let mut count = 1;
    while self.is_punctuation(",") {
        if !self.is_or_expression()? {
            return Err(self.error_at("expected expression after comma"));
        }
        count += 1;
    }
    Ok(count)
}
```

In `is_postfix_expression`, replace the unconditional call:

```rust
self.advance(); // consume "("
let arg_count = self.parameter_list()?;
```

with an explicit check for the empty case:

```rust
self.advance(); // consume "("
let arg_count = if matches!(
    self.peek_token(),
    Some(Token::CloseDelim {
        delimiter: Delimiter::Parenthesis,
        ..
    })
) {
    0
} else {
    self.parameter_list()?
};
```

- [ ] **Step 4: Update the grammar doc comments**

Top-of-file summary (`//!` block):

```text
postfix_expression = primary_expression { "(" [ parameter_list ] ")" | "." unsuffixed_integer }.
```

```text
parameter_list = or_expression { "," or_expression }.
```

`is_postfix_expression`'s own doc comment (currently `` `postfix_expression = primary_expression { "(" parameter_list ")" | "." unsuffixed_integer }.` ``) gets the same `[ parameter_list ]` update.

- [ ] **Step 5: Run the new test and the existing call/parameter-list tests to verify no regressions**

Run: `cargo test -p cel-parser call_leading_comma_reports_expected_expression_at_the_comma call_empty_arg_list call_single_arg call_multiple_args call_missing_closing_paren call_trailing_comma call_undefined_call_op`
Expected: PASS — the new test now gets `"expected expression"`; all six pre-existing tests are unaffected (`call_empty_arg_list`'s `"f()"` is caught by the new peek-for-`)` check before `parameter_list` is ever called; `call_missing_closing_paren`'s `"f(42 43)"` and `call_trailing_comma`'s `"f(42,)"` both still reach the same error paths as before, since their first token after `(` is a valid expression start, not `)`).

- [ ] **Step 6: Run the full workspace test suite**

Run: `cargo test --workspace`
Expected: PASS, no regressions elsewhere (in particular any `adam-lang`/`cel-rs-macros` test that exercises a function call with arguments).

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "refactor(cel-parser): left-factor parameter_list's optionality to its call site"
```

---

### Task 2: Lexer — combine `.`+`.` and `.`+`.`+`=` into single tokens

**Files:**
- Modify: `cel-parser/src/lex_lexer.rs`
- Test: `cel-parser/src/lex_lexer.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `PunctOp::Three([char; 3])` variant (alongside the existing `One`/`Two`); `LexLexer` emits `Token::Punct { op: PunctOp::Two(['.', '.']), .. }` for `..` and `Token::Punct { op: PunctOp::Three(['.', '.', '=']), .. }` for `..=`. Consumed by Task 5's `is_punctuation("..")`/`is_punctuation("..=")` calls, which need no changes themselves — `is_punctuation` already compares generically via `PunctOp`'s `PartialEq<str>`.

- [ ] **Step 1: Write a failing test proving `proc_macro2` doesn't merge a trailing dot-dot into a float literal**

This is the one real unknown before writing anything else: Rust's own lexer (which `proc_macro2` mirrors) special-cases `5..10` to tokenize as `Int(5)`, two separate `.` puncts, `Int(10)` — not `Float("5.")` — precisely so range syntax works with no whitespace. Confirm this holds before building the combiner on top of it.

```rust
#[test]
fn digit_followed_by_double_dot_does_not_lex_as_a_float_literal() {
    let input = TokenStream::from_str("5..10").unwrap();
    let mut lexer = LexLexer::new(input.into_iter());
    match lexer.next() {
        Some(Token::Literal(Lit::Int(lit))) => assert_eq!(lit.base10_parse::<i32>().unwrap(), 5),
        other => panic!("expected an integer literal, got {other:?}"),
    }
    // The next two tokens must each be a lone `.` Punct at this pre-combining stage —
    // asserted directly on proc_macro2's TokenStream, before any of this file's own
    // combining logic runs, so this test fails for the right reason if the assumption
    // about proc_macro2's tokenization is wrong, not because of code we haven't written yet.
}
```

- [ ] **Step 2: Run it to confirm the tokenization assumption**

Run: `cargo test -p cel-parser digit_followed_by_double_dot_does_not_lex_as_a_float_literal`
Expected: PASS (this test doesn't exercise any new code — it documents/locks in a `proc_macro2` behavior the rest of this task depends on). If it fails, stop and re-examine the assumption before proceeding — everything below depends on it.

- [ ] **Step 3: Write failing tests for the two new combined tokens**

```rust
#[test]
fn double_dot_combines_into_two_char_punct() {
    let stream: TokenStream = "..".parse().unwrap();
    let mut lexer = LexLexer::new(stream.into_iter());
    let tok = lexer.next().expect("one token");
    assert!(
        matches!(tok, Token::Punct { op: PunctOp::Two(['.', '.']), .. }),
        "expected PunctOp::Two(['.', '.']), got {tok:?}"
    );
    assert!(lexer.next().is_none(), "expected no more tokens");
}

#[test]
fn double_dot_equals_combines_into_three_char_punct() {
    let stream: TokenStream = "..=".parse().unwrap();
    let mut lexer = LexLexer::new(stream.into_iter());
    let tok = lexer.next().expect("one token");
    assert!(
        matches!(tok, Token::Punct { op: PunctOp::Three(['.', '.', '=']), .. }),
        "expected PunctOp::Three(['.', '.', '=']), got {tok:?}"
    );
    assert!(lexer.next().is_none(), "expected no more tokens");
}

#[test]
fn double_dot_not_followed_by_equals_stays_two_char_and_does_not_consume_the_next_token() {
    let stream: TokenStream = "..5".parse().unwrap();
    let mut lexer = LexLexer::new(stream.into_iter());
    let first = lexer.next().expect("dot-dot token");
    assert!(
        matches!(first, Token::Punct { op: PunctOp::Two(['.', '.']), .. }),
        "expected PunctOp::Two(['.', '.']), got {first:?}"
    );
    match lexer.next() {
        Some(Token::Literal(Lit::Int(lit))) => assert_eq!(lit.base10_parse::<i32>().unwrap(), 5),
        other => panic!("expected integer literal 5, got {other:?}"),
    }
}

#[test]
fn single_dot_before_a_digit_is_unaffected_by_range_combining() {
    // Regression guard: tuple field access (`x.0`) must still lex as a lone `.` Punct
    // followed by an integer literal, not get swept into the new `..`/`..=` combining.
    let stream: TokenStream = ".0".parse().unwrap();
    let mut lexer = LexLexer::new(stream.into_iter());
    let first = lexer.next().expect("dot token");
    assert!(
        matches!(first, Token::Punct { op: PunctOp::One('.'), .. }),
        "expected PunctOp::One('.'), got {first:?}"
    );
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test -p cel-parser double_dot_combines_into_two_char_punct double_dot_equals_combines_into_three_char_punct double_dot_not_followed_by_equals_stays_two_char_and_does_not_consume_the_next_token`
Expected: FAIL — `is_compound_operator` doesn't yet recognize `('.', '.')`, so `..` currently lexes as two separate `PunctOp::One('.')` tokens; `PunctOp::Three` doesn't exist yet (compile error) until the enum is extended.

(`single_dot_before_a_digit_is_unaffected_by_range_combining` should already PASS unchanged — it's a regression guard, not new behavior.)

- [ ] **Step 5: Extend `PunctOp` and its comparisons**

```rust
/// Punctuation operator (1, 2, or 3 chars) without heap allocation.
#[derive(Clone, Debug)]
pub enum PunctOp {
    /// Single character (e.g. `+`, `-`).
    One(char),
    /// Two characters (e.g. `&&`, `<=`, `..`).
    Two([char; 2]),
    /// Three characters (e.g. `..=`).
    Three([char; 3]),
}

impl PartialEq<str> for PunctOp {
    fn eq(&self, other: &str) -> bool {
        match self {
            PunctOp::One(c) => other.starts_with(*c) && other.len() == 1,
            PunctOp::Two([a, b]) => {
                let mut it = other.chars();
                it.next() == Some(*a) && it.next() == Some(*b) && it.next().is_none()
            }
            PunctOp::Three([a, b, c]) => {
                let mut it = other.chars();
                it.next() == Some(*a)
                    && it.next() == Some(*b)
                    && it.next() == Some(*c)
                    && it.next().is_none()
            }
        }
    }
}
```

- [ ] **Step 6: Recognize `('.', '.')` as a compound operator**

```rust
fn is_compound_operator(first: char, second: char) -> bool {
    matches!(
        (first, second),
        ('&', '&')
            | ('|', '|')
            | ('=', '=')
            | ('!', '=')
            | ('<', '=')
            | ('>', '=')
            | ('<', '<')
            | ('>', '>')
            | ('-', '>')
            | ('=', '>')
            | (':', '=')
            | ('.', '.')
    )
}

/// Check if three characters form a known triple-character operator. Currently only `..=`.
fn is_triple_compound_operator(first: char, second: char, third: char) -> bool {
    matches!((first, second, third), ('.', '.', '='))
}
```

- [ ] **Step 7: Extend the Joint-punct combining branch to attempt a third character**

Replace the `if Self::is_compound_operator(ch, next_ch) { ... }` arm inside `Iterator::next`'s Punct handling with:

```rust
if Self::is_compound_operator(ch, next_ch) {
    // Try to extend to a known 3-char op (currently only ".." + "=" -> "..=").
    // Only attempt this if the second punct is itself immediately followed by
    // something (Joint spacing) — otherwise there's nothing to peek.
    if next_punct.spacing() == Spacing::Joint {
        match self.next_token_tree() {
            Some(TokenTree::Punct(third_punct)) => {
                let third_ch = third_punct.as_char();
                if Self::is_triple_compound_operator(ch, next_ch, third_ch) {
                    return Some(Token::Punct {
                        op: PunctOp::Three([ch, next_ch, third_ch]),
                        span,
                    });
                }
                self.pending_token = Some(TokenTree::Punct(third_punct));
                return Some(Token::Punct {
                    op: PunctOp::Two([ch, next_ch]),
                    span,
                });
            }
            Some(other) => {
                self.pending_token = Some(other);
                return Some(Token::Punct {
                    op: PunctOp::Two([ch, next_ch]),
                    span,
                });
            }
            None => {
                return Some(Token::Punct {
                    op: PunctOp::Two([ch, next_ch]),
                    span,
                });
            }
        }
    }
    return Some(Token::Punct {
        op: PunctOp::Two([ch, next_ch]),
        span,
    });
} else {
    self.pending_token = Some(TokenTree::Punct(next_punct));
    return Some(Token::Punct {
        op: PunctOp::One(ch),
        span,
    });
}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo test -p cel-parser lex_lexer`
Expected: PASS — all four new tests, plus every pre-existing `lex_lexer` test (in particular `arrow_is_two_char_punct`, `fat_arrow_is_two_char_punct`, `walrus_operator_lexes_as_one_compound_token`, `colon_followed_by_non_equals_stays_two_separate_tokens`, and the tuple-index-relevant `test_punct`/`test_mixed_tokens`), still pass unchanged.

- [ ] **Step 9: Commit**

```bash
git add cel-parser/src/lex_lexer.rs
git commit -m "feat(cel-parser): lex '..' and '..=' as combined punctuation tokens"
```

---

### Task 3: Op-table — two-endpoint range constructors (`Range<T>`, `RangeInclusive<T>`)

**Files:**
- Modify: `cel-parser/src/op_table.rs`
- Test: `cel-parser/src/op_table.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing from Task 2 (this task can be written and tested independently, via direct `OpLookup::lookup` calls, before the grammar wiring in Task 5 exists).
- Produces: op-table entries `"range"` (arity 2) and `"range_inclusive"` (arity 2), one signature per numeric type each, dispatched through the existing `BuiltinScope::lookup`/`OpLookup::lookup` machinery exactly like `"+"`. Consumed by Task 5's `is_range_expression`.

- [ ] **Step 1: Write failing tests for both op-table entries**

```rust
#[test]
fn range_i32_constructs_a_range() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(1i32);
    segment.just(5i32);
    lookup.lookup("range", &mut segment, 2, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::Range<i32>>()?, 1i32..5i32);
    Ok(())
}

#[test]
fn range_f64_constructs_a_range() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(1.5f64);
    segment.just(5.5f64);
    lookup.lookup("range", &mut segment, 2, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::Range<f64>>()?, 1.5f64..5.5f64);
    Ok(())
}

#[test]
fn range_inclusive_i32_constructs_a_range_inclusive() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(1i32);
    segment.just(5i32);
    lookup.lookup("range_inclusive", &mut segment, 2, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::RangeInclusive<i32>>()?, 1i32..=5i32);
    Ok(())
}

#[test]
fn range_rejects_mismatched_operand_types() {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(1i32);
    segment.just(5.0f64);
    let result = lookup.lookup("range", &mut segment, 2, Span::call_site(), Span::call_site());
    assert!(result.is_err(), "expected a type-mismatch error, got Ok");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser range_i32_constructs_a_range range_f64_constructs_a_range range_inclusive_i32_constructs_a_range_inclusive range_rejects_mismatched_operand_types`
Expected: FAIL with "no scope or built-in handles the request" (or equivalent) — `"range"`/`"range_inclusive"` aren't registered yet.

- [ ] **Step 3: Add the two signature arrays and register them**

```rust
// Range construction (`a..b`, `a..=b`) — no operations beyond construction; see the
// design spec's "Compatibility Notes" for why the recognized structural form is a real
// `RangeInclusive<T>` type rather than a name-matched builtin.
static RANGE_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a..b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a..b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a..b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a..b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a..b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg.op2(|a: usize, b: usize| a..b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a..b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a..b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a..b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a..b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a..b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2(|a: isize, b: isize| a..b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a..b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a..b)),
];

static RANGE_INCLUSIVE_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 2, |seg, _span| seg.op2(|a: u8, b: u8| a..=b)),
    sig!(TYPE_U16, 2, |seg, _span| seg.op2(|a: u16, b: u16| a..=b)),
    sig!(TYPE_U32, 2, |seg, _span| seg.op2(|a: u32, b: u32| a..=b)),
    sig!(TYPE_U64, 2, |seg, _span| seg.op2(|a: u64, b: u64| a..=b)),
    sig!(TYPE_U128, 2, |seg, _span| seg.op2(|a: u128, b: u128| a..=b)),
    sig!(TYPE_USIZE, 2, |seg, _span| seg.op2(|a: usize, b: usize| a..=b)),
    sig!(TYPE_I8, 2, |seg, _span| seg.op2(|a: i8, b: i8| a..=b)),
    sig!(TYPE_I16, 2, |seg, _span| seg.op2(|a: i16, b: i16| a..=b)),
    sig!(TYPE_I32, 2, |seg, _span| seg.op2(|a: i32, b: i32| a..=b)),
    sig!(TYPE_I64, 2, |seg, _span| seg.op2(|a: i64, b: i64| a..=b)),
    sig!(TYPE_I128, 2, |seg, _span| seg.op2(|a: i128, b: i128| a..=b)),
    sig!(TYPE_ISIZE, 2, |seg, _span| seg.op2(|a: isize, b: isize| a..=b)),
    sig!(TYPE_F32, 2, |seg, _span| seg.op2(|a: f32, b: f32| a..=b)),
    sig!(TYPE_F64, 2, |seg, _span| seg.op2(|a: f64, b: f64| a..=b)),
];
```

Add both to the `BUILTINS` phf map:

```rust
static BUILTINS: phf::Map<&'static str, &'static [OpSignature]> = phf_map! {
    "+" => ADD_SIGNATURES,
    "-" => SUB_SIGNATURES,
    "*" => MUL_SIGNATURES,
    "/" => DIV_SIGNATURES,
    "%" => MOD_SIGNATURES,
    "&" => BITWISE_AND_SIGNATURES,
    "|" => BITWISE_OR_SIGNATURES,
    "^" => BITWISE_XOR_SIGNATURES,
    "!" => LOGICAL_NOT_SIGNATURES,
    "==" => EQUAL_SIGNATURES,
    "!=" => NOT_EQUAL_SIGNATURES,
    "<" => LESS_THAN_SIGNATURES,
    "<=" => LESS_THAN_OR_EQUAL_SIGNATURES,
    ">" => GREATER_THAN_SIGNATURES,
    ">=" => GREATER_THAN_OR_EQUAL_SIGNATURES,
    "range" => RANGE_SIGNATURES,
    "range_inclusive" => RANGE_INCLUSIVE_SIGNATURES,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser range_i32_constructs_a_range range_f64_constructs_a_range range_inclusive_i32_constructs_a_range_inclusive range_rejects_mismatched_operand_types`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): register Range<T>/RangeInclusive<T> construction ops"
```

---

### Task 4: Op-table — one-endpoint and zero-endpoint range constructors

**Files:**
- Modify: `cel-parser/src/op_table.rs`

**Interfaces:**
- Consumes: the `sig!` macro, `BUILTINS` map, and `push_library_scope`/`OpLookup::new` — all already present (Task 3 doesn't need to land first; this task is independent of it, just conventionally ordered after).
- Produces: op-table entries `"range_from"`, `"range_to"`, `"range_to_inclusive"` (arity 1 each) and `"range_full"` (arity 0, via a new `range_full_scope`). Consumed by Task 5's `is_range_expression`.

- [ ] **Step 1: Write failing tests for all four**

```rust
#[test]
fn range_from_i32_constructs_a_range_from() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(3i32);
    lookup.lookup("range_from", &mut segment, 1, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::RangeFrom<i32>>()?, 3i32..);
    Ok(())
}

#[test]
fn range_to_i32_constructs_a_range_to() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(7i32);
    lookup.lookup("range_to", &mut segment, 1, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::RangeTo<i32>>()?, ..7i32);
    Ok(())
}

#[test]
fn range_to_inclusive_i32_constructs_a_range_to_inclusive() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    segment.just(7i32);
    lookup.lookup("range_to_inclusive", &mut segment, 1, Span::call_site(), Span::call_site())?;
    assert_eq!(segment.call0::<std::ops::RangeToInclusive<i32>>()?, ..=7i32);
    Ok(())
}

#[test]
fn range_full_constructs_a_range_full() -> Result<()> {
    let lookup = OpLookup::new();
    let mut segment = DynSegment::new::<()>();
    lookup.lookup("range_full", &mut segment, 0, Span::call_site(), Span::call_site())?;
    segment.call0::<std::ops::RangeFull>()?;
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser range_from_i32_constructs_a_range_from range_to_i32_constructs_a_range_to range_to_inclusive_i32_constructs_a_range_to_inclusive range_full_constructs_a_range_full`
Expected: FAIL — none of these four names are registered yet.

- [ ] **Step 3: Add the three per-type signature arrays**

```rust
static RANGE_FROM_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 1, |seg, _span| seg.op1(|a: u8| a..)),
    sig!(TYPE_U16, 1, |seg, _span| seg.op1(|a: u16| a..)),
    sig!(TYPE_U32, 1, |seg, _span| seg.op1(|a: u32| a..)),
    sig!(TYPE_U64, 1, |seg, _span| seg.op1(|a: u64| a..)),
    sig!(TYPE_U128, 1, |seg, _span| seg.op1(|a: u128| a..)),
    sig!(TYPE_USIZE, 1, |seg, _span| seg.op1(|a: usize| a..)),
    sig!(TYPE_I8, 1, |seg, _span| seg.op1(|a: i8| a..)),
    sig!(TYPE_I16, 1, |seg, _span| seg.op1(|a: i16| a..)),
    sig!(TYPE_I32, 1, |seg, _span| seg.op1(|a: i32| a..)),
    sig!(TYPE_I64, 1, |seg, _span| seg.op1(|a: i64| a..)),
    sig!(TYPE_I128, 1, |seg, _span| seg.op1(|a: i128| a..)),
    sig!(TYPE_ISIZE, 1, |seg, _span| seg.op1(|a: isize| a..)),
    sig!(TYPE_F32, 1, |seg, _span| seg.op1(|a: f32| a..)),
    sig!(TYPE_F64, 1, |seg, _span| seg.op1(|a: f64| a..)),
];

static RANGE_TO_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 1, |seg, _span| seg.op1(|a: u8| ..a)),
    sig!(TYPE_U16, 1, |seg, _span| seg.op1(|a: u16| ..a)),
    sig!(TYPE_U32, 1, |seg, _span| seg.op1(|a: u32| ..a)),
    sig!(TYPE_U64, 1, |seg, _span| seg.op1(|a: u64| ..a)),
    sig!(TYPE_U128, 1, |seg, _span| seg.op1(|a: u128| ..a)),
    sig!(TYPE_USIZE, 1, |seg, _span| seg.op1(|a: usize| ..a)),
    sig!(TYPE_I8, 1, |seg, _span| seg.op1(|a: i8| ..a)),
    sig!(TYPE_I16, 1, |seg, _span| seg.op1(|a: i16| ..a)),
    sig!(TYPE_I32, 1, |seg, _span| seg.op1(|a: i32| ..a)),
    sig!(TYPE_I64, 1, |seg, _span| seg.op1(|a: i64| ..a)),
    sig!(TYPE_I128, 1, |seg, _span| seg.op1(|a: i128| ..a)),
    sig!(TYPE_ISIZE, 1, |seg, _span| seg.op1(|a: isize| ..a)),
    sig!(TYPE_F32, 1, |seg, _span| seg.op1(|a: f32| ..a)),
    sig!(TYPE_F64, 1, |seg, _span| seg.op1(|a: f64| ..a)),
];

static RANGE_TO_INCLUSIVE_SIGNATURES: &[OpSignature] = &[
    sig!(TYPE_U8, 1, |seg, _span| seg.op1(|a: u8| ..=a)),
    sig!(TYPE_U16, 1, |seg, _span| seg.op1(|a: u16| ..=a)),
    sig!(TYPE_U32, 1, |seg, _span| seg.op1(|a: u32| ..=a)),
    sig!(TYPE_U64, 1, |seg, _span| seg.op1(|a: u64| ..=a)),
    sig!(TYPE_U128, 1, |seg, _span| seg.op1(|a: u128| ..=a)),
    sig!(TYPE_USIZE, 1, |seg, _span| seg.op1(|a: usize| ..=a)),
    sig!(TYPE_I8, 1, |seg, _span| seg.op1(|a: i8| ..=a)),
    sig!(TYPE_I16, 1, |seg, _span| seg.op1(|a: i16| ..=a)),
    sig!(TYPE_I32, 1, |seg, _span| seg.op1(|a: i32| ..=a)),
    sig!(TYPE_I64, 1, |seg, _span| seg.op1(|a: i64| ..=a)),
    sig!(TYPE_I128, 1, |seg, _span| seg.op1(|a: i128| ..=a)),
    sig!(TYPE_ISIZE, 1, |seg, _span| seg.op1(|a: isize| ..=a)),
    sig!(TYPE_F32, 1, |seg, _span| seg.op1(|a: f32| ..=a)),
    sig!(TYPE_F64, 1, |seg, _span| seg.op1(|a: f64| ..=a)),
];
```

Add them to `BUILTINS`:

```rust
    "range" => RANGE_SIGNATURES,
    "range_inclusive" => RANGE_INCLUSIVE_SIGNATURES,
    "range_from" => RANGE_FROM_SIGNATURES,
    "range_to" => RANGE_TO_SIGNATURES,
    "range_to_inclusive" => RANGE_TO_INCLUSIVE_SIGNATURES,
```

- [ ] **Step 4: Add `range_full_scope` and register it**

`RangeFull` has no type parameter — there's nothing to key an `OpSignature` on, exactly the situation the existing `round_scope` (arity-0 `round`) already solves. Mirror it directly, placed right after `round_scope`:

```rust
/// Scope function implementing the arity-0 `range_full` internal op: constructs
/// `std::ops::RangeFull`, the value a bare `..` produces. Unlike `round_scope`, there is
/// no second half — `RangeFull` is never called with arguments, so there's no paired
/// `"()"` arm to add.
///
/// Registered by every [`OpLookup::new()`] (see there). `"range_full"` is an internal
/// dispatch name the parser selects when it recognizes a bare `..` with neither a left
/// nor right endpoint — never a name CEL source can reference directly, matching
/// `"range"`/`"range_inclusive"`/`"range_from"`/`"range_to"`/`"range_to_inclusive"`.
fn range_full_scope(
    name: &str,
    segment: &mut DynSegment,
    num_operands: usize,
    _span: SourceSpan,
) -> Result<bool> {
    match (name, num_operands) {
        ("range_full", 0) => {
            segment.op0(|| std::ops::RangeFull);
            Ok(true)
        }
        _ => Ok(false),
    }
}
```

In `OpLookup::new()`, register it alongside `round_scope`:

```rust
    pub fn new() -> Self {
        let mut lookup = OpLookup {
            scopes: Vec::new(),
            library_scope_count: 0,
            builtin_scope: BuiltinScope,
            tuple_signatures: Vec::new(),
        };
        lookup.push_library_scope(round_scope);
        lookup.push_library_scope(range_full_scope);
        lookup
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p cel-parser range_from_i32_constructs_a_range_from range_to_i32_constructs_a_range_to range_to_inclusive_i32_constructs_a_range_to_inclusive range_full_constructs_a_range_full`
Expected: PASS.

- [ ] **Step 6: Run the full op_table test module to check for regressions**

Run: `cargo test -p cel-parser op_table`
Expected: PASS — in particular any test that iterates `BUILTINS` (if one exists) still passes with the five new entries present.

- [ ] **Step 7: Commit**

```bash
git add cel-parser/src/op_table.rs
git commit -m "feat(cel-parser): register RangeFrom/RangeTo/RangeToInclusive/RangeFull construction ops"
```

---

### Task 5: Grammar — `range_expression` production and end-to-end parsing

**Files:**
- Modify: `cel-parser/src/lib.rs`

**Interfaces:**
- Consumes: `is_bitwise_or_expression` (existing, unchanged), `self.context.apply_op` (existing `ParserContext` method), the six op-table names from Tasks 3–4 (`"range"`, `"range_inclusive"`, `"range_from"`, `"range_to"`, `"range_to_inclusive"`, `"range_full"`).
- Produces: `is_range_expression(&mut self) -> Result<bool>`, wired into `is_comparison_expression` in place of its two direct `is_bitwise_or_expression()?` calls. No new public API — `parse_str`/`parse_str_ast` pick this up automatically since they already route through `is_expression -> is_or_expression -> ... -> is_comparison_expression`.

- [ ] **Step 1: Write failing end-to-end tests for all six forms**

Add to the existing `#[cfg(test)] mod tests` block in `cel-parser/src/lib.rs`:

```rust
#[test]
fn range_expression_constructs_a_range() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("1i32..5i32").unwrap();
    assert_eq!(seg.call0::<std::ops::Range<i32>>().unwrap(), 1i32..5i32);
}

#[test]
fn range_inclusive_expression_constructs_a_range_inclusive() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("1i32..=5i32").unwrap();
    assert_eq!(seg.call0::<std::ops::RangeInclusive<i32>>().unwrap(), 1i32..=5i32);
}

#[test]
fn range_from_expression_constructs_a_range_from() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("3i32..").unwrap();
    assert_eq!(seg.call0::<std::ops::RangeFrom<i32>>().unwrap(), 3i32..);
}

#[test]
fn range_to_expression_constructs_a_range_to() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("..7i32").unwrap();
    assert_eq!(seg.call0::<std::ops::RangeTo<i32>>().unwrap(), ..7i32);
}

#[test]
fn range_to_inclusive_expression_constructs_a_range_to_inclusive() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("..=7i32").unwrap();
    assert_eq!(seg.call0::<std::ops::RangeToInclusive<i32>>().unwrap(), ..=7i32);
}

#[test]
fn range_full_expression_constructs_a_range_full() {
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("..").unwrap();
    seg.call0::<std::ops::RangeFull>().unwrap();
}

#[test]
fn range_endpoints_are_full_bitwise_or_expressions() {
    // `1 + 2..3 * 4` must group as `(1 + 2)..(3 * 4)`, matching Rust's own precedence
    // (range binds looser than every arithmetic/bitwise operator).
    let mut parser = CELParser::new(OpLookup::new());
    let mut seg = parser.parse_str("1i32 + 2i32..3i32 * 4i32").unwrap();
    assert_eq!(seg.call0::<std::ops::Range<i32>>().unwrap(), 3i32..12i32);
}

#[test]
fn chained_ranges_are_a_parse_error() {
    // Ranges don't chain, matching Rust (`1..2..3` is also a compile error there). No
    // special "non-chainable" check is needed in `is_range_expression` itself: after
    // parsing `1..2`, the leftover `..3` fails the top-level `expression = or_expression
    // <EOF>` check the same way `"10 + 25 25"` already does (see `incomplete_expression`).
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("1i32..2i32..3i32");
    assert!(result.is_err(), "expected a parse error, got Ok");
}

#[test]
fn range_to_inclusive_without_a_right_operand_is_a_parse_error() {
    // `..=` always requires a right endpoint — there is no inclusive-from-only range.
    let mut parser = CELParser::new(OpLookup::new());
    let result = parser.parse_str("..=");
    assert!(result.is_err(), "expected a parse error, got Ok");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cel-parser range_expression_constructs_a_range range_inclusive_expression_constructs_a_range_inclusive range_from_expression_constructs_a_range_from range_to_expression_constructs_a_range_to range_to_inclusive_expression_constructs_a_range_to_inclusive range_full_expression_constructs_a_range_full range_endpoints_are_full_bitwise_or_expressions chained_ranges_are_a_parse_error range_to_inclusive_without_a_right_operand_is_a_parse_error`
Expected: FAIL — `is_range_expression` doesn't exist yet; `.`/`..`/`..=` aren't recognized at the grammar level (only at the lexer level from Task 2).

- [ ] **Step 3: Add `is_range_expression` and wire it into `is_comparison_expression`**

Replace the existing `is_comparison_expression` (and add the new production immediately after it):

```rust
/// `comparison_expression = range_expression [ comparison_op range_expression ].`
fn is_comparison_expression(&mut self) -> Result<bool> {
    let start_span = self.peek_span();
    if self.is_range_expression()? {
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
            if !self.is_range_expression()? {
                return Err(self.error_at("expected range_expression"));
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

/// `range_expression = bitwise_or_expression [ ".." [ bitwise_or_expression ] | "..=" bitwise_or_expression ]
///                   | ".." [ bitwise_or_expression ]
///                   | "..=" bitwise_or_expression .`
///
/// Left-factored so every alternative is chosen by one concrete leading token rather
/// than by first deciding whether an optional `bitwise_or_expression` is present: the
/// three alternatives start with `bitwise_or_expression`'s own FIRST set, the literal
/// `".."`, or the literal `"..="` respectively — pairwise disjoint (`..`/`..=` can never
/// be the first token of a `bitwise_or_expression`), so picking among them needs exactly
/// one token, and none of them opens with a bracketed, possibly-empty non-terminal.
///
/// `..`'s right operand is optional wherever it appears (covering, across the three
/// alternatives, `Range`/`RangeFrom`/`RangeTo`/`RangeFull`); `..=`'s right operand is
/// never optional (covering `RangeInclusive`/`RangeToInclusive`) — there is no
/// inclusive-from-only range in Rust, and no such form is registered in the op-table for
/// it to dispatch to, so a bare `..=`, or a left operand followed by `..=` and nothing
/// after, is a parse error, not a valid empty match.
///
/// Endpoints are `bitwise_or_expression`s — the same level this production sits just
/// above — so `1 + 2..3 * 4` and `a | b..c & d` both parse with the expected grouping.
fn is_range_expression(&mut self) -> Result<bool> {
    let start_span = self.peek_span();

    if self.is_punctuation("..=") {
        if !self.is_bitwise_or_expression()? {
            return Err(self.error_at("expected bitwise_or_expression"));
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
        if self.is_bitwise_or_expression()? {
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

    if self.is_bitwise_or_expression()? {
        if self.is_punctuation("..=") {
            if !self.is_bitwise_or_expression()? {
                return Err(self.error_at("expected bitwise_or_expression"));
            }
            self.context.apply_op(
                &self.op_lookup,
                "range_inclusive",
                2,
                start_span.expect("production has token at start"),
                self.last_span,
            )?;
        } else if self.is_punctuation("..") {
            if self.is_bitwise_or_expression()? {
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

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cel-parser range_expression_constructs_a_range range_inclusive_expression_constructs_a_range_inclusive range_from_expression_constructs_a_range_from range_to_expression_constructs_a_range_to range_to_inclusive_expression_constructs_a_range_to_inclusive range_full_expression_constructs_a_range_full range_endpoints_are_full_bitwise_or_expressions chained_ranges_are_a_parse_error range_to_inclusive_without_a_right_operand_is_a_parse_error`
Expected: PASS.

- [ ] **Step 5: Run the full workspace test suite and lint**

Run: `cargo test --workspace`
Expected: PASS, zero new warnings (in particular, confirm no existing tuple-index test — e.g. `index_first_element_of_tuple`, `out_of_range_index_is_a_parse_error` — regressed).

Run: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
Expected: clean.

Run: `cargo fmt --all -- --check`
Expected: clean (or run `cargo fmt --all` and include the formatting diff in this commit).

- [ ] **Step 6: Commit**

```bash
git add cel-parser/src/lib.rs
git commit -m "feat(cel-parser): add range_expression grammar production for all six range forms"
```

---

## Out of Scope (confirmed, not deferred by accident)

- Any operation on a constructed range value beyond holding/returning it: no `==`, no `.contains()`, no iteration, no `for` loops, no slicing/indexing with a range.
- `cel-parser::ast`/`ty.rs` changes: none needed — named operators already flow through the generic `Expr::Op`/`builtin_operand_types` machinery. Confirmed by Task 5's tests exercising `parse_str` (the `DynSegmentContext` path); if a follow-up plan touches `parse_str_ast`, add an equivalent `Expr::Op { name: "range", .. }`-shape assertion there rather than assuming it works.
- `cel-runtime` changes: none needed — `op0`/`op1`/`op2` already push/pop any `'static` type generically.
- `cel-rs-macros` changes: none anticipated (it calls into `cel-parser` generically); not verified by a dedicated task here — if `cargo test -p cel-rs-macros` regresses in Step 5 of Task 5, treat that as a signal this assumption was wrong, not as unrelated flakiness.
- `adam-lang`'s filter grammar (deduced dependencies, `_` placeholder, `FilterKind`, `begin` UI): separate, later plans per the spec.
