# Cell Kinds: `source` and a Non-Terminal `out`

**Date:** 2026-08-29
**Branch:** worktree-adam-rs-cell-kinds
**Status:** Draft, awaiting review

## Summary

Three changes to `adam-rs`'s cell model, designed together because each one only fully
makes sense in light of the others:

1. **`out` stops being terminal.** An `out` cell keeps its current meaning (always
   derived, exactly one fixed writer method, never `write()`-able) but can now be
   referenced anywhere a plain cell can: as another relationship's input, a
   conditional's match subject, another `out`'s writer input, a filter argument. Only
   "may be produced by more than one thing" and "may be written directly" stay
   forbidden.
2. **A new `source` cell kind.** The mirror image of `out`: never produced by any
   method, always a planner source, but ordinary in every other respect (writable,
   filterable, referenceable).
3. **`filter` and `require` generalize to every cell kind.** A filter gains a
   mandatory name and may now attach to a `source` or `out` cell, not just a plain
   `cell`. A `require` block may now attach to any cell (previously `out`-only) and
   may hold any number of named requirements.

None of this needs new propagation or planning machinery. `adam-rs`'s cells already
carry a `source`/`derived` split per round (`CellData::source`/`CellData::derived`);
`source` and `out` just pin a cell permanently to one side of that split instead of
letting the planner choose per round. Filter's existing dual-mode behavior (write-time
self-correction for a source, read-only diagnostic for a derived value) and
Requirement's existing pure-diagnostic behavior already do exactly what each pinned
kind needs — this design removes the preconditions that currently keep them from
running on `source`/`out` cells, rather than adding a new mechanism.

This closes a gap the previous syntax pass ([2026-08-19](2026-08-19-adam-lang-syntax-design.md))
named explicitly and left open: "the unimplemented feature this gap actually wants is
**input filters**... not `require` extended to interior cells." Filters shipped since;
this design is the `require`-extended-to-interior-cells half, plus the `out`
relaxation and `source` kind that make the three-way split symmetric.

---

## 1. Motivation

`out` today conflates two independent properties: "always derived by one fixed
method" and "unusable anywhere else." The second was never load-bearing for the
motivating use case (command preconditions, see
[2026-08-07](2026-08-07-output-cells-design.md) §1) — it was a simplification, not a
requirement. Dropping it turns `out` into an ordinary spreadsheet formula cell: a
value other cells can read and build on, still guaranteed never to silently take on
a value from anywhere but its own formula.

`source` fills the matching gap on the other side: a value cell that can never be
quietly overridden by a relationship the planner decides to solve in the "wrong"
direction. This isn't currently expressible — a plain `cell`'s source/derived status
depends on runtime strength competition with everything else in the sheet, which is
exactly the flexibility a sheet author sometimes wants to opt out of for a
specific cell (a literal constant, a cell meant to always represent direct user
input).

Generalizing `filter`/`require` off the back of these two kinds is what makes them
worth having: a `source` cell's whole value proposition is "guaranteed domain,
guaranteed provenance," and both `filter` (domain) and `require` (arbitrary named
invariants) are the mechanisms that already exist for exactly that, just currently
gated to the wrong subset of cells.

---

## 2. Naming decisions (confirmed)

