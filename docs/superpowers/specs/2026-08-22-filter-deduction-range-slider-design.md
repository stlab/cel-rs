# Design: Deduced Filter Dependencies, `_` Placeholder, and `RangeInclusive` Filters

**Date:** 2026-08-22
**Branch:** `worktree-sean_parent+adam-filter-range-slider`

## Summary

Simplify `adam-lang`'s cell-filter syntax so a filter is a single expression — dependencies on
other cells are deduced (mirroring how `relationship`/`out`/`conditional` bodies already deduce
their inputs), and the candidate value being conformed is written as `_` instead of a named
closure parameter. Add a `RangeInclusive<T>` type and `..=` operator to CEL, recognized
structurally at compile time as a distinct kind of filter (a clamp) rather than an opaque
closure. Add a `FilterKind` tag to `adam-rs::Filter`, queryable from `Sheet`, so a consumer like
`begin` can tell — without inspecting the filter's opaque function — that a cell's filter is a
range clamp and what its current bounds are. Use that in `begin` to render numeric cells with a
dedicated number field, plus a slider (bounds read live from the filter) when one is present.

## Motivation

`begin`'s Inspector currently renders every non-bool cell as a plain text field
([inspector.rs:321-353](../../../begin/src/inspector.rs#L321-L353)); numeric cells get no
numeric affordances at all. Adding a slider for range-clamped numeric cells requires the UI
layer to know, for a given cell, "does it have a range filter, and what are its current bounds"
— today that's impossible without evaluating the filter's opaque `Box<dyn Fn>`
([filter.rs:16](../../../adam-rs/src/filter.rs#L16)), which carries no information about *why*
it does what it does. This design gives filters enough structure to answer that question for the
range case, while keeping the fully general filter form for everything else, and does so via a
syntax simplification (deduced deps + `_`) that's valuable independent of the UI motivation.

## Current State

- `adam-lang`'s `CellFilter` AST node holds an explicit `arg_cells: Vec<(String, ExprSpan)>` list
  plus a closure-literal `closure: cel_parser::Expr`
  ([ast.rs:213-223](../../../adam-lang/src/ast.rs#L213-L223)).
- The compile phase, `AdamParser::parse_cell_filter`
  ([parser.rs:285-379](../../../adam-lang/src/parser.rs#L285-L379)), parses the `(arg_cells)`
  list, evaluates the closure literal to a `cel_runtime::DynClosure`, checks its declared
  parameter/return types against the cell's type and the arg cells' types, and wraps it in
  `adam_rs::Filter::new`.
- `adam_rs::Filter`/`FilterData` stores a type-erased `value_type`, `args: Vec<CellId>`,
  `arg_types: Vec<TypeId>`, and an opaque `FilterFn`
  ([filter.rs:26-34](../../../adam-rs/src/filter.rs#L26-L34)) — no structural information about
  what the function *does*.
- A *different* deduction mechanism already exists for relationship bindings, conditionals'
  match expressions, and `out`/`require` bodies: `AdamParser::parse_deduced_expr`
  ([parser.rs:629-690](../../../adam-lang/src/parser.rs#L629-L690)) pushes a scope onto the CEL
  op-lookup that resolves any 0-arity reference to an already-declared cell name to the next
  free argument slot, deduplicating repeat references. Filters don't use this mechanism today.
- `begin`'s `CellMeta` ([bridge.rs:20-32](../../../begin/src/bridge.rs#L20-L32)) has `label`,
  `is_bool`, `display`, `write_str` — no filter-derived information at all.

## Design Overview

Four pieces, in dependency order:

1. Filters become a single deduced expression with `_` denoting the candidate value.
2. CEL gains a `RangeInclusive<T>` type and `lo..=hi` syntax.
3. `adam-rs::Filter` gains a `FilterKind` tag (`Opaque` or `Range`), set by the compile phase when
   a filter's expression is `RangeInclusive`-typed; `Sheet` gains a query for it.
4. `begin` renders a number field for numeric cells, and a slider (bounds read from `FilterKind::
   Range`) when one is present.

## 1. Deduced Filter Dependencies + `_` Placeholder

### Filter Grammar

Replaces the current production entirely (no explicit arg list, no closure literal):

```ebnf
cell_filter = "filter" or_expression .
```

### Semantics

- `_` refers to the value being conformed (what today is the closure's first parameter). It is
  **required** to appear at least once in a general (non-`RangeInclusive`, see §2) filter
  expression — no use case was found for a filter that ignores its own candidate value, so this
  is a compile error rather than a silently-accepted degenerate case.
- `_` may appear more than once; every occurrence denotes the *same* candidate value (not
  distinct parameters — unlike, e.g., Scala's underscore-placeholder convention). This is a real
  requirement, not just permitted for uniformity: idempotent filters like snap-to-grid
  (`_ - (_ % step)`) or a deadzone (`if _.abs() < epsilon { 0.0 } else { _ }`) need to reference
  the candidate value more than once to be expressed at all.
- Every other identifier that resolves to an already-declared cell is a deduced dependency,
  exactly as `parse_deduced_expr` already does for bindings — first reference allocates the next
  argument slot, repeat references reuse it.
- `_` is a reserved identifier inside a filter expression, mirroring the wildcard `_` adam-lang
  already uses for a conditional's default branch (`_ => { ... }`,
  [ast.rs:389](../../../adam-lang/src/ast.rs#L389)) — it is not looked up as a cell name.

### Examples

```rust
cell a: i32 filter min(_, hi);                       // deduces `hi`
cell a: f64 filter _ - (_ % step);                    // deduces `step`
cell a: f64 filter if _ < lo { lo } else if _ > hi { hi } else { _ };  // deduces `lo`, `hi`
```

`min` is used here (and `.clamp`/`.abs` elsewhere in this document) as an illustrative stand-in
for exposition — none of `min`/`max`/`.clamp`/`.abs` are currently registered CEL builtins
(checked against `cel-parser/src/op_table.rs`). Adding them is straightforward, ordinary
op-table work, orthogonal to this design: the deduction/`_`-placeholder mechanism works over
*any* expression and doesn't depend on which specific functions exist. The `if`/`else` example
above and the `..=`-based range filter (§2) use only constructs that exist today.

### Compile-phase mechanism

Extend `parse_deduced_expr`'s push-table construction (or add a sibling used only for filters)
with one additional, always-present entry: the identifier `_` resolves to argument slot 0 (the
candidate value), ahead of any cell-derived slots. This reuses the existing scope-pushing
mechanism verbatim — filters need no new deduction logic, only one reserved name in the table
plus a post-parse check that `_` was referenced at least once for the general form.

Because the deduced expression's `DynSegment` already IS a callable computation over its
argument slots — no closure abstraction (`DynClosure`, `param_types`/`return_type` checking) is
needed. `parse_cell_filter`'s current type-checking dance against a closure's declared parameter
types goes away; the deduced expression's own inferred output type is checked against the
filtered cell's declared type instead, the same way `build_cell_from_segment` already checks an
initializer's inferred type against a declared one.

### Removed

The current `filter(arg_cells) |params| body` closure-literal syntax is removed outright, not
kept alongside the new form — this project has no releases or clients yet (see root
[CLAUDE.md](../../../CLAUDE.md), "Project Status"), so there's no reason to carry both.

## 2. `RangeInclusive<T>` Type and `..=` Operator

### Motivation for a real type, not just sugar

Keying the structural recognition on an expression's *type* rather than a special-cased builtin
function name is what makes it survive indirection: if `range` (or any future spelling) were
recognized by matching a call's callee name, wrapping it in a user-defined function would defeat
the recognition. Typing the whole filter expression's *result* as `RangeInclusive<T>` survives
that, because adam-lang's typecheck already resolves the expression's result type regardless of
how many function calls sit between the `filter` clause and the `lo..=hi` literal that produced
it — this holds today even before adam-lang gains general closures-returned-from-functions,
and continues to hold once it does, with no changes needed at this layer.

### Range Grammar

**Corrected from an earlier draft of this spec**, which placed `range_expression` between
`comparison_expression` and `additive_expression` — too tight: Rust's own range operator binds
*looser* than `||` (it sits just above assignment/`return`/closures in Rust's precedence table,
below everything else), not down among the arithmetic/comparison operators. The corrected grammar:

```ebnf
expression       = range_expression .
range_expression = ( or_expression [ ".." [ or_expression ] | "..=" or_expression ] )
                  | ( ".." [ or_expression ] )
                  | ( "..=" or_expression ) .
```

`range_expression` replaces `or_expression` as `expression`'s own top-level production, with
`or_expression` operands — checked once, at the top of the grammar, rather than inserted partway
down the comparison chain. Still non-chainable (`a..=b..=c` is not a valid range expression,
matching Rust) — with `range_expression` no longer nested inside a lower-precedence production,
this falls out from ordinary leftover-token handling at whatever now plays `expression`'s old
"and nothing else follows" role (see below), not from a bespoke check.

This generalizes at the *grammar* level to all six of Rust's range forms
(`Range`/`RangeFrom`/`RangeTo`/`RangeFull` alongside `RangeInclusive`/`RangeToInclusive`) — that
generalization is the implementing plan's concern, not this spec's; a filter only ever needs the
one inclusive, both-endpoints form, `lo..=hi`.

**A consequence for `cel-parser` itself:** `expression` previously meant both "this production"
and "and nothing else follows" (`expression = or_expression ?eos?.`, enforced by
`is_expression()`). Once `range_expression` sits above `or_expression`, that's no longer a
property `expression` itself should bake in — the end-of-stream check moves to a separate
`parse_expression()`-style helper, kept only for the one real caller that needs it:
`cel-rs-macros`'s `expression!` proc-macro (which must still reject trailing tokens in a macro
body). adam-lang's own entry points (cell initializers, relationship bindings, conditionals'
match expressions, etc.) already parse an expression embedded in a larger token stream and never
required end-of-stream — they gain range syntax for free by parsing `expression` instead of
`or_expression` directly, with no behavior change to their own "there's more sheet syntax after
this" handling.

**Implementation note for the plan:** `cel-parser`'s lexer currently combines 2-token punctuation
(`==`, `<=`, `>=`, `&&`, `||`) into single ops; `..=` is three tokens (`.`, `.`, `=`) and needs
either a 3-token lookahead added to the combiner or an intermediate `..` token combined with a
trailing `=`. (Already implemented, alongside full six-form support, in
`docs/superpowers/plans/2026-08-24-cel-range-syntax.md`; this note is left for historical
context.)

### Type checking

`T ..= T -> RangeInclusive<T>`, both operands the same numeric type. Scoped to numeric `T` for
now, matching the immediate need (numeric range-clamp filters) — extending to other orderable
types is a later decision, not blocked by anything here.

### Runtime representation

Reuse `std::ops::RangeInclusive<T>` directly as the CEL runtime value — it already provides
`.start()`/`.end()`, `Clone`, equality — rather than introducing a new `cel-runtime` type. Only
the operator (constructing one from two operands) and its `TypeRegistry` entry are new.

## 3. `FilterKind` and the `Sheet` Query API (`adam-rs`)

```rust
/// What shape of validation/derivation a `Filter` performs, beyond its opaque function —
/// set by adam-lang's compile phase when a filter's expression matches a recognized
/// structural form. `Opaque` carries no extra information; consumers that don't care about
/// structure treat every kind identically at write/propagate time — `FilterKind` is purely
/// informational, queried by consumers like `begin`'s UI that want to render a specialized
/// editor without inspecting the filter's function.
pub enum FilterKind {
    /// The filter's expression wasn't a recognized structural form.
    Opaque,
    /// Compiled from a `RangeInclusive<T>`-typed expression (`lo..=hi`). `bounds` re-evaluates
    /// that expression against the filter's current argument values, returning the resulting
    /// `(lo, hi)` as type-erased values of the filtered cell's own type `T`.
    Range { bounds: Box<dyn Fn(&[&dyn Any]) -> (Box<dyn Any>, Box<dyn Any>)> },
}
```

- `Filter`/`FilterData` gains a `kind: FilterKind` field. `Filter::new`/`from_fn_0/1/2` (kept,
  unchanged, for direct Rust construction — tests, non-adam-lang sheets) default it to `Opaque`.
- A new constructor, `Filter::range(...)`, built by the compile phase when it recognizes a
  `RangeInclusive`-typed filter expression — internally still just a clamp function plus the
  `bounds` re-evaluator; not a new execution path, just a tagged variant of what `from_fn_2`
  already builds today for the two-arg numeric case. Exact signature (how `bounds`/the clamp
  function share the deduced expression) is an implementation-plan detail, not fixed here.
- `Sheet::filter_kind(id: CellId) -> Option<&FilterKind>` — `None` for no filter (mirrors
  `filter_args`'s existing `Option` convention); `Some(Opaque)` for a filter with unrecognized
  structure; `Some(Range { .. })` for a range filter.
- `Sheet::filter_range<T: Any + Clone>(id: CellId) -> Option<(T, T)>` — convenience for `begin`
  and similar consumers: resolves the filter's current argument values via the same
  `effective()` path `add_filter` already uses, calls `bounds`, downcasts both sides to `T`.
  Returns `None` if `id` has no filter or its `FilterKind` isn't `Range`.

No change to `add_filter`'s existing validation/conform-on-attach behavior — `kind` rides along
with the rest of `FilterData` unchanged by anything in `add_filter`'s current logic
([sheet.rs:547-588](../../../adam-rs/src/sheet.rs#L547-L588)).

## 4. `begin` UI: Number Fields and Range Sliders

- `CellMeta` gains a field for the live slider range, e.g.
  `pub range: Option<Box<dyn Fn(&Sheet) -> (f64, f64)>>`, populated in
  `labels_from_cell_names` for numeric types by checking `sheet.filter_kind(id)` for
  `FilterKind::Range` and wrapping `Sheet::filter_range::<T>` (cast to `f64` for display purposes,
  matching how `format_rounded` already treats every numeric type as an `f64` for display).
- Numeric, non-bool cells get a dedicated number-field component (a new `SpNumberfield` wrapper
  in [spectrum.rs](../../../begin/src/spectrum.rs), mirroring the existing `SpTextfield`/
  `SpCheckbox` wrapper pattern) instead of the current plain `SpTextfield`
  ([inspector.rs:321-353](../../../begin/src/inspector.rs#L321-L353)).
- When `meta.range` is `Some`, `CellRow` additionally renders a slider (a new `SpSlider` wrapper
  around Spectrum's `<sp-slider>`), whose min/max are recomputed from `(meta.range)(&sheet.read
  ())` on every render — so if `lo`/`hi` themselves are driven by other cells or relationships,
  the slider's range updates automatically, with no separate cache to invalidate.
- The number field and slider both write to the same cell; either one's edit goes through the
  existing `write_and_propagate` path unchanged ([inspector.rs:154-205](../../../begin/src/inspector.rs#L154-L205)).

## Compatibility Notes

- **Closures-returned-from-functions**: filters no longer need general closures at all under this
  design (a filter compiles to a deduced expression, not a boxed `Fn` built from a closure
  literal) — this fully decouples the filter feature from that future language capability. When
  it lands, nothing here needs to change.
- **No name-based special-casing**: `_.clamp(lo, hi)` (the general placeholder form, written out
  by hand) deliberately stays `Opaque` — only the `RangeInclusive`-typed `lo..=hi` spelling gets
  tagged. Layering a second, name-based recognition rule (matching calls to `.clamp`) alongside
  the type-based one was considered and rejected — one canonical tagged spelling avoids two
  overlapping recognition mechanisms fighting over the same cell.
- **`_` required, ≥1 occurrence, all same value**: confirmed against real idempotent examples
  (snap-to-grid, deadzone) that need repeated reference to the candidate value; no use case found
  for permitting zero occurrences.

## Error Messages

| Situation | Message |
| --- | --- |
| General filter expression never references `_` | `"filter must reference '_' (the value being filtered)"` |
| Filter expression's inferred type doesn't match the cell's type (and isn't `RangeInclusive<cell type>`) | `"cell '{name}': filter must produce '{cell type}'"` |
| `RangeInclusive`-typed filter expression whose `T` doesn't match the cell's type | `"cell '{name}': filter range bounds must be '{cell type}'"` |
| `..=` operands of different types | `"range operands must have the same type"` (typecheck, `cel-parser`) |
| `..=` operand not numeric | `"range operands must be numeric"` (typecheck, `cel-parser`) |
| Filter references an undeclared cell identifier | `"undeclared cell '{name}'"` (unchanged — same error `parse_deduced_expr` already raises for bindings) |

## Tests

- **`cel-parser`**: `lo..=hi` lexes/parses to a `RangeInclusive` value; mismatched-type and
  non-numeric operands are parse errors; `a..=b..=c` is a parse error (non-chainable).
- **`adam-lang`**: filter with no `_` is a compile error; filter with multiple `_` compiles and
  each evaluation substitutes the same current value; filter referencing an undeclared cell is a
  compile error (unchanged message, now raised by the filter path too); a `lo..=hi` filter
  compiles to a `Filter` whose `FilterKind` is `Range`; a general filter (`min(_, a)`) compiles to
  `FilterKind::Opaque`; snap-to-grid and deadzone examples round-trip through write/propagate
  correctly and idempotently.
- **`adam-rs`**: `Sheet::filter_kind` returns `None`/`Opaque`/`Range` correctly; `Sheet::
  filter_range::<T>` returns live bounds that change when the underlying `lo`/`hi` cells change
  (via a relationship, not just a direct write); existing `Filter::new`/`from_fn_0/1/2` behavior
  is unchanged (they still default to `Opaque`).
- **`begin`**: a numeric cell with no filter gets a number field, no slider; a numeric cell with a
  `Range` filter gets a number field plus a slider whose bounds match `Sheet::filter_range`; the
  slider's bounds update after a write that changes `lo`/`hi` without requiring the cell's own
  row to be the one edited. Per [begin/CLAUDE.md](../../../begin/CLAUDE.md), this last group is
  verified by actually rendering the UI (`verifying-begin-ui` skill), not just by compiling.

## Files Changed

| File | Change |
| --- | --- |
| `cel-parser/src/lib.rs` | Add `is_range_expression`; wire into the expression grammar between comparison and additive |
| `cel-parser/src/lexer.rs` (or wherever multi-char combination lives) | Combine `..=` |
| `cel-parser/src/op_table.rs` / typecheck | `T ..= T -> RangeInclusive<T>` for numeric `T` |
| `cel-runtime` | Register `std::ops::RangeInclusive<T>` as a known value type per numeric `T` |
| `adam-lang/src/ast.rs` | `CellFilter` loses `arg_cells`; `closure` becomes the sole `or_expression` |
| `adam-lang/src/parser.rs` | Rewrite `parse_cell_filter` to use deduced dependencies + `_`; branch on `RangeInclusive`-typed result to call `adam_rs::Filter::range` instead of the general path |
| `adam-lang/src/fmt.rs`, `typecheck.rs` | Update for the new `cell_filter` grammar |
| `adam-rs/src/filter.rs` | Add `FilterKind` enum, `kind` field on `FilterData`, `Filter::range` constructor |
| `adam-rs/src/sheet.rs` | Add `Sheet::filter_kind`, `Sheet::filter_range::<T>` |
| `begin/src/bridge.rs` | `CellMeta` gains `range`; `labels_from_cell_names` populates it via `Sheet::filter_kind`/`filter_range` |
| `begin/src/spectrum.rs` | Add `SpNumberfield`, `SpSlider` wrapper components |
| `begin/src/inspector.rs` | `CellRow` renders a number field (+ slider when `meta.range.is_some()`) instead of `SpTextfield` for numeric cells |
| `adam-lsp` | Update any filter-syntax-aware diagnostics/completions for the new grammar (existing filter-support fixtures per recent commits) |
