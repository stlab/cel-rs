# Property Model Crate Design

**Date:** 2026-06-22
**Author:** Sean Parent
**Status:** Approved

## Overview

A new standalone Rust crate `property-model` implementing the runtime library for a property model — a bipartite graph of value cells and multi-way relationships, as described in the PCL paper (Järvi, Marcus, Parent et al.). This crate provides only the construction and execution API; the DSL layer will be a separate future crate that depends on this one and on `cel-runtime`.

Reference: [Declarative Forms: A Path to Correct, Efficient, and Accessible Software](https://github.com/sean-parent/pcl-paper/blob/main/pcl-paper.md)

## Placement

New crate `property-model/` added to the workspace `members` list in the root `Cargo.toml`. No existing workspace crates depend on it. The future DSL crate will add this as a dependency.

## Core Data Model

The bipartite graph has two node types: **cells** and **relationships**. Graph storage uses two `SlotMap`s (one per node type), giving typed stable handles with O(1) access that survive node removal.

### Cells

```text
CellId  →  CellData {
    value:    Box<dyn Any>,
    type_id:  TypeId,
    strength: u64,           // monotonically increasing logical clock; write() increments this
    changed:  bool,          // set during propagate(), cleared by clear_changed()
    adj:      Vec<RelationshipId>,
}
```

`strength` serves as the priority signal for the planner: higher strength = more recently written = preferred as a source (input to a method rather than an output).

### Relationships

```text
RelationshipId  →  RelationshipData {
    methods:  Vec<Method>,
    adj:      Vec<CellId>,   // union of all cells across all methods
}

Method {
    inputs:       Vec<CellId>,
    outputs:      Vec<CellId>,
    input_types:  Vec<TypeId>,   // declared at construction; validated against cell TypeIds
    output_types: Vec<TypeId>,   // declared at construction; validated against cell TypeIds
    function:     Box<dyn Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error>>,
}
```

TypeId checking occurs at **`add_relationship` time** (each method's declared input/output `TypeId`s are validated against the registered cell `TypeId`s) and at **`write` time** (the written value's `TypeId` is checked against the cell's `TypeId`). No runtime type checks occur during propagation.

### Graph Representation Choice

`petgraph` was considered but rejected: its `NodeIndex` handles are untyped (cell and relationship handles would be the same type), and its general-purpose API does not fit the bipartite structure. `slotmap` gives distinct typed key types (`CellId`, `RelationshipId`) so mixing them is a compile-time error.

## Public API

```rust
pub struct Sheet { ... }
pub struct CellId(/* slotmap key */);
pub struct RelationshipId(/* slotmap key */);

impl Sheet {
    pub fn new() -> Self;

    /// Register a cell. Returns a stable handle.
    pub fn add_cell<T: Any + 'static>(&mut self, value: T) -> CellId;

    /// Register a relationship. Validates all method TypeIds against registered cells.
    /// Errors: TypeMismatch, InvalidId, InvalidMethod.
    pub fn add_relationship(&mut self, methods: Vec<Method>) -> Result<RelationshipId, Error>;

    /// Write a value to a cell, incrementing its strength.
    /// Errors: TypeMismatch, InvalidId.
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error>;

    /// Read the current value of a cell.
    /// Errors: TypeMismatch, InvalidId.
    pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error>;

    /// Run the planning pass then execute selected methods. Populates the changed-cell set.
    /// Errors: Conflict, Cycle, MethodFailed.
    pub fn propagate(&mut self) -> Result<(), Error>;

    /// Iterate cells whose values changed during the last propagate() call.
    pub fn changed(&self) -> impl Iterator<Item = CellId> + '_;

    /// Clear the changed-cell set (call after your observation pass).
    pub fn clear_changed(&mut self);
}
```

### Method Construction

`Method` is constructed via typed helpers that capture TypeIds at the call site and erase them into the stored function. Common arities are covered; an escape hatch handles arbitrary arities.

```rust
impl Method {
    /// Typed 1-in, 1-out helper. TypeIds captured from A and B automatically.
    pub fn from_fn_1_1<A, B, F>(input: CellId, output: CellId, f: F) -> Self
    where
        A: Any + 'static,
        B: Any + 'static,
        F: Fn(&A) -> Result<B, anyhow::Error> + 'static;

    /// Typed 2-in, 1-out helper.
    pub fn from_fn_2_1<A, B, C, F>(inputs: [CellId; 2], output: CellId, f: F) -> Self
    where
        A: Any + 'static, B: Any + 'static, C: Any + 'static,
        F: Fn(&A, &B) -> Result<C, anyhow::Error> + 'static;

    /// Escape hatch: caller declares TypeIds explicitly and provides a type-erased function.
    /// Used by the future DSL crate and for unusual arities.
    pub fn new(
        inputs: Vec<CellId>,
        outputs: Vec<CellId>,
        input_types: Vec<TypeId>,
        output_types: Vec<TypeId>,
        f: impl Fn(&[&dyn Any]) -> Result<Vec<Box<dyn Any>>, anyhow::Error> + 'static,
    ) -> Self;
}
```

## Planning Algorithm

`propagate()` runs three sequential phases.

### Phase 1 — Method Selection (Greedy)

Each cell has a `strength` (u64) that is incremented by `write()`. The planner uses strength to decide which cells to preserve as sources and which to derive.

For each relationship in **insertion order** (`Sheet` maintains an explicit `Vec<RelationshipId>` alongside the `SlotMap`, since `SlotMap` does not guarantee iteration order):

1. Score each method by the **minimum strength** of its output cells. Lower score = better (we prefer to overwrite weak cells, not strong ones).
2. Select the highest-scoring valid method. A method is **valid** if none of its output cells have already been claimed as an output by a previously selected method.
3. Mark the selected method's output cells as claimed.

If no valid method exists for any relationship, return `Error::Conflict`.

### Phase 2 — Topological Sort (Kahn's Algorithm)

Build an execution DAG over the selected methods: an edge from method A → method B when A writes a cell that B reads as input. Run Kahn's algorithm. If the sort cannot complete (a cycle remains), return `Error::Cycle`.

### Phase 3 — Execution

Execute methods in topological order:

1. Gather input cell values as `&[&dyn Any]`.
2. Call the stored function.
3. Write returned values into output cells and set their `changed` flag.

After Phase 3, the sheet is in a consistent state. The client calls `changed()` to iterate changed cells and `clear_changed()` after processing them.

### Future Work: Model Checker

A future addition will verify that the constraint graph is always solvable while preserving at least the strongest cell. This is a static analysis pass (not part of runtime propagation) and will be a separate API entry point.

## Error Handling

```rust
#[non_exhaustive]
pub enum Error {
    /// A value's TypeId did not match the cell's registered TypeId.
    TypeMismatch { expected: TypeId, found: TypeId },

    /// A CellId or RelationshipId was not found in the sheet.
    InvalidId,

    /// No valid method assignment could be found (overconstrained).
    Conflict,

    /// The selected methods form a cycle.
    Cycle,

    /// A method's function returned an error during execution.
    MethodFailed(anyhow::Error),

    /// A method is structurally invalid (e.g. inputs ∩ outputs is non-empty,
    /// or inputs/outputs lists are empty).
    InvalidMethod,
}
```

`#[non_exhaustive]` allows adding variants (e.g., model checker diagnostics) without breaking downstream.

## Crate Structure

```text
property-model/
├── Cargo.toml          (deps: slotmap, anyhow)
├── src/
│   ├── lib.rs          (pub re-exports; crate-level doc)
│   ├── sheet.rs        (Sheet, propagate, changed tracking)
│   ├── cell.rs         (CellId, CellData)
│   ├── relationship.rs (RelationshipId, RelationshipData, Method)
│   ├── planner.rs      (phase 1 selection + phase 2 topo sort)
│   └── error.rs        (Error enum)
```

## Dependencies

| Crate     | Role                                              |
|-----------|---------------------------------------------------|
| `slotmap` | Stable typed handles for cells and relationships  |
| `anyhow`  | Ergonomic error propagation from method functions |

No dependency on any other crate in this workspace. The future DSL crate will depend on both `property-model` and `cel-runtime`.
