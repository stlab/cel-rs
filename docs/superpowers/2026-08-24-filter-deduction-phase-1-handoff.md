# Deduced Filter Args + Range Slider — Handoff into Phase 2 (§3/§4)

Status snapshot as of 2026-08-24, written for whoever picks up §3/§4 in a new
conversation/context. Read
`docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md` first for the full
design and phasing — this doc only summarizes what's done, what's deliberately deferred, and
what's left before the whole design is complete.

## What's done

**§1 (deduced filter dependencies + `_` placeholder) — complete, this branch
(`worktree-sean_parent+adam-filter-range-slider`).**
`adam-lang`'s cell `filter` clause is now a single deduced expression, exactly mirroring how
`relationship`/`out`/`conditional` bodies already deduce their inputs via
`AdamParser::parse_deduced_expr`, rather than a standalone closure literal with an explicit arg
list:

- Grammar: `cell_filter = "filter" or_expression .` The old `filter(arg_cells) |params| body`
  closure-literal syntax is gone outright — removed, not kept alongside the new form (this
  project has no releases or clients yet, so there was no reason to carry both).
- `_` denotes the value being filtered (what used to be the closure's first parameter). It
  resolves to argument slot 0 ahead of any cell-derived slots, reusing `parse_deduced_expr`'s
  existing scope-pushing mechanism verbatim — no new deduction logic was needed, only one
  reserved name in the table.
- `_` is required to appear at least once in a filter expression (`filter must reference '_' (the
  value being filtered)` is a compile error otherwise) and may appear more than once, with every
  occurrence denoting the same candidate value — verified by both a compile-time check and a
  runtime test (`cell_filter_referencing_the_same_value_twice_is_idempotent`).
- Every other identifier that resolves to an already-declared cell is a deduced dependency,
  first-reference-allocates/repeat-reuses, same as bindings.
- `CellFilter` in `adam-lang/src/ast.rs` lost its `arg_cells` field; `closure` became the sole
  `or_expression` body. `AdamParser::parse_cell_filter` (`adam-lang/src/parser.rs`) was rewritten
  to build the deduced expression directly rather than compiling and type-checking a
  `DynClosure` — the deduced expression's own inferred output type is checked against the
  filtered cell's declared type, the same way `build_cell_from_segment` already checks an
  initializer's type.
- `adam-lang/src/fmt.rs` and `adam-lang/src/typecheck.rs` were updated for the new grammar;
  `adam-lsp`'s filter-aware fixtures were updated to match (`51e72bca`).

Built via subagent-driven development; task tracking lives in
`.superpowers/sdd/2026-08-24-adam-lang-deduced-filter-args/`.

Two things surfaced during execution worth flagging for whoever picks up the next phase:

1. **A discovered plan gap, not a plan defect**: `begin/examples/inequality.adm2` still used the
   old closure-literal filter syntax and needed migrating to the new grammar (commit `a471ca6b`).
   While fixing it, a latent pre-existing bug surfaced: the original closure declared a `range`
   argument but ignored it, hardcoding the clamp bounds (`0.0, 100.0`) instead of reading
   `range.0`/`range.1`. The fix migrates the example to `filter clamp(_, range.0, range.1)`,
   which both adopts the new syntax and actually honors the declared range cell.
2. **A discovered-and-fixed design defect in the original `_`-reference-tracking approach.** The
   first version of `check_filter`'s "`_` must be referenced" check (in
   `adam-lang/src/typecheck.rs`) tracked whether `_` was seen via a `Cell<bool>` flag mutated
   from inside the type-checking closure used to resolve identifiers. Because that closure could
   run more than once over nested sub-expressions for a tuple-typed filter body containing its
   own type error, the flag could be set and read in a way that double-diagnosed (or
   under/over-fired) rather than reflecting one authoritative pass over the whole expression. This
   was replaced (commit `c5a55daa`, before Task 4 was marked complete — so the final code shipped
   on this branch never had the bug) with a dedicated `expr_references_ident` tree-walk helper: a
   plain, side-effect-free structural walk over every `Expr` variant, called once at the end of
   `check_filter` after a pure `body_resolve` closure has done type resolution. This is a cleaner
   design in its own right (the reference check is now fully decoupled from — and can't be
   perturbed by — how many times type resolution revisits a sub-expression), not just a bugfix,
   and the test `filter_references_underscore_nested_inside_a_call` pins the corrected behavior.

## Deliberately deferred

