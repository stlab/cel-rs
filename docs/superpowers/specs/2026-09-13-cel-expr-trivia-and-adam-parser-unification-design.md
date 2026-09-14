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
adam-lang's own trivia model is itself narrower than it looks: it only recovers a comment
immediately preceding a whole declaration, or immediately preceding a block's own closing brace —
never a comment between two of a single declaration's own tokens (e.g. between a cell's name and
its `:`). Fixing only the CEL side would leave that class of loss in place and invite the next
"comments vanish in this other spot" report.

## Goals

- Fix #201, generally: no comment is lost when reformatting a sheet, whether it sits before,
  after, or between any two tokens — inside a CEL expression, or inside adam-lang's own
  declaration syntax.
- Converge adam-lang onto a single grammar implementation, removing the maintenance burden of two
  hand-duplicated parsers.
- Do both in a way that sets up, rather than forecloses, the longer-term direction: a compile-time
  `cel-rs-macros` backend that emits real Rust closures, compiling a whole adam sheet to a native
  "sheet instance," and eventually a grammar-description DSL with pluggable backends.

## Non-goals

- The `cel-rs-macros` compile-time closure backend, sheet-to-Rust-instance compilation, and the
  grammar-description DSL. These are named in the Roadmap section below as tracked future work,
  not designed here.
- Preserving the *exact* original whitespace layout around a comment (blank-line count beyond
  "was there at least one," column alignment, etc.). Comments themselves are never lost; the
  whitespace immediately around them is still normalized to the formatter's usual style.
- Any change to `AdamParser`'s public error-handling contract (`Result<ParsedSheet>`, failing on
  the first error encountered).

## Part A: CEL expression trivia — the `fill_gap` primitive

`Expr` gains no new fields. It stays exactly as documented today — no resolved types, no
trivia — matching `ExprSpan`'s own anticipation of a separate, span-based trivia pass rather than
stored comment data.

Recovering only a leading, own-line comment before each structural child (the original, narrower
design) misses every comment that doesn't sit at a "before the next child" boundary — one between
an operand and its own operator, one trailing the operator on the same line, one trailing an
operand before its own comma or closing paren. None of these align with `Expr`'s tree structure
(the operator token itself, for instance, has no span of its own — only the whole `Op` node's
aggregate span is stored). Getting to zero loss means working from the raw source text of a gap,
not from `Expr`'s child boundaries.

### The `fill_gap` primitive

A gap between two known source positions (the end of one token/sub-expression and the start of
the next) that is grammatically guaranteed to contain nothing but whitespace, comments, and a
fixed, known sequence of literal tokens (an operator symbol, `,`, `(`/`)`, `{`/`}`, `as`,
`if`/`else`, `|`) can be re-emitted losslessly: scan the gap's raw text left to right, alternately
recognizing a comment (`//` to end of line, or `/* */`, possibly multi-line) or the next expected
literal token, and emit everything found — comments and required tokens alike — in original
order, normalizing only the whitespace between them (line breaks and indentation, per the
formatter's usual style; `Comment::Line`/`Comment::Block` rendering is reused from adam-lang's
existing `write_comment`, moved down to `cel-parser` alongside the primitive itself — see below).

This is one shared function, roughly `fill_gap(out: &mut String, source: &str, prev_end: Span,
next_start: Span, expected: &[&str], depth: usize)`, living in a new `cel-parser/src/trivia.rs`
module. `Comment` (`Line`/`Block`) moves there too, out of `adam-lang::ast` — `adam-lang` already
depends on `cel-parser`, so this introduces no circular dependency, and `adam-lang`'s own trivia
code (Part A2, below) becomes a consumer of the same primitive instead of a separate
implementation.

### `format_expr` changes

`format_expr`'s signature changes from `fn format_expr(expr: &Expr) -> String` to
`fn format_expr(expr: &Expr, source: &str) -> String` — recovering a gap requires the whole
source string plus each span's line/column position, not just what `Span::source_text()` gives
for a single span.

Every place `format_expr` currently writes a fixed literal separator between two things — an
operand and its infix/prefix operator (`Op`), a callee and its arguments and the commas between
them (`Apply`), the elements and commas of a `Tuple`, the `if`/`{`/`}`/`else` structure of an
`If`, an operator and its operand (`Logical`), the `as` and type name of a `Cast`, the `|`s and
`:`/`,` separators of a `Closure`'s parameter list — becomes a `fill_gap` call instead, using
whatever the grammar guarantees appears in that gap (usually a single literal token; a `Closure`'s
empty parameter list still needs both `|`s accounted for, an `Apply` with zero arguments still
needs its `(` and `)`).

