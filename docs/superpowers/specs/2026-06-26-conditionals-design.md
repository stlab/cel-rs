# Conditionals for property-model

**Date:** 2026-06-26
**Branch:** worktree-conditionals

## Summary

Add a `Conditional` construct to the `property-model` crate. A conditional binds to a *match cell* and holds a set of branches, each keyed by one or more values and associated with a list of pre-created relationships. When the sheet propagates, the branch whose keys contain the current match cell value is activated; its relationships participate in the general planning pass. Unmatched conditionals with no default contribute no relationships and succeed silently.

This feature also introduces **strength partitioning** — a correctness and stability invariant that ensures explicitly written cells always outrank derived cells in the Adam planner.

---

## 1. Equality Support

Match-cell evaluation requires comparing the cell's live type-erased value against branch keys. `dyn PartialEq` is not object-safe, so equality is captured as a function pointer at cell-creation time.

### Changes to `CellData`

```rust
pub(crate) struct CellData {
    pub(crate) value: Box<dyn Any>,
    pub(crate) type_id: TypeId,
    pub(crate) strength: u64,
    pub(crate) changed: bool,
    pub(crate) adj: Vec<RelationshipId>,
    pub(crate) eq_fn: fn(&dyn Any, &dyn Any) -> bool,  // new
}
```

### Changes to `Sheet::add_cell`

```rust
pub fn add_cell<T: Any + PartialEq + 'static>(&mut self, value: T) -> CellId
```

The `PartialEq` bound is new and **breaking**. `eq_fn` is captured as:

```rust
eq_fn: |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>()
```

`add_cell` is semantically equivalent to "create a blank cell, then `write()` a value to it," so it also assigns a write-strength (see §2).

---

## 2. Strength Partitioning

### Motivation

When a conditional branch changes, cells derived by the old branch become "orphaned" — they carry stale derived-strength values but have no in-edge in the new plan. Without partitioning, an orphaned cell could have higher strength than a new-branch source, causing the Adam planner to treat it as a source and producing a spurious `Conflict`.

### Invariant

> `min(written/added cell strength) > max(derived cell strength)` at all times.

### Write-side (add_cell and write())

Both operations apply the high-order bit before storing:

```rust
self.next_strength += 1;
cell.strength = self.next_strength | (1u64 << 63);
// Strength range: [0x8000_0000_0000_0001, ...]
```

### Post-plan strength assignment

After every full propagation (see §5), a post-processing pass walks the **Phase 3 execution order** and re-assigns derived-cell strengths:

- **Source cells** (not the output of any selected method): strength is unchanged.
- **Derived cells** (output of a selected method): assigned from a counter initialised to `0x7FFF_FFFF_FFFF_FFFF` and decremented for each successive derived cell.

Cells evaluated earlier (closer to sources) receive higher derived strengths. This preserves relative evaluation order across propagations, which is the stability property: when branches switch, sources (high-bit strength) naturally outrank all previously derived cells (low-bit strength), and the Adam algorithm produces a correct new plan without any explicit orphan reset.

---

## 3. Conditional Data Structures

### New file: `conditional.rs`

```rust
new_key_type! {
    /// A stable handle to a conditional in a [`Sheet`].
    pub struct ConditionalId;
}

pub(crate) struct Branch {
    /// Type-erased key values; each has TypeId matching the match cell.
    pub(crate) keys: Vec<Box<dyn Any>>,
    pub(crate) relationships: Vec<RelationshipId>,
}

pub(crate) struct ConditionalData {
    pub(crate) cell: CellId,
    pub(crate) branches: Vec<Branch>,
    /// Relationships active when no branch key matches. Empty = no default.
    pub(crate) default: Vec<RelationshipId>,
}
```

### New `Sheet` fields

```rust
conditionals: SlotMap<ConditionalId, ConditionalData>,
/// Union of all RelationshipIds assigned to any conditional branch.
/// Used to exclude them from the pre-plan and build the active set.
conditional_relationships: HashSet<RelationshipId>,
```

### New error variant

```rust
/// A conditional is structurally invalid: the cell was not found, a
/// referenced relationship was not found or has more than one method,
/// a relationship appears in more than one branch, or the branch key
/// type does not match the cell's registered type.
InvalidConditional,
```

---

## 4. `add_conditional` API

```rust
pub fn add_conditional<T: Any + PartialEq + 'static>(
    &mut self,
    cell: CellId,
    branches: Vec<(Vec<T>, Vec<RelationshipId>)>,
    default: Vec<RelationshipId>,
) -> Result<ConditionalId, Error>
```

