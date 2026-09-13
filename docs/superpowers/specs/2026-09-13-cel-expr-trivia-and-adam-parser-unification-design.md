# CEL Expression Trivia and adam-lang Parser Unification

**Resolves:** stlab/cel-rs#201 ("adam-fmt formatter: comments inside a CEL expression body are not
recoverable")

## Background

`cel-parser` already implements a one-grammar/multiple-backends model: `Parser<C: ParserContext>`
drives either `DynSegmentContext` (compiles straight into an executable `DynSegment`) or
`AstContext` (builds a span-carrying `Expr` tree). `Expr`'s own doc comment already names its
intended consumers beyond adam-lang: the language server, the formatter, and "the future
macro-compilation backend." `ExprSpan`'s doc comment separately anticipates comment/trivia
recovery as "a separate, deferred pass" operating on spans, not as data carried on `Expr` itself.

`adam-lang`, by contrast, still has two independently hand-written parsers. `AdamParser` parses
declaration syntax and executes into a live `adam_rs::Sheet` in one pass, resolving
`TypeRegistry` entries and building `Method`/`Filter`/`Requirement` values as it goes.
`AdamAstParser` re-implements the same declaration syntax separately to build `ast::Sheet`, an
untyped structural tree consumed by the formatter and language server. The two share only a pure
token cursor. Both already delegate CEL sub-expressions to `cel_parser::Parser<DynSegmentContext>`
and `Parser<AstContext>` respectively, so the CEL grammar itself is not duplicated; what's
duplicated is the surrounding adam-lang declaration grammar (`cell`, `relate { }`,
`out ... require { }`, and so on) — evidenced by every past grammar change (e.g. the
`2026-08-19-adam-lang-syntax-revision.md` rewrite) needing to touch both parsers in lockstep. A
prior plan's self-review flagged, but deferred, a follow-on: retire `AdamParser`'s inline grammar
and replace it with "parse via `AdamAstParser`, then compile `ast::Sheet` into a `Sheet`."

Issue #201's root cause is a special case of the same split: `cel_parser::Expr` carries no
trivia, and adam-lang's trivia recovery (`trivia.rs`) only scans gaps between adam-lang's own AST
nodes, so a comment inside a CEL expression body has nowhere to attach in either backend.

## Goals

- Fix #201: a comment on its own line inside a CEL expression body survives an `adam-fmt`
  reformat.
- Converge adam-lang onto a single grammar implementation, removing the maintenance burden of two
  hand-duplicated parsers.
- Do both in a way that sets up, rather than forecloses, the longer-term direction: a compile-time
  `cel-rs-macros` backend that emits real Rust closures, compiling a whole adam sheet to a native
  "sheet instance," and eventually a grammar-description DSL with pluggable backends.

## Non-goals

- A fully general trivia system that recovers a comment at *any* token boundary, including a
  same-line trailing comment (`1 + 2 // trailing`). Only a comment on its own line, immediately
  preceding the next token, is recovered — the same granularity adam-lang already recovers at the
  declaration level. This is a deliberate, documented boundary, not a silent gap.
- The `cel-rs-macros` compile-time closure backend, sheet-to-Rust-instance compilation, and the
  grammar-description DSL. These are named in the Roadmap section below as tracked future work,
  not designed here.
- Any change to `AdamParser`'s public error-handling contract (`Result<ParsedSheet>`, failing on
  the first error encountered).

## Part A: CEL expression trivia (#201)

`Expr` gains no new fields. It stays exactly as documented today — no resolved types, no
trivia — matching `ExprSpan`'s own anticipation of a separate, span-based trivia pass rather than
stored comment data. Trivia recovery becomes a responsibility of `cel_parser::format_expr`, since
only the function regenerating text token-by-token can correctly map an original source gap onto
the position it's currently emitting.

### Shared trivia primitive

adam-lang's existing `trivia.rs` already implements the core algorithm needed here: given a raw
source substring between two known points, recover a trailing comment run (`analyze_gap`) and
whether a blank line remains. `format_expr` needs the same operation, just applied between an
`Expr` node's own child spans instead of between adam-lang's sheet items. Rather than duplicating
this logic in `cel-parser`, extract it, along with the `Comment` enum (`Line`/`Block`), out of
`adam-lang::ast` and into a new `cel-parser/src/trivia.rs` module, alongside `ExprSpan`'s own file.
`adam-lang` already
depends on `cel-parser`, so this introduces no circular dependency; `adam-lang`'s own trivia pass
switches to calling the shared primitive instead of its own copy.

### `format_expr` changes

`format_expr`'s signature changes from `fn format_expr(expr: &Expr) -> String` to
`fn format_expr(expr: &Expr, source: &str) -> String` — `Span::source_text()` returns a span's own
text, not the text between two spans, so recovering a gap requires the whole source string plus
each span's line/column position, the same inputs adam-lang's `trivia.rs` already uses.

