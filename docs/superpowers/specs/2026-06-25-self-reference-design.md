# Self-Referencing Relationships Design

**Date:** 2026-06-25  
**Author:** Sean Parent  
**Status:** Approved

## Overview

Extend the property model to support self-referencing methods — methods where a cell appears in
both `inputs` and `outputs`. This enables idempotent constraint clamping such as
`a <== min(a, 0)` and two-way inequality constraints such as `relate { a <== min(a, b); b <== max(a, b); }`.

Reference: Adam property model (Adobe Source Libraries)
[adobe_source_libraries/source/adam.cpp](https://github.com/stlab/adobe_source_libraries/blob/4a83677650a259594aff600b08a318cb4786e18e/source/adam.cpp)

## Semantics

A method whose `inputs ∩ outputs ≠ ∅` is a **self-referencing method**. The cells in the
intersection are **self-referencing cells**; they act as both input (reading the pre-execution
value) and output (writing a new value).

Two rules govern self-referencing methods:

1. **Source-only**: a self-referencing method may only be selected when every self-referencing
   cell is a *source* — i.e., its current value came from a `write()` call, not from a prior
   method. A cell derived by another method cannot be used as a self-referencing input.

2. **Idempotency (by convention)**: the method function must be idempotent with respect to the
   self-referencing cells. Applying it twice must produce the same result as applying it once.
   `min(a, 0)` is idempotent; `a + 1` is not. This is a caller contract; it is not enforced
   by the runtime.

### Execution-order guarantee

Any method that reads a self-referencing cell as a *pure input* (not also writing it) will
execute after the self-referencing method that last modified it. This is automatic: the
flood-fill selects the self-referencing method when the source cell is first determined, so it
appears earlier in `execution_order` than any downstream method.

## Data Model

No changes to `CellData`, `Method`, `RelationshipData`, or the public API.

`execute_plan` requires no changes: it already gathers all inputs into a temporary vector before
writing any outputs, so a self-referencing cell is always read at its pre-execution value.

## Changes

### `add_relationship` — validation relaxation

Remove the check that rejects methods with overlapping inputs and outputs:

```rust
// REMOVED:
for output in &method.outputs {
    if method.inputs.contains(output) {
        return Err(Error::InvalidMethod);
    }
}
```

All other checks remain: non-empty inputs/outputs, TypeId agreement, and cell existence.

### `planner.rs` — three modifications

#### 1. `source_cells` tracking

Add a `source_cells: HashSet<CellId>` set alongside the existing `determined` set.

- When the outer loop promotes a cell as a source: add to both `determined` and `source_cells`.
- When a method is selected and its outputs are marked determined: add to `determined` only.
  For self-referencing outputs (cells in both `inputs` and `outputs` of the selected method),
  also **remove** from `source_cells`: the method has overwritten the source value, so the cell
  must no longer qualify as a source for subsequent self-referencing eligibility checks.

`source_cells` records which cells were determined via `write()` rather than by a method. This
distinguishes "may be read as self-referencing input" from "was derived; cannot be used as
self-referencing input."

#### 2. Modified eligibility rule

For each method M, classify cells into three disjoint groups:

| Group | Definition | Eligibility condition |
|---|---|---|
| **pure inputs** | `inputs(M) ∖ outputs(M)` | all in `determined` |
| **self-referencing** | `inputs(M) ∩ outputs(M)` | all in `source_cells` |
| **pure outputs** | `outputs(M) ∖ inputs(M)` | none in `determined` |

The existing rule is the special case where every cell is either a pure input or a pure output.

#### 3. Modified pre-claiming feasibility

The feasibility check used before pre-claiming must treat self-referencing outputs differently.
Currently any output already in `determined` makes a method infeasible. For self-referencing
outputs, being in `source_cells` must count as feasible — the method is permitted to overwrite
its own source cell.

A self-referencing output is infeasible only if it was placed in `determined` by another method
(i.e., `determined.contains(o) && !source_cells.contains(o)`).

#### 4. Disambiguation

Because two self-referencing methods in the same relationship can become simultaneously eligible
(when all participating cells are sources), the single-eligible-method invariant must be relaxed.
When multiple methods are eligible during inner-queue processing of cell `c`, select the method
whose outputs contain `c`.

This naturally resolves to the correct method: the outer loop processes cells in descending
strength order, so `c` is always the weakest source processed so far — the cell that should
yield to the stronger sources.

The existing `debug_assert` that fires when multiple methods are simultaneously eligible must be
**removed**. It was correct when only pure-output methods existed (at most one could satisfy the
old eligibility rule at a time), but it no longer holds once self-referencing methods are allowed
to be simultaneously eligible. Remove the assertion rather than attempting to narrow it — the
disambiguation selection above provides the correct single-selection guarantee.

#### 5. Self-referencing outputs and re-queuing

When a self-referencing method is selected, its self-referencing outputs are already in
`determined`. Use the return value of `determined.insert(id)` (true if newly inserted) to avoid
pushing already-determined cells back onto the inner queue:

```rust
for &output in &method.outputs {
    let newly_determined = determined.insert(output);
    pre_claimed.remove(&output);
    if newly_determined {
        queue.push_back(output);
    }
}
```

## Correctness Test

The following integration test verifies the chain `a <= b <= c` under four write-order scenarios.
It is written to fail with the current implementation and pass after the changes above.

```rust
#[test]
fn self_ref_le_chain() {
    // a <= b enforced by R1:
    //   M0: a = min(a, b)  — fires when b is the stronger source
    //   M1: b = max(a, b)  — fires when a is the stronger source
    // b <= c enforced by R2:
    //   M2: b = min(b, c)  — fires when c is the stronger source
    //   M3: c = max(b, c)  — fires when b is the stronger source

    let mut sheet = Sheet::new();
    let a = sheet.add_cell(0_i32);
    let b = sheet.add_cell(0_i32);
    let c = sheet.add_cell(0_i32);

    sheet.add_relationship(vec![
        Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
        Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
    ]).unwrap();

    sheet.add_relationship(vec![
        Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
        Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
    ]).unwrap();

    // Case 1: already satisfied — no adjustment
    // Write order c, b, a → strengths: a highest.
    sheet.write(c, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 1_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 2: a > b and a > c, a is strongest → b and c raised to a.
    // Execution order: M1 (b = max(a,b)) then M3 (c = max(b,c)).
    // M3 reads the post-M1 value of b, so c is raised by the updated b.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 5);

    // Case 3: b > c, c is strongest → b lowered to c; a already ≤ b.
    // Execution order: M2 (b = min(b,c)) then M0 (a = min(a,b)).
    // M0 reads the post-M2 value of b.
    sheet.write(a, 1_i32).unwrap();
    sheet.write(b, 5_i32).unwrap();
    sheet.write(c, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 1);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);

    // Case 4: b is strongest, a above and c below → a clamped to b, c raised to b.
    sheet.write(c, 1_i32).unwrap();
    sheet.write(a, 5_i32).unwrap();
    sheet.write(b, 3_i32).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 3);
}
```

## Migration path to a split-cell model

A future conditional-constraint feature ("if `a` is true, `b` = 42; if `a` is false, `b`
reverts to its prior value 10") requires knowing the pre-propagation value of `b` when the
condition deactivates. That is an **execution concern only**; the planner changes in this design
are migration-stable.

The migration adds approximately 50–80 lines across `cell.rs` and `sheet.rs`:

| Location | Change |
|---|---|
| `CellData` | Add `source_value: Option<Box<dyn Any>>`. `write()` populates both `value` and `source_value`. |
| `execute_plan` | Self-referencing inputs read from `source_value`; all other reads remain from `value`. |
| `propagate` | When a conditional relation deactivates, restore derived cells' `value` from `source_value`. |

`planner.rs` requires **no additional changes** during that migration. The `source_cells` set
and modified eligibility rules carry over unchanged.
