# adam-fmt Comment Support — Design

## Goal

Improve comment support in `adam-fmt`'s pipeline (`cel-parser`'s shared lexer, `adam-lang`'s
grammar/AST, and its formatter) by resolving five related, previously-filed issues:

- [#58](https://github.com/stlab/cel-rs/issues/58) — `///`/`//!` doc comments are tokenized as
  `#[doc = "..."]`/`#![doc = "..."]` attribute-shaped tokens by `proc_macro2` and always fail to
  parse, since the grammar has no production expecting a bare `#[...]`. Fixed here by adding
  first-class, narrowly-scoped doc-comment support.
- [#105](https://github.com/stlab/cel-rs/issues/105) — a multi-line `/* ... */` block comment (its
  `/*` and `*/` on different lines) is silently dropped by `attach_trivia`'s gap scan, which only
  recognizes a block comment fully contained on one line.
- [#53](https://github.com/stlab/cel-rs/issues/53) — recovered comments collapse to a single
  `String` with no memory of delimiter style, so every `/* ... */` block comment is re-emitted by
  the formatter as one or more `//` lines.
- [#52](https://github.com/stlab/cel-rs/issues/52) — a comment or blank line between a block's
  *last* item and its own closing `}` has nothing to attach to, so it's silently dropped —
  affecting `Sheet`, `RelationshipDecl`, `ConditionalDecl`/its default arm, `ConditionalBranch`,
  and `OutDecl`.
- [#57](https://github.com/stlab/cel-rs/issues/57) — the already-merged PR that introduced the
  formatter (`leading_comment`/`blank_line_before`, `attach_trivia`, `format_sheet`) and filed
  #52–#56 as follow-up issues, #52/#58 among them. Reviewed for background/context only; not
  itself an open defect, so there is no code change attributable to it beyond what #52/#53/#58
  already cover.

## Background

`adam-lang` tokenizes source via `proc_macro2::TokenStream::from_str` (a deliberate choice so the
same tokenizer can later run inside a real `#[proc_macro]`, per
`docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`). `cel-parser`'s
`LexLexer` (`cel-parser/src/lex_lexer.rs`) flattens that stream into a `Token` sequence shared by
both `cel-parser`'s own CEL expression grammar and `adam-lang`'s declaration grammar (via
`adam-lang/src/token_cursor.rs::TokenCursor`).

Plain `//`/`/* */` comments are invisible to this tokenizer (`proc_macro2` discards them as
whitespace) and are recovered separately by `adam-lang/src/trivia.rs::attach_trivia`, which
re-scans raw *source text* in the gap between two consecutive AST nodes' spans and attaches a
recovered comment/blank-line-before flag to the following node. This gap-scanning approach cannot
see `///`/`//!` doc comments as *text* at all, because `proc_macro2` has already turned them into
real tokens (`#[doc = "..."]`/`#![doc = "..."]`) before `attach_trivia` ever runs — they must be
handled by the grammar/lexer layer instead, which is what #58 punted on ("a materially bigger
change... revisit if/when adam-lang ever wants real attribute syntax").

## Scope decisions (confirmed during brainstorming)

- Doc-comment support is scoped **narrowly to the doc-comment shape** (`#[doc = "..."]`/
  `#![doc = "..."]`) that `proc_macro2` already produces for `///`/`//!` — not general `#[...]`
  attribute syntax. A `#`-led token sequence that isn't this exact shape is a lexer-level error,
  not a supported attribute.
- Outer doc comments (`///`) are legal only immediately before the four top-level sheet-item kinds:
  `cell`, `relationship`, `conditional`, `out`. Not on methods, conditional branches, or
  conditions.
- Inner doc comments (`//!`) are legal only at the very top of the file, before the `sheet`
  keyword, analogous to Rust's module-level `//!` — becoming the sheet's own doc comment.
- Doc comments bind to the following declaration regardless of blank lines in between (matching
  real `///`/`//!` semantics) — unlike plain comments, where a blank line breaks the attachment.
- #52's fix restructures `ConditionalDecl::default` from `Option<Vec<RelationshipDecl>>` to
  `Option<DefaultBranch>` (a new struct mirroring `ConditionalBranch`) so the default arm has
  somewhere to carry its own trailing-comment slot, and also covers a comment inside a completely
  empty block (e.g. `relationship { /* only this */ }`), not just the literal
  last-item-to-closing-brace case — see §5.
- Out of scope, unchanged from existing deferred items: general `#[...]` attribute syntax; doc
  comments surfaced via `adam-lsp` hover (design doc Phase 5, not yet built); #54 (column-aware
  line-wrapping); #55 (range formatting).

## 1. `cel-parser::lex_lexer` — recognizing doc comments

`LexLexer::next()` currently flattens every `TokenTree` uniformly. Add a committed (non-
speculative) parse triggered by `Punct('#')`:

1. Consume the next token tree (via the existing `next_token_tree()` primitive, the same lookahead
   primitive already used for compound-operator combining). If it's `Punct('!')` (`Spacing::Alone`),
   record `inner = true` and consume the token after that as the candidate group; otherwise treat
   the token just fetched as the candidate group directly, with `inner = false`.
2. Require the candidate to be a `TokenTree::Group` with `Delimiter::Bracket`. Anything else
   (no group, wrong delimiter, end of input) is an error.
3. Walk the group's own token stream and require exactly: `Ident("doc")`, `Punct('=')`
   (`Spacing::Alone`), a `Literal` that parses as `syn::Lit::Str`, and nothing further. Any
   mismatch (wrong ident text, non-string literal, extra trailing tokens, missing tokens) is an
   error.

On success, emit a new token:

```rust
Token::DocComment {
    text: String,   // the string literal's unescaped value, e.g. " the total" for `/// the total`
    inner: bool,     // true for `//!`/`#![doc]`, false for `///`/`#[doc]`
    span: Span,      // the `#` token's span
}
```

On any mismatch, emit a new token:

```rust
Token::Error {
    message: String,
    span: Span,
}
```

There is no fallback re-emission of the raw `#`/group/etc. as ordinary tokens in the mismatch
case — once `#` is seen, the lexer is committed to this being a doc-comment attribute or an error,
never silently falling through to `Punct`/`OpenDelim`/... tokens the way it does today. This
means a stray `#[foo]` or `#[doc = 5]` in adam-lang source now produces a `Token::Error` instead of
flattened tokens that would eventually fail elsewhere — same failure class (a syntax error) as
today, with a strictly more specific message available if a caller chooses to surface it (see
below).

`LexLexer::Item` stays `Token` (not `Result<Token, _>`), so `TokenCursor` and
`cel_parser::Parser<C>` — both of which iterate `LexLexer` directly — need no changes to their
iteration surface. `Token::Error` simply flows through as an ordinary token and fails to match
whatever pattern a grammar production expects at that point, surfacing as a normal `ParseError`
exactly like any other unexpected token today. As a quality-of-life refinement (not required for
correctness), the couple of call sites that currently build a generic `ParseError::new("unexpected
token", tok.span())` may special-case `Token::Error` to use its own `message` instead.

The module doc's current claim ("This lexer does not produce errors... any impossible state uses
`unreachable!()`") is updated to carve out this one exception: a `#`-led token sequence that
doesn't match the doc-comment shape is a real, expected failure mode (not an impossible state),
because general attribute syntax is deliberately unsupported.

**Testing:** direct unit tests feeding `TokenStream::from_str("/// x")` / `"//! x"` and asserting
the resulting `Token::DocComment` fields, alongside malformed-attribute cases (`#[foo]`,
`#[doc = 5]`, `#[doc]`, `#(x)`, `#[doc = "x", extra]`) asserting `Token::Error` — matching this
module's existing token-shape-assertion test style (`test_compound_operator`,
`arrow_is_two_char_punct`).

## 2. `adam-lang` grammar/AST — attaching doc comments to declarations

Grammar additions (as doc-comment-style grammar productions, per this repo's convention):

```text
sheet             = [ inner_doc_comment ] "sheet" identifier "{" { sheet_item } "}".
sheet_item        = [ outer_doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl).
outer_doc_comment = { doc_comment }.   (* consecutive `///` tokens; inner = false *)
inner_doc_comment = { doc_comment }.   (* consecutive `//!` tokens; inner = true; legal only before `sheet` *)
```

**Shared primitive.** `adam-lang/src/token_cursor.rs::TokenCursor` is already the pure-tokenizing
layer shared by both of adam-lang's parsers (`AdamAstParser` in `ast_parser.rs`, building the
tooling-facing span-carrying AST, and `AdamParser` in `parser.rs`, building the live `adam_rs::Sheet`
that `begin` actually executes — see `ast_parser.rs`'s module doc). It gains one new method:

```rust
/// Consumes a leading run of consecutive `Token::DocComment` tokens matching `inner`, returning
/// their joined text (`\n`-separated) and the first token's span, or `None` if the next token
/// isn't a matching doc comment.
pub(crate) fn consume_doc_comment_run(&mut self, inner: bool) -> Option<(String, Span)>
```

Both parsers call this at the same grammar positions (sheet-level with `inner = true`, before each
sheet-item's `///` with `inner = false`). `AdamAstParser` uses the returned text/span as described
below. `AdamParser` — which preserves no comments of any kind today — simply discards the result,
matching its existing `ctx.consume_ident()?; // sheet name (ignored at runtime)` pattern for
information it deliberately doesn't need. This keeps the two parsers' *accepted grammar* in sync:
without it, a `.adm2` file using doc comments would format correctly (via `AdamAstParser`) but fail
to load in `begin` (via `AdamParser`), since `Token::DocComment` would otherwise reach neither
parser's dispatch and fail as an unexpected token in both.

`AdamAstParser::parse_str` peels a leading run of `inner` `Token::DocComment`s before dispatching
to `parse_sheet`, joining their `text` fields with `\n` into a new `Sheet::doc_comment:
Option<String>` field.

For outer (`///`) doc comments, peeling happens in `parse_sheet`'s own item loop — **not** inside
`parse_sheet_item` — specifically so the text survives the existing declaration-level error-
recovery path. `parse_sheet`'s loop already records `item_start` and calls `set_last_span` before
dispatching to `parse_sheet_item`; it now also peels a leading run of `outer` `Token::DocComment`s
right there, capturing their joined text and the first token's span. On `parse_sheet_item`'s
success, the returned item's new `doc_comment: Option<String>` field (added to `CellDecl`,
`RelationshipDecl`, `ConditionalDecl`, `OutDecl`, and `SheetItem::Error` for symmetry with the
existing `leading_comment` field, which `Error` already carries) is set from the captured text, via
a setter alongside the existing `SheetItem::set_leading_comment`, and the item's span-start is
widened per below. On failure, the already-captured doc comment text is threaded into the
`SheetItem::Error` placeholder `parse_sheet` builds instead of being discarded — mirroring how a
plain `leading_comment` already survives onto an `Error` item today (see the
`attaches_a_comment_preceding_a_recovered_error_item` test). A doc comment appearing anywhere the
grammar doesn't explicitly peel one (inside a method, a conditional branch, a condition, or
mid-expression) is simply never consumed by this new logic and falls through to the pre-existing
"unexpected token" handling — no new error path is needed there.