- **Cell kinds:** `cell` (either, today's behavior, unchanged), `source` (new), `out`
  (kept — dataflow/circuit terminology has never implied "sink"; an output pin is
  routinely wired into other gates, so the word stays accurate once the terminal
  restriction is dropped). Renaming `out` to `derived`/`formula` for symmetry with
  `source` was considered and rejected: the rename cost (`OutDecl`, `add_output`,
  every doc citing "output cells") isn't earned by a purely cosmetic symmetry gain.
- **`filter`/`require`:** unchanged. `require`'s generalized behavior (named,
  possibly-multiple, arbitrary-cell-referencing, non-transforming boolean checks)
  is exactly what "required" already names in the constraint-hierarchy literature
  this planner descends from (DeltaBlue/Cassowary's `required` strength). `filter`
  still describes a value transform, unaffected in kind by which cells it can now
  attach to.
- **`require` never gates `write()`, on any cell kind.** `write()` recently lost its
  own filter-gating for the same reason this would reintroduce: `adam-rs` removed
  write-time filter rejection (§2026-08-26) specifically because a synchronous
  conform-or-reject at `write()` made outcomes depend on write *order* relative to
  other cells, and permanently destroyed the pre-write value. A hard-gating `require`
  would be the one remaining exception to "diagnostics observe or self-correct, never
  abort a caller's operation" (2026-08-26 §4) and would reintroduce exactly that
  order-dependence. `require` is a pure post-`propagate()` diagnostic, uniformly,
  regardless of the checked cell's kind or its source/derived status this round.
- **Attaching a requirement whose current value already fails it is a hard error.**
  Unlike a filter (which can silently conform a non-matching value), a requirement is
  a pure boolean with nothing to repair — `add_requirement` rejects outright rather
  than attaching a requirement already known to be violated.

---

## 3. Data model

### 3.1 `CellKind`

New enum in `cell.rs`, replacing the boolean-shaped `terminal_cells: HashSet<CellId>`
on `Sheet`:

```rust
/// A cell's fixed role in the planner's per-round source/derived assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// May be a source or derived, chosen per round by the planner. Default kind.
    Cell,
    /// Always a source: never claimable as any method's output.
    Source,
    /// Always derived by exactly one fixed writer method; never `write()`-able.
    Out,
}
```

`CellData` gains:

```rust
pub(crate) struct CellData {
    // ...existing fields...
    pub(crate) kind: CellKind,
    /// This cell's requirements, in attachment order. Empty for most cells.
    pub(crate) requirements: Vec<RequirementId>,
}
```

`Sheet::cell_kind(&self, id: CellId) -> Option<CellKind>` is a new public query.

`is_source`'s existing dynamic definition ("no selected method in `last_plan` outputs
`id`") is unchanged — for a `Source`-kind cell it is now provably always `true`, and
for an `Out`-kind cell always `false`, purely as a consequence of the new
`add_relationship`/`add_out` checks below, not by adding a special case to
`is_source` itself.

### 3.2 `OutputId`/`OutputData` are removed

Today's `OutputData { cell: CellId, requirements: Vec<RequirementId> }` carries
nothing that isn't already better expressed as `CellKind::Out` plus the
cell-attached `requirements` list from §3.1 — once an out cell is an ordinary,
referenceable `CellId`, wrapping it in a second handle type is the exact asymmetry
`source` (which gets no handle type of its own) argues against. `OutputId`,
`OutputData`, `Sheet::outputs`, and `output.rs` are deleted; every query that took an
`OutputId` is re-expressed in terms of `CellId` (§4.3).

This is the one place this design goes beyond a minimal diff — flagged here
explicitly since it's a real public API removal, not just a rename, and worth
confirming during spec review rather than discovering during implementation.

### 3.3 `RequirementData` rekeys from `OutputId` to `CellId`

```rust
pub(crate) struct RequirementData {
    pub(crate) name: String,
    pub(crate) cell: CellId,   // was `output: OutputId`
    pub(crate) inputs: Vec<CellId>,
    pub(crate) function: RequirementFn,
}
```

`Requirement`'s own public shape (`Requirement::new`/`from_fn_1`/`from_fn_2`) is
unchanged — only the `RequirementData` it becomes once attached moves from an
`OutputId` backreference to a `CellId` one.

### 3.4 `FilterData` gains a name

```rust
pub(crate) struct FilterData {
    pub(crate) name: String,   // new
    // ...existing fields unchanged...
}
```

`Filter`'s public constructors (`from_fn_0`/`from_fn_1`/`from_fn_2`/`new`/`range`)
are unchanged; the name is supplied separately, at `add_filter` (§4.2a), mirroring how
a requirement's name is supplied at `add_requirement` rather than baked into
`Requirement::new`.

### 3.5 `Sheet` fields

```rust
pub struct Sheet {
    // ...existing fields...
    // removed: terminal_cells: HashSet<CellId>
    // removed: outputs: SlotMap<OutputId, OutputData>
    /// Cells whose requirement(s) did not all hold as of the last full `propagate()`
    /// call. Sparse: a cell with every requirement holding has no entry. Was
    /// `last_violated: HashMap<OutputId, Vec<RequirementId>>`.
    last_requirement_violations: HashMap<CellId, Vec<RequirementId>>,
}
```

`last_filter_violations: HashMap<CellId, FilterViolation>` (already `CellId`-keyed)
is unaffected.

---

## 4. `Sheet` API changes

### 4.1 Cell construction

```rust
/// Registers a cell that may be a source or derived, chosen per round by the
/// planner. Unchanged from today.
pub fn add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId;

/// Registers a cell that can never be claimed as any method's output — always a
/// planner source, forever.
pub fn add_source<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId;
```

`add_source` is `add_cell` plus `kind: CellKind::Source`; no other behavioral
difference. Both remain O(1).

### 4.2 `add_out` (renames `add_output`)

```rust
/// Registers `writer` as the sole producer of its one output cell, which becomes an
/// `out` cell: always derived by `writer`, never `write()`-able, but otherwise an
/// ordinary, freely-referenceable cell. `requirements` are attached to that cell
/// exactly as `add_requirement` (§4.4) would, one at a time, in order.
///
/// # Errors
///
/// - `Error::InvalidOutput` — `writer` does not have exactly one output cell.
/// - `Error::InvalidCellKind` — the writer's output cell is already `Source` or
///   `Out` kind, or already belongs to another method's output set (already
///   claimed as a producer target before this call).
/// - Any error `add_relationship` or `add_requirement` can return.
pub fn add_out(
    &mut self,
    writer: Method,
    requirements: Vec<(&str, Requirement)>,
) -> Result<CellId, Error>;
```

Steps: identical to today's `add_output` through relationship registration, except:

- The "already terminal" check becomes "already `Source`/`Out` kind, or the target
  of an existing method's output set" — see §4.5 for the refined prior-use check.
  Referencing the not-yet-`out` cell as an *input* somewhere before this call is now
  fine; only prior claim as an *output* still isn't.
- Marking terminal (`terminal_cells.insert`) becomes `self.cells[cell].kind =
  CellKind::Out`.
- Requirements attach via `add_requirement` (§4.4) instead of being built inline —
  same validation, now shared code with the general case.
- Returns the `CellId` directly; no `OutputId` is minted.

### 4.2a `add_filter` gains a name

```rust
/// Attaches `filter` to `cell` under `name`.
///
/// # Errors
///
/// - `Error::InvalidId` — `cell`, or one of `filter`'s argument cells, is not a
///   live cell in this sheet.
/// - `Error::InvalidFilter` — `name` is empty, `cell` already has a filter,
///   `filter`'s own value type does not match `cell`'s registered type, or
///   `filter`'s argument list names `cell` itself.
/// - `Error::TypeMismatch` — an argument cell's registered type does not match the
///   type `filter` declared for it.
pub fn add_filter(
    &mut self,
    cell: CellId,
    name: impl Into<String>,
    filter: Filter,
) -> Result<(), Error>;
```

The only behavioral change from today's `add_filter` is the new `name` parameter and
the removal of the "`cell` is terminal" rejection — a filter may now attach to a
`Source` or `Out` kind cell exactly as it can to a plain `cell`. `Sheet::filter_name(id:
CellId) -> Option<&str>` is a new query, mirroring `requirement_name`.

### 4.3 Query renames (`OutputId` → `CellId`, generalized off `out`-only)

| Before | After |
| --- | --- |
| `output_cell(OutputId) -> Option<CellId>` | removed (the `CellId` *is* the handle) |
| `output_valid(OutputId) -> bool` | `cell_requirements_valid(CellId) -> bool` — any cell |
| `violated_conditions`/`violated_requirements(OutputId) -> impl Iterator<RequirementId>` | `violated_requirements(CellId) -> impl Iterator<RequirementId>` — any cell |
| `output_conditions`/`output_requirements(OutputId) -> Option<&[RequirementId]>` | `cell_requirements(CellId) -> Option<&[RequirementId]>` — any cell |
| `condition_output`/`requirement_output(RequirementId) -> OutputId` | `requirement_cell(RequirementId) -> CellId` |
| `condition_name`/`requirement_name`, `condition_inputs`/`requirement_inputs`, `condition_contributing_cells`/`requirement_contributing_cells` | unchanged in shape, just no longer implicitly `out`-only |

`cell_requirements_valid`/`violated_requirements`/`cell_requirements` follow
`output_valid`'s existing "not yet propagated → not valid / empty" convention
(check `last_plan.is_none()` first), for every cell, not only former `out` cells.

### 4.4 `add_requirement` (new; `add_out` calls it internally)

```rust
/// Attaches a named requirement to `cell`. `requirement.inputs` may be any cells in
/// the sheet, not only `cell` itself.
///
/// # Errors
///
/// - `Error::InvalidId` — `cell`, or one of `requirement`'s input cells, is not a
///   live cell in this sheet.
/// - `Error::TypeMismatch` — an input's declared type does not match its cell's
///   registered type.
/// - `Error::InvalidRequirement` — `name` is empty, `cell` already has a
///   same-named requirement, or evaluating `requirement` against the referenced
///   cells' current effective values returns `Ok(false)`.
/// - `Error::MethodFailed` — evaluating `requirement` against current values
///   returns `Err`.
pub fn add_requirement(
    &mut self,
    cell: CellId,
    name: impl Into<String>,
    requirement: Requirement,
) -> Result<RequirementId, Error>;
```

The current-value check at attach time mirrors `add_filter`'s existing retroactive
check in spirit, not in outcome: a filter can conform a non-matching value in place,
a requirement cannot, so the only sound response to "already violated" is rejecting
the attachment outright, per §2's confirmed decision.

### 4.5 `add_relationship` / `add_conditional`: new `Source` check

Every place `add_relationship` and `add_conditional` currently check
`terminal_cells.contains(&cell_id)` for a method's *output* cells, add a check that
the cell's `kind != CellKind::Source`. Input-side checks lose their `Out`-kind
check entirely (an `out` cell is a legal input everywhere now); they gain no new
`Source`-side check (a `source` cell is a legal input everywhere, exactly like
today's plain cell).

`write()` keeps exactly one kind check: reject if `kind == CellKind::Out`.

`cell_has_prior_use` (used by `add_out`, private) narrows from "any adjacency at
all" to "already claimed as some existing method's output, or already a
conditional's match cell":

```rust
fn cell_has_prior_use(&self, id: CellId) -> bool {
    self.relationships.values().any(|rel| {
        rel.methods.iter().any(|m| m.outputs.contains(&id))
    }) || self.conditionals.values().any(|c| c.match_cells().contains(&id))
}
```

(Match-cell exclusion is unchanged from today — unrelated to the terminal
relaxation, kept as-is since nothing about this design touches conditional match
semantics.)

### 4.6 `Error` changes

```rust
/// A relationship or conditional attempted to claim a `Source`-kind cell as a
/// method's output, `write()` targeted an `Out`-kind cell, or `add_out` targeted a
/// cell that is already `Source`/`Out` kind or already claimed as another method's
/// output.
InvalidCellKind,  // replaces `TerminalCell`

/// An `add_requirement` call is structurally invalid: the name is empty, `cell`
/// already has a same-named requirement, or the requirement evaluates to `Ok(false)`
/// against the referenced cells' current values.
InvalidRequirement,
```

`TerminalCell` is renamed, not merely repurposed: the name described a property
(terminal-ness) this design removes, so keeping the identifier while changing its
meaning would leave a misleading name in the API. `InvalidOutput` is unchanged
(still governs `add_out`'s "writer must have exactly one output" structural check).

---

## 5. Propagation: no new phases

Phase 6 (post-`execute_plan` diagnostics) generalizes from "iterate every
`RequirementData`, keyed by its `OutputId`" to "iterate every `RequirementData`,
keyed by its `CellId`" — the evaluation logic (call `function` against inputs'
current `effective()` values, record `Ok(false)` as a violation, abort on `Err` via
`Error::MethodFailed`) is unchanged. This runs once, at the end of the round, for
every cell with a non-empty `requirements` list, regardless of whether that cell
was a source or derived this round: a requirement never mutates anything, so unlike
`Filter`'s `PlanStep::FilterReclamp` (which needed planner placement because it
writes into `derived` mid-round, see 2026-08-25 §2.1), a single end-of-round pass
already sees every input's fully-settled value. **No planner integration, no new
`PlanStep`, no cycle-detection interaction, and no staleness gap** — the property
that took two designs to establish for `Filter` on a `source` cell (2026-08-25,
2026-08-26) holds for `Requirement` on any cell kind for free, because `Requirement`
was already a pure diagnostic and stays one.

Filter's existing per-round dispatch (`PlanStep::FilterReclamp` for a source this
round, the read-only Phase 4 diagnostic for a derived value this round) needs no new
logic once `add_filter`'s `Out`-kind precondition is dropped:

