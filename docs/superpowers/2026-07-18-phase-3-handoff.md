# pm-lang Language Server — Handoff into Phase 3

Status snapshot as of 2026-07-20, written for whoever picks up the rest of Phase 3 in a new
conversation/context. Read `docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`
first for the full design and phasing — this doc only summarizes what's done, what's
deliberately deferred, and what's left before Phase 3 is complete.

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

**Phase 2, pm-lang half — complete, merged (PR #44).**
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

**Phase 3, sub-plan 1 (type checking v1) — complete, PR #45
(`worktree-language-server-vs-code-extension-3`, commits `c280c2b`..`949032e`).**
The design doc's Phase 3 was split into three sub-plans per writing-plans' scope-check guidance
(type checking / `pm-lsp` / VS Code extension) rather than one plan spanning all three — see
"What's left" below. This sub-plan resolved the "decide up front" sequencing question the previous
handoff draft flagged (Option A: build directly on `PmAstParser`, leaving `PmParser`/`PmAstParser`
unification for a later cleanup pass) and added:

- `cel_parser::Ty` (`cel-parser/src/ty.rs`) — a minimal static type model: the built-in primitives
  `TypeRegistry::new()` registers by default, plus `Ty::Any`. `Ty::from_literal`,
  `Ty::from_type_id`, `Ty::type_id`, `Ty::name`, and `Ty::unifies_with` (where `Any` unifies
  silently with everything, in both directions — the hard correctness property the whole design
  leans on).
- `cel_parser::op_table::{OperandTypes, builtin_operand_types}` (`cel-parser/src/op_table.rs`) —
  projects the existing runtime operator-dispatch tables (`BUILTINS`, `LEFT_SHIFT_SIGNATURES`,
  `RIGHT_SHIFT_SIGNATURES`) into a checker-usable `(arity, lhs TypeId, rhs TypeId)` shape, via a
  `signatures_for` helper now also shared internally by `BuiltinScope::lookup` — the static checker
  and the runtime dispatcher read one source of truth and can't drift apart. Never exposes
  `op_fn` (execution-only).
- `cel_parser::ty::check_expr` (`cel-parser/src/ty.rs`) — walks a `cel_parser::Expr` tree. Checks
  `Expr::Op` (via `builtin_operand_types`) and `Expr::Logical` (fixed `&&`/`||` bool semantics)
  directly; recurses into `Expr::Apply`/`Expr::Tuple`/`Expr::TupleIndex`/`Expr::If` (so a nested
  `Op` error still surfaces) but infers those node kinds themselves as `Ty::Any` — checking
  call/tuple/if shapes is deferred, matching "not a complete type system."
- `pm_lang::typecheck::check_sheet` (`pm-lang/src/typecheck.rs`) — the pm-lang-specific layer:
  checks each `cell`'s literal initializer against its `: type_name` annotation (resolving the
  raw, unresolved `cel_parser::lex_lexer::Literal`/`syn::Lit` via pm-lang's own suffix-defaulting
  convention), and each `relationship`/`conditional` method's body against its declared outputs
  (arity: single value vs. n-tuple, including a tuple-shaped body over-producing values for a
  single declared output; and per-output type). Returns `Vec<cel_parser::ParseError>` — the same
  diagnostic type `Sheet.errors` already uses for syntax errors.

Built via subagent-driven development (4 tasks, each independently reviewed, plus a final
whole-branch review whose Minor findings were fixed in a follow-up polish commit). Full plan:
`docs/superpowers/plans/2026-07-18-pm-lang-type-checking-v1.md`.

