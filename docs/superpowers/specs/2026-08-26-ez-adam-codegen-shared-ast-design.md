# ez-adam Codegen Revision: Share adam-lang's AST/Formatter Instead of Hand-Rolled Strings

**Date:** 2026-08-26
**Branch:** worktree-ez-adam (revises Phase 1, PR #150, not yet merged)
**Status:** Implemented in PR #150, with one revision from this design.

> **Superseded (implementation note):** The `MatchLiteral` extension to
> `adam-lang`'s AST described below (a `Scalar`/`Tuple` enum on
> `ConditionalBranch.literal`, plus parser/formatter support) was **not**
> implemented. Instead, `ez-adam` **decomposes** a multi-cell `Cells`-mode
> conditional into one top-level conditional per non-empty branch, keyed by a
> boolean conjunction over that branch's cell values (e.g. `flag_a && !flag_b`)
> — see `ez-adam/src/codegen/ast_builder.rs`'s
> `build_decomposed_multi_cell_conditionals`. So no `adam-lang` AST change was
> needed. Full tuple-branch-key support in `adam-lang` remains tracked as
> [#173](https://github.com/stlab/cel-rs/issues/173). Read the `MatchLiteral`
> sections below as historical design context, not the shipped approach.

## Summary

Revises `ez-adam`'s `codegen::generate_adm2` (shipped in Phase 1, still
unmerged) to construct an `adam_lang::ast::Sheet` and call the existing,
shared `format_sheet` instead of hand-formatting `.adm2` text via string
templates. This closes a real gap Sean Parent flagged in review: two
independent serialization paths existed (`adam-lang`'s own AST formatter,
and `ez-adam`'s bespoke string builder) where there should be one. (As
originally scoped this also required a small `adam-lang` AST extension for
multi-cell *tuple* conditional-branch keys; as implemented that was avoided
by conjunction-based decomposition instead — see the Superseded note above
and [#173](https://github.com/stlab/cel-rs/issues/173).)

This is a revision to Phase 1's already-implemented `codegen` module, not
new Phase 2 (UI) work — hence landing in `worktree-ez-adam`/PR #150 rather
than the Phase 2 UI worktree.

---

## 1. Motivation

Phase 1's `codegen::generate_adm2` builds `.adm2` text by hand: string
concatenation, manually-threaded indentation, hand-written literal
formatting (the `i64`-suffix/`f64`-`Debug`-formatting logic worked out
during Phase 1's own execution to dodge literal-type-inference ambiguity).
`adam-lang` already has a general, tested "AST → text" formatter
(`format_sheet`, backing `adam-fmt`'s round-trip and the VS Code
extension's formatting support) that handles this exact problem — correct
indentation, canonical spacing — for any `ast::Sheet`. Building a second,
independent formatter inside `ez-adam` duplicates that logic and risks the
two diverging over time. This project is a library-first codebase: shared
capabilities belong in one place, used by every client, not reimplemented
per consumer.

---

## 2. Design decisions (settled during brainstorming)

- **`ez-adam` constructs `adam_lang::ast::Sheet` values and calls
  `format_sheet` for all output.** No more hand-formatted indentation or
  literal-text templates in `ez-adam`'s codegen.
- **User-authored CEL text (formulas, restrict expressions, clamp-call
  bodies) is parsed into `cel_parser::Expr`**, reusing exactly the parser
  entry point `validation::validate_cel_expression` already uses
  (`Parser::<AstContext>::parse_str_ast`), and plugged directly into
  `BindingDecl.body`/`CellFilter.closure`'s body — no hand-built expression
  trees for content a user (or `ez-adam` itself) already expressed as CEL
  text.
- **`generate_adm2` becomes fallible: `Result<String, ExportError>`.**
  Parsing formula/restrict text can genuinely fail (e.g. a still-empty
  formula box mid-edit) — today that silently produces broken `.adm2`
  text; going forward it's a caught, reportable error. This is a
  correctness improvement, not an incidental side effect.
- **A small `ExprSpan`-from-text helper is added to `cel-parser`**, not
  kept private to `ez-adam`. Every hand-built `TypeExpr::Named`,
  `ClosureParamTypeExpr::Named`, and (see below) tuple-branch-literal leaf
  needs a span whose `source_text()` returns the real text (`format_sheet`
  reads these back from the span, not from a semantic value) — achieved by
  tokenizing the exact text wanted (e.g. `"i64".parse::<TokenStream>()`)
  and taking the resulting token's span. This mirrors a trick
  `cel-rs-macros` already relies on internally; making it a reusable
  utility means any future AST-constructing caller gets it for free
  instead of reinventing it.
- **`adam-lang`'s AST gains tuple-branch-literal support**, closing a real
  capability gap rather than working around it: `ast::ConditionalBranch`'s
  match key (`cel_parser::lex_lexer::Literal`) is strictly scalar, but the
  *direct* parser (what `adam-rs` actually runs) already accepts a full
  `or_expression` as a branch key — including a tuple like `(false, true)`
  — via `parser.rs`'s `parse_conditional_with_tuple_typed_match_cell`
  (tested, shipping today). `ez-adam`'s own Phase 1 round-trip test
  already emits and successfully parses exactly this construct through the
  direct parser. The AST-only side should be able to represent what the
  direct parser already accepts — extending it, not adding a documented
  exception, keeps `ez-adam` on one fully-shared path with no carve-outs.

---

## 3. `adam-lang` AST extension: `MatchLiteral`

`adam-lang/src/ast.rs`:

```rust
/// A conditional branch's match key: a single literal, or a
/// parenthesized tuple of them (mirroring a multi-cell condition's tuple
/// value, e.g. `(false, true) => { ... }`).
pub enum MatchLiteral {
    Scalar(cel_parser::lex_lexer::Literal),
    Tuple(Vec<MatchLiteral>),
}
```

`ConditionalBranch.literal: cel_parser::lex_lexer::Literal` becomes
`ConditionalBranch.literal: MatchLiteral`.

**Parser (`ast_parser.rs`):** `parse_conditional_decl`'s branch-key parsing
(currently `cursor.consume_literal()`, single-token only) is extended to
also accept a parenthesized, comma-separated list of literals, producing
`MatchLiteral::Tuple`, mirroring how the direct parser's
`parse_conditional_with_tuple_typed_match_cell` already recognizes this
shape at the token-cursor level. A bare literal continues to produce
`MatchLiteral::Scalar`.

**Formatter (`fmt.rs`):** `write_branch`'s literal-rendering step matches
on `MatchLiteral`: `Scalar` renders via the existing
`source_text_or_empty(span)` path unchanged; `Tuple` renders each element
the same way, joined with `, ` inside parens — matching `.adm2`'s existing
tuple-literal surface syntax exactly (same shape `cell_type_init`'s tuple
type annotations already use).

**Typechecker (`typecheck.rs`) / direct parser (`parser.rs`):** unaffected
— both already handle tuple match keys via their own, separate code path
(`Expr`-based, not `ast::ConditionalBranch`-based). This extension only
brings the AST-only side up to parity.

---

## 4. `ez-adam` codegen redesign

`codegen::generate_adm2(doc: &Document) -> Result<String, ExportError>`
(new error type — see §5), roughly:

```rust
fn generate_adm2(doc: &Document) -> Result<String, ExportError> {
    let sheet = build_sheet_ast(doc)?;
    Ok(adam_lang::format_sheet(&sheet))
}
```

**Type names / filter param types:** `TypeExpr::Named(name, span_for_text(name))`
for each of `ez-adam`'s four types (`f64`/`i64`/`bool`/`String`), using the
new `cel_parser` helper (§2) — no more `type_name`-returns-`&'static str`
used only for display; the string still exists, just also becomes the
seed for a real span.

**Formulas (`BindingDecl.body`) and restrict expressions:** parsed from
their stored `String` via `cel_parser::Parser::<AstContext>::parse_str_ast`
(same call `validation::validate_cel_expression` already makes) — a parse
failure surfaces as `ExportError::InvalidFormula { cell, group, source }`
or similar, not a panic or silently-broken text.

**Clamp filter (`CellFilter`):** the existing logic that decides *which*
clamp/min/max call to synthesize, and formats its literal arguments with
explicit `i64` suffixes / `f64` `Debug` formatting, is unchanged — it still
produces a small CEL snippet like `"clamp(_, 0i64, 100i64)"`. That snippet
is now parsed into an `Expr` (same parser call as formulas) and wrapped in
a hand-built `Expr::Closure { params: [ClosureParam { name: "_", type_expr:
ClosureParamTypeExpr::Named(ty_name, span_for_text(ty_name)), .. }], body,
.. }`, rather than formatted as a complete `filter |_: i64| ...` string.

**`restrict` and `output`:** unchanged in scope — still not emitted
(issues #146, #147 respectively). This revision changes *how* emitted
content is serialized, not *what* gets emitted.

**Relationship groups → `RelationshipDecl`/`BindingDecl`,** conditional
groups → `ConditionalDecl`/`ConditionalBranch`/`DefaultBranch`: direct,
mostly mechanical translation of `Document`'s existing structures into the
corresponding `ast` types, using `MatchLiteral::Scalar`/`::Tuple` (§3) for
branch keys depending on whether the source `ConditionalGroup`'s condition
has one cell or several. All `leading_comment`/`doc_comment`/
`blank_line_before*` fields on hand-built nodes are `None`/`false` —
`attach_trivia` is a separate, parse-only post-processing step that
doesn't apply to hand-built ASTs.

---

## 5. Error handling

```rust
pub enum ExportError {
    /// A relationship-group member's formula text isn't valid CEL.
    InvalidFormula { group: RelationshipGroupId, cell: CellId, source: cel_parser::ParseError },
    /// A conditional group's Formula-mode condition expression isn't valid CEL.
    InvalidCondition { conditional: ConditionalGroupId, source: cel_parser::ParseError },
}
```

(Exact variant shape/naming is an implementation-plan-level detail, not
fully pinned down here — the contract is "codegen fails loudly and
specifically on invalid stored CEL text, rather than emitting broken
`.adm2`.")

---

## 6. Testing implications

- All of Phase 1's existing codegen tests (`codegen::tests`,
  `adm2_round_trip.rs`, `end_to_end.rs`) get re-verified against the new
  implementation — same inputs, same expected `.adm2` *output text*
  (character-for-character, since `format_sheet` is expected to produce
  equivalent canonical formatting to what was hand-rolled, modulo any
  incidental whitespace differences that get reconciled during
  implementation).
- New tests for `adam-lang`'s `MatchLiteral` extension: parsing a
  tuple-literal branch key via `AdamAstParser`, formatting a hand-built
  `ConditionalBranch { literal: MatchLiteral::Tuple(..), .. }` back to
  text, and a round-trip (parse → format → parse again, same AST)
  matching `adam-fmt`'s existing testing convention for this file.
- New tests for `ExportError`: an empty/invalid formula produces
  `Err(ExportError::InvalidFormula { .. })`, not a panic or bad text.
- `span_for_text`-equivalent helper in `cel-parser`: unit tests confirming
  `source_text()` on the returned span actually returns the input text (a
  single-token guarantee — the helper's contract requires its input to
  tokenize to exactly one token; multi-token input is a precondition
  violation, not a supported use).

---

## 7. Deferred / explicitly out of scope

- Everything Phase 1 and the Phase 2 UI design already deferred, unchanged
  (issues #146, #147, #148; live evaluation; `.adm2` import; multiple
  sheets per document).
- Extracting shared cell↔widget-binding logic between `begin` and
  `ez-adam` (Sean's second piece of feedback) — tracked as its own,
  separate piece of work, sequenced after this codegen revision per the
  agreed plan; not addressed by this design.
- Further unifying `adam-lang`'s direct parser and AST-only parser beyond
  the specific `MatchLiteral` gap closed here (e.g. they remain two
  separate parsers/trees in general) — out of scope; this design closes
  the one concrete capability gap `ez-adam` actually hit, not a general
  parser-unification effort.