- On a `Source`-kind cell: always takes the `FilterReclamp` path, every round,
  because `is_source` is always `true` for it.
- On an `Out`-kind cell: always takes the read-only diagnostic path, every round,
  because `is_source` is always `false` for it.
- On a plain `Cell`-kind cell: unchanged, dispatches per round exactly as today.

---

## 6. `adam-lang` grammar

```text
sheet_item     = [ doc_comment ] (cell_decl | source_decl | relationship_decl
                   | conditional_decl | out_decl).

cell_decl      = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
source_decl    = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
cell_type_init = (":" type_expr ["=" or_expression]) | ("=" or_expression).

out_decl       = "out" identifier [ ":" type_expr ] ":=" or_expression
                   [ cell_filter ] [ require_block ] ";".

cell_filter    = "filter" identifier ":" or_expression.
require_block  = "require" "{" { requirement } "}".
requirement    = identifier ":" or_expression ";".
```

`cell_filter` and `require_block` become shared productions consumed by all three
declaration kinds, instead of `cell_decl` and `out_decl` each wiring up their own
copy (today `cell_decl` has an anonymous `filter` clause and no `require`; `out_decl`
has `require` and no `filter`). `source_decl` reuses `cell_type_init` unchanged — a
`source` cell's initializer is a one-time literal exactly like a plain `cell`'s,
never a continuously-re-evaluated `:=` (that stays `out`-only, matching today).