**Trivia-boundary fix.** Plain `//`/`/* */` comments are recovered by `trivia.rs` re-scanning raw
source text in the gap between two nodes' spans; it never sees tokens. A doc comment, by contrast,
*is* now a real token sitting in that same source region. If a node's span still started at its
keyword (e.g. `cell`), `attach_trivia`'s gap scan would also see the `///` text and misparse it as
a stray `//` line (since `"///".strip_prefix("//")` succeeds, yielding a spurious extra line).
Fix: when a doc-comment run is consumed, the resulting node's outer `span.start` is widened to the
*first* doc-comment token's span instead of the keyword's. This shortens the gap `trivia.rs` scans
so it stops exactly before the doc comment's source text begins — no new trait method or field is
needed, and the two mechanisms (grammar-level doc comments, gap-scanned plain comments) stay
cleanly separated. A plain `//` comment appearing before the doc comment (e.g. `// TODO\n/// docs
\ncell x;`) still attaches correctly as `leading_comment`, since it's a trailing run within that
now-shortened gap; the two coexist and print in source order (plain comment, then doc comment,
then the declaration).

Doc comments bind to the following declaration regardless of blank lines between them and it
(matching real `///`/`//!` semantics) — they are consumed as tokens immediately adjacent in the
stream, never gap-scanned text, so there is nothing for a blank line to interrupt.