Branch keys are provided as typed `Vec<T>` and stored type-erased as `Vec<Box<dyn Any>>`.

### Validation (all checked at call time, returns `Error::InvalidConditional` on failure)

1. `cell` exists in the sheet and its `TypeId` equals `TypeId::of::<T>()`.
2. Every `RelationshipId` in every branch and in `default` exists in the sheet.
3. Every such relationship has **exactly one method**. (Required so the pre-plan is deterministic regardless of strength.)
4. No relationship appears in more than one branch, across all conditionals on the sheet (checked against `conditional_relationships`).

On success, all referenced relationship IDs are inserted into `conditional_relationships`.

### Semantics

- Branches are evaluated in definition order; the first matching branch wins.
- Multiple keys per branch use `|` semantics: any key matching the cell value activates the branch.
- A branch may be associated with multiple relationships; all of them are added to the active set.
- A relationship may appear in at most one branch across all conditionals.
- If no branch matches and `default` is empty, the conditional contributes no relationships — this is valid and common (e.g., `match proportional { true => relate { w == h } }`).

---

## 5. Modified Propagation Algorithm

`Sheet::propagate()` becomes a four-phase operation. `propagate_without_replan()` re-executes the cached combined execution order without re-evaluating conditionals; its existing precondition (no previously-derived source cell has been written) is unchanged, but the caller must additionally ensure no match cell value has changed.

### Phase 1 — Pre-plan (match cell computation)

Goal: ensure match cell values are current before conditional evaluation.

1. Collect all match cells (one per conditional).
2. For each match cell, BFS upstream through cell adjacency lists, collecting relationships that:
   - Are **not** in `conditional_relationships`, and
   - Include the match cell (or a transitively needed cell) in their outputs.
3. This forms the *match-cell subgraph*. Relationships that are themselves in `conditional_relationships` are excluded. Relationships in the subgraph may have multiple methods; the Adam planner uses cell strengths to resolve them.
4. Plan and execute the match-cell subgraph.

If all match cells are written sources (no in-edges in the unconditional graph), this phase is a no-op.

### Phase 2 — Conditional evaluation

For each conditional, read the match cell's current value and compare it against each branch's keys using the cell's `eq_fn`. First matching branch wins. Collect all activated relationship IDs.

Build the **active relationship set**:

```text
active = (all relationships) − conditional_relationships
       ∪ (selected branch rels for each conditional)
```

### Phase 3 — General plan

Run the existing Adam algorithm on the active relationship set. Unconditional relationships from Phase 1 are included and re-executed (same inputs → same outputs; this is correct and gives the authoritative execution order).

The Phase 3 execution order is stored in `last_plan`. Phase 1 relationships are a subset of the Phase 3 active set and appear in the Phase 3 order; storing them separately would cause double-execution in `propagate_without_replan`.

### Phase 4 — Strength post-processing

Walk the Phase 3 execution order. For each output cell of each selected method, assign the next decrementing derived-strength value (starting at `0x7FFF_FFFF_FFFF_FFFF`). Source cells are skipped.

### Error handling

| Condition | Error |
| --- | --- |
| Phase 1 or Phase 3 plan cannot assign all relationships | `Error::Conflict` |
| A selected method's function returns an error | `Error::MethodFailed` |
| A method output's runtime type mismatches the cell | `Error::TypeMismatch` |

---

## 6. Validation Rules Summary

| Rule | Checked at |
| --- | --- |
| Match cell exists and TypeId matches branch key type | `add_conditional` |
| All branch relationship IDs exist | `add_conditional` |
| All branch relationships have exactly one method | `add_conditional` |
| No relationship in more than one conditional branch | `add_conditional` |
| Each branch has at least one key | `add_conditional` |
| No matching branch + no default → silent success | `propagate` (Phase 2) |

---

## 7. Files Changed / Added

| File | Change |
| --- | --- |
| `src/cell.rs` | Add `eq_fn` field to `CellData`; tighten `add_cell` bound |
| `src/sheet.rs` | Add `conditionals`, `conditional_relationships` fields; add `add_conditional`; rewrite `propagate` |
| `src/planner.rs` | Accept a relationship filter (active set) so Phase 1 and Phase 3 can plan subsets |
| `src/conditional.rs` | New: `ConditionalId`, `Branch`, `ConditionalData` |
| `src/error.rs` | Add `InvalidConditional` variant |
| `src/lib.rs` | Re-export `ConditionalId`; add `pub mod conditional` |
| `tests/integration.rs` | New tests for conditional activation, no-match, branch switching, stability |
