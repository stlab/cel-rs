# pm-lang Language Server — Handoff into Phase 3

Status snapshot as of 2026-07-18, written for whoever picks up Phase 3 in a new
conversation/context. Read `docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`
first for the full design and phasing — this doc only summarizes what's done, what's
deliberately deferred, and what Phase 3 needs to decide/do next.

## What's done

**Phase 1 (parser generalization) — complete, merged (PR #41, #42 area).**
`cel-parser`'s `CELParser` is now `Parser<C: ParserContext>`, generic over a pluggable backend.
`DynSegmentContext` reproduces the original runtime-execution behavior exactly. See
`docs/superpowers/plans/2026-07-17-pm-lang-lsp-parser-context.md`.

**Phase 2, cel-parser half — complete, merged (PR #42).**
`cel_parser::ast::{Expr, Literal, ExprSpan, LogicalOp}` plus `cel_parser::AstContext`
(`Parser<AstContext>`) build a span-carrying CEL expression tree instead of executing.
`AstContext` resolves nothing and never fails on semantic grounds — all validation is deferred.
Entry points: `Parser::<AstContext>::parse_str_ast`/`parse_tokens_ast`/`parse_or_expression_ast`.
See `docs/superpowers/plans/2026-07-17-cel-parser-ast-context.md`.

**Phase 2, pm-lang half — complete, this branch (`worktree-language-server-vs-code-extension-2b`,
commits `1b74787`..`3282746`).**
Added to the `pm-lang` crate:
- `pm_lang::TokenCursor` (private) — pure token-stream cursor shared by both parsers.
- `pm_lang::ast::{Sheet, SheetItem, CellDecl, RelationshipDecl, ConditionalDecl,
  ConditionalBranch, MethodDecl}` — structural AST, referencing `cel_parser::Expr` for method
  bodies. Carries no resolved types/`TypeRegistry` lookups; all semantic validation deferred.
- `pm_lang::PmAstParser` (`new()`, `parse_str(&mut self, source: &str) -> Result<ast::Sheet>`) —
  a **standalone** parser, entirely separate from the existing `pm_lang::PmParser`. Coarse,
  declaration-level error recovery: a malformed `cell`/`relationship`/`conditional` item is
  recorded in `Sheet.errors` and replaced with a `SheetItem::Error` placeholder; parsing resumes
  at the next sheet item.
- `pm_lang::attach_trivia(source, &mut sheet)` — recovers comments `proc_macro2` discards by
  re-slicing gaps between consecutive items' spans, attaching the trailing comment block as the
  following item's `leading_comment`. Only inter-item gaps are covered (not the gap before the
  sheet's first item).

`pm_lang::PmParser` (the existing `Sheet`/`TypeRegistry`/`DynSegment`-executing path used by
`begin`'s hot reload) is **completely untouched**. Full plan:
`docs/superpowers/plans/2026-07-18-pm-lang-ast.md`.

## Deliberately deferred / not done

1. **`PmParser`'s grammar is still hand-duplicated against `PmAstParser`'s.** The 2026-07-18 plan's
   Architecture section explains why (genericizing over a shared trait would thread a
   `&TypeRegistry` parameter through methods half the implementations ignore — see that plan for
   the full reasoning) and explicitly calls this out as a **required immediate follow-on plan**:
   migrate `PmParser::parse_str` to be implemented as "parse via `PmAstParser`, then compile the
   AST into a `Sheet`" (a new `pm_lang::compile` module doing `TypeRegistry` resolution + a new
   `Expr → DynSegment` compiler reusing `cel_parser::ParserContext`'s existing trait against
   `DynSegmentContext`), deleting the old inline grammar from `parser.rs` entirely. **This has not
   been started.** Until it lands, the pm-lang grammar genuinely lives in two places — anyone
   changing pm-lang's syntax must update both `parser.rs` and `ast_parser.rs` by hand.
2. **Type checking (v1)** (design doc's own section) is not implemented at all — no `Ty` model, no
   op-signature-table refactor out of `cel_parser::op_table`. Phase 3's diagnostics need this for
   type errors (arity mismatches, incompatible operand types, output-type mismatches); syntax
   errors alone (from `PmAstParser`'s recovery) don't need it.
3. **Known, accepted limitation** in `PmAstParser`'s error recovery: a CEL `if`-expression whose
   then/else body fails to parse and leaves a dangling, unmatched `}` behind can still abort the
   *whole* `parse_str` call instead of recovering just the one malformed item (root cause: CEL's
   `if`/`else` grammar reuses `Delimiter::Brace`, the same kind pm-lang's own grammar tracks, so a
   delimiter-kind-based recovery guard can't tell them apart). Documented in doc comments on
   `pm_lang::token_cursor::TokenCursor::skip_to_recovery_point` and a regression test
   (`recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse` in
   `pm-lang/src/ast_parser.rs`) that pins today's behavior. Tracked in
   [github.com/stlab/cel-rs#43](https://github.com/stlab/cel-rs/issues/43) — a general fix needs
   `cel_parser::Parser<C>` to report back what it left unbalanced on a failed parse.

## Decision Phase 3 needs to make early

The design doc's Phase 3 is "`pm-lsp` diagnostics + minimal VS Code extension... syntax errors
(from `AstContext` recovery) and type errors... First end-to-end usable milestone." It doesn't
explicitly require item 1 above (`PmParser`/`PmAstParser` unification) first. Decide up front:

- **Option A:** Start Phase 3 (type checking + `pm-lsp` + VS Code extension) directly against
  today's `PmAstParser`, leaving the `PmParser` duplication for a later cleanup pass. Faster to a
  usable milestone; the duplication risk is real but contained (both parsers already have full
  test coverage, so a drift would surface as a test failure, not silent breakage).
- **Option B:** Do the `PmParser`/`PmAstParser` unification follow-on first, so pm-lang's grammar
  lives in exactly one place before building more on top of `PmAstParser`. Safer long-term, delays
  the first usable `pm-lsp` milestone.

This is a real product-sequencing call, not a technical one — flag it to the user early in the
Phase 3 conversation rather than assuming either way.

## Key files for Phase 3 to build on

- `cel_parser::{Parser, AstContext, Expr, ExprSpan, Literal, LogicalOp}` — `cel-parser/src/ast.rs`,
  `cel-parser/src/parser_context.rs`, `cel-parser/src/lib.rs`.
- `pm_lang::{PmAstParser, ast::*, attach_trivia}` — `pm-lang/src/ast.rs`, `ast_parser.rs`,
  `trivia.rs`, `token_cursor.rs`.
- `cel_parser::op_table::{OpLookup, BuiltinScope, OpSignature}` (`cel-parser/src/op_table.rs`) —
  the existing runtime op-dispatch table Phase 3's type checker needs to refactor a shared
  signature table out of (see design doc's "Type checking (v1)" section).
- New crate to create: `pm-lsp` (workspace member, `lsp-server` + `lsp-types`, depends on
  `cel-parser`/`pm-lang` directly via their `AstContext`/`PmAstParser` entry points).
- New top-level directory: `editors/vscode-pm-lang/` (TypeScript, own `package.json`, excluded
  from the Cargo workspace).
