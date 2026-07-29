# pm-lang Language Server & VS Code Extension — Design

## Goal

Give `.adm2` (pm-lang) source files real editor tooling in VS Code: diagnostics, an
auto-formatter following `cargo fmt` conventions, hover, go-to-definition/find-references,
completion, and document symbols — without duplicating the pm-lang/CEL grammar into a second,
independently-maintained parser. pm-lang's operator/identifier/type system is extensible at
runtime (host binaries register types and operators), so the checker is explicitly best-effort,
not a complete type system.

This spec covers architecture and phasing only. Each phase below becomes its own dated
implementation plan (matching this repo's existing convention), executed one at a time.

## Background / constraints that shaped this design

- `cel-parser`'s `CELParser` and `pm-lang`'s `PmParser` currently parse and *execute* in the
  same recursive-descent pass: grammar productions call directly into `DynSegment` (building
  runtime stack ops) or `Sheet` (adding cells/relationships), rather than building an
  intermediate tree. There is no error recovery — the first syntax error aborts the parse.
- Both tokenize via `proc_macro2::TokenStream` (through `cel_parser::lex_lexer::LexLexer`).
  `proc_macro2`'s tokenizer discards comments as trivia before either parser ever sees them, the
  same way `rustc`'s own tokenizer does for `rustfmt`.
- This tokenization choice is deliberate and must not change: it's what lets `cel-parser` run
  standalone (as today) *and* inside a real `#[proc_macro]` (planned — see `VISION.md`'s "second
  parser backend: compiling CEL directly to Rust via a macro," expected to live in
  `cel-rs-macros`). `proc_macro2::Span` bridges transparently to `proc_macro::Span` in that
  context.
- `begin`'s hot-reload of `demo.adm2` (editing the file, `dx serve` picking it up, diagnostics to
  stderr) already exists; the only change it needs is the `.pm` → `.adm2` rename of the demo asset
  and the `begin` codepaths that reference it — this project is purely about editor-side tooling
  for authoring `.adm2` files, not about the running-app reload path.
- No existing LSP or VS Code extension work exists in this repo to build on.

## Parser architecture

### `ParserContext`: one grammar, pluggable backends

Generalize `CELParser`/`PmParser`'s recursive-descent grammar into `Parser<C: ParserContext>`
(monomorphized generics, not `Box<dyn Trait>` — this project's code style already prefers
generics over trait objects when the backend set is statically known, and it is here). The
grammar productions (`is_or_expression`, `is_additive_expression`, ..., and pm-lang's
`parse_cell_decl`, `parse_relationship_decl`, ...) are written once, generic over the context,
and call context methods like "push a literal," "apply a named operator to N operands," "declare
a cell," "declare a relationship" instead of touching `DynSegment`/`Sheet` directly.

Two concrete contexts:

- **`DynSegmentContext`** — today's behavior, verbatim, now expressed through the trait instead
  of hardcoded. Zero behavior change, zero overhead added, all existing `cel-parser`/`pm-lang`
  tests continue to pass unchanged. This stays the runtime/hot-reload execution path.
- **`AstContext`** — builds a real, structured, span-carrying tree (`cel_parser::ast::Expr` for
  CEL expressions; `pm_lang::ast::Sheet`/`CellDecl`/`Relationship`/`Method`/`Conditional` for
  pm-lang structure, referencing `Expr` nodes for method bodies) instead of executing anything.
  This is the one new consumer this project adds, and it's what the LSP, the formatter, and
  (later, separately) the planned macro-compilation backend all build on — that backend is not a
  third trait implementation; it's a downstream consumer of this same AST, with its own
  static-type-inference and `quote!`-emission passes layered on top whenever that work starts
  (deferred, out of scope here).

Why two contexts and not three, and not "always build the AST": `DynSegmentContext` dispatches
on the *runtime* types of values already on its stack — a single-pass, no-tree operation well
suited to the interpreter's "fast dev-time iteration" mission (VISION.md), which building and
then walking a tree would only add overhead to for no benefit. A future codegen backend, to
preserve CEL semantics (e.g. `checked_add`+error-propagation instead of bare `+`), needs each
sub-expression's *static* type — information only available after inference over the whole
expression, not derivable one token at a time during parsing — so it's much better served by
consuming an already-built AST than by being forced into a third single-pass trait
implementation it can't faithfully use anyway.