**Testing:** doc comment attachment at each of the four site types plus sheet-level `//!`; a doc
comment coexisting with a plain leading comment (order preserved); a doc comment separated from
its item by a blank line still attaches; no plain-comment leakage into `leading_comment` when a
doc comment is present (verifying the span-widening fix); a stray doc comment in an unsupported
position (e.g. before a `method`) still produces the same class of syntax error as before; and,
for `AdamParser` (`parser.rs`), a `.adm2` source string using doc comments at each of the same
positions parses successfully end-to-end (`parse_str(...).unwrap()`), confirming the two parsers'
grammars stay in sync.

## 3. Plain-comment representation fix (#53, #105)

Add a new public enum to `adam-lang/src/ast.rs`, replacing every existing
`leading_comment: Option<String>` field's type:

```rust
/// A recovered `//`/`/* */` comment, remembering which delimiter style the source used so the
/// formatter can reproduce it instead of normalizing everything to `//`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comment {
    /// One or more consecutive `// text` lines, joined by `\n`, each with its leading `//`/space
    /// stripped.
    Line(String),
    /// A single `/* text */` block comment (single- or multi-line), its inner text joined by
    /// `\n` with the opening `/*`/closing `*/` and per-line indentation stripped.
    Block(String),
}
```

Every `leading_comment: Option<String>` field (on `Sheet`, `SheetItem::Error`, `CellDecl`,
`RelationshipDecl`, `OutDecl`, `OutMethodDecl`, `ConditionDecl`, `ConditionalDecl`,
`ConditionalBranch`, `MethodDecl`) becomes `leading_comment: Option<Comment>`. This is a breaking
change to a widely-used field, acceptable per this project's pre-release "prefer redesigning over
patching" stance — and it's the direct fix for #53, whose root cause is exactly this missing
distinction.

**#105 fix**, in `trivia.rs::analyze_gap`'s backward line scan: today a line matches the
block-comment branch only when `/*` and `*/` both appear on that same line. Extend the scan so
that when a trimmed line ends with `*/` but does not also start with `/*` on the same line (the
close of a comment that opened earlier), it keeps popping and collecting preceding lines until it
finds one that starts with `/*` (the open), then stops — mirroring the existing "a block comment is
one unit; don't merge with an earlier `//` run" rule, just letting that unit span multiple lines.
If the scan exhausts the gap without finding a matching `/*` open (not expected for well-formed
input, but must not panic), it aborts having collected nothing rather than fabricating a comment.

**Formatter re-emission** (`write_trivia` in `fmt.rs`): `Comment::Line` prints exactly as today
(`//` per stored line). `Comment::Block` prints `/* text */` on one line when `text` has no
internal `\n`, or

```text
/*
    <line>
    ...
*/
```

(indented one level past the surrounding declaration) when it does — always a valid,
round-trippable block comment. This is not necessarily byte-identical to the original source's
indentation, consistent with the formatter's existing normalize-don't-preserve philosophy
elsewhere (e.g. blank-line-run collapsing).

**Testing:** the #105 issue's own license-header repro, verbatim, as a formatter test; multi-line
and single-line block comment recovery in `trivia.rs`; `Comment::Line` vs `Comment::Block`
discrimination at every site that carries `leading_comment`; idempotency
(`format(format(x)) == format(x)`) extended to cover block comments.

## 4. Doc-comment formatter re-emission

Each of `write_cell`/`write_relationship`/`write_conditional`/`write_out` (in `fmt.rs`) gains a
doc-comment emission step immediately before its existing `write_trivia` (plain-comment) call,
printing each line of the item's `doc_comment` as `/// <line>` at the item's indent level.
`format_sheet` does the same for `Sheet::doc_comment` as `//! <line>`, before its existing
leading-comment/`sheet` line. When both a plain comment and a doc comment are present on the same
item, they print in source order: plain comment lines, then doc comment lines, then the
declaration itself.