**Phase 3, sub-plan 2 (`pm-lsp`) — complete, PR #46
(`worktree-language-server-vs-code-extension-3b`, commits `cc52b02`..`92d10ce`).**
Added a new workspace crate, `pm-lsp` — the language server binary, built on `lsp-server` 0.10 +
`lsp-types` 0.97 (synchronous, no async runtime, matching the design doc's choice). Resolved both
open questions the previous handoff draft (above) flagged for this sub-plan up front, rather than
letting them emerge mid-implementation:

- **`ParseError → CELError` conversion point:** decided to convert immediately inside
  `pm_lsp::diagnostics::diagnostics_for_source`, before the function returns — `PmAstParser`/
  `check_sheet` themselves are unchanged and still return `cel_parser::ParseError`, matching every
  other consumer. No `ParseError` is ever stored or passed across a thread/task boundary inside
  `pm-lsp`.
- **Name-resolution diagnostics (undeclared cell references):** explicitly *not* added in this
  sub-plan. `check_sheet` still resolves unknown identifiers to `Ty::Any` and never flags them
  (correct for a type checker); a separate resolution pass is deferred to whichever later phase
  needs identifier resolution anyway (hover/goto-def/completion — Phase 3's phase 5 per the
  design doc, which already plans to reuse `pm_lang::parser`'s `cell_names` scoping).

Also decided, not previously flagged: **no in-memory document store.** Every diagnostic recompute
(`didOpen`/`didChange`) already carries the client's full current text in its own notification
params (the server declares `TextDocumentSyncKind::FULL`), so there's nothing to persist for a
diagnostics-only server — adding a `Uri -> text` map now would sit unused until a later phase
(hover/goto-def/completion) actually reads from it. `textDocument/didClose` is intentionally
unhandled for the same reason.

Structure: `pm_lsp::diagnostics::diagnostics_for_source(&str) -> Vec<lsp_types::Diagnostic>` is a
pure function (`pm-lsp/src/diagnostics.rs`) wrapping `PmAstParser::parse_str` + `check_sheet`,
tested directly with no LSP transport involved. `pm_lsp::{run, serve}` (`pm-lsp/src/dispatch.rs`)
wire that function to `lsp-server`'s `Connection`: the initialize handshake, then
`didOpen`/`didChange` → `publishDiagnostics`; a malformed notification is logged to stderr and
skipped rather than crashing the server (added in a post-review fix pass), while a broken
transport (a failed send) still ends the server, matching `lsp-server`'s own idiom for `shutdown`/
`exit`. Tested via `lsp_server::Connection::memory()` (an in-process client/server pair) rather
than a real subprocess — full handshake, `didOpen`, `didChange`, the malformed-notification path,
and the unhandled-method-not-found fallback are all covered. Built via subagent-driven development
(2 tasks, each independently reviewed, plus a final whole-branch review whose one Important and
two Minor findings were fixed in a follow-up commit and re-reviewed clean). Full plan:
`docs/superpowers/plans/2026-07-20-pm-lsp.md`.

## What's left

The design doc's Phase 3 still needs, as a separate sub-plan (not started):

1. **VS Code extension** (`editors/vscode-pm-lang/`) — TextMate grammar, `vscode-languageclient`
   wiring to `pm-lsp` over stdio, `pm-lang.serverPath` setting. `pm-lsp` now exists (see above) —
   nothing else blocks starting this sub-plan.

Also still deliberately deferred (unchanged from before sub-plan 1, and not addressed by it):

1. **`PmParser`'s grammar is still hand-duplicated against `PmAstParser`'s.** The
   2026-07-18 pm-lang-ast plan's Architecture section explains why (genericizing over a shared
   trait would thread a `&TypeRegistry` parameter through methods half the implementations
   ignore) and explicitly calls this out as a **required immediate follow-on plan**: migrate
   `PmParser::parse_str` to be implemented as "parse via `PmAstParser`, then compile the AST into
   a `Sheet`" (a new `pm_lang::compile` module doing `TypeRegistry` resolution + a new `Expr →
   DynSegment` compiler reusing `cel_parser::ParserContext`'s existing trait against
   `DynSegmentContext`), deleting the old inline grammar from `parser.rs` entirely. **This has
   not been started.** Until it lands, the pm-lang grammar genuinely lives in two places —
   anyone changing pm-lang's syntax must update both `parser.rs` and `ast_parser.rs` by hand.
2. **Known, accepted limitation** in `PmAstParser`'s error recovery: a CEL `if`-expression whose
   then/else body fails to parse and leaves a dangling, unmatched `}` behind can still abort the
   *whole* `parse_str` call instead of recovering just the one malformed item (root cause: CEL's
   `if`/`else` grammar reuses `Delimiter::Brace`, the same kind pm-lang's own grammar tracks, so a
   delimiter-kind-based recovery guard can't tell them apart). Documented in doc comments on
   `pm_lang::token_cursor::TokenCursor::skip_to_recovery_point` and a regression test
   (`recovery_known_limitation_if_expr_dangling_brace_aborts_whole_parse` in
   `pm-lang/src/ast_parser.rs`) that pins today's behavior. Tracked in
   [github.com/stlab/cel-rs#43](https://github.com/stlab/cel-rs/issues/43) — a general fix needs
   `cel_parser::Parser<C>` to report back what it left unbalanced on a failed parse.
3. **A declared-type manifest or in-source syntax for richer static checking of custom types** —
   the type checker (v1) only checks the built-in primitives `TypeRegistry::new()` registers by
   default; any custom type a host binary registers resolves to `Ty::Any` and is never checked.
   Explicitly out of scope per the design doc.

Neither `pm-lsp` nor the VS Code extension needs the `PmParser`/`PmAstParser` unification (item 1
above) first — both build only on `PmAstParser`/`check_sheet`, which are already unaffected by
that duplication. Flag the unification as a separate, independent cleanup task if/when it's
picked up, not a blocker for the remaining Phase 3 work.

## Key files for the remaining Phase 3 work to build on

- `cel_parser::{Parser, AstContext, Expr, ExprSpan, Literal, LogicalOp}` — `cel-parser/src/ast.rs`,
  `cel-parser/src/parser_context.rs`, `cel-parser/src/lib.rs`.
- `cel_parser::{Ty, ty::check_expr}` (`cel-parser/src/ty.rs`) and
  `cel_parser::op_table::{OperandTypes, builtin_operand_types}` (`cel-parser/src/op_table.rs`) —
  the type checker (v1), this handoff's new addition.
- `pm_lang::{PmAstParser, ast::*, attach_trivia, check_sheet}` — `pm-lang/src/ast.rs`,
  `ast_parser.rs`, `trivia.rs`, `token_cursor.rs`, `typecheck.rs`.
- `cel_parser::{ParseError, CELError}` (`cel-parser/src/error.rs`) — the `!Send + !Sync` bridge
  `pm_lsp::diagnostics::diagnostics_for_source` converts across; see sub-plan 2 above.
- `pm_lsp::{diagnostics::diagnostics_for_source, run, serve}` — `pm-lsp/src/diagnostics.rs`,
  `dispatch.rs`. The VS Code extension's only dependency on this crate is the `pm-lsp` *binary*
  (spawned over stdio by `vscode-languageclient`), not these Rust APIs directly.
- New top-level directory to create: `editors/vscode-pm-lang/` (TypeScript, own `package.json`,
  excluded from the Cargo workspace).