A `filter lo..=hi` expression **parses** today — `..=` range syntax already landed on `main`
(merged via #144/#145, the `cel-range-syntax` plan) and is reachable from every adam-lang entry
point, filters included. It currently **fails this plan's own type check**: the filter body's
inferred type is `RangeInclusive<T>`, which is compared against the filtered cell's declared type
`T` and rejected as a mismatch (`cell '{name}': filter must produce '{cell type}'`). This is
expected and correct under §1's scope — recognizing a `RangeInclusive`-typed filter body as a
distinct clamp *kind* rather than a type error is explicitly §3's job, not this plan's. The
rejection is expected to be *replaced*, not merely loosened, when §3 lands (see below).

A tuple-typed filtered cell (e.g. `cell a: (i32, f64) filter (_.0, _.1);`) is rejected by both
layers with `cell '{name}': filter on a tuple-typed cell is not yet supported` — found by the
final whole-branch review (the CST type checker had briefly diverged from the runtime parser here:
the runtime rejected it cleanly while the type checker still structurally accepted it, so the LSP
would show a clean sheet the runtime couldn't actually build). Both now agree, in
`adam-lang/src/typecheck.rs`'s `check_filter` and `adam-lang/src/parser.rs`'s `parse_cell_filter`
(commit `ee763b41`). §3 is expected to touch this exact code path when it teaches the runtime to
build filters from non-scalar (and `RangeInclusive`) expression shapes — whoever picks that up
should search both files for "tuple-typed cell" before changing either one, so the two layers stay
in sync.

## Done (§3/§4)

Both remaining pieces from the design spec are complete, via
`docs/superpowers/plans/2026-08-24-filter-kind-range-slider.md`, executed with
subagent-driven-development (ledger: `.superpowers/sdd/2026-08-24-filter-kind-range-slider/progress.md`).

**§3 — `FilterKind` tag + `Sheet` query API.** `adam-rs::FilterKind` (`Opaque` | `Range { bounds }`)
and `Filter::range(...)` (`adam-rs/src/filter.rs`), `Sheet::filter_kind`/`Sheet::filter_range::<T>`
(`adam-rs/src/sheet.rs`), a `RangeEntry` lookup table for the 14 numeric primitives
(`adam-lang/src/type_registry.rs`), and `parse_cell_filter`'s range-recognition branch
(`adam-lang/src/parser.rs`) — a `RangeInclusive<T>`-typed filter body now compiles to a tagged
range-clamp `Filter` instead of failing the old type check, with the "`_` must be referenced"
check correctly bypassed for any recognized range shape (both at the runtime layer and, via a
matching exemption in `adam-lang/src/typecheck.rs`, at the CST layer `adam-lsp` uses).

**§4 — `begin` UI: number fields and range sliders.** `CellMeta` gains `is_numeric`/`range`
(`begin/src/bridge.rs`); `SpNumberfield`/`SpSlider` wrapper components (`begin/src/spectrum.rs`);
`CellRow` renders checkbox / number-field(+slider) / text-field per cell (`begin/src/inspector.rs`);
`begin/examples/inequality.adm2` migrated to the `lo..=hi` spelling so it exercises `FilterKind::Range`.
Verified by actually rendering `begin` (served as a web app, driven via headless Edge + CDP): numeric
cells get a stepper-equipped number field; a range-filtered cell additionally gets a slider with
live min/max; editing the number field and dragging the slider both write-and-propagate correctly,
including clamping out-of-range writes; a six-step simulated slider drag showed no jitter or
snap-back; non-numeric (bool/String) cells and disabled/forced states are unaffected, confirmed
across four different examples.

## Key files for future filter/UI work

- `adam-lang/src/parser.rs` (`parse_cell_filter`, `parse_filter_expr`, `parse_deduced_expr`) and
  `adam-lang/src/typecheck.rs` (`check_filter`, `is_range_inclusive_body`,
  `expr_references_ident`) — the deduced-expression compile path, including range recognition.
- `adam-lang/src/type_registry.rs` — `TypeRegistry::range_entry`/`RangeEntry`, the per-numeric-type
  `RangeInclusive<T>` clamp/bounds dispatch table.
- `adam-rs/src/filter.rs` — `Filter`/`FilterData`/`FilterKind`, `Filter::new`/`from_fn_0/1/2`/`range`.
- `adam-rs/src/sheet.rs` — `add_filter`, `filter_kind`, `filter_range`.
- `begin/src/bridge.rs` (`CellMeta`, `labels_from_cell_names`, `mark_numeric`), `begin/src/spectrum.rs`
  (`SpTextfield`/`SpCheckbox`/`SpNumberfield`/`SpSlider`), `begin/src/inspector.rs` (`CellRow`,
  `write_and_propagate`).
- `docs/superpowers/specs/2026-08-22-filter-deduction-range-slider-design.md` — full design.