### Error recovery (AstContext only)

`DynSegmentContext` keeps today's fail-fast behavior (a syntax error aborts the parse — correct
for runtime execution). `AstContext` adds coarse, declaration/statement-boundary recovery: on a
syntax error inside a `cell`/`relationship`/`conditional` item or a `method` declaration, record
a diagnostic, insert an error placeholder node, skip tokens until the next likely recovery point
(`;` or a balancing `}`), and keep parsing the rest of the file. This is what lets the LSP report
every syntax error in a file instead of just the first. Recovery is *not* attempted inside
individual expressions (e.g. mid-arithmetic-expression) — statement-level granularity is enough
for useful multi-error diagnostics without the complexity of fine-grained expression recovery.

### Comments and trivia (AstContext only, orthogonal to the trait)

Comments are recovered without a second lexer: `proc_macro2::Span::source_text()` (already used
in this codebase — see `cel-parser/src/lex_lexer.rs`'s `test_span_preservation`) returns the
exact original text for any span, so the gap between two consecutive tokens' spans can be
re-sliced from the original source string to recover whitespace/comment text — the same
technique `rustfmt` uses for the identical problem (`rustc`'s tokenizer also discards comments).
This is a standalone pass, run only when using `AstContext`, that attaches leading/trailing
comment text to the nearest AST node by span; `DynSegmentContext` and any future codegen
consumer never need to know it exists.

## Type checking (v1)

A minimal `Ty` model: the built-in primitives `TypeRegistry::new()` already registers by default
(`i8`..`i128`, `u8`..`u128`, `isize`/`usize`, `f32`/`f64`, `bool`, `String`) plus `Ty::Any`. Any
type name not recognized as a built-in resolves to `Ty::Any` — not an error. Checking rules:

- `Ty::Any` unifies with anything silently, in both directions — no diagnostic, matching pm-lang
  and CEL's extensible type system (custom types registered by a host binary are invisible to
  the LSP, and treating them as unchecked-but-valid is correct rather than reporting false
  errors).
- Two concrete built-in types are checked against a shared signature table (operand types →
  result type, per operator) — refactored out of `cel_parser::op_table`'s existing
  `BuiltinScope`/`OpSignature` machinery so the runtime dispatch and the static checker read the
  same data and can't drift apart. This is where real diagnostics come from: arity mismatches,
  incompatible operand types, and output-type mismatches (e.g. an `f64` method body assigned to
  an `i32`-declared cell) — the same class of error the runtime already catches today, just
  available before propagation, from the editor.

Declaring a project's custom registered types/operators for richer static checking (a manifest
or in-source declaration syntax) is explicitly deferred — not part of this project.

## Formatter

A module in `pm-lang` (the crate that owns the top-level `Sheet`-shaped AST) that walks the
`AstContext`-built, trivia-annotated tree and pretty-prints it, delegating to a sibling
formatting module in `cel-parser` for individual `Expr` subtrees (method bodies, initializers).
Follows `cargo fmt` conventions: 4-space indent, 100-column width, opening braces on the same
line, single-space normalization around operators (not preserving hand-authored alignment like
the space-padded `cell width:  f64 = 1920.0;` seen in current test fixtures — `rustfmt` doesn't
preserve manual alignment either), at most one blank line collapsed between items, and comments
re-emitted verbatim, attached to their nearest node. Exposed as a plain function usable by both
the LSP (`textDocument/formatting`) and, later if wanted, a small standalone CLI — no separate
crate needed for this.

## Crate/file layout