`Expr`'s own outer boundary — the gap before its first token and after its last — is not
`format_expr`'s to own, since it only emits the expression's own text; that boundary belongs to
whatever embeds an expression into surrounding syntax (adam-lang's `=`/`:=` and trailing `;`,
handled in Part A2).

This is a breaking change to a public `cel-parser` function. That's acceptable — this project has
no clients yet — but it does touch every existing call site: `adam-lang/src/fmt.rs`, `ez-adam`'s
codegen, and `cel-rs-macros`'s doctests.

### Testing strategy: `format_expr`

One test per gap kind: a comment before, after, and between an operand and its operator (both
infix and prefix); before, after, and between a call's arguments and their commas, including a
zero-argument call; the same for a tuple, including a one-element and a zero-element (unit) case;
around `if`/`else`'s own braces and keywords, including the no-`else` case; around `as` and a cast
target; around a closure's `|`s and parameter separators, including a zero-parameter closure.
Multiple comments back to back in a single gap (`/* a */ /* b */`). Idempotency
(`format(format(x, source), out) == format(x, source)`, run against the *new* output as its own
new source). A regression test reproducing #201's own repro string, and the fuller example from
this design's own discussion (`/* */ 1 /* */ + // hello\n/* */ 2 /* */`).

## Part A2: adam-lang's own declaration-grammar trivia

adam-lang's current trivia model (`trivia.rs`) recovers exactly two shapes: a comment immediately
preceding a whole declaration, and a trailing comment immediately preceding a block's own closing
brace. It recovers nothing between a single declaration's own tokens (`cell` and its name, the
name and `:`, a type and `=`, an initializer and `;`, and so on for `relate`/`out`/`require`
blocks) — the same class of gap Part A closes for CEL expressions, just at the adam-lang grammar
level instead of the CEL grammar level. Left alone, this is exactly the kind of gap that
surfaces as its own future bug report.

### Design

`adam-lang/src/fmt.rs`'s `write_cell`/`write_relate`/`write_binding`/`write_out`/`write_requirement`
/etc. are rewritten to call the same `fill_gap` primitive from Part A between every one of their
own literal sub-tokens, rather than concatenating fixed strings. This includes the boundary Part A
explicitly left to its caller: the gap between `=`/`:=` and an initializer/binding/requirement
body's first token, and the gap between that body's last token and whatever follows (`;`,
`require`, the block's closing brace). `ast::Sheet`'s existing per-node `ExprSpan`s already carry
enough position information for this — no new AST fields are needed, only the formatter's own
internal structure changes.

This does not touch `trivia.rs`'s existing job (recovering a leading comment/blank-line before a
whole declaration, and a trailing comment/blank-line before a block's closing brace) — that
scan operates between *sibling declarations*, a different, still-necessary gap kind that
`fill_gap` doesn't replace, only complements.

### Testing strategy: adam-lang `fmt.rs`

One test per newly-covered gap in each rewritten `write_*` function: between `cell`/`out`/`relate`
/`require` and the token that follows each; between a declared type and `=`/`:=`; between an
initializer/binding/requirement body and its trailing `;`; between a `filter`/`require` clause's
own keyword and its body. A regression test combining several of these in one declaration, and an
idempotency test matching Part A's.

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

Tracked separately once Parts A/A2/B land, not designed here:

- A compile-time `cel-rs-macros` backend that walks an already-parsed `Expr` and emits real Rust
  closure code, reusing the tree-walking shape Part B's compile phase deliberately avoided
  building early.
- Compiling a whole adam sheet to a native "sheet instance" at compile time, building on
  `ast::Sheet` (Part B's substrate) plus the above backend.
- A grammar-description DSL with pluggable backends (formatting, the LSP, direct Rust
  compilation, the dynamic runtime) — the long-term generalization beyond hand-written Rust
  grammar productions.

## Sequencing

Part A2 depends on Part A's shared `fill_gap` primitive, so they land together as one
implementation plan/PR: Part A (the primitive plus `cel-parser::format_expr`) first, Part A2
(adam-lang's `fmt.rs`) immediately after, in the same branch. Part B is independent of both (it
doesn't touch `Expr`, `format_expr`, or adam-lang's formatter at all) and lands as a separate,
second implementation plan/PR, since it's the larger, riskier migration and isn't blocking #201
itself.
