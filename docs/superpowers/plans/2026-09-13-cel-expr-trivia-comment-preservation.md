# CEL Expression & adam-lang Trivia Comment Preservation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** No comment is ever lost when reformatting an adam-lang sheet — whether the comment sits
before, after, or between any two tokens, inside a CEL expression body or inside adam-lang's own
declaration syntax (resolves stlab/cel-rs#201).

**Architecture:** Add one shared primitive, `scan_gap`, to `cel-parser`: given the raw source text
of a grammatically-constrained gap (whitespace + comments + a known, fixed sequence of literal
tokens) it returns the interleaved sequence of comments and those tokens in source order.
`cel_parser::format_expr` is reworked to thread the original `source` + an indentation `depth` and
to emit each inter-child gap via `scan_gap` instead of a hardcoded separator; a `//` line comment
found mid-expression forces a line break (the formatter becomes wrapping-capable, but only when a
`//` comment requires it — a block-comment-only or comment-free expression still prints on one
line exactly as today). adam-lang's `fmt.rs` then uses the same primitive for the gaps *within* a
single declaration's tokens, which its existing AST-attachment trivia system (`trivia.rs`) never
covered. That existing system — recovering leading comments before a whole declaration, trailing
comments before a block's closing brace, same-line trailing comments, and blank-line flags — is
left entirely intact; `scan_gap` only fills the disjoint intra-declaration/intra-expression gaps
nothing handled before.

**Tech Stack:** Rust (edition 2024), `cel-parser` (`Expr`/`ExprSpan`/`fmt.rs`), `adam-lang`
(`ast`/`fmt.rs`/`trivia.rs`), `proc-macro2` with the `span-locations` feature (already enabled in
`cel-parser/Cargo.toml`, giving `Span::start()`/`Span::end() -> LineColumn`).

**Spec:** `docs/superpowers/specs/2026-09-13-cel-expr-trivia-and-adam-parser-unification-design.md`
(this plan implements Parts A and A2 only; Part B — the `AdamParser`/`AdamAstParser` unification —
is a separate, later plan.)

## Global Constraints

- `cargo fmt --all` before every commit (enforced by the `.githooks` pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` (incl. `cargo test --doc --workspace`)
  must produce **zero** compiler warnings.
- All three clippy invocations must pass with `-D warnings` (run once at the end, Task 12):
  - `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`
  - `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`
  - `cargo clippy -p begin --all-targets -- -D warnings`
- Every new/changed function (`pub`, `pub(crate)`, or private) needs a contract-style `///` doc
  comment per the workspace `CLAUDE.md` (Summary; `- Precondition:`/`# Errors`/`- Postcondition:`/
  `- Complexity:` bullets as applicable). Preconditions are `debug_assert!`-checked, never
  `Result`-checked; only genuine runtime error conditions return `Err`.
- Tests are derived from each function's contract/public interface, never from reading the
  implementation. Precondition violations are not tested.
- Prefer `&str`/slices over owned `String`/`Vec` clones; the one unavoidable exception is recovered
  comment text, which must be owned (`String`) because it's sliced from a transient gap scan.
- No back-compat shims: this project has no released clients. `format_expr`'s signature change is a
  hard break; every call site is updated in the same task that changes it (Task 3).

## File Structure

- **Create `cel-parser/src/trivia.rs`** — the shared trivia module: the `Comment` enum (moved here
  from `adam-lang::ast`), the `GapPiece` enum, the `scan_gap` primitive, and the
  `LineColumn`→byte-offset helpers (moved here from `adam-lang::trivia`). One responsibility:
  turning raw gap text into an ordered comment/token sequence.
- **Modify `cel-parser/src/lib.rs`** — add `pub mod trivia;` and `pub use trivia::Comment;`.
- **Modify `cel-parser/src/fmt.rs`** — rework `render`/`format_at`/`format_expr` to thread
  `source: &str` + `depth: usize` and emit gaps via `scan_gap`.
- **Modify `adam-lang/src/ast.rs`** — replace the local `Comment` definition with
  `pub use cel_parser::Comment;` (keeps the `ast::Comment` path working everywhere).
- **Modify `adam-lang/src/trivia.rs`** — drop its private `line_start_byte_offsets`/
  `line_column_to_byte` (now `pub(crate)` in `cel_parser::trivia`, imported); keep `analyze_gap`
  and the whole AST-attachment system otherwise unchanged.
- **Modify `adam-lang/src/fmt.rs`** — thread the sheet `source` through `format_sheet` and every
  `write_*`; replace hardcoded intra-declaration separators with `scan_gap` emission.
- **Modify `ez-adam/src/codegen/ast_builder.rs`** — update its 3 `format_expr` call sites to the
  new signature (pass `""` + `0`, since these are hand-built `Expr`s with no real source).

## Interfaces (the whole plan's shared vocabulary)

Defined in Task 1, used throughout:

```rust
// cel-parser/src/trivia.rs — all `pub`, since adam-lang (Tasks 10–11) consumes them cross-crate.
pub enum Comment { Line(String), Block(String) }
pub enum GapPiece { Comment(Comment), Punct(&'static str) }
pub fn scan_gap(gap: &str, expected: &[&'static str]) -> Vec<GapPiece>;
pub fn line_start_byte_offsets(source: &str) -> Vec<usize>;
pub fn line_column_to_byte(source: &str, line_starts: &[usize], pos: proc_macro2::LineColumn) -> usize;
```

Task 2 adds, in `cel-parser/src/fmt.rs`:

```rust
// Emits `pieces` between two already-rendered operands, applying `spacing`, and wrapping
// (newline + continuation indent at `depth`) after any Comment::Line. Returns the joined text.
fn emit_gap(pieces: &[GapPiece], spacing: Spacing, depth: usize) -> String;
enum Spacing { Around, None, CommaAfter } // " + ", "..", ", "
```

`format_expr`'s new signature (Task 3):

```rust
pub fn format_expr(expr: &Expr, source: &str, depth: usize) -> String;
```

- `source` is the exact text `expr` was parsed from (matching `attach_trivia`'s precondition). For
  a hand-built `Expr` with synthetic spans, pass `""`: every gap then resolves to an empty slice,
  `scan_gap` finds no comments and re-synthesizes the expected tokens, and output is identical to
  today's comment-free rendering.
- `depth` is the indentation level (in 4-space units) at which the expression starts, so a
  `//`-comment-forced wrap continues at `depth + 1`.

---

## Task 1: `scan_gap` primitive + `Comment`/offset helpers in `cel-parser`

**Files:**
- Create: `cel-parser/src/trivia.rs`
- Modify: `cel-parser/src/lib.rs` (add `pub mod trivia;` and `pub use trivia::Comment;`)
- Test: inline `#[cfg(test)] mod tests` in `trivia.rs`

**Interfaces:**
- Consumes: `proc_macro2::LineColumn` (already a dependency).
- Produces: `Comment`, `GapPiece`, `scan_gap`, `line_start_byte_offsets`, `line_column_to_byte`
  (exact signatures in the Interfaces section above).

- [ ] **Step 1: Write the failing tests**

Create `cel-parser/src/trivia.rs` with only the module doc, `use`s, and this test module:

```rust
//! Shared trivia recovery: turns the raw source text of a grammatically-constrained gap
//! (whitespace, comments, and a known fixed sequence of literal tokens) into an ordered sequence
//! of [`GapPiece`]s, and converts `proc_macro2` span positions to byte offsets. Used by
//! `cel-parser`'s own expression formatter and by `adam-lang`'s declaration formatter, so both
//! recover comments the same way. See <https://github.com/stlab/cel-rs/issues/201>.

use proc_macro2::LineColumn;

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(pieces: Vec<GapPiece>) -> Vec<Comment> {
        pieces
            .into_iter()
            .filter_map(|p| match p {
                GapPiece::Comment(c) => Some(c),
                GapPiece::Punct(_) => None,
            })
            .collect()
    }

    fn puncts(pieces: &[GapPiece]) -> Vec<&'static str> {
        pieces
            .iter()
            .filter_map(|p| match p {
                GapPiece::Punct(s) => Some(*s),
                GapPiece::Comment(_) => None,
            })
            .collect()
    }

    #[test]
    fn empty_gap_yields_just_the_expected_tokens() {
        let pieces = scan_gap(" ", &["+"]);
        assert_eq!(puncts(&pieces), vec!["+"]);
        assert!(comments(pieces).is_empty());
    }

    #[test]
    fn a_block_comment_before_the_token_is_recovered_in_order() {
        let pieces = scan_gap(" /* a */ + ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "a"));
        assert!(matches!(pieces[1], GapPiece::Punct("+")));
    }

    #[test]
    fn a_block_comment_after_the_token_is_recovered_in_order() {
        let pieces = scan_gap(" + /* b */ ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Punct("+")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Block(ref t)) if t == "b"));
    }

    #[test]
    fn comments_on_both_sides_of_the_token_keep_source_order() {
        let pieces = scan_gap(" /* a */ + /* b */ ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "a"));
        assert!(matches!(pieces[1], GapPiece::Punct("+")));
        assert!(matches!(pieces[2], GapPiece::Comment(Comment::Block(ref t)) if t == "b"));
    }

    #[test]
    fn two_consecutive_block_comments_are_two_pieces() {
        let pieces = scan_gap(" /* a */ /* b */ + ", &["+"]);
        assert_eq!(comments(pieces.clone()).len(), 2);
        assert!(matches!(pieces[2], GapPiece::Punct("+")));
    }

    #[test]
    fn a_line_comment_is_recovered_as_comment_line() {
        let pieces = scan_gap(" + // hi\n", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Punct("+")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Line(ref t)) if t == "hi"));
    }

    #[test]
    fn a_multi_line_block_comment_joins_its_inner_lines() {
        let pieces = scan_gap(" /*\n  one\n  two\n*/ + ", &["+"]);
        assert!(matches!(pieces[0], GapPiece::Comment(Comment::Block(ref t)) if t == "one\ntwo"));
    }

    #[test]
    fn multiple_expected_tokens_are_returned_in_order() {
        // Between a callee and its first arg with a trailing comma case:
        let pieces = scan_gap(" ( /* c */ ", &["("]);
        assert!(matches!(pieces[0], GapPiece::Punct("(")));
        assert!(matches!(pieces[1], GapPiece::Comment(Comment::Block(ref t)) if t == "c"));
    }

    #[test]
    fn missing_expected_token_is_still_synthesized() {
        // A synthetic/empty gap (hand-built Expr) must still yield every expected token.
        let pieces = scan_gap("", &["(", ")"]);
        assert_eq!(puncts(&pieces), vec!["(", ")"]);
    }

    #[test]
    fn line_column_to_byte_maps_a_position_on_the_second_line() {
        let source = "ab\ncd";
        let starts = line_start_byte_offsets(source);
        // line 2 (1-based), column 1 (0-based) => the 'd', byte offset 4.
        let pos = LineColumn { line: 2, column: 1 };
        assert_eq!(line_column_to_byte(source, &starts, pos), 4);
    }
}
```

Add `#[derive(Clone)]` on `GapPiece`/`Comment` as the tests' `.clone()` requires.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser trivia::`
Expected: FAIL to compile — `Comment`/`GapPiece`/`scan_gap`/`line_start_byte_offsets`/
`line_column_to_byte` don't exist yet.

- [ ] **Step 3: Write the implementation**

Add above the test module in `cel-parser/src/trivia.rs`:

```rust
/// A recovered `//`/`/* */` comment, remembering which delimiter style the source used so a
/// formatter can reproduce it instead of normalizing every comment to one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Comment {
    /// A single `// text` line, its leading `//` and surrounding whitespace stripped. A
    /// multi-line block recovered as `Line` (consecutive `//` lines) joins them with `\n`.
    Line(String),
    /// A `/* text */` block comment (single- or multi-line), inner text `\n`-joined with the
    /// `/*`/`*/` delimiters and per-line indentation stripped.
    Block(String),
}

/// One element of a scanned gap: either a recovered comment or one of the literal tokens the
/// caller declared it expected to find there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GapPiece {
    /// A comment found in the gap.
    Comment(Comment),
    /// One expected literal token (an operator symbol, delimiter, comma, or keyword).
    Punct(&'static str),
}

/// Scans the raw text of a grammatically-constrained gap — whitespace, `//`/`/* */` comments, and
/// exactly the literal tokens in `expected`, in that order — returning every comment found plus
/// every `expected` token, interleaved in source order.
///
/// - Postcondition: the result contains each element of `expected` exactly once, as a
///   `GapPiece::Punct`, in `expected`'s order, regardless of how much of `gap` actually matched
///   (an empty or synthetic `gap` still yields all expected tokens, so a hand-built `Expr` with no
///   real source re-synthesizes its separators).
///
/// - Complexity: O(n) in `gap.len()`.
pub(crate) fn scan_gap(gap: &str, expected: &[&'static str]) -> Vec<GapPiece> {
    let mut pieces = Vec::new();
    let mut next = 0usize;
    let mut rest = gap;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix("//") {
            let end = after.find('\n').unwrap_or(after.len());
            pieces.push(GapPiece::Comment(Comment::Line(after[..end].trim().to_string())));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix("/*") {
            let close = after.find("*/").unwrap_or(after.len());
            pieces.push(GapPiece::Comment(Comment::Block(normalize_block(&after[..close]))));
            rest = after.get(close + 2..).unwrap_or("");
        } else if next < expected.len() && rest.starts_with(expected[next]) {
            pieces.push(GapPiece::Punct(expected[next]));
            rest = &rest[expected[next].len()..];
            next += 1;
        } else {
            // The gap didn't match the next expected token (synthetic/empty source, or an
            // unforeseen shape). Stop scanning; remaining expected tokens are synthesized below.
            break;
        }
    }
    while next < expected.len() {
        pieces.push(GapPiece::Punct(expected[next]));
        next += 1;
    }
    pieces
}

/// Normalizes a block comment's inner text (between `/*` and `*/`): trims each line and joins the
/// non-empty ones with `\n`, matching `adam-lang`'s existing `Comment::Block` convention.
///
/// - Complexity: O(n) in `inner.len()`.
fn normalize_block(inner: &str) -> String {
    inner
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the byte offset of the start of each line in `source`: `result[line - 1]` is the start
/// of 1-based line `line` (matching [`proc_macro2::LineColumn::line`]'s convention).
///
/// - Complexity: O(n) in `source.len()`.
pub(crate) fn line_start_byte_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    let mut byte = 0usize;
    for line in source.split_inclusive('\n') {
        byte += line.len();
        offsets.push(byte);
    }
    offsets
}

/// Converts a [`LineColumn`] (1-based line, 0-based character column) to a byte offset in
/// `source`, using `line_starts` (from [`line_start_byte_offsets`]).
///
/// - Precondition: `line_starts` was built from exactly `source`, and `pos.line - 1` is in range.
///
/// - Complexity: O(k) in `pos.column`.
pub(crate) fn line_column_to_byte(source: &str, line_starts: &[usize], pos: LineColumn) -> usize {
    let line_start = line_starts[pos.line - 1];
    line_start
        + source[line_start..]
            .chars()
            .take(pos.column)
            .map(char::len_utf8)
            .sum::<usize>()
}
```

- [ ] **Step 4: Wire the module into the crate root**

In `cel-parser/src/lib.rs`, add alongside the other `pub mod`/`pub use` lines:

```rust
pub mod trivia;
pub use trivia::Comment;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cel-parser trivia::`
Expected: PASS (all tests in this task).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add cel-parser/src/trivia.rs cel-parser/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add scan_gap trivia primitive and Comment type

scan_gap turns a grammatically-constrained gap's raw text (whitespace,
comments, and a known fixed token sequence) into an ordered
comment/token piece list, always re-synthesizing expected tokens so a
hand-built Expr with no source still round-trips. Groundwork for
recovering comments inside CEL expressions (#201).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: `emit_gap` + `Spacing` helper in `cel-parser/src/fmt.rs`

**Files:**
- Modify: `cel-parser/src/fmt.rs` (add `emit_gap`/`Spacing`; add `use` for `scan_gap`/`GapPiece`)
- Test: inline additions to `cel-parser/src/fmt.rs`'s `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `GapPiece`/`Comment` (Task 1).
- Produces: `fn emit_gap(pieces: &[GapPiece], spacing: Spacing, depth: usize) -> String` and
  `enum Spacing { Around, None, CommaAfter }`, used by every render arm in Tasks 4–9.

`emit_gap` renders the piece sequence *between* two operands (the operands themselves are emitted
by the caller). Spacing rules:
- `Spacing::Around` — a single space on each side of a `Punct` (binary operators: `a + b`).
- `Spacing::None` — no spaces around a `Punct` (range operators/delimiters: `a..b`, `(`, `)`).
- `Spacing::CommaAfter` — `Punct` then a space (list separators: `a, b`).
- A `Comment::Block` is emitted inline, space-separated from its neighbors.
- A `Comment::Line` is emitted, then a newline and `indent(depth + 1)` (the wrap continuation),
  since a `//` comment always ends its line.

- [ ] **Step 1: Write the failing tests**

Add to `cel-parser/src/fmt.rs`'s test module:

```rust
    #[test]
    fn emit_gap_around_a_plain_operator_uses_single_spaces() {
        let pieces = vec![GapPiece::Punct("+")];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + ");
    }

    #[test]
    fn emit_gap_none_spacing_glues_a_range_operator() {
        let pieces = vec![GapPiece::Punct("..")];
        assert_eq!(emit_gap(&pieces, Spacing::None, 0), "..");
    }

    #[test]
    fn emit_gap_comma_after_puts_one_trailing_space() {
        let pieces = vec![GapPiece::Punct(",")];
        assert_eq!(emit_gap(&pieces, Spacing::CommaAfter, 0), ", ");
    }

    #[test]
    fn emit_gap_inlines_a_block_comment_before_an_operator() {
        let pieces = vec![
            GapPiece::Comment(Comment::Block("a".to_string())),
            GapPiece::Punct("+"),
        ];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " /* a */ + ");
    }

    #[test]
    fn emit_gap_inlines_a_block_comment_after_an_operator() {
        let pieces = vec![
            GapPiece::Punct("+"),
            GapPiece::Comment(Comment::Block("b".to_string())),
        ];
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + /* b */ ");
    }

    #[test]
    fn emit_gap_wraps_after_a_line_comment_to_the_continuation_indent() {
        let pieces = vec![
            GapPiece::Punct("+"),
            GapPiece::Comment(Comment::Line("why".to_string())),
        ];
        // depth 0 => continuation at depth 1 (4 spaces). The operator keeps its leading space; the
        // line comment ends the line, and the next operand resumes at the continuation indent.
        assert_eq!(emit_gap(&pieces, Spacing::Around, 0), " + // why\n    ");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser fmt::tests::emit_gap`
Expected: FAIL to compile — `emit_gap`/`Spacing` don't exist.

- [ ] **Step 3: Write the implementation**

At the top of `cel-parser/src/fmt.rs`, extend the imports:

```rust
use crate::ast::{Expr, LogicalOp};
use crate::trivia::{Comment, GapPiece, line_column_to_byte, line_start_byte_offsets, scan_gap};
```

Add `indent`/`emit_gap`/`Spacing` (place near the top, after the `Level` block):

```rust
/// 4 spaces per nesting level (mirrors `adam-lang::fmt::indent`).
fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// How a gap's expected `Punct` tokens are spaced against their operands.
#[derive(Clone, Copy)]
enum Spacing {
    /// A single space on each side of the token: `a + b`.
    Around,
    /// No surrounding spaces: `a..b`, or a bare delimiter.
    None,
    /// The token then one space: `a, b`.
    CommaAfter,
}

/// Renders the pieces of one scanned gap into the text that goes *between* two already-rendered
/// operands. A `Comment::Block` is inlined space-separated; a `Comment::Line` ends its line and
/// resumes at `indent(depth + 1)` (the wrap continuation), since a `//` comment runs to
/// end-of-line. A `Punct` is spaced per `spacing`.
///
/// - Postcondition: the returned string starts and ends exactly as `spacing` dictates for the
///   comment-free case (e.g. `Spacing::Around` with a lone `Punct` returns `" + "`), so a
///   comment-free gap reprints identically to the pre-`scan_gap` formatter.
///
/// - Complexity: O(n) in `pieces.len()` plus their text lengths.
fn emit_gap(pieces: &[GapPiece], spacing: Spacing, depth: usize) -> String {
    let mut out = String::new();
    let cont = indent(depth + 1);
    for piece in pieces {
        match piece {
            GapPiece::Punct(tok) => match spacing {
                Spacing::Around => {
                    out.push(' ');
                    out.push_str(tok);
                    out.push(' ');
                }
                Spacing::None => out.push_str(tok),
                Spacing::CommaAfter => {
                    out.push_str(tok);
                    out.push(' ');
                }
            },
            GapPiece::Comment(Comment::Block(text)) => {
                // Trim any trailing space the previous piece left, add one, inline the block, add
                // one, so `/* a */` never doubles or drops spaces at either edge.
                if !out.ends_with(' ') && !out.is_empty() {
                    out.push(' ');
                }
                out.push_str("/* ");
                out.push_str(text);
                out.push_str(" */ ");
            }
            GapPiece::Comment(Comment::Line(text)) => {
                let trimmed = out.trim_end().to_string();
                out = trimmed;
                out.push_str(" // ");
                out.push_str(text);
                out.push('\n');
                out.push_str(&cont);
            }
        }
    }
    out
}
```

Note: the `emit_gap_wraps_after_a_line_comment_to_the_continuation_indent` test expects
`" + // why\n    "`. Trace: `Punct("+")` with `Around` → `" + "`; then `Line("why")` trims to
`" +"`, appends `" // why\n"` + `indent(1)` (`"    "`) → `" + // why\n    "`. Matches.

For the block-comment-before-operator test (`" /* a */ + "`): first piece `Block("a")` with empty
`out` → the `!out.is_empty()` guard skips the leading space, giving `"/* a */ "`... but the test
expects a **leading** space (`" /* a */ + "`). Fix: seed `out` with a single leading space when
the first piece is anything, matching the `Around`/`CommaAfter` convention that a gap opens with a
space. Adjust the implementation: before the loop, for `Spacing::Around`, the natural leading space
comes from the first `Punct`; when the gap *starts* with a comment, prepend a space. Implement by
initializing `out` to `" "` when `spacing` is `Around` and the first piece is a `Comment`:

```rust
    let mut out = String::new();
    if matches!(spacing, Spacing::Around) && matches!(pieces.first(), Some(GapPiece::Comment(_))) {
        out.push(' ');
    }
```

Re-trace `" /* a */ + "`: seed `" "`; `Block("a")`: `out` ends with `' '` so no extra space →
`" /* a */ "`; `Punct("+")` Around → `" /* a */  + "` — double space. Refine the block arm to not
add its own trailing space when the next piece is a spaced `Punct`; simplest correct rule: build
the whole thing by joining tokens with single spaces and special-casing `None`/`CommaAfter`. Given
the fiddliness, implement `emit_gap` as: produce a `Vec<String>` of rendered fragments, then join.
Replace the body with:

```rust
fn emit_gap(pieces: &[GapPiece], spacing: Spacing, depth: usize) -> String {
    let cont = indent(depth + 1);
    // Render each piece to a fragment, tracking whether a Line comment forced a wrap.
    let mut out = String::new();
    let mut pending_wrap = false;
    for (i, piece) in pieces.iter().enumerate() {
        let first = i == 0;
        match piece {
            GapPiece::Punct(tok) => {
                match spacing {
                    Spacing::Around => {
                        if !pending_wrap {
                            out.push(' ');
                        }
                        out.push_str(tok);
                        out.push(' ');
                    }
                    Spacing::None => out.push_str(tok),
                    Spacing::CommaAfter => {
                        out.push_str(tok);
                        out.push(' ');
                    }
                }
                pending_wrap = false;
            }
            GapPiece::Comment(Comment::Block(text)) => {
                if first && matches!(spacing, Spacing::Around) {
                    out.push(' ');
                }
                out.push_str("/* ");
                out.push_str(text);
                out.push_str(" */ ");
                pending_wrap = false;
            }
            GapPiece::Comment(Comment::Line(text)) => {
                let trimmed_len = out.trim_end().len();
                out.truncate(trimmed_len);
                out.push_str(" // ");
                out.push_str(text);
                out.push('\n');
                out.push_str(&cont);
                pending_wrap = true;
            }
        }
    }
    out
}
```

Re-trace all six tests against this body and confirm each equals its expected string before
running. (`Around`+`Punct` only: `" + "`. `None`+`Punct`: `".."`. `CommaAfter`+`Punct`: `", "`.
`Block`,`Punct`: `" /* a */ + "`. `Punct`,`Block`: `" + /* b */ "`. `Punct`,`Line`: `" + // why\n    "`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::tests::emit_gap`
Expected: PASS (6 tests). If any fragment mismatches, fix `emit_gap` (not the test) until exact.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): add emit_gap for rendering scanned expression gaps

emit_gap turns a scanned gap's pieces into the inter-operand text,
inlining block comments, and wrapping to a continuation indent after a
// line comment. Not yet wired into render (next task). Groundwork for #201.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Thread `source` + `depth` through `format_expr`/`format_at`/`render` (no behavior change)

This task changes signatures and all call sites, but every render arm keeps emitting exactly as
today (it does not yet call `scan_gap`/`emit_gap`). Every existing test keeps passing with output
unchanged — this isolates the mechanical ripple from the behavioral change.

**Files:**
- Modify: `cel-parser/src/fmt.rs` (signatures of `format_expr`, `format_at`, `render`; all internal
  recursion; the doctest; the crate's own `#[cfg(test)]` call sites)
- Modify: `adam-lang/src/fmt.rs` (every `cel_parser::format_expr(x)` → `cel_parser::format_expr(x, source, depth)`)
- Modify: `ez-adam/src/codegen/ast_builder.rs` (3 call sites → pass `""`, `0`)

**Interfaces:**
- Produces: `pub fn format_expr(expr: &Expr, source: &str, depth: usize) -> String`, consumed by
  Tasks 4–9 (cel-parser) and Task 10/11 (adam-lang).

- [ ] **Step 1: Change the three signatures in `cel-parser/src/fmt.rs`**

`render(expr: &Expr) -> (String, Level)` → `render(expr: &Expr, source: &str, depth: usize) -> (String, Level)`.
`format_at(expr: &Expr, min_level: Level) -> String` → `format_at(expr: &Expr, source: &str, depth: usize, min_level: Level) -> String`.
`format_expr(expr: &Expr) -> String` → `format_expr(expr: &Expr, source: &str, depth: usize) -> String`.

Thread the two new args through every recursive call inside `render`/`format_at` unchanged (e.g.
`format_at(lhs, level)` → `format_at(lhs, source, depth, level)`; `render(else_branch)` →
`render(else_branch, source, depth)`). Do **not** yet change what any arm emits. Update the bodies:

```rust
fn format_at(expr: &Expr, source: &str, depth: usize, min_level: Level) -> String {
    let (text, level) = render(expr, source, depth);
    if level < min_level {
        format!("({text})")
    } else {
        text
    }
}

pub fn format_expr(expr: &Expr, source: &str, depth: usize) -> String {
    format_at(expr, source, depth, Level::RANGE)
}
```

Update `format_expr`'s doc comment and its doctest example:

```rust
/// let expr = parser.parse_str_ast("(1i32 + 2i32) * 3i32").unwrap();
/// assert_eq!(format_expr(&expr, "(1i32 + 2i32) * 3i32", 0), "(1i32 + 2i32) * 3i32");
```

Add a `source`/`depth` sentence to the summary contract: `source` is the text `expr` was parsed
from (or `""` for a hand-built `Expr`); `depth` is the indent level for `//`-comment wrap
continuations.

- [ ] **Step 2: Update `cel-parser/src/fmt.rs`'s own test call sites**

Every `format_expr(&expr)` / `format_expr(&parse(source))` in the test module becomes
`format_expr(&expr, source, 0)`. The `parse` helper currently takes `source: &str` and returns
`Expr`; change the tests to keep `source` in scope and pass it. For the two hand-built-tree tests
(`a_right_leaning_tree_at_the_same_precedence_needs_parens`,
`nested_comparison_needs_parens_on_both_sides`), pass `""` as source (synthetic spans, no real
source) — output is unchanged since those trees have no recoverable gaps.

Concretely, change the `parse` helper and add a paired accessor so tests can pass source:

```rust
    fn parse(source: &str) -> Expr {
        Parser::<AstContext>::new(OpLookup::new())
            .parse_str_ast(source)
            .unwrap()
    }

    fn fmt(source: &str) -> String {
        format_expr(&parse(source), source, 0)
    }
```

Then mechanically replace `format_expr(&parse(source), ...)`-style calls with `fmt(source)`, and
inline `format_expr(&parse("..."))` calls with `fmt("...")`. For the two hand-built-tree tests,
replace `format_expr(&expr)` with `format_expr(&expr, "", 0)`. For
`format_is_idempotent_through_a_reparse`, becomes:

```rust
        let source = "(1i32 + 2i32) * 3i32 - -4i32";
        let once = fmt(source);
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice);
```

- [ ] **Step 3: Update `adam-lang/src/fmt.rs` call sites**

`adam-lang`'s `format_sheet` has no `source` today; the trivia system runs separately via
`attach_trivia(source, &mut sheet)`. To pass `source` into `format_expr`, thread it through
`format_sheet` and every `write_*`. Change `format_sheet`'s signature:

```rust
pub fn format_sheet(sheet: &ast::Sheet, source: &str) -> String {
```

and thread `source` as a new first-after-`out` parameter into `write_sheet_item`, `write_cell`,
`write_source`, `write_out`, `write_relationship`, `write_binding`, `write_conditional`,
`write_branch`, `write_branch_relationships`, `write_requirement`, `write_require_clause`. At each
existing `cel_parser::format_expr(expr)` call, pass `(expr, source, depth)` — using that call
site's own `depth`. (There are 8 such calls: `write_binding` body; `write_conditional` match_expr;
`write_cell` initializer and filter; `write_out` initializer and filter; `write_source`
initializer and filter; `write_requirement` body. Confirm the exact set by searching the file for
`cel_parser::format_expr` before editing.)

Update `format_source` (same file) to pass `source`:

```rust
    crate::attach_trivia(source, &mut sheet);
    Ok(format_sheet(&sheet, source))
```

Update `format_sheet`'s doctest and every `format(source)` test helper call — the test helper
already has `source` in scope:

```rust
    fn format(source: &str) -> String {
        let mut sheet = AdamAstParser::new().parse_str(source).unwrap();
        crate::attach_trivia(source, &mut sheet);
        format_sheet(&sheet, source)
    }
```

and the two direct `format_sheet(&sheet)` test calls (`formats_a_filter`,
`format_sheet` doctest) pass their source string.

- [ ] **Step 4: Update `ez-adam/src/codegen/ast_builder.rs` call sites**

Its 3 `cel_parser::format_expr(&x)` calls format hand-built `Expr`s with synthetic spans. Change
each to `cel_parser::format_expr(&x, "", 0)`.

- [ ] **Step 5: Build and run the full affected test suites**

Run:
```bash
cargo build -p cel-parser -p adam-lang -p ez-adam
cargo test -p cel-parser -p adam-lang -p ez-adam
cargo test --doc -p cel-parser -p adam-lang
```
Expected: everything compiles and **every existing test passes with identical output** — this task
is behavior-preserving. If any output changed, a threading edit accidentally altered emission; fix
it, don't update the test.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs adam-lang/src/fmt.rs ez-adam/src/codegen/ast_builder.rs
git commit -m "$(cat <<'EOF'
refactor: thread source + depth through format_expr (no behavior change)

format_expr now takes the original source text and an indent depth,
threaded through render/format_at and every adam-lang write_*. No arm
uses them yet; output is byte-for-byte identical. Isolates the
mechanical signature ripple ahead of wiring in comment recovery (#201).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Recover comments in binary-operator gaps (the architectural proof)

This is the first arm to actually use `scan_gap`/`emit_gap`, proving the end-to-end recovery +
wrapping model on the most common case before the remaining arms follow the same shape.

**Files:**
- Modify: `cel-parser/src/fmt.rs` (the general binary `Expr::Op` arm — the last arm, currently
  `format!("{lhs_s} {name} {rhs_s}")`)
- Test: inline additions to the `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `scan_gap`, `emit_gap`, `Spacing`, `line_start_byte_offsets`, `line_column_to_byte`.
- Produces: a private helper `gap_between(source, prev_end, next_start, expected) -> Vec<GapPiece>`
  used by every later arm.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_block_comment_before_a_binary_operator_is_preserved() {
        let source = "1i32 /* a */ + 2i32";
        assert_eq!(fmt(source), "1i32 /* a */ + 2i32");
    }

    #[test]
    fn a_block_comment_after_a_binary_operator_is_preserved() {
        let source = "1i32 + /* b */ 2i32";
        assert_eq!(fmt(source), "1i32 + /* b */ 2i32");
    }

    #[test]
    fn comments_on_both_sides_of_a_binary_operator_are_preserved() {
        let source = "1i32 /* a */ + /* b */ 2i32";
        assert_eq!(fmt(source), "1i32 /* a */ + /* b */ 2i32");
    }

    #[test]
    fn a_line_comment_after_a_binary_operator_wraps_the_expression() {
        let source = "1i32 +\n    // why 2\n    2i32";
        assert_eq!(fmt(source), "1i32 + // why 2\n    2i32");
    }

    #[test]
    fn a_comment_free_binary_op_still_prints_on_one_line() {
        assert_eq!(fmt("1i32 + 2i32 * 3i32"), "1i32 + 2i32 * 3i32");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cel-parser fmt::tests::`
Expected: the four comment tests FAIL (comments dropped today); the one-line regression PASSES.

- [ ] **Step 3: Add `gap_between` and rewrite the binary `Expr::Op` arm**

Add near `render_literal`:

```rust
/// Scans the source gap between two adjacent AST positions for comments interleaved with
/// `expected` tokens. Returns just the expected tokens (no comments) when `source` is empty or the
/// positions don't resolve within it (a hand-built `Expr`), so a source-less format still works.
///
/// - Complexity: O(n) in the gap's length.
fn gap_between(
    source: &str,
    prev_end: proc_macro2::Span,
    next_start: proc_macro2::Span,
    expected: &[&'static str],
) -> Vec<GapPiece> {
    if source.is_empty() {
        return scan_gap("", expected);
    }
    let line_starts = line_start_byte_offsets(source);
    let start = line_column_to_byte(source, &line_starts, prev_end.end());
    let end = line_column_to_byte(source, &line_starts, next_start.start());
    let gap = source.get(start..end).unwrap_or("");
    scan_gap(gap, expected)
}
```

Rewrite the general binary arm (currently the final `Expr::Op { name, operands, .. }` arm):

```rust
        Expr::Op { name, operands, span } => {
            let level = binary_op_level(name);
            let (lhs_min, rhs_min) = if level == Level::COMPARISON {
                (level.tighter(), level.tighter())
            } else {
                (level, level.tighter())
            };
            let lhs_s = format_at(&operands[0], source, depth, lhs_min);
            let rhs_s = format_at(&operands[1], source, depth, rhs_min);
            // The operator token itself has no span; it lives in the gap between the two operands.
            let op_static = binary_op_token(name);
            let pieces = gap_between(
                source,
                operands[0].span().end,
                operands[1].span().start,
                &[op_static],
            );
            let _ = span;
            (format!("{lhs_s}{}{rhs_s}", emit_gap(&pieces, Spacing::Around, depth)), level)
        }
```

`emit_gap`/`scan_gap` need `&'static str` tokens, but `name` is a `String`. Add a mapper from the
operator name to its static token text (identical text, but `'static`):

```rust
/// Returns the `'static` source spelling of a binary operator name, for `scan_gap`'s expected-token
/// list (which needs `&'static str`). The spelling equals `name` for every binary operator this
/// formatter's `binary_op_level` accepts.
fn binary_op_token(name: &str) -> &'static str {
    match name {
        "|" => "|", "^" => "^", "&" => "&", "<<" => "<<", ">>" => ">>",
        "+" => "+", "-" => "-", "*" => "*", "/" => "/", "%" => "%",
        "==" => "==", "!=" => "!=", "<" => "<", ">" => ">", "<=" => "<=", ">=" => ">=",
        other => unreachable!("binary_op_token called with unknown operator `{other}`"),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: PASS — the four comment tests now recover, and every pre-existing binary-op test still
prints identically (a comment-free gap yields `emit_gap(&[Punct(op)], Around, _) == " op "`).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): preserve comments in binary-operator gaps (#201)

Rewrites the binary Expr::Op arm to recover comments from the source
gap between its operands via scan_gap/emit_gap, wrapping the expression
when a // line comment forces it. Proves the recovery+wrap model before
the remaining arms follow. Comment-free binary ops print unchanged.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Recover comments in unary, cast, and range-operator gaps

**Files:**
- Modify: `cel-parser/src/fmt.rs` (the arity-1 `Expr::Op` arm; the `range_from`/`range_to`/
  `range_to_inclusive` arm; the `range`/`range_inclusive` arm; the `Expr::Cast` arm)
- Test: inline additions

**Interfaces:** Consumes `gap_between`/`emit_gap`/`Spacing` (Task 4). Produces nothing new.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_comment_after_unary_minus_is_preserved() {
        assert_eq!(fmt("- /* neg */ 1i32"), "- /* neg */ 1i32");
    }

    #[test]
    fn a_comment_around_a_cast_operator_is_preserved() {
        assert_eq!(fmt("x /* c */ as i32"), "x /* c */ as i32");
    }

    #[test]
    fn a_comment_inside_a_range_is_preserved() {
        assert_eq!(fmt("1i32 /* r */ ..5i32"), "1i32 /* r */ ..5i32");
    }

    #[test]
    fn comment_free_unary_cast_and_range_are_unchanged() {
        assert_eq!(fmt("-1i32"), "-1i32");
        assert_eq!(fmt("x as i32"), "x as i32");
        assert_eq!(fmt("1i32..5i32"), "1i32..5i32");
    }
```

- [ ] **Step 2: Run to verify the comment tests fail, the regression passes**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 3: Rewrite the four arms**

**Arity-1 (`-x`, `!x`) arm** — the operator precedes its single operand, so its gap is between the
node's own `span.start` and the operand's `span().start`:

```rust
        Expr::Op { name, operands, span } if operands.len() == 1 => {
            let operand_s = format_at(&operands[0], source, depth, Level::UNARY);
            let op_static = unary_op_token(name);
            let pieces = gap_between(source, span.start, operands[0].span().start, &[op_static]);
            // Preserve the existing "- -1" disambiguating space when there are no comments.
            let default_sep = if operand_s.starts_with('-') || operand_s.starts_with('!') {
                " "
            } else {
                ""
            };
            let gap = if pieces.iter().any(|p| matches!(p, GapPiece::Comment(_))) {
                emit_gap(&pieces, Spacing::None, depth)
            } else {
                format!("{op_static}{default_sep}")
            };
            (format!("{gap}{operand_s}"), Level::UNARY)
        }
```

Add:

```rust
/// Returns the `'static` source spelling of a unary operator name (`"-"` or `"!"`).
fn unary_op_token(name: &str) -> &'static str {
    match name {
        "-" => "-",
        "!" => "!",
        other => unreachable!("unary_op_token called with unknown operator `{other}`"),
    }
}
```

Note: `Spacing::None` renders the operator with no surrounding spaces (`-`), and any comment in the
gap is inlined; the double-`-` disambiguating space only matters in the comment-free path, handled
by `default_sep`.

**`range_from`/`range_to`/`range_to_inclusive` arm** (arity 1) — for `range_from` (`x..`) the
`..` follows the operand (gap: operand end → node span end); for `range_to`/`range_to_inclusive`
(`..y`, `..=y`) it precedes (gap: node span start → operand start). Rewrite:

```rust
        Expr::Op { name, operands, span }
            if operands.len() == 1
                && matches!(name.as_str(), "range_from" | "range_to" | "range_to_inclusive") =>
        {
            let operand_s = format_at(&operands[0], source, depth, Level::RANGE.tighter());
            let text = match name.as_str() {
                "range_from" => {
                    let pieces = gap_between(source, operands[0].span().end, span.end, &[".."]);
                    format!("{operand_s}{}", emit_gap(&pieces, Spacing::None, depth))
                }
                "range_to" => {
                    let pieces = gap_between(source, span.start, operands[0].span().start, &[".."]);
                    format!("{}{operand_s}", emit_gap(&pieces, Spacing::None, depth))
                }
                "range_to_inclusive" => {
                    let pieces = gap_between(source, span.start, operands[0].span().start, &["..="]);
                    format!("{}{operand_s}", emit_gap(&pieces, Spacing::None, depth))
                }
                _ => unreachable!("guarded by the outer match arm"),
            };
            (text, Level::RANGE)
        }
```

**`range`/`range_inclusive` arm** (arity 2) — the `..`/`..=` sits between the two operands:

```rust
        Expr::Op { name, operands, .. } if name == "range" || name == "range_inclusive" => {
            let lhs_s = format_at(&operands[0], source, depth, Level::RANGE.tighter());
            let rhs_s = format_at(&operands[1], source, depth, Level::RANGE.tighter());
            let op_static: &'static str = if name == "range_inclusive" { "..=" } else { ".." };
            let pieces = gap_between(
                source,
                operands[0].span().end,
                operands[1].span().start,
                &[op_static],
            );
            (format!("{lhs_s}{}{rhs_s}", emit_gap(&pieces, Spacing::None, depth)), Level::RANGE)
        }
```

**`Expr::Cast` arm** — the `as` keyword sits between the operand and the type name (the type name
has no `Expr` node; it's `type_name: String`). The gap is operand end → node span end, but that
gap contains `as <typename>`. Since we want comments only *around* `as` (a comment after the type
name would be outside the cast node), scan for `["as"]` in the gap between the operand and the type
name. The type name's own start isn't a span we have, so scan the whole operand-end → cast-span-end
gap expecting `["as"]` and append the type name after:

```rust
        Expr::Cast { expr, type_name, span } => {
            let expr_s = format_at(expr, source, depth, Level::CAST);
            let pieces = gap_between(source, expr.span().end, span.end, &["as"]);
            // Everything up to and including `as` (plus any comments) comes from the gap; the type
            // name is appended after a single space.
            (format!("{expr_s}{} {type_name}", emit_gap(&pieces, Spacing::Around, depth)), Level::CAST)
        }
```

Note the `emit_gap(..., Around, ...)` for `["as"]` yields `" as "`; appending `" {type_name}"`
would double the space. Use `Spacing::Around` but drop the trailing space by trimming: append
`emit_gap(...).trim_end()` then `" {type_name}"`. Concretely:

```rust
            let gap = emit_gap(&pieces, Spacing::Around, depth);
            (format!("{expr_s}{} {type_name}", gap.trim_end()), Level::CAST)
```

Confirm the comment-free case: `gap == " as "`, `trim_end() == " as"`, result `"{expr} as {type}"`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: PASS — comment recovery works for all four arms, and every pre-existing unary/cast/range
test still prints identically.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): preserve comments in unary/cast/range gaps (#201)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 6: Recover comments in logical-operator gaps

**Files:** Modify `cel-parser/src/fmt.rs` (the `Expr::Logical` arm). Test: inline additions.

**Interfaces:** Consumes `gap_between`/`emit_gap`. Produces nothing new.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_comment_around_a_logical_operator_is_preserved() {
        assert_eq!(fmt("a /* x */ || b"), "a /* x */ || b");
        assert_eq!(fmt("a && /* y */ b"), "a && /* y */ b");
    }

    #[test]
    fn comment_free_logical_is_unchanged() {
        assert_eq!(fmt("a || b && c"), "a || b && c");
    }
```

- [ ] **Step 2: Run to verify the comment tests fail, the regression passes**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 3: Rewrite the `Expr::Logical` arm**

```rust
        Expr::Logical { op, lhs, rhs, .. } => {
            let level = match op {
                LogicalOp::Or => Level::OR,
                LogicalOp::And => Level::AND,
            };
            let op_static: &'static str = match op {
                LogicalOp::Or => "||",
                LogicalOp::And => "&&",
            };
            let lhs_s = format_at(lhs, source, depth, level);
            let rhs_s = format_at(rhs, source, depth, level.tighter());
            let pieces = gap_between(source, lhs.span().end, rhs.span().start, &[op_static]);
            (format!("{lhs_s}{}{rhs_s}", emit_gap(&pieces, Spacing::Around, depth)), level)
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): preserve comments in logical-operator gaps (#201)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 7: Recover comments in call, tuple, and tuple-index gaps

**Files:** Modify `cel-parser/src/fmt.rs` (the `Expr::Apply`, `Expr::Tuple`, `Expr::TupleIndex`
arms). Test: inline additions.

**Interfaces:** Consumes `gap_between`/`emit_gap`. Produces a helper `emit_list` for
comma-separated element lists with per-gap comment recovery.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn comments_in_a_call_argument_list_are_preserved() {
        assert_eq!(fmt("f(/* a */ 1i32, /* b */ 2i32)"), "f(/* a */ 1i32, /* b */ 2i32)");
    }

    #[test]
    fn a_comment_before_a_closing_call_paren_is_preserved() {
        assert_eq!(fmt("f(1i32 /* end */)"), "f(1i32 /* end */)");
    }

    #[test]
    fn comments_in_a_tuple_are_preserved() {
        assert_eq!(fmt("(1i32, /* mid */ 2i32)"), "(1i32, /* mid */ 2i32)");
    }

    #[test]
    fn comment_free_calls_and_tuples_are_unchanged() {
        assert_eq!(fmt("f(1i32, 2i32)"), "f(1i32, 2i32)");
        assert_eq!(fmt("(1i32, 2i32)"), "(1i32, 2i32)");
        assert_eq!(fmt("(1i32,)"), "(1i32,)");
        assert_eq!(fmt("(1i32, 2i32).1"), "(1i32, 2i32).1");
    }
```

- [ ] **Step 2: Run to verify the comment tests fail, the regressions pass**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 3: Add `emit_list` and rewrite the three arms**

Add helper:

```rust
/// Renders a comma-separated element list with per-gap comment recovery: the gap before the first
/// element (after `open_start`, e.g. a `(`), each inter-element `,` gap, and the gap after the last
/// element (before `close_end`, e.g. a `)`). `open`/`close` are the delimiter tokens. When
/// `trailing_comma` is set (a 1-tuple), a `,` is emitted after the sole element.
///
/// - Complexity: O(n) in the number of elements plus their gap lengths.
fn emit_list(
    source: &str,
    depth: usize,
    open: &'static str,
    close: &'static str,
    open_start: proc_macro2::Span,
    close_end: proc_macro2::Span,
    elements: &[Expr],
    trailing_comma: bool,
) -> String {
    let mut out = String::new();
    if elements.is_empty() {
        // Whole gap between the delimiters: `open`, comments, `close`.
        let pieces = gap_between(source, open_start, close_end, &[open, close]);
        out.push_str(&emit_gap(&pieces, Spacing::None, depth));
        return out;
    }
    // Opening delimiter + any comment before the first element.
    let pre = gap_between(source, open_start, elements[0].span().start, &[open]);
    out.push_str(&emit_gap(&pre, Spacing::None, depth));
    for (i, el) in elements.iter().enumerate() {
        out.push_str(&format_at(el, source, depth, Level::OR));
        if i + 1 < elements.len() {
            let pieces = gap_between(source, el.span().end, elements[i + 1].span().start, &[","]);
            out.push_str(&emit_gap(&pieces, Spacing::CommaAfter, depth));
        }
    }
    if trailing_comma {
        out.push(',');
    }
    // Any comment before the closing delimiter, then the delimiter.
    let post = gap_between(source, elements[elements.len() - 1].span().end, close_end, &[close]);
    out.push_str(&emit_gap(&post, Spacing::None, depth));
    out
}
```

Rewrite `Expr::Apply`:

```rust
        Expr::Apply { callee, args, span } => {
            let callee_s = format_at(callee, source, depth, Level::POSTFIX);
            let list = emit_list(
                source,
                depth,
                "(",
                ")",
                callee.span().end,
                span.end,
                args,
                false,
            );
            (format!("{callee_s}{list}"), Level::POSTFIX)
        }
```

Rewrite `Expr::Tuple`:

```rust
        Expr::Tuple { elements, span } => {
            let list = emit_list(
                source,
                depth,
                "(",
                ")",
                span.start,
                span.end,
                elements,
                elements.len() == 1,
            );
            (list, Level::PRIMARY)
        }
```

Rewrite `Expr::TupleIndex` (the `.N` has no comment gap of interest between base and index in
practice — the `.index` glues to base; keep it thread-only):

```rust
        Expr::TupleIndex { base, index, .. } => (
            format!("{}.{}", format_at(base, source, depth, Level::POSTFIX), index),
            Level::POSTFIX,
        ),
```

Note `emit_list`'s empty-list path for `f()` yields `gap_between(callee.end, span.end, &["(",")"])`
→ `"()"` (comment-free), matching today; `(1i32,)` sets `trailing_comma` and its post-gap scans
for `")"` after the sole element.

Trace `(1i32,)`: `open_start = span.start`. `pre = gap_between(span.start, elements[0].start, &["("])`
→ `"("`. element → `"1i32"`. `trailing_comma` → `","`. `post = gap_between(elements[0].end, span.end, &[")"])`
→ `")"`. Result `"(1i32,)"`. Matches the existing `one_tuple_keeps_its_trailing_comma` test.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: PASS — new comment tests recover; all pre-existing call/tuple/tuple-index tests
(`multi_element_tuple_has_no_trailing_comma`, `one_tuple_keeps_its_trailing_comma`,
`call_with_no_args_and_with_args`, `tuple_index_single_and_chained`, etc.) still print identically.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): preserve comments in call/tuple gaps (#201)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 8: Recover comments in `if`/`else` gaps

**Files:** Modify `cel-parser/src/fmt.rs` (the `Expr::If` arm). Test: inline additions.

The `If` arm has several fixed tokens with no spans: `if`, `{`, `}`, `else`. The gaps:
- node `span.start` → `cond.span().start`: the `if` keyword.
- `cond.span().end` → `then_branch.span().start`: the `{`.
- `then_branch.span().end` → (either the `else` or the closing `}`): the `}` (plus, if no else,
  end of node).
- for the else: between the then-branch's closing `}` and the else branch: `else` + `{`.

Because the `}`/`{`/`else` positions aren't individually spanned, scan each gap for its expected
token sequence.

**Interfaces:** Consumes `gap_between`/`emit_gap`. Produces nothing new.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_comment_after_if_condition_is_preserved() {
        assert_eq!(
            fmt("if a /* c */ { 1i32 }"),
            "if a /* c */ { 1i32 }"
        );
    }

    #[test]
    fn a_comment_before_else_is_preserved() {
        assert_eq!(
            fmt("if a { 1i32 } /* e */ else { 2i32 }"),
            "if a { 1i32 } /* e */ else { 2i32 }"
        );
    }

    #[test]
    fn comment_free_if_else_is_unchanged() {
        assert_eq!(fmt("if true { 1i32 } else { 2i32 }"), "if true { 1i32 } else { 2i32 }");
        assert_eq!(fmt("if true { 1i32 }"), "if true { 1i32 }");
    }
```

- [ ] **Step 2: Run to verify the comment tests fail, the regressions pass**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 3: Rewrite the `Expr::If` arm**

```rust
        Expr::If { cond, then_branch, else_branch, span } => {
            let cond_s = format_at(cond, source, depth, Level::OR);
            let then_s = format_at(then_branch, source, depth, Level::OR);
            // `if` <cond> `{` <then> `}` [ `else` ( `{` <else> `}` | <else-if> ) ]
            let if_gap = gap_between(source, span.start, cond.span().start, &["if"]);
            let open_gap = gap_between(source, cond.span().end, then_branch.span().start, &["{"]);
            let mut text = format!(
                "{}{cond_s}{} {then_s} }}",
                emit_gap(&if_gap, Spacing::None, depth).trim_end().to_string() + " ",
                emit_gap(&open_gap, Spacing::None, depth),
            );
            // The above is fiddly; prefer the explicit construction below instead.
            let _ = &mut text;
            let mut text = String::new();
            text.push_str(emit_gap(&if_gap, Spacing::None, depth).trim_start());
            text.push(' ');
            text.push_str(&cond_s);
            text.push(' ');
            text.push_str(emit_gap(&open_gap, Spacing::None, depth).trim());
            text.push_str(" ");
            text.push_str(&then_s);
            text.push_str(" }");
            if let Some(else_branch) = else_branch {
                let else_gap = gap_between(
                    source,
                    then_branch.span().end,
                    else_branch.span().start,
                    if matches!(else_branch.as_ref(), Expr::If { .. }) { &["else"] } else { &["else", "{"] },
                );
                if matches!(else_branch.as_ref(), Expr::If { .. }) {
                    let (else_s, _) = render(else_branch, source, depth);
                    text.push(' ');
                    text.push_str(emit_gap(&else_gap, Spacing::None, depth).trim());
                    text.push(' ');
                    text.push_str(&else_s);
                } else {
                    let else_s = format_at(else_branch, source, depth, Level::OR);
                    text.push(' ');
                    text.push_str(emit_gap(&else_gap, Spacing::None, depth).trim());
                    text.push_str(" ");
                    text.push_str(&else_s);
                    text.push_str(" }");
                }
            }
            (text, Level::PRIMARY)
        }
```

The doubled `text` above is intentional guidance: delete the first `format!`-based attempt and
keep only the explicit push-based construction (the first block is left in the plan to show the
dead-end to avoid). The implementer keeps only the second construction. Verify against the
comment-free tests: `if true { 1i32 }` → `if` gap `"if"` → `text = "if true { 1i32 }"`; with else →
appends ` else { 2i32 }`. The `else if` chain path recurses via `render`.

Because the `emit_gap(...).trim()` calls here strip the block-comment inlining down to just the
recovered comments plus the keyword/brace, confirm `if a /* c */ { 1i32 }`:
- `if_gap` (span.start→cond.start): `"if"` → trimmed `"if"` → `text = "if "`.
- `cond_s = "a"` → `text = "if a "`.
- `open_gap` (cond.end→then.start): source between `a` and `{` is ` /* c */ `; scan for `["{"]` →
  `[Comment(Block "c"), Punct("{")]`; `emit_gap(None, _)` → `"/* c */ {"` (Block adds its own
  spaces: `"/* c */ {"`); `.trim()` → `"/* c */ {"`; `text = "if a /* c */ { "` then `then_s` +
  `" }"` → `"if a /* c */ { 1i32 }"`. Matches.

(If the exact spaces don't match, adjust the `.trim()`/space handling until the three tests pass;
do not change the tests.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: PASS — including pre-existing `if_without_else_omits_the_else_clause`,
`if_else_reprints_both_branches`, `else_if_chain_has_no_braces_around_the_nested_if`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "feat(cel-parser): preserve comments in if/else gaps (#201)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 9: Recover comments in closure gaps + cross-arm idempotency & #201 repro

**Files:** Modify `cel-parser/src/fmt.rs` (the `Expr::Closure` arm). Test: inline additions,
including the issue-#201 repro and a broad idempotency test.

**Interfaces:** Consumes `gap_between`/`emit_gap`. Produces nothing new.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_comment_before_a_closure_body_is_preserved() {
        assert_eq!(fmt("|x: i32| /* b */ x + 1i32"), "|x: i32| /* b */ x + 1i32");
    }

    #[test]
    fn comment_free_closures_are_unchanged() {
        assert_eq!(fmt("|x: i32| x + 1i32"), "|x: i32| x + 1i32");
        assert_eq!(fmt("|| 1i32"), "|| 1i32");
        assert_eq!(fmt("|x: i32, y: i32| x + y"), "|x: i32, y: i32| x + y");
    }

    #[test]
    fn issue_201_line_comment_inside_an_expression_round_trips() {
        // The exact repro from stlab/cel-rs#201.
        let source = "1i32 +\n    // why 2\n    2i32";
        let once = fmt(source);
        assert!(once.contains("// why 2"), "comment must survive: {once:?}");
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice, "formatting must be idempotent");
    }

    #[test]
    fn the_full_inline_comment_example_round_trips() {
        // From the design discussion: block comments on both operands and around the operator,
        // plus a trailing line comment.
        let source = "/* p */ 1i32 /* q */ + // hello\n2i32 /* r */";
        let once = fmt(source);
        assert!(once.contains("/* p */") && once.contains("/* q */")
            && once.contains("// hello") && once.contains("/* r */"), "all comments survive: {once:?}");
        let twice = format_expr(&parse(&once), &once, 0);
        assert_eq!(once, twice);
    }
```

- [ ] **Step 2: Run to verify the comment/repro tests fail, the regressions pass**

Run: `cargo test -p cel-parser fmt::`

- [ ] **Step 3: Rewrite the `Expr::Closure` arm**

The closure's parameter list and the `|...|` delimiters carry comment gaps; the body follows the
closing `|`. The parameters are `ClosureParam { name, name_span, type_expr }` (not `Expr` nodes),
so recover only the gap between the closing `|` and the body (the most common #201-style case);
parameter-internal comments are a rarer case handled by scanning the header gap for its fixed
tokens.

```rust
        Expr::Closure { params, body, span } => {
            let body_s = format_at(body, source, depth, Level::OR);
            let header = if params.is_empty() {
                "||".to_string()
            } else {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("|{params_s}|")
            };
            // Gap between the closing `|` of the header and the body's first token.
            let pieces = gap_between(source, header_end_span(span, params), body.span().start, &[]);
            let gap = emit_gap(&pieces, Spacing::None, depth);
            let sep = if gap.trim().is_empty() { " ".to_string() } else { format!(" {} ", gap.trim()) };
            (format!("{header}{sep}{body_s}"), Level::PRIMARY)
        }
```

Because there's no span for the closing `|`, `header_end_span` can't be computed precisely; instead
scan the gap from the last param's `name_span`/type end (or, for a zero-param closure, from
`span.start`) to `body.span().start`, expecting the tokens that close the header. Simpler and
robust: scan the whole `span.start → body.span().start` gap for the header's fixed tokens and rely
on `scan_gap` to surface any comments while re-synthesizing `|`/`|`. Replace the body with:

```rust
        Expr::Closure { params, body, span } => {
            let body_s = format_at(body, source, depth, Level::OR);
            let header = if params.is_empty() {
                "||".to_string()
            } else {
                let params_s = params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, render_closure_param_type(&p.type_expr)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("|{params_s}|")
            };
            // Only recover comments that sit between the header's closing `|` and the body. Scan
            // the whole header-through-body gap for comments, keeping only those whose source
            // position is after the header text (i.e. between `|` and the body).
            let pieces = gap_between(source, span.start, body.span().start, &[]);
            let comments: Vec<GapPiece> = pieces
                .into_iter()
                .filter(|p| matches!(p, GapPiece::Comment(_)))
                .collect();
            let gap = emit_gap(&comments, Spacing::None, depth);
            let sep = if gap.trim().is_empty() {
                " ".to_string()
            } else {
                format!(" {} ", gap.trim())
            };
            (format!("{header}{sep}{body_s}"), Level::PRIMARY)
        }
```

With `expected = &[]`, `scan_gap` returns only comments (no tokens to match), so a param-list
comment and the header-to-body comment both surface; the header text itself is reconstructed from
`params`. For `|x: i32| /* b */ x + 1i32`: comments = `[Block("b")]`, `gap = "/* b */ "`, `sep =
" /* b */ "`, result `"|x: i32| /* b */ x + 1i32"`. For the comment-free closures, `comments`
empty → `sep = " "` → unchanged.

(Accept as a known, documented limitation that a comment *inside* a closure parameter list, e.g.
`|x: i32 /* c */| body`, attaches after the header rather than at its exact source spot — note this
in the arm's doc comment. This is rarer than the #201 case and still lossless.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cel-parser fmt::`
Expected: PASS — closures, the #201 repro, and the full inline example all round-trip; every
pre-existing closure test prints identically.

- [ ] **Step 5: Run the whole `cel-parser` suite**

Run: `cargo test -p cel-parser && cargo test --doc -p cel-parser`
Expected: PASS, zero warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add cel-parser/src/fmt.rs
git commit -m "$(cat <<'EOF'
feat(cel-parser): preserve comments in closure gaps; #201 repro passes

Completes CEL-expression comment recovery: every inter-child gap now
round-trips its comments, and a // line comment wraps the expression.
Adds the issue #201 repro and a full inline-comment idempotency test.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: adam-lang — move `Comment` to a re-export; drop duplicated offset helpers

**Files:**
- Modify: `adam-lang/src/ast.rs` (replace the `Comment` enum definition with a re-export)
- Modify: `adam-lang/src/trivia.rs` (import the offset helpers from `cel_parser::trivia` instead of
  defining them locally)

**Interfaces:**
- Consumes: `cel_parser::Comment`, `cel_parser::trivia::{line_start_byte_offsets, line_column_to_byte}`.
- Produces: `adam_lang::ast::Comment` now aliases `cel_parser::Comment` (every existing
  `ast::Comment::Line`/`Block` use keeps compiling).

Note: `line_start_byte_offsets`/`line_column_to_byte` are `pub(crate)` in `cel_parser::trivia`, so
`adam-lang` (a separate crate) cannot import them directly. Make them `pub` in `cel_parser::trivia`
(promote from `pub(crate)` — update Task 1's implementation note: they must be `pub`, not
`pub(crate)`, because adam-lang consumes them). `scan_gap`/`GapPiece` also need to be `pub` for
Task 11's adam-lang use. Revise Task 1 accordingly if not already `pub`.

- [ ] **Step 1: Promote the primitives to `pub` in `cel_parser::trivia`**

In `cel-parser/src/trivia.rs`, change `pub(crate) enum GapPiece`, `pub(crate) fn scan_gap`,
`pub(crate) fn line_start_byte_offsets`, `pub(crate) fn line_column_to_byte` to `pub`. In
`cel-parser/src/lib.rs`, they're reachable as `cel_parser::trivia::*`. Run `cargo build -p
cel-parser` to confirm still clean.

- [ ] **Step 2: Replace `Comment` in `adam-lang/src/ast.rs`**

Find the `Comment` enum definition (the `#[derive(Debug, Clone, PartialEq, Eq)] pub enum Comment {
Line(String), Block(String) }` block) and replace it with:

```rust
pub use cel_parser::Comment;
```

- [ ] **Step 3: Import offset helpers in `adam-lang/src/trivia.rs`**

Delete the local `line_start_byte_offsets` and `line_column_to_byte` function definitions. Add to
the `use` block:

```rust
use cel_parser::trivia::{line_column_to_byte, line_start_byte_offsets};
```

Leave `analyze_gap` and everything else in `trivia.rs` unchanged.

- [ ] **Step 4: Build and test**

Run: `cargo test -p adam-lang && cargo test --doc -p adam-lang`
Expected: PASS unchanged — `Comment` is structurally identical, only its definition site moved, and
the offset helpers are byte-identical logic now sourced from `cel-parser`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add cel-parser/src/trivia.rs cel-parser/src/lib.rs adam-lang/src/ast.rs adam-lang/src/trivia.rs
git commit -m "$(cat <<'EOF'
refactor(adam-lang): reuse cel-parser's Comment and offset helpers

Comment moves to cel-parser (adam-lang re-exports it); the
LineColumn->byte-offset helpers move too, so both crates share one
copy. No behavior change.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: adam-lang — recover comments in intra-declaration gaps via `scan_gap`

adam-lang's `trivia.rs` covers gaps *between* sibling declarations and *before closing braces*. It
never covers gaps *within* one declaration's own tokens (`cell`↔name, name↔`:`, type↔`=`, `=`↔
initializer, initializer↔`;`/`filter`/`require`, and the equivalents for `out`/`source`/`relate`/
`conditional`/`binding`/`requirement`). This task adds `scan_gap`-based recovery for those gaps,
using the sheet `source` threaded in Task 3.

Because adam-lang's declaration tokens (`cell`, the name, `:`, `=`, `filter`, `;`) mostly have
spans available only for the name and the type/initializer `Expr`/`TypeExpr`, recover the gaps that
have both endpoints spanned: primarily the gap between a type annotation and its `=`/`:=` and
initializer, and the gap between an initializer and a trailing `filter`/`require`/`;`. This task
targets the highest-value, reliably-spanned intra-declaration gaps; gaps between two keyword-only
tokens with no spans are out of scope (documented).

**Files:**
- Modify: `adam-lang/src/fmt.rs` (`write_cell`, `write_source`, `write_out`, `write_binding`)
- Test: inline additions

**Interfaces:** Consumes `cel_parser::trivia::{scan_gap, GapPiece}`, `cel_parser::Comment`, and the
threaded `source`. Produces a private `emit_decl_gap` helper local to adam-lang's `fmt.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn preserves_a_block_comment_between_a_cell_initializer_and_its_semicolon() {
        let source = "sheet s {\n    cell a: i32 = 1 /* trailing */;\n}";
        assert_eq!(
            format(source),
            "sheet s {\n    cell a: i32 = 1 /* trailing */;\n}\n"
        );
    }

    #[test]
    fn preserves_a_block_comment_between_a_type_and_the_equals() {
        let source = "sheet s {\n    cell a: i32 /* t */ = 1;\n}";
        assert_eq!(
            format(source),
            "sheet s {\n    cell a: i32 /* t */ = 1;\n}\n"
        );
    }

    #[test]
    fn a_declaration_with_no_intra_token_comments_is_unchanged() {
        assert_eq!(
            format("sheet s { cell a: i32 = 1; }"),
            "sheet s {\n    cell a: i32 = 1;\n}\n"
        );
    }
```

- [ ] **Step 2: Run to verify the comment tests fail, the regression passes**

Run: `cargo test -p adam-lang fmt::`

- [ ] **Step 3: Add `emit_decl_gap` and wire it into the reliably-spanned gaps**

Add to `adam-lang/src/fmt.rs`:

```rust
/// Renders the comments (if any) in the source gap between two adam-lang declaration positions,
/// inlined block-style, for a gap that otherwise contains only a fixed token the caller emits
/// itself. Returns `""` when the gap has no comments (the common case), so a comment-free
/// declaration prints exactly as before. `//` line comments inside a declaration are recovered as
/// inline blocks are — a declaration is already `;`-terminated on its own line, so a mid-declaration
/// `//` is rare; recover it losslessly by emitting it and continuing.
fn emit_decl_gap(source: &str, prev_end: ast::ExprSpan, next_start: ast::ExprSpan) -> String {
    use cel_parser::trivia::{GapPiece, line_column_to_byte, line_start_byte_offsets, scan_gap};
    if source.is_empty() {
        return String::new();
    }
    let line_starts = line_start_byte_offsets(source);
    let start = line_column_to_byte(source, &line_starts, prev_end.end.end());
    let end = line_column_to_byte(source, &line_starts, next_start.start.start());
    let Some(gap) = source.get(start..end) else {
        return String::new();
    };
    let mut out = String::new();
    for piece in scan_gap(gap, &[]) {
        if let GapPiece::Comment(comment) = piece {
            out.push(' ');
            match comment {
                cel_parser::Comment::Block(text) => {
                    out.push_str("/* ");
                    out.push_str(&text);
                    out.push_str(" */");
                }
                cel_parser::Comment::Line(text) => {
                    out.push_str("// ");
                    out.push_str(&text);
                }
            }
        }
    }
    out
}
```

Wire it into `write_cell` for the two reliably-spanned intra-declaration gaps — between the type
annotation and the initializer, and between the initializer and the terminator. The type
annotation has a span via `type_expr.span()`; the initializer is a `cel_parser::Expr` with
`.span()`. Between the initializer's end and the `;` there's no next spanned node, so use the
`CellDecl`'s own `span.end` as the gap end. Update `write_cell`:

```rust
    if let Some(type_expr) = &cell.type_name {
        out.push_str(": ");
        out.push_str(&source_text_or_empty(type_expr.span()));
        if let Some(expr) = &cell.initializer {
            out.push_str(&emit_decl_gap(source, type_expr.span(), expr.span()));
        }
    }
    if let Some(expr) = &cell.initializer {
        out.push_str(" = ");
        out.push_str(&cel_parser::format_expr(expr, source, depth));
        // Comment between the initializer and whatever terminates the declaration.
        out.push_str(&emit_decl_gap(source, expr.span(), cell.span));
    }
```

(Confirm `CellDecl` has a `span: ExprSpan` field — it does per `adam-lang/src/ast.rs`. If the
`emit_decl_gap` between type and `=` would double a comment already recovered as a same-line
trailing comment by `trivia.rs`, guard by only scanning gaps strictly inside the declaration's own
token run, which these two are — the initializer and terminator are within the `CellDecl.span`.)

Apply the same two-gap wiring to `write_source` and `write_out` (mirroring `write_cell`; `out` uses
`:=` and `write_source` uses `=`, but the gap logic is identical), and to `write_binding` (between
the binding's body and its `;`, i.e. `emit_decl_gap(source, binding.body.span(), binding.span)`).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p adam-lang fmt::`
Expected: PASS — the intra-declaration comment tests recover; every pre-existing adam-lang fmt test
still prints identically (comment-free gaps yield `""`).

- [ ] **Step 5: Run the whole adam-lang suite**

Run: `cargo test -p adam-lang && cargo test --doc -p adam-lang`
Expected: PASS, zero warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add adam-lang/src/fmt.rs
git commit -m "$(cat <<'EOF'
feat(adam-lang): recover comments inside a declaration's own tokens (#201)

Adds scan_gap-based recovery for the intra-declaration gaps the
existing sibling/closing-brace trivia system never covered (between a
type and its =, an initializer and its terminator, a binding body and
its ;). Comment-free declarations print unchanged.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Full-workspace verification

**Files:** none (verification only).

- [ ] **Step 1: Full build and test**

Run:
```bash
cargo build --workspace
cargo test --workspace
cargo test --doc --workspace
```
Expected: all pass, zero compiler warnings. Read the output for any `warning:` lines and fix them
before proceeding (per the project's zero-warning rule).

- [ ] **Step 2: All three clippy invocations**

Run:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all three clean.

- [ ] **Step 3: Rustdoc as CI checks it**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`
Expected: clean (no broken intra-doc links from the moved `Comment`/helpers).

- [ ] **Step 4: Final commit if any fixups were needed**

```bash
cargo fmt --all
git add -A
git commit -m "chore: workspace-wide warning/clippy/doc fixups for #201 trivia work

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review Notes

**Spec coverage (Parts A + A2):**
- Part A `fill_gap` primitive → Tasks 1–2 (`scan_gap` + `emit_gap`).
- Part A `format_expr` signature change + every inter-child gap → Tasks 3–9 (one arm group per
  task, all `Op`/`Cast`/`Apply`/`Tuple`/`TupleIndex`/`If`/`Logical`/`Closure`/range variants).
- Part A `//`-comment wrapping → the `emit_gap` `Comment::Line` branch (Task 2), exercised in
  Tasks 4 and 9 (#201 repro).
- Part A hand-built-`Expr` graceful degradation → `scan_gap`'s token re-synthesis (Task 1) +
  `gap_between`'s empty-source guard (Task 4) + ez-adam call sites (Task 3).
- Part A2 shared primitive reuse + intra-declaration gaps → Tasks 10–11.
- `Comment` moved to `cel-parser`, re-exported by adam-lang → Task 10.
- Existing adam-lang trivia system left intact → Tasks 10–11 touch only `Comment`'s definition
  site, the offset-helper imports, and `fmt.rs`'s intra-declaration emission; `analyze_gap` and the
  AST-attachment machinery are untouched.
- Full verification (build/test/clippy/doc, zero warnings) → Task 12.

**Known, documented limitations (lossless but not position-perfect):** a comment inside a closure
parameter list (Task 9) or between two span-less adam-lang keyword tokens (Task 11) is recovered
and re-emitted, but may attach slightly after its exact original spot rather than at it. No comment
text is dropped. These are noted in the relevant arms' doc comments.

**Type consistency:** `scan_gap`/`GapPiece`/offset helpers are `pub` (Task 10 promotes them from the
`pub(crate)` shown in Task 1 — the implementer should make them `pub` from the start given Task 10's
requirement). `format_expr(&Expr, &str, usize)` is used consistently at every call site (Tasks 3,
9, 11). `emit_gap(&[GapPiece], Spacing, usize)` and `gap_between(&str, Span, Span, &[&'static str])`
signatures match across Tasks 4–9.

**Placeholder scan:** Task 8's `Expr::If` arm deliberately shows a dead-end `format!` attempt
followed by the correct push-based construction, with an explicit instruction to keep only the
latter — this is guidance to avoid a known trap, not a placeholder. All other steps contain
complete code.