No new crate for syntax — the AST types and `AstContext` live directly in `cel-parser` (CEL
expression AST) and `pm-lang` (structural AST, referencing `cel-parser`'s `Expr`), alongside the
existing parsers. One new crate is added for the server itself:

- **`pm-lsp`** (new workspace member) — the language server binary. Built on `lsp-server` +
  `lsp-types` (rust-analyzer's own choice: synchronous, no async runtime needed, and this
  workspace has no existing async runtime dependency to justify `tower-lsp`). Depends on
  `cel-parser` and `pm-lang` directly, using their `AstContext` parse entry points; never
  constructs a `Sheet`, so it never needs `cel-runtime`/`property-model` at the type-checking
  layer (those crates are still transitively pulled in via `pm-lang`'s existing dependencies —
  acceptable compile-time cost for now, not a blocking concern; revisit with Cargo features
  later only if it becomes one).
- **`editors/vscode-pm-lang/`** (new top-level directory, TypeScript, own `package.json`,
  excluded from the Cargo workspace) — the VS Code extension. Static TextMate grammar for v1
  syntax highlighting (LSP-driven semantic highlighting deferred). Client wiring via
  `vscode-languageclient`, spawning `pm-lsp` over stdio. A `pm-lang.serverPath` setting (default:
  search `PATH`/workspace target dir) rather than bundling prebuilt binaries, matching this
  project's pre-release status (no distribution story needed yet).

## Phasing

Each phase becomes its own implementation plan.

1. **Generalize the parser.** Introduce `ParserContext` (or a small family of related traits
   spanning `cel-parser`'s expression grammar and `pm-lang`'s declaration-level grammar) and
   `DynSegmentContext`. Behavior-preserving refactor — every existing `cel-parser`/`pm-lang` test
   must keep passing unchanged.
2. **`AstContext` + AST types.** Add the CEL expression AST (`cel-parser`) and pm-lang structural
   AST (`pm-lang`), coarse error recovery, and the comment/trivia reattachment pass. Unit tests
   assert AST shape per grammar construct, recovery producing multiple diagnostics from one
   file, and trivia round-tripping.
3. **`pm-lsp` diagnostics + minimal VS Code extension.** Syntax errors (from `AstContext`
   recovery) and type errors (§ Type checking) surfaced as `textDocument/publishDiagnostics`.
   Extension: TextMate grammar, client wiring, diagnostics only. First end-to-end usable
   milestone.
4. **Formatter.** Wired into `pm-lsp`'s `textDocument/formatting` and the extension's
   format-on-save. Golden-file tests, idempotency tests (`format(format(x)) == format(x)`),
   comment-preservation tests.
5. **Hover, go-to-definition/find-references, completion, document symbols.** Built on the same
   AST; identifier resolution reuses the cell-name scoping already established by
   `pm_lang::parser`'s existing `cell_names` map concept.

**Deferred, explicitly out of scope for this project:** a declared-type manifest or in-source
syntax for richer static checking of custom types; a live companion-binary checker; LSP semantic
highlighting; the macro-compilation backend itself (this design only ensures the AST it needs
will already exist).

## Testing strategy

- Phase 1: the existing `cel-parser`/`pm-lang` test suites are the regression net — they must
  pass byte-for-byte unchanged after the `ParserContext` generalization.
- Phase 2: new tests per grammar construct asserting `AstContext` tree shape; multi-error files
  asserting more than one diagnostic is recovered; comment-placement tests.
- Type checker: one test per built-in operator/type-pair combination (mirroring the existing
  `op_table` test matrix), plus tests asserting `Ty::Any` unifies silently with every built-in
  and with itself.
- Formatter: golden-file input/output pairs, idempotency, and comment-preservation tests.
- `pm-lsp`: handler-level unit tests (calling the diagnostic/hover/etc. logic directly against an
  `AstContext` tree) rather than full protocol round-trip tests where possible; a small number of
  real stdio-transport tests for the server entry point itself.
- VS Code extension: no automated UI test suite planned for v1 (small, internal, pre-release
  tool); manual verification checklist per phase (open a `.adm2` file, confirm diagnostics /
  formatting / hover / goto-def / completion as each phase lands), similar in spirit to this
  repo's existing `verifying-begin-ui` manual-verification approach.