Every recursive point in `format_expr` where it currently emits a fixed separator between two
child expressions — an operand pair inside `Op`, a call's arguments inside `Apply`, elements
inside `Tuple`, the branches inside `If`, a closure's parameters and body — gains a check: scan
the source gap between the previous child's end and the next child's start for a leading, own-line
comment, and if found, emit it (respecting the surrounding indentation) before continuing as
normal.

This is a breaking change to a public `cel-parser` function. That's acceptable — this project has
no clients yet — but it does touch every existing call site: `adam-lang/src/fmt.rs`, `ez-adam`'s
codegen, and `cel-rs-macros`'s doctests.

### Testing strategy: `format_expr`

Contract tests on the new `format_expr` signature, one per recovery position: a comment before an
operand in a binary and in a prefix operator application, before a tuple element, before a call
argument, before an `if`/`else` branch's contents, before a closure parameter. Idempotency tests
(`format(format(x)) == format(x)`) for each of the above. A regression test reproducing #201's
exact repro string. A test confirming a same-line trailing comment is *not* recovered (documenting
the stated boundary, not silently passing).

## Part B: retire `AdamParser`'s inline grammar

`AdamParser::parse_str` changes from "parse declaration syntax and execute into a `Sheet` in one
pass" to "parse via `AdamAstParser` into `ast::Sheet`, then compile `ast::Sheet` into a `Sheet`."
A new `adam-lang/src/compile.rs` module takes over all of `AdamParser`'s current semantic work —
`TypeRegistry` resolution, undeclared-cell/arity/type-mismatch checks, building
`Method`/`Filter`/`Requirement` values — operating on already-parsed `ast::*` nodes instead of a
live token cursor. The validation rules themselves don't change, only their input shape.

### CEL sub-expression compilation

The compile phase needs to turn each already-parsed `Expr` into a `DynSegment` to build the live
`Sheet`. Rather than building a new `Expr`-tree-walking compiler for this, it re-parses each
expression's already-known source span through `Parser<DynSegmentContext>` — the same thing
`AdamParser` does today, just invoked from the compile phase instead of inline during declaration
parsing. This is a deliberate choice to not build that primitive yet: a genuine `Expr`-walking
compiler is exactly what the future Rust-codegen phase (Roadmap, below) will need to build for
real, walking `Expr` and emitting Rust tokens instead of `DynSegment` ops. Building a throwaway
version now, solely to avoid one extra parse pass at sheet-load time, isn't justified.

### Error-handling contract

`AdamParser::parse_str` keeps its existing public shape: `Result<ParsedSheet>`, failing on the
first error. If `AdamAstParser` recorded any syntax errors, the first one is returned and the
compile phase never runs; otherwise the compile phase proceeds and can fail on the first semantic
error it hits, exactly as `AdamParser` does today. No behavior change for existing embedders of
`AdamParser`. The ability to collect every syntax error across a whole sheet remains exactly where
it already lives — `adam-lsp` and `adam-fmt`, via direct `AdamAstParser` use — since
`AdamParser`'s own contract was never that in the first place.

### Testing strategy: `compile.rs`

Every existing `AdamParser` test continues to exercise the same public behavior (same inputs,
same `Result<ParsedSheet>`/error outcomes) — this migration changes internal shape, not observable
behavior, so no test's assertions should need to change, only (where necessary) its setup. New
tests target `compile.rs` directly: each semantic-validation rule it now owns (unknown type name,
undeclared cell, arity mismatch, filter type mismatch) gets a test constructing the relevant
`ast::*` node directly, rather than only being reachable indirectly through `AdamParser::parse_str`.

## Roadmap: future phases

Tracked separately once Parts A and B land, not designed here:

- A compile-time `cel-rs-macros` backend that walks an already-parsed `Expr` and emits real Rust
  closure code, reusing the tree-walking shape Part B's compile phase deliberately avoided
  building early.
- Compiling a whole adam sheet to a native "sheet instance" at compile time, building on
  `ast::Sheet` (Part B's substrate) plus the above backend.
- A grammar-description DSL with pluggable backends (formatting, the LSP, direct Rust
  compilation, the dynamic runtime) — the long-term generalization beyond hand-written Rust
  grammar productions.

## Sequencing

Part A and Part B are independent of each other (Part A is entirely inside `cel-parser`, plus
adam-lang's own trivia/`Comment` extraction; Part B doesn't touch `Expr`/`format_expr` at all) and
land as two separate implementation plans/PRs. Part A first, since it's the direct fix for #201
and self-contained; Part B second, since it's the larger, riskier migration and isn't blocking the
issue itself.