**Testing:** doc-comment re-emission at each of the four sheet-item kinds and at sheet level;
combined plain-comment + doc-comment ordering; idempotency through a reparse.

## 5. `attach_trivia`/formatter — trailing trivia before a block's closing `}` (#52)

**Root cause:** `attach_gaps` only ever computes a gap *between two consecutive items*, so the
gap between a container's *last* item and its own closing `}` — or, for an empty container, the
gap between its opening `{` and closing `}` — is never scanned at all. Fixing this means every
container type that owns a `{ ... }` child list grows its own trailing-trivia slot, separate from
any child's leading trivia:

- `Sheet` (its `items`)
- `RelationshipDecl` (its `methods`)
- `ConditionalDecl` (its `branches`, i.e. the gap before its own outer `}` — not to be confused
  with each branch's own trailing gap)
- `ConditionalBranch` (its `relationships`)
- new `DefaultBranch` struct (its `relationships`)
- `OutDecl` (its `conditions`, or the writer if `conditions` is empty — extending slightly past
  the issue's literal wording to keep this pass exhaustive, since `OutDecl` already gets full
  leading-trivia recursion today and a partial fix would be an inconsistent stopping point)

Each of these gains two fields: `trailing_comment: Option<Comment>` and
`blank_line_before_close: bool`.

**Default-arm restructuring:** `ConditionalDecl::default` changes from
`Option<Vec<RelationshipDecl>>` to `Option<DefaultBranch>`, a new struct mirroring
`ConditionalBranch`'s shape:

```rust
pub struct DefaultBranch {
    pub relationships: Vec<RelationshipDecl>,
    pub trailing_comment: Option<Comment>,
    pub blank_line_before_close: bool,
    pub span: ExprSpan,
}
```

This touches every existing read of `.default` (`ast_parser.rs`'s `parse_conditional_decl`,
`trivia.rs`'s `attach_conditional`, `fmt.rs`'s `write_conditional`, and their tests), each becoming
a small, mechanical adjustment (`Vec<RelationshipDecl>` → `DefaultBranch.relationships`).

**Empty-block coverage:** `Sheet`, `RelationshipDecl`, `ConditionalDecl`, `ConditionalBranch`, and
`DefaultBranch` each also gain an `open_brace_span: ExprSpan` field, capturing their own `{`
token's span (currently parsed via `cursor.expect_open_brace()?` and discarded everywhere) —
needed only as the trailing-gap's *start* boundary when their child list is empty. `OutDecl` is
exempt: its writer is grammar-mandatory, so its block can never be child-empty. When a container's
list is non-empty, the gap starts at the last child's `span.end` instead, and always ends at the
container's own `span.end` (already stored today, since every container's `span.end` is already
exactly its closing `}`'s span — no new field needed for that side).

**`trivia.rs` mechanism:** a new small trait (alongside the existing `TriviaTarget`), implemented
by each container type above, exposing its `open_brace_span()`, its own closing span
(`self.span().end`), and setters for the two new fields. One generic helper computes the trailing
gap (last child's end, or the open-brace span if the list is empty, through to the container's
close) and reuses the existing `analyze_gap` unchanged — it's already agnostic to what sits on
either side of a gap, so no new gap-parsing logic is needed, only new call sites.
`ConditionalDecl`'s own trailing gap is computed specially (mirroring the existing special-casing
`attach_out` already does for `OutDecl`): its "last child" is its `default` if present, else its
last branch, else its own open brace.