`ast::CellDecl`/`ast::OutDecl` both gain a `require: Option<RequireBlock>` field (already
present on `OutDecl`, new on `CellDecl`); `ast::CellFilter` gains `name: String` and
`name_span: ExprSpan`; a new `ast::SourceDecl` mirrors `CellDecl` structurally (same
fields, `cell_type_init` only, `filter` and `require` both optional). `SheetItem`
gains a `Source(SourceDecl)` variant.

`parser.rs`'s direct-to-`Sheet` path: `parse_cell_decl` gains an optional trailing
`require_block`, wired through `add_requirement` after `add_filter` (matching §2's
filter-then-require ordering at the *parse* level — there is no ordering question at
runtime since requirements never gate, but attach order still needs the filter in
place first so a requirement's attach-time current-value check, if it reads the
filtered cell itself, sees the filter's structural validation already done); a new
`parse_source_decl` mirrors `parse_cell_decl` minus the `filter`/`require` wiring
differences (none — identical shape); `parse_out_decl` gains an optional `cell_filter`
alongside its existing `require_block`, both routed through the same `add_filter`/
`add_requirement` helpers `parse_cell_decl`/`parse_source_decl` use.

`fmt.rs`, `ast_parser.rs`, `trivia.rs`, and `typecheck.rs` each need the analogous
`source_decl` case added and the `filter`/`require` clauses generalized off their
current single-decl-kind assumptions; `typecheck.rs`'s requirement-body-must-be-`bool`
check applies unchanged, now run once per requirement regardless of which
declaration kind it's attached to.

---

## 7. `adam-rs` renames (summary table)

| Before | After |
| --- | --- |
| `terminal_cells: HashSet<CellId>` field | removed; `CellData::kind: CellKind` |
| `Error::TerminalCell` | `Error::InvalidCellKind` |
| `output.rs` (file), `OutputId`, `OutputData`, `Sheet::outputs` | removed |
| `Sheet::add_output` | `Sheet::add_out`, returns `CellId` not `OutputId` |
| `Sheet::output_cell` | removed (the `CellId` is already the handle) |
| `Sheet::output_valid` | `Sheet::cell_requirements_valid`, any cell |
| `Sheet::violated_requirements(OutputId)` | `Sheet::violated_requirements(CellId)`, any cell |
| `Sheet::output_requirements` | `Sheet::cell_requirements`, any cell |
| `Sheet::requirement_output` | `Sheet::requirement_cell` |
| `RequirementData::output: OutputId` | `RequirementData::cell: CellId` |
| `FilterData` (no name) | `FilterData::name: String` |
| — | `Sheet::add_source`, `Sheet::add_requirement`, `Sheet::cell_kind` (new) |

Untouched: `Requirement`/`RequirementId` themselves, `Filter`'s constructors,
`Conditional`/`ConditionalId`/`add_conditional`, everything in §5's propagation
phases beyond the precondition relaxations already described.

---

## 8. Downstream impact

- **`begin`** (`begin/src/inspector.rs`): consumes the query surface (`OutputStatus`,
  `cell_flags`, `filter_violated_cells`, etc.) generically, not `OutputId` directly —
  a quick check found no direct `OutputId` usage in `begin`. Its `compute_output_status`
  and related helpers will need to call the renamed queries (§4.3, §7) and should
  naturally start covering `source`/plain-`cell` requirement violations for free once
  they do, but this is a mechanical follow-up, not a design question — left to its own
  implementation pass rather than folded into this spec.
- **`adam-lsp`**: no direct references to `OutputId`/`output_cell`/`add_output` found;
  it works through `ast.rs`/`typecheck.rs` generically, so the `SourceDecl`/`CellFilter`
  name field/`CellDecl.require` additions in §6 should reach it through the existing
  AST-consuming machinery rather than needing bespoke LSP changes. Verify during
  implementation rather than assuming.
- **`editors/vscode-adam-lang`**: gains one new keyword, `source`, for syntax
  highlighting, matching the existing `cell`/`out`/`relationship`/`conditional`/
  `filter`/`require` keyword list.
- **`adam-lang-book`** (merged since this spec's first draft, via the `worktree-live-book`
  PR): its `.adm2` examples are live-mounted and actually parsed/resolved by the real
  parser at doc-build time (`xtask prepare-live-book-assets`), so this is a real,
  build-checked consumer, not just prose. Several chapters currently document exactly
  the restrictions this design lifts:
  - [`filters.md`](../../../adam-lang-book/book-src/filters.md) §6.1's grammar
    (`cell_filter = "filter" expression.`, anonymous) and every example under
    `examples/filters/` use the pre-naming syntax; all need the `filter name: expr`
    form. §6.6's closing line ("a filter cannot attach to an output cell") states the
    exact precondition §4.2a removes and needs to go, replaced with an example showing
    a filter on an `out` or `source` cell.
  - [`outputs.md`](../../../adam-lang-book/book-src/outputs.md) §7.2, "An output's
    cell is terminal," needs reframing: it currently asserts the input-reference
    restriction this design removes, but its own closing paragraph ("An output cell is
    nonetheless an ordinary cell for *reading*...") already describes this design's
    target behavior, not today's actual behavior — `parser.rs`'s
    `parse_out_cell_referenced_elsewhere_is_terminal_cell_error` and
    `..._in_conditional_is_terminal_cell_error` tests confirm today's parser still
    rejects it. That's a pre-existing doc/code mismatch worth noting regardless of
    this design; shipping this design makes that paragraph true instead of aspirational.
    §7.3/7.4 need a lead-in noting `require` is no longer `out`-only.
  - [`cells.md`](../../../adam-lang-book/book-src/cells.md) and
    [`reference.md`](../../../adam-lang-book/book-src/reference.md) (Appendix A) both
    state the current grammar directly (`cell_decl`, `cell_filter`, the keyword list,
    the `A.8`/`A.9` bullet lists) and need the same grammar update as §6 of this spec.
    Reference.md's `A.8` bullet "a filter cannot attach to an output cell" is the same
    stale claim as `filters.md` §6.6.
  - [`SUMMARY.md`](../../../adam-lang-book/book-src/SUMMARY.md) needs a new chapter for
    `source` cells. Chapters are cross-referenced by number in prose (e.g. "Chapter 6",
    "6.1", "A.11") throughout every `.md` file in the book, so inserting a chapter
    renumbers everything after it — a mechanical but book-wide edit, not a single-file
    change.
  - Not yet checked in detail: `tutorial.md`, `relationships.md`, `conditionals.md`,
    `expressions.md`, `style.md` — likely lower-impact (they don't center on
    filter/require/out grammar) but should be swept for stale cross-references once
    the chapter renumbering above happens.
  This is real, in-scope follow-up work, not optional polish — a stale example under
  the new grammar renders as a parse-error diagnostic in the live book instead of the
  working example it's supposed to demonstrate, which `xtask prepare-live-book-assets`
  would surface as a build problem, not a silent doc rot. Scoped as its own
  implementation-plan phase (§ below), after the `adam-rs`/`adam-lang` code changes
  land, so the book is updated against real, working syntax rather than against this
  spec's prose.
- No migration/back-compat path needed, per root `CLAUDE.md` (no clients yet).

---

## 9. Testing notes

Derived from the contracts in §4–§6 only:

- `add_source` produces a cell for which `is_source` is `true` before and after any
  `propagate()`, and for which `add_relationship`/`add_conditional` return
  `Error::InvalidCellKind` if any method attempts to claim it as an output.
- `add_out`'s cell is referenceable as another relationship's input, a conditional's
  match subject, and a filter argument, all without error, and still returns
  `Error::InvalidCellKind` for `write()` and for a second method claiming it as an
  output.
- `add_out` returns `Error::InvalidCellKind` (not `Error::InvalidOutput`) when the
  target cell already has prior use as some method's output, but succeeds when the
  target cell was already used as some other relationship's *input*.
- `add_filter` succeeds on a `Source`-kind and an `Out`-kind cell; a `Source`-kind
  cell's filter always reclamps via `PlanStep::FilterReclamp`; an `Out`-kind cell's
  filter only ever produces the read-only diagnostic, never a reclamp.
- `add_requirement` succeeds on every cell kind; returns `Error::InvalidRequirement`
  when the name is empty, is a duplicate on that cell, or the requirement evaluates
  `Ok(false)` against current values; returns `Error::MethodFailed` when it evaluates
  `Err`.
- After `propagate()`, `cell_requirements_valid`/`violated_requirements` reflect
  requirement results for a plain `cell` and a `source` cell exactly as they
  already do for an `out` cell today, including the case where the checked cell is a
  source in one round and derived in a later round (the requirement is evaluated
  against its `effective()` inputs identically either way).
- `write()` on a cell with a failing requirement succeeds (requirements never gate);
  the subsequent `propagate()` records the violation.
- A sheet with no `source`/generalized-`require` usage at all produces byte-identical
  behavior to today (regression coverage for the `terminal_cells` → `CellKind`
  migration and the `OutputId` removal).

---

## 10. Non-goals for this phase

- Any `begin` UI work surfacing `source`/generalized-`require` violations visually —
  tracked as a follow-up once this lands, per §8.
- Revisiting the `release::resolve` filter-blind boundary (2026-08-25 §3,
  `Error::FilterCycle`) — untouched by this design; a `Source`-kind cell's
  output-claim rejection happens at `add_relationship` time, not during planning, so
  it introduces no new interaction with that boundary.
- A `source_decl`/`out_decl` combinator sugar analogous to the deferred `cell ... :=
  expr` sugar from 2026-08-19 — out of scope, orthogonal to this design.
- Changing how a plain `cell`'s filter/requirement behaves when it happens to be a
  source or derived cell this round — entirely unchanged; `source`/`out` only pin
  which side of that existing behavior a cell permanently sits on.