**Formatter:** each container's writer emits its trailing comment (honoring
`blank_line_before_close`) immediately before writing its closing `}`, using the same
comment-printing routine `write_trivia` uses for leading comments (factored out once the
`Comment` enum lands in §3, and reused here rather than duplicated).

**Testing:** the issue's literal repro (comment after a sheet's last item, before the closing
`}`) plus one per nested container (relationship's last method, conditional's last branch, default
arm, a branch's own relationships, out's last condition, out's writer with zero conditions); the
empty-block case at every one of those sites; `blank_line_before_close` detection at each;
idempotency through a reparse for all of the above.

## Summary of issue disposition

| Issue | Disposition |
| --- | --- |
| #105 | Fixed — §3's `analyze_gap` rewrite recognizes multi-line block comments. |
| #53 | Fixed — §3's `Comment` enum + formatter changes preserve block-vs-line style. |
| #52 | Fixed — §5 adds trailing-trivia slots to every container type. |
| #58 | Fixed — §1/§2 add first-class, narrowly-scoped `///`/`//!` support. |
| #57 | Reviewed for background only (already-merged PR); no separate action item. |

## Explicitly deferred / out of scope

- General `#[...]` attribute syntax (only the doc-comment shape is recognized).
- Doc comments surfaced via `adam-lsp` hover/completion (design doc Phase 5, not yet built).
- [#54](https://github.com/stlab/cel-rs/issues/54) — column-aware line-wrapping.
- [#55](https://github.com/stlab/cel-rs/issues/55) — range formatting.
