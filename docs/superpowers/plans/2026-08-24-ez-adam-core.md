# ez-adam Core (Document Model, Codegen, Persistence) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the headless, fully-tested core of `ez-adam` — its document model, pure mutation operations, `.adm2` code generation, and JSON persistence — with no UI. This is Phase 1 of the crate; a follow-up plan adds the Dioxus desktop UI on top of it.

**Architecture:** A `Document` struct holds `slotmap::SlotMap`-keyed collections of cells, cell-node canvas placements, relationship groups, and conditional groups, plus explicit declaration-order `Vec`s (SlotMap iteration order is unspecified, but `.adm2` generation needs deterministic output). All mutations go through small pure functions in `ops::*` rather than direct field access, so later UI event handlers stay thin. `codegen::generate_adm2` is a pure `&Document -> String` function, one-way only. `persistence` wraps `serde_json` for save/load.

**Tech Stack:** Rust 2024, `slotmap` (with `serde` feature), `serde`/`serde_json`, `cel-parser`, `cel-std`; `adam-lang` as a dev-dependency for round-trip `.adm2` parse validation in integration tests.

**Spec:** `docs/superpowers/specs/2026-08-24-ez-adam-design.md`

## Global Constraints

- Every public item needs a `///` contract-style doc comment (Summary / Preconditions / Postconditions / Complexity as applicable) — the workspace lints on `missing_docs` (`Cargo.toml`'s `[workspace.lints.rust]`).
- Precondition violations are checked with `debug_assert!` (or a natural `SlotMap` index panic), never a `Result` — preconditions describe caller bugs, not runtime errors.
- Every commit step runs `cargo fmt --all` first (enforced by this repo's pre-commit hook).
- Unit tests live inline in `#[cfg(test)] mod tests` within the file they test, matching `adam-rs`'s convention. Integration tests (needing `adam-lang`) live under `ez-adam/tests/`.
- `Cell.restrict` is captured in the model and round-trips through save/load, but is **not** emitted by `generate_adm2` — see <https://github.com/stlab/cel-rs/issues/146>. Do not implement restrict codegen in this plan.
- No cell/relationship/conditional-group *deletion* operations in this plan (out of scope — nothing in the spec requires it yet).

---

### Task 1: Crate scaffold

**Files:**
- Create: `ez-adam/Cargo.toml`
- Create: `ez-adam/src/lib.rs`
- Modify: `Cargo.toml:16-25` (root workspace `members` list)

**Interfaces:**
- Produces: an empty, compiling `ez-adam` library crate registered in the workspace.

- [ ] **Step 1: Create the crate's `Cargo.toml`**

```toml
[package]
name = "ez-adam"
version = "0.1.0"
edition = "2024"
description = "Standalone visual editor for adam-rs property models"

[dependencies]
slotmap = { version = "1.1", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
cel-parser = { path = "../cel-parser" }
cel-std = { path = "../cel-std" }

[dev-dependencies]
adam-lang = { path = "../adam-lang" }

[lints]
workspace = true
```

- [ ] **Step 2: Create `ez-adam/src/lib.rs`**

```rust
//! `ez-adam`: a standalone, diagrammatic visual editor for `adam-rs` property
//! models. See `docs/superpowers/specs/2026-08-24-ez-adam-design.md` for the
//! full design.
```

- [ ] **Step 3: Register the crate in the workspace**

In the root `Cargo.toml`, add `"ez-adam"` to the `[workspace] members` list, after `"begin"`:

```toml
[workspace]
members = [
    "cel-runtime",
    "cel-parser",
    "cel-rs-macros",
    "cel-std",
    "adam-rs",
    "adam-lang",
    "adam-lsp",
    "begin",
    "ez-adam",
    "xtask",
]
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p ez-adam`
Expected: succeeds with no warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add Cargo.toml Cargo.lock ez-adam/Cargo.toml ez-adam/src/lib.rs
git commit -m "feat(ez-adam): scaffold empty crate"
```

---

### Task 2: `model::geometry::Point`

**Files:**
- Create: `ez-adam/src/model/mod.rs`
- Create: `ez-adam/src/model/geometry.rs`
- Modify: `ez-adam/src/lib.rs`

**Interfaces:**
- Produces: `model::geometry::Point { x: f64, y: f64 }`, `Point::new(x, y) -> Point`.

- [ ] **Step 1: Write the failing test**

Create `ez-adam/src/model/geometry.rs`:

```rust
//! 2D positions for canvas node placement.

use serde::{Deserialize, Serialize};

/// A 2D position in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    /// Creates a point at the given coordinates.
    #[must_use]
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_x_and_y() {
        let p = Point::new(1.5, -2.0);
        assert_eq!(p.x, 1.5);
        assert_eq!(p.y, -2.0);
    }

    #[test]
    fn round_trips_through_json() {
        let p = Point::new(3.0, 4.0);
        let json = serde_json::to_string(&p).unwrap();
        let back: Point = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
```

- [ ] **Step 2: Wire up modules**

Create `ez-adam/src/model/mod.rs`:

```rust
//! The `ez-adam` document model: cells, canvas placements, relationship
//! groups, and conditional groups (see `crate::model::document::Document`).

pub mod geometry;
```

Add to `ez-adam/src/lib.rs`:

```rust
pub mod model;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p ez-adam --lib model::geometry::tests`
Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/lib.rs ez-adam/src/model/mod.rs ez-adam/src/model/geometry.rs
git commit -m "feat(ez-adam): add Point"
```

---

### Task 3: `model::cell` — `CellId`, `ClampRange<T>`, `CellType`, `Cell`

**Files:**
- Create: `ez-adam/src/model/cell.rs`
- Modify: `ez-adam/src/model/mod.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `CellId` (slotmap key), `ClampRange<T> { min: Option<T>, max: Option<T> }`, `CellType::{F64{clamp}, I64{clamp}, Bool, Text}`, `CellType::f64()`/`CellType::i64()` (no-clamp constructors), `Cell { name, ty, output, restrict }`, `Cell::new(name, ty) -> Cell`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/model/cell.rs`:

```rust
//! Cell data: a named, typed value in the property model, independent of
//! where it's placed on the canvas (see
//! [`crate::model::cell_node::CellNode`]).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

new_key_type! {
    /// A stable handle to a [`Cell`] in a
    /// [`crate::model::document::Document`].
    pub struct CellId;
}

/// Optional lower/upper bounds for a numeric cell, stored in the cell's own
/// concrete type so an out-of-range or fractional bound for that type
/// cannot be represented.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ClampRange<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

/// The value type of a [`Cell`], carrying a numeric variant's clamp range
/// inline so a clamp bound can't be attached to a `Bool`/`Text` cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellType {
    F64 { clamp: ClampRange<f64> },
    I64 { clamp: ClampRange<i64> },
    Bool,
    Text,
}

impl CellType {
    /// An `F64` cell type with no clamp bounds.
    #[must_use]
    pub fn f64() -> Self {
        CellType::F64 {
            clamp: ClampRange::default(),
        }
    }

    /// An `I64` cell type with no clamp bounds.
    #[must_use]
    pub fn i64() -> Self {
        CellType::I64 {
            clamp: ClampRange::default(),
        }
    }
}

/// A named, typed value in the property model.
///
/// A `Cell`'s data is shared by every
/// [`CellNode`](crate::model::cell_node::CellNode) that places it on the
/// canvas — editing a `Cell`'s properties through any one of its nodes
/// updates the single shared value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub name: String,
    pub ty: CellType,
    pub output: bool,
    /// Raw CEL boolean expression text; `_` refers to this cell's own
    /// value. Not currently emitted by `generate_adm2` — see
    /// <https://github.com/stlab/cel-rs/issues/146>.
    pub restrict: Option<String>,
}

impl Cell {
    /// Creates a new, non-output cell with no restriction.
    #[must_use]
    pub fn new(name: impl Into<String>, ty: CellType) -> Self {
        Cell {
            name: name.into(),
            ty,
            output: false,
            restrict: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f64_has_no_clamp_bounds_by_default() {
        assert_eq!(
            CellType::f64(),
            CellType::F64 {
                clamp: ClampRange { min: None, max: None }
            }
        );
    }

    #[test]
    fn i64_has_no_clamp_bounds_by_default() {
        assert_eq!(
            CellType::i64(),
            CellType::I64 {
                clamp: ClampRange { min: None, max: None }
            }
        );
    }

    #[test]
    fn new_is_not_output_and_has_no_restrict() {
        let cell = Cell::new("width_pixels", CellType::i64());
        assert_eq!(cell.name, "width_pixels");
        assert!(!cell.output);
        assert!(cell.restrict.is_none());
    }

    #[test]
    fn clamp_range_can_hold_only_a_minimum() {
        let range = ClampRange { min: Some(0i64), max: None };
        assert_eq!(range.min, Some(0));
        assert_eq!(range.max, None);
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/model/mod.rs`:

```rust
pub mod cell;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib model::cell::tests`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/model/mod.rs ez-adam/src/model/cell.rs
git commit -m "feat(ez-adam): add Cell, CellType, ClampRange"
```

---

### Task 4: `model::cell_node` — `CellNodeId`, `CellNode`

**Files:**
- Create: `ez-adam/src/model/cell_node.rs`
- Modify: `ez-adam/src/model/mod.rs`

**Interfaces:**
- Consumes: `model::cell::CellId` (Task 3), `model::geometry::Point` (Task 2).
- Produces: `CellNodeId` (slotmap key), `CellNode { cell: CellId, position: Point }`, `CellNode::new(cell, position) -> CellNode`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/model/cell_node.rs`:

```rust
//! Canvas placements of cells (see [`crate::model::cell::Cell`]).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell::CellId;
use crate::model::geometry::Point;

new_key_type! {
    /// A stable handle to a [`CellNode`] in a
    /// [`crate::model::document::Document`].
    pub struct CellNodeId;
}

/// A visual placement of a [`Cell`](crate::model::cell::Cell) on the
/// canvas.
///
/// Multiple `CellNode`s may reference the same [`CellId`] — "two instances
/// of the same value in the graph" — each with its own [`Point`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CellNode {
    pub cell: CellId,
    pub position: Point,
}

impl CellNode {
    /// Creates a node placing `cell` at `position`.
    #[must_use]
    pub fn new(cell: CellId, position: Point) -> Self {
        CellNode { cell, position }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn new_sets_cell_and_position() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = cells.insert(());
        let node = CellNode::new(cell, Point::new(1.0, 2.0));
        assert_eq!(node.cell, cell);
        assert_eq!(node.position, Point::new(1.0, 2.0));
    }

    #[test]
    fn two_nodes_of_the_same_cell_at_different_positions_are_distinct() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let cell = cells.insert(());
        let a = CellNode::new(cell, Point::new(0.0, 0.0));
        let b = CellNode::new(cell, Point::new(10.0, 0.0));
        assert_eq!(a.cell, b.cell);
        assert_ne!(a, b);
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/model/mod.rs`:

```rust
pub mod cell_node;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib model::cell_node::tests`
Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/model/mod.rs ez-adam/src/model/cell_node.rs
git commit -m "feat(ez-adam): add CellNode"
```

---

### Task 5: `model::relationship_group`

**Files:**
- Create: `ez-adam/src/model/relationship_group.rs`
- Modify: `ez-adam/src/model/mod.rs`

**Interfaces:**
- Consumes: `model::cell_node::CellNodeId` (Task 4), `model::geometry::Point` (Task 2).
- Produces: `RelationshipGroupId` (slotmap key), `RelationshipGroup { display_name, position, members: Vec<(CellNodeId, String)> }`, `RelationshipGroup::new(display_name, position) -> RelationshipGroup`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/model/relationship_group.rs`:

```rust
//! Relationship groups: an alternative method for deriving one or more
//! bound cells.

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell_node::CellNodeId;
use crate::model::geometry::Point;

new_key_type! {
    /// A stable handle to a [`RelationshipGroup`] in a
    /// [`crate::model::document::Document`].
    pub struct RelationshipGroupId;
}

/// A group of cell bindings representing one `.adm2` `relationship { ... }`
/// block (or a branch entry inside a `conditional`).
///
/// `display_name` (e.g. `"r1"`) is UI bookkeeping only — `.adm2`
/// relationship blocks are anonymous, so it is never emitted by
/// `generate_adm2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipGroup {
    pub display_name: String,
    pub position: Point,
    /// One entry per bound cell: the node gives the edge's canvas
    /// endpoint, the `String` is that member's RHS formula text (CEL
    /// source, empty until the user fills it in).
    pub members: Vec<(CellNodeId, String)>,
}

impl RelationshipGroup {
    /// Creates an empty relationship group at `position` with no members.
    #[must_use]
    pub fn new(display_name: impl Into<String>, position: Point) -> Self {
        RelationshipGroup {
            display_name: display_name.into(),
            position,
            members: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_no_members() {
        let group = RelationshipGroup::new("r1", Point::new(0.0, 0.0));
        assert_eq!(group.display_name, "r1");
        assert!(group.members.is_empty());
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/model/mod.rs`:

```rust
pub mod relationship_group;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib model::relationship_group::tests`
Expected: 1 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/model/mod.rs ez-adam/src/model/relationship_group.rs
git commit -m "feat(ez-adam): add RelationshipGroup"
```

---

### Task 6: `model::conditional_group`

**Files:**
- Create: `ez-adam/src/model/conditional_group.rs`
- Modify: `ez-adam/src/model/mod.rs`

**Interfaces:**
- Consumes: `model::cell::CellId` (Task 3), `model::relationship_group::RelationshipGroupId` (Task 5), `model::geometry::Point` (Task 2).
- Produces: `ConditionalGroupId` (slotmap key), `CellValueLiteral::{Bool, I64, Text}`, `ConditionExpr::{Cells(Vec<CellId>), Formula{referenced_cells, expr}}`, `ConditionalBranch { values: Vec<CellValueLiteral>, enabled_groups: Vec<RelationshipGroupId> }`, `ConditionalGroup { display_name, position, condition, branches, default }`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/model/conditional_group.rs`:

```rust
//! Conditional groups: alternative sets of relationship-group activations
//! selected by a condition (mirrors `.adm2`'s
//! `conditional <expr> { <literal> => {...} }`).

use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

use crate::model::cell::CellId;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;

new_key_type! {
    /// A stable handle to a [`ConditionalGroup`] in a
    /// [`crate::model::document::Document`].
    pub struct ConditionalGroupId;
}

/// A literal value matched against a conditional group's branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellValueLiteral {
    Bool(bool),
    I64(i64),
    Text(String),
}

/// The expression a [`ConditionalGroup`] branches on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConditionExpr {
    /// Implicit tuple of the dragged-in cells' own values. Auto-enumerable
    /// into a full branch table only when every cell is `Bool`.
    Cells(Vec<CellId>),
    /// A user-authored CEL expression referencing `referenced_cells` (e.g.
    /// `x > 100`). Branches are added manually.
    Formula {
        referenced_cells: Vec<CellId>,
        expr: String,
    },
}

/// One row of a conditional group's enable-table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalBranch {
    /// One literal per [`ConditionExpr`] cell, aligned by index.
    pub values: Vec<CellValueLiteral>,
    /// The relationship groups active (checked) on this branch.
    pub enabled_groups: Vec<RelationshipGroupId>,
}

/// A set of alternative relationship-group activations selected by
/// [`ConditionExpr`]'s current value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalGroup {
    pub display_name: String,
    pub position: Point,
    pub condition: ConditionExpr,
    pub branches: Vec<ConditionalBranch>,
    /// The relationship groups active when no branch matches. Always
    /// present — `adam-rs`'s `Sheet::add_conditional` requires a default
    /// non-optionally.
    pub default: Vec<RelationshipGroupId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    #[test]
    fn cell_value_literals_of_different_variants_are_unequal() {
        assert_ne!(CellValueLiteral::Bool(true), CellValueLiteral::Bool(false));
        assert_ne!(CellValueLiteral::I64(1), CellValueLiteral::I64(2));
    }

    #[test]
    fn condition_expr_cells_stores_the_given_cell_ids() {
        let mut cells: SlotMap<CellId, ()> = SlotMap::with_key();
        let a = cells.insert(());
        let b = cells.insert(());
        let condition = ConditionExpr::Cells(vec![a, b]);
        assert_eq!(condition, ConditionExpr::Cells(vec![a, b]));
    }

    #[test]
    fn conditional_branch_stores_values_and_enabled_groups() {
        let mut groups: SlotMap<RelationshipGroupId, ()> = SlotMap::with_key();
        let group = groups.insert(());
        let branch = ConditionalBranch {
            values: vec![CellValueLiteral::Bool(true)],
            enabled_groups: vec![group],
        };
        assert_eq!(branch.values, vec![CellValueLiteral::Bool(true)]);
        assert_eq!(branch.enabled_groups, vec![group]);
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/model/mod.rs`:

```rust
pub mod conditional_group;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib model::conditional_group::tests`
Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/model/mod.rs ez-adam/src/model/conditional_group.rs
git commit -m "feat(ez-adam): add ConditionalGroup"
```

---

### Task 7: `model::document::Document`

**Files:**
- Create: `ez-adam/src/model/document.rs`
- Modify: `ez-adam/src/model/mod.rs`

**Interfaces:**
- Consumes: `Cell`/`CellId` (Task 3), `CellNode`/`CellNodeId` (Task 4), `RelationshipGroup`/`RelationshipGroupId` (Task 5), `ConditionalGroup`/`ConditionalGroupId` (Task 6).
- Produces: `Document { format_version, sheet_name, cells, cell_order, cell_nodes, relationship_groups, relationship_group_order, conditional_groups, conditional_group_order }`, `Document::new(sheet_name) -> Document`, `Document::cells_in_order(&self) -> impl Iterator<Item = (CellId, &Cell)>`, `Document::relationship_groups_in_order(&self) -> impl Iterator<Item = (RelationshipGroupId, &RelationshipGroup)>`, `Document::conditional_groups_in_order(&self) -> impl Iterator<Item = (ConditionalGroupId, &ConditionalGroup)>`, `CURRENT_FORMAT_VERSION: u32`.

`SlotMap` iteration order is not a documented guarantee, but `.adm2` generation (Task 14+) needs deterministic output — hence the explicit `*_order: Vec<Id>` fields alongside each `SlotMap`, appended to by every `ops::*` insertion function (Tasks 8–12).

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/model/document.rs`:

```rust
//! The single source-of-truth document a `ez-adam` editor session edits.

use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

use crate::model::cell::{Cell, CellId};
use crate::model::cell_node::{CellNode, CellNodeId};
use crate::model::conditional_group::{ConditionalGroup, ConditionalGroupId};
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// The current on-disk format version for [`Document`]'s JSON
/// serialization.
///
/// Bump this and add a migration path in [`crate::persistence`] whenever
/// `Document`'s shape changes in a way that breaks deserializing older
/// files.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// A complete `ez-adam` editor document: one `.adm2` `sheet`'s worth of
/// cells, canvas placements, relationship groups, and conditional groups.
///
/// The `*_order` fields record declaration order explicitly, since
/// `SlotMap` iteration order is unspecified but `.adm2` generation needs
/// deterministic output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format_version: u32,
    pub sheet_name: String,
    pub cells: SlotMap<CellId, Cell>,
    pub cell_order: Vec<CellId>,
    pub cell_nodes: SlotMap<CellNodeId, CellNode>,
    pub relationship_groups: SlotMap<RelationshipGroupId, RelationshipGroup>,
    pub relationship_group_order: Vec<RelationshipGroupId>,
    pub conditional_groups: SlotMap<ConditionalGroupId, ConditionalGroup>,
    pub conditional_group_order: Vec<ConditionalGroupId>,
}

impl Document {
    /// Creates a new, empty document for a sheet named `sheet_name`.
    #[must_use]
    pub fn new(sheet_name: impl Into<String>) -> Self {
        Document {
            format_version: CURRENT_FORMAT_VERSION,
            sheet_name: sheet_name.into(),
            cells: SlotMap::with_key(),
            cell_order: Vec::new(),
            cell_nodes: SlotMap::with_key(),
            relationship_groups: SlotMap::with_key(),
            relationship_group_order: Vec::new(),
            conditional_groups: SlotMap::with_key(),
            conditional_group_order: Vec::new(),
        }
    }

    /// Iterates over `(CellId, &Cell)` in declaration order.
    ///
    /// - Complexity: O(n) in the number of cells.
    pub fn cells_in_order(&self) -> impl Iterator<Item = (CellId, &Cell)> {
        self.cell_order.iter().map(move |id| (*id, &self.cells[*id]))
    }

    /// Iterates over `(RelationshipGroupId, &RelationshipGroup)` in
    /// declaration order.
    ///
    /// - Complexity: O(n) in the number of relationship groups.
    pub fn relationship_groups_in_order(
        &self,
    ) -> impl Iterator<Item = (RelationshipGroupId, &RelationshipGroup)> {
        self.relationship_group_order
            .iter()
            .map(move |id| (*id, &self.relationship_groups[*id]))
    }

    /// Iterates over `(ConditionalGroupId, &ConditionalGroup)` in
    /// declaration order.
    ///
    /// - Complexity: O(n) in the number of conditional groups.
    pub fn conditional_groups_in_order(
        &self,
    ) -> impl Iterator<Item = (ConditionalGroupId, &ConditionalGroup)> {
        self.conditional_group_order
            .iter()
            .map(move |id| (*id, &self.conditional_groups[*id]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let doc = Document::new("demo");
        assert_eq!(doc.sheet_name, "demo");
        assert_eq!(doc.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(doc.cells_in_order().count(), 0);
        assert_eq!(doc.relationship_groups_in_order().count(), 0);
        assert_eq!(doc.conditional_groups_in_order().count(), 0);
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/model/mod.rs`:

```rust
pub mod document;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib model::document::tests`
Expected: 1 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/model/mod.rs ez-adam/src/model/document.rs
git commit -m "feat(ez-adam): add Document"
```

---

### Task 8: `ops::cells`

**Files:**
- Create: `ez-adam/src/ops/mod.rs`
- Create: `ez-adam/src/ops/cells.rs`
- Modify: `ez-adam/src/lib.rs`

**Interfaces:**
- Consumes: `Document` (Task 7), `Cell`/`CellId`/`CellType` (Task 3), `CellNode`/`CellNodeId` (Task 4), `Point` (Task 2).
- Produces: `ops::cells::add_cell(doc, name, ty) -> CellId`, `ops::cells::add_cell_node(doc, cell, position) -> CellNodeId`, `ops::cells::set_output(doc, cell, output: bool)`, `ops::cells::set_restrict(doc, cell, restrict: Option<String>)`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/ops/cells.rs`:

```rust
//! Pure mutation functions over a [`Document`]'s cells.

use crate::model::cell::{Cell, CellId, CellType};
use crate::model::cell_node::{CellNode, CellNodeId};
use crate::model::document::Document;
use crate::model::geometry::Point;

/// Adds a new, non-output cell named `name` with no restriction.
///
/// - Postcondition: the returned id resolves to a [`Cell`] with `output ==
///   false` and `restrict == None`.
#[must_use]
pub fn add_cell(doc: &mut Document, name: impl Into<String>, ty: CellType) -> CellId {
    let id = doc.cells.insert(Cell::new(name, ty));
    doc.cell_order.push(id);
    id
}

/// Places a new visual instance of `cell` at `position`.
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
#[must_use]
pub fn add_cell_node(doc: &mut Document, cell: CellId, position: Point) -> CellNodeId {
    debug_assert!(doc.cells.contains_key(cell), "cell is not a valid key");
    doc.cell_nodes.insert(CellNode::new(cell, position))
}

/// Sets whether `cell` is an output cell (emits `out <name> := <name>;`).
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
pub fn set_output(doc: &mut Document, cell: CellId, output: bool) {
    doc.cells[cell].output = output;
}

/// Sets `cell`'s restrict-expression text (or clears it with `None`).
///
/// - Precondition: `cell` is a valid key in `doc.cells`.
pub fn set_restrict(doc: &mut Document, cell: CellId, restrict: Option<String>) {
    doc.cells[cell].restrict = restrict;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_cell_inserts_a_non_output_cell_with_no_restrict() {
        let mut doc = Document::new("demo");
        let id = add_cell(&mut doc, "width_pixels", CellType::i64());
        assert_eq!(doc.cells[id].name, "width_pixels");
        assert!(!doc.cells[id].output);
        assert!(doc.cells[id].restrict.is_none());
        assert_eq!(doc.cell_order, vec![id]);
    }

    #[test]
    fn add_cell_node_places_the_cell_at_the_position() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        let node = add_cell_node(&mut doc, cell, Point::new(10.0, 20.0));
        assert_eq!(doc.cell_nodes[node].cell, cell);
        assert_eq!(doc.cell_nodes[node].position, Point::new(10.0, 20.0));
    }

    #[test]
    fn set_output_updates_the_cells_output_flag() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);
        assert!(doc.cells[cell].output);
    }

    #[test]
    fn set_restrict_updates_the_cells_restrict_text() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_restrict(&mut doc, cell, Some("_ > 0".to_string()));
        assert_eq!(doc.cells[cell].restrict.as_deref(), Some("_ > 0"));
    }
}
```

- [ ] **Step 2: Wire up modules**

Create `ez-adam/src/ops/mod.rs`:

```rust
//! Pure mutation functions over a [`crate::model::document::Document`].
//! Every editor interaction (toolbar clicks, side-panel edits) goes through
//! one of these functions rather than mutating `Document`'s fields
//! directly, so UI event handlers stay thin passthroughs.

pub mod cells;
```

Add to `ez-adam/src/lib.rs`:

```rust
pub mod ops;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib ops::cells::tests`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/lib.rs ez-adam/src/ops/mod.rs ez-adam/src/ops/cells.rs
git commit -m "feat(ez-adam): add cell mutation ops"
```

---

### Task 9: `ops::relationships` — create, add member, set formula

**Files:**
- Create: `ez-adam/src/ops/relationships.rs`
- Modify: `ez-adam/src/ops/mod.rs`

**Interfaces:**
- Consumes: `Document` (Task 7), `RelationshipGroup`/`RelationshipGroupId` (Task 5), `CellNodeId` (Task 4), `Point` (Task 2).
- Produces: `ops::relationships::create_relationship(doc, a, b, position) -> RelationshipGroupId`, `ops::relationships::add_member(doc, group, node)`, `ops::relationships::set_member_formula(doc, group, node, formula)`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/ops/relationships.rs`:

```rust
//! Pure mutation functions over a [`Document`]'s relationship groups.

use crate::model::cell_node::CellNodeId;
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// Creates a new relationship group binding `a` and `b` as members with
/// empty formula text, auto-named `"r<n>"` from `doc`'s current
/// relationship-group count.
///
/// - Precondition: `a` and `b` are valid keys in `doc.cell_nodes`.
/// - Postcondition: the returned group's `members` is `[(a, ""), (b, "")]`.
#[must_use]
pub fn create_relationship(
    doc: &mut Document,
    a: CellNodeId,
    b: CellNodeId,
    position: Point,
) -> RelationshipGroupId {
    debug_assert!(doc.cell_nodes.contains_key(a), "a is not a valid key");
    debug_assert!(doc.cell_nodes.contains_key(b), "b is not a valid key");
    let display_name = format!("r{}", doc.relationship_group_order.len() + 1);
    let mut group = RelationshipGroup::new(display_name, position);
    group.members.push((a, String::new()));
    group.members.push((b, String::new()));
    let id = doc.relationship_groups.insert(group);
    doc.relationship_group_order.push(id);
    id
}

/// Adds `node` as a new member of `group` with empty formula text.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Precondition: `node` is a valid key in `doc.cell_nodes`.
/// - Precondition: `node` is not already a member of `group`.
pub fn add_member(doc: &mut Document, group: RelationshipGroupId, node: CellNodeId) {
    debug_assert!(doc.cell_nodes.contains_key(node), "node is not a valid key");
    let g = &mut doc.relationship_groups[group];
    debug_assert!(
        !g.members.iter().any(|(n, _)| *n == node),
        "node is already a member"
    );
    g.members.push((node, String::new()));
}

/// Sets `node`'s RHS formula text within `group`.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Precondition: `node` is a member of `group`.
pub fn set_member_formula(
    doc: &mut Document,
    group: RelationshipGroupId,
    node: CellNodeId,
    formula: impl Into<String>,
) {
    let g = &mut doc.relationship_groups[group];
    let entry = g.members.iter_mut().find(|(n, _)| *n == node);
    debug_assert!(entry.is_some(), "node is not a member of group");
    if let Some((_, f)) = entry {
        *f = formula.into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};

    fn two_nodes(doc: &mut Document) -> (CellNodeId, CellNodeId) {
        let a = add_cell(doc, "width_pixels", CellType::i64());
        let b = add_cell(doc, "height_pixels", CellType::i64());
        (
            add_cell_node(doc, a, Point::new(0.0, 0.0)),
            add_cell_node(doc, b, Point::new(10.0, 0.0)),
        )
    }

    #[test]
    fn create_relationship_binds_both_nodes_with_empty_formulas() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(5.0, 5.0));
        assert_eq!(
            doc.relationship_groups[group].members,
            vec![(a, String::new()), (b, String::new())]
        );
    }

    #[test]
    fn create_relationship_auto_names_sequentially() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let g1 = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        let g2 = create_relationship(&mut doc, a, b, Point::new(1.0, 1.0));
        assert_eq!(doc.relationship_groups[g1].display_name, "r1");
        assert_eq!(doc.relationship_groups[g2].display_name, "r2");
    }

    #[test]
    fn add_member_appends_a_new_member_with_empty_formula() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        let c = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let c_node = add_cell_node(&mut doc, c, Point::new(20.0, 0.0));
        add_member(&mut doc, group, c_node);
        assert_eq!(doc.relationship_groups[group].members.len(), 3);
        assert_eq!(
            doc.relationship_groups[group].members[2],
            (c_node, String::new())
        );
    }

    #[test]
    fn set_member_formula_updates_the_matching_members_formula() {
        let mut doc = Document::new("demo");
        let (a, b) = two_nodes(&mut doc);
        let group = create_relationship(&mut doc, a, b, Point::new(0.0, 0.0));
        set_member_formula(&mut doc, group, a, "height_pixels * 2");
        assert_eq!(doc.relationship_groups[group].members[0].1, "height_pixels * 2");
        assert_eq!(doc.relationship_groups[group].members[1].1, "");
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/ops/mod.rs`:

```rust
pub mod relationships;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib ops::relationships::tests`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/ops/mod.rs ez-adam/src/ops/relationships.rs
git commit -m "feat(ez-adam): add relationship-group creation/editing ops"
```

---

### Task 10: `ops::relationships::duplicate_relationship_group`

**Files:**
- Modify: `ez-adam/src/ops/relationships.rs`

**Interfaces:**
- Consumes: everything from Task 9, plus `CellNode` (Task 4).
- Produces: `ops::relationships::duplicate_relationship_group(doc, group, offset) -> RelationshipGroupId`.

- [ ] **Step 1: Write the failing tests**

Add to `ez-adam/src/ops/relationships.rs` (below `set_member_formula`):

```rust
use crate::model::cell_node::CellNode;

/// Creates a copy of `group`'s formula "shape": a new relationship group
/// bound to new [`CellNode`]s over the *same* underlying cells as `group`'s
/// members (offset by `offset`), with formula text cleared.
///
/// This is "two instances of the same value in the graph" — the duplicated
/// nodes reference the same [`CellId`](crate::model::cell::CellId)s, not
/// copies of the cells themselves.
///
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Postcondition: the returned group has the same number of members as
///   `group`, each bound to a fresh node over the same cell, with empty
///   formula text.
///
/// - Complexity: O(n) in `group`'s member count.
#[must_use]
pub fn duplicate_relationship_group(
    doc: &mut Document,
    group: RelationshipGroupId,
    offset: Point,
) -> RelationshipGroupId {
    let source_members = doc.relationship_groups[group].members.clone();
    let source_position = doc.relationship_groups[group].position;

    let mut new_members = Vec::with_capacity(source_members.len());
    for (node, _formula) in &source_members {
        let CellNode { cell, position } = doc.cell_nodes[*node];
        let new_position = Point::new(position.x + offset.x, position.y + offset.y);
        let new_node = doc.cell_nodes.insert(CellNode::new(cell, new_position));
        new_members.push((new_node, String::new()));
    }

    let display_name = format!("r{}", doc.relationship_group_order.len() + 1);
    let new_position = Point::new(source_position.x + offset.x, source_position.y + offset.y);
    let mut new_group = RelationshipGroup::new(display_name, new_position);
    new_group.members = new_members;
    let id = doc.relationship_groups.insert(new_group);
    doc.relationship_group_order.push(id);
    id
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};

    #[test]
    fn duplicate_binds_new_nodes_to_the_same_cells() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        let dup_cells: Vec<_> = doc.relationship_groups[dup]
            .members
            .iter()
            .map(|(n, _)| doc.cell_nodes[*n].cell)
            .collect();
        assert_eq!(dup_cells, vec![a, b]);
    }

    #[test]
    fn duplicate_clears_formula_text() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        for (_, formula) in &doc.relationship_groups[dup].members {
            assert_eq!(formula, "");
        }
    }

    #[test]
    fn duplicate_creates_distinct_node_instances() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        let dup_nodes: Vec<_> = doc.relationship_groups[dup]
            .members
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert!(!dup_nodes.contains(&a_node));
        assert!(!dup_nodes.contains(&b_node));
    }

    #[test]
    fn duplicate_auto_names_sequentially() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let dup = duplicate_relationship_group(&mut doc, group, Point::new(0.0, 100.0));

        assert_eq!(doc.relationship_groups[group].display_name, "r1");
        assert_eq!(doc.relationship_groups[dup].display_name, "r2");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ez-adam --lib ops::relationships`
Expected: 8 passed (4 from Task 9 + 4 new).

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add ez-adam/src/ops/relationships.rs
git commit -m "feat(ez-adam): add duplicate_relationship_group"
```

---

### Task 11: `ops::conditionals::add_conditional_from_bool_cells`

**Files:**
- Create: `ez-adam/src/ops/conditionals.rs`
- Modify: `ez-adam/src/ops/mod.rs`

**Interfaces:**
- Consumes: `Document` (Task 7), `CellId`/`CellType` (Task 3), `RelationshipGroupId` (Task 5), `ConditionalGroup`/`ConditionalGroupId`/`ConditionExpr`/`ConditionalBranch`/`CellValueLiteral` (Task 6), `Point` (Task 2).
- Produces: `ops::conditionals::add_conditional_from_bool_cells(doc, cells, group, position) -> ConditionalGroupId`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/ops/conditionals.rs`:

```rust
//! Pure mutation functions over a [`Document`]'s conditional groups.

use crate::model::cell::{CellId, CellType};
use crate::model::conditional_group::{
    CellValueLiteral, ConditionExpr, ConditionalBranch, ConditionalGroup, ConditionalGroupId,
};
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;

/// Wraps `group` in a new conditional group whose condition is the tuple of
/// `cells`' own boolean values, auto-enumerated into every combination of
/// `true`/`false` (`2.pow(cells.len())` branches). `group` is enabled on
/// the branch where every cell is `true`; every other branch (and the
/// default) starts with no enabled groups.
///
/// - Precondition: `cells` is non-empty.
/// - Precondition: every cell in `cells` has [`CellType::Bool`].
/// - Precondition: `group` is a valid key in `doc.relationship_groups`.
/// - Postcondition: the returned group has exactly `2.pow(cells.len())`
///   branches and an empty `default`.
///
/// - Complexity: O(2^n) in `cells.len()`.
#[must_use]
pub fn add_conditional_from_bool_cells(
    doc: &mut Document,
    cells: Vec<CellId>,
    group: RelationshipGroupId,
    position: Point,
) -> ConditionalGroupId {
    debug_assert!(!cells.is_empty(), "cells must be non-empty");
    debug_assert!(
        cells.iter().all(|c| matches!(doc.cells[*c].ty, CellType::Bool)),
        "every condition cell must be Bool"
    );
    debug_assert!(
        doc.relationship_groups.contains_key(group),
        "group is not a valid key"
    );

    let branch_count = 1usize << cells.len();
    let mut branches = Vec::with_capacity(branch_count);
    for combo in 0..branch_count {
        let values: Vec<CellValueLiteral> = (0..cells.len())
            .map(|i| CellValueLiteral::Bool((combo >> i) & 1 == 1))
            .collect();
        let all_true = values
            .iter()
            .all(|v| matches!(v, CellValueLiteral::Bool(true)));
        let enabled_groups = if all_true { vec![group] } else { Vec::new() };
        branches.push(ConditionalBranch {
            values,
            enabled_groups,
        });
    }

    let display_name = format!("c{}", doc.conditional_group_order.len() + 1);
    let id = doc.conditional_groups.insert(ConditionalGroup {
        display_name,
        position,
        condition: ConditionExpr::Cells(cells),
        branches,
        default: Vec::new(),
    });
    doc.conditional_group_order.push(id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    fn setup_group_over_two_bool_cells(doc: &mut Document) -> (CellId, RelationshipGroupId) {
        let condition_cell = add_cell(doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(doc, "width_pixels", CellType::i64());
        let b = add_cell(doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(doc, a_node, b_node, Point::new(5.0, 5.0));
        (condition_cell, group)
    }

    #[test]
    fn one_bool_cell_creates_two_branches() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert_eq!(doc.conditional_groups[cond].branches.len(), 2);
    }

    #[test]
    fn two_bool_cells_creates_four_branches() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let second_cell = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell, second_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert_eq!(doc.conditional_groups[cond].branches.len(), 4);
    }

    #[test]
    fn group_is_enabled_only_on_the_all_true_branch() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        let all_true_branch = doc.conditional_groups[cond]
            .branches
            .iter()
            .find(|b| b.values == vec![CellValueLiteral::Bool(true)])
            .unwrap();
        assert_eq!(all_true_branch.enabled_groups, vec![group]);

        let false_branch = doc.conditional_groups[cond]
            .branches
            .iter()
            .find(|b| b.values == vec![CellValueLiteral::Bool(false)])
            .unwrap();
        assert!(false_branch.enabled_groups.is_empty());
    }

    #[test]
    fn default_starts_empty() {
        let mut doc = Document::new("demo");
        let (condition_cell, group) = setup_group_over_two_bool_cells(&mut doc);
        let cond = add_conditional_from_bool_cells(
            &mut doc,
            vec![condition_cell],
            group,
            Point::new(0.0, 0.0),
        );
        assert!(doc.conditional_groups[cond].default.is_empty());
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/ops/mod.rs`:

```rust
pub mod conditionals;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib ops::conditionals::tests`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/ops/mod.rs ez-adam/src/ops/conditionals.rs
git commit -m "feat(ez-adam): add add_conditional_from_bool_cells"
```

---

### Task 12: `ops::conditionals` — formula mode, add branch, toggle group

**Files:**
- Modify: `ez-adam/src/ops/conditionals.rs`

**Interfaces:**
- Produces: `ops::conditionals::add_conditional_with_formula(doc, referenced_cells, expr, position) -> ConditionalGroupId`, `ops::conditionals::add_branch(doc, conditional, values) -> usize`, `ops::conditionals::toggle_enabled_group(doc, conditional, branch_index, group)`.

- [ ] **Step 1: Write the failing tests**

Add to `ez-adam/src/ops/conditionals.rs` (below `add_conditional_from_bool_cells`):

```rust
/// Creates a new conditional group whose condition is a user-authored CEL
/// formula over `referenced_cells`, with no branches yet (added via
/// [`add_branch`]) and an empty default.
#[must_use]
pub fn add_conditional_with_formula(
    doc: &mut Document,
    referenced_cells: Vec<CellId>,
    expr: impl Into<String>,
    position: Point,
) -> ConditionalGroupId {
    let display_name = format!("c{}", doc.conditional_group_order.len() + 1);
    let id = doc.conditional_groups.insert(ConditionalGroup {
        display_name,
        position,
        condition: ConditionExpr::Formula {
            referenced_cells,
            expr: expr.into(),
        },
        branches: Vec::new(),
        default: Vec::new(),
    });
    doc.conditional_group_order.push(id);
    id
}

/// Adds a new branch to `conditional` matching `values`, with no
/// relationship groups enabled yet.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Precondition: `values.len()` matches `conditional`'s condition arity
///   (the number of cells in [`ConditionExpr::Cells`] or
///   [`ConditionExpr::Formula`]'s `referenced_cells`).
/// - Postcondition: returns the new branch's index within
///   `conditional.branches`.
pub fn add_branch(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    values: Vec<CellValueLiteral>,
) -> usize {
    let group = &mut doc.conditional_groups[conditional];
    let arity = match &group.condition {
        ConditionExpr::Cells(cells) => cells.len(),
        ConditionExpr::Formula {
            referenced_cells, ..
        } => referenced_cells.len(),
    };
    debug_assert_eq!(values.len(), arity, "values.len() must match condition arity");
    group.branches.push(ConditionalBranch {
        values,
        enabled_groups: Vec::new(),
    });
    group.branches.len() - 1
}

/// Toggles whether `group` is active on `conditional`'s branch at
/// `branch_index` — enables it if absent, disables it if present.
///
/// - Precondition: `conditional` is a valid key in `doc.conditional_groups`.
/// - Precondition: `branch_index < conditional.branches.len()`.
pub fn toggle_enabled_group(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    branch_index: usize,
    group: RelationshipGroupId,
) {
    let branch = &mut doc.conditional_groups[conditional].branches[branch_index];
    if let Some(pos) = branch.enabled_groups.iter().position(|g| *g == group) {
        branch.enabled_groups.remove(pos);
    } else {
        branch.enabled_groups.push(group);
    }
}

#[cfg(test)]
mod formula_tests {
    use super::*;
    use crate::model::geometry::Point;
    use crate::ops::cells::add_cell;

    #[test]
    fn add_conditional_with_formula_starts_with_no_branches() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond =
            add_conditional_with_formula(&mut doc, vec![x], "aspect_ratio > 2.0", Point::new(0.0, 0.0));
        assert!(doc.conditional_groups[cond].branches.is_empty());
        assert!(doc.conditional_groups[cond].default.is_empty());
    }

    #[test]
    fn add_branch_appends_a_branch_with_no_enabled_groups() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond =
            add_conditional_with_formula(&mut doc, vec![x], "aspect_ratio > 2.0", Point::new(0.0, 0.0));
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        assert_eq!(doc.conditional_groups[cond].branches.len(), 1);
        assert!(doc.conditional_groups[cond].branches[0]
            .enabled_groups
            .is_empty());
    }

    #[test]
    fn add_branch_returns_the_new_branchs_index() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond =
            add_conditional_with_formula(&mut doc, vec![x], "aspect_ratio > 2.0", Point::new(0.0, 0.0));
        let i0 = add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        let i1 = add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(false)]);
        assert_eq!(i0, 0);
        assert_eq!(i1, 1);
    }

    #[test]
    fn toggle_enabled_group_enables_then_disables() {
        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let cond =
            add_conditional_with_formula(&mut doc, vec![x], "aspect_ratio > 2.0", Point::new(0.0, 0.0));
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);

        let mut groups: slotmap::SlotMap<RelationshipGroupId, ()> = slotmap::SlotMap::with_key();
        let group = groups.insert(());

        toggle_enabled_group(&mut doc, cond, 0, group);
        assert_eq!(doc.conditional_groups[cond].branches[0].enabled_groups, vec![group]);

        toggle_enabled_group(&mut doc, cond, 0, group);
        assert!(doc.conditional_groups[cond].branches[0]
            .enabled_groups
            .is_empty());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p ez-adam --lib ops::conditionals`
Expected: 8 passed (4 from Task 11 + 4 new).

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add ez-adam/src/ops/conditionals.rs
git commit -m "feat(ez-adam): add formula-mode conditional ops"
```

---

### Task 13: `validation::validate_cel_expression`

**Files:**
- Create: `ez-adam/src/validation.rs`
- Modify: `ez-adam/src/lib.rs`

**Interfaces:**
- Produces: `validation::validate_cel_expression(text: &str) -> Result<(), cel_parser::ParseError>`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/validation.rs`:

```rust
//! Validation of user-entered CEL expression text (relationship-group
//! formulas, restrict expressions) against `cel-parser`'s grammar.

use cel_parser::{AstContext, OpLookup, ParseError, Parser};

/// Checks that `text` is a syntactically valid CEL expression, with
/// `cel-std`'s functions (e.g. `clamp`, `min`, `max`) in scope.
///
/// # Errors
///
/// Returns the underlying [`ParseError`] if `text` does not parse as a CEL
/// expression.
pub fn validate_cel_expression(text: &str) -> Result<(), ParseError> {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = Parser::<AstContext>::new(lookup);
    parser.parse_str_ast(text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_simple_arithmetic_expression() {
        assert!(validate_cel_expression("width_pixels / height_pixels").is_ok());
    }

    #[test]
    fn accepts_a_call_to_a_cel_std_function() {
        assert!(validate_cel_expression("clamp(width_pixels, 0, 100)").is_ok());
    }

    #[test]
    fn rejects_malformed_syntax() {
        assert!(validate_cel_expression("width_pixels / ").is_err());
    }

    #[test]
    fn rejects_an_empty_string() {
        assert!(validate_cel_expression("").is_err());
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/lib.rs`:

```rust
pub mod validation;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib validation::tests`
Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/lib.rs ez-adam/src/validation.rs
git commit -m "feat(ez-adam): add CEL expression validation"
```

---

### Task 14: `codegen` — cells and top-level relationships

**Files:**
- Create: `ez-adam/src/codegen/mod.rs`
- Modify: `ez-adam/src/lib.rs`

**Interfaces:**
- Consumes: `Document` (Task 7), `Cell`/`CellType` (Task 3), `RelationshipGroup` (Task 5).
- Produces: `codegen::generate_adm2(doc: &Document) -> String`, covering cell declarations (no filters yet) and every relationship group not owned by a conditional group.

A relationship group is "top-level" if no conditional group's `default` or any branch's `enabled_groups` references it — conditional-group-owned groups are emitted later, nested inside their `conditional` block (Task 17).

- [ ] **Step 1: Write the failing test**

Create `ez-adam/src/codegen/mod.rs`:

```rust
//! Generates `.adm2` source text from a
//! [`crate::model::document::Document`].
//!
//! Generation is one-way: `.adm2` output is never parsed back into a
//! `Document`. See `docs/superpowers/specs/2026-08-24-ez-adam-design.md`.

use std::collections::HashSet;

use crate::model::cell::{Cell, CellType};
use crate::model::document::Document;
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// Returns `.adm2` source text for `doc`.
///
/// - Complexity: O(n) in the total number of cells, relationship groups,
///   and conditional-group branches.
#[must_use]
pub fn generate_adm2(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str(&format!("sheet {} {{\n", doc.sheet_name));

    for (_, cell) in doc.cells_in_order() {
        out.push_str("    ");
        out.push_str(&generate_cell_decl(cell));
        out.push('\n');
    }

    let owned = groups_owned_by_conditionals(doc);
    for (id, group) in doc.relationship_groups_in_order() {
        if owned.contains(&id) {
            continue;
        }
        out.push_str("    ");
        out.push_str(&generate_relationship_block(doc, group, "    "));
        out.push('\n');
    }

    out.push_str("}\n");
    out
}

fn groups_owned_by_conditionals(doc: &Document) -> HashSet<RelationshipGroupId> {
    let mut owned = HashSet::new();
    for (_, cond) in doc.conditional_groups_in_order() {
        owned.extend(cond.default.iter().copied());
        for branch in &cond.branches {
            owned.extend(branch.enabled_groups.iter().copied());
        }
    }
    owned
}

fn type_name(ty: &CellType) -> &'static str {
    match ty {
        CellType::F64 { .. } => "f64",
        CellType::I64 { .. } => "i64",
        CellType::Bool => "bool",
        CellType::Text => "String",
    }
}

fn generate_cell_decl(cell: &Cell) -> String {
    format!("cell {}: {};", cell.name, type_name(&cell.ty))
}

/// Renders `group` as a `relationship { ... }` block, with `indent` as the
/// prefix for its opening/closing braces (member lines are indented one
/// level deeper than `indent`). Takes an explicit `indent` rather than a
/// fixed one so the same function produces correctly nested output whether
/// called at the top level ([`generate_adm2`]) or inside a `conditional`
/// branch ([`generate_conditional_block`], Task 17).
fn generate_relationship_block(doc: &Document, group: &RelationshipGroup, indent: &str) -> String {
    let mut s = String::from("relationship {\n");
    let member_indent = format!("{indent}    ");
    for (node, formula) in &group.members {
        let cell = &doc.cells[doc.cell_nodes[*node].cell];
        s.push_str(&format!("{member_indent}{} := {};\n", cell.name, formula));
    }
    s.push_str(indent);
    s.push('}');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    fn generates_bare_cell_declarations() {
        let mut doc = Document::new("demo");
        add_cell(&mut doc, "width_pixels", CellType::i64());
        add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell aspect_ratio: f64;\n}\n"
        );
    }

    #[test]
    fn generates_a_top_level_relationship_block() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    relationship {\n        width_pixels := height_pixels * 2;\n        height_pixels := ;\n    }\n}\n"
        );
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/lib.rs`:

```rust
pub mod codegen;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib codegen::tests`
Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/lib.rs ez-adam/src/codegen/mod.rs
git commit -m "feat(ez-adam): generate cell decls and top-level relationships"
```

---

### Task 15: `codegen` — output declarations

**Files:**
- Modify: `ez-adam/src/codegen/mod.rs`

**Interfaces:**
- Extends `generate_adm2` to emit `out <name> := <name>;` for every cell with `output == true`.

- [ ] **Step 1: Write the failing test**

Add to `ez-adam/src/codegen/mod.rs`'s `tests` module:

```rust
    #[test]
    fn generates_an_out_decl_for_an_output_cell() {
        use crate::ops::cells::set_output;

        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "width_pixels", CellType::i64());
        set_output(&mut doc, cell, true);

        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64;\n    out width_pixels := width_pixels;\n}\n"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p ez-adam --lib codegen::tests::generates_an_out_decl_for_an_output_cell`
Expected: FAIL (no `out` line generated).

- [ ] **Step 3: Implement**

In `generate_adm2`, after the cell-declaration loop and before the relationship-group loop:

```rust
    for (_, cell) in doc.cells_in_order() {
        if cell.output {
            out.push_str(&format!("    out {name} := {name};\n", name = cell.name));
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ez-adam --lib codegen::tests`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/mod.rs
git commit -m "feat(ez-adam): generate out decls for output cells"
```

---

### Task 16: `codegen` — clamp filter clauses

**Files:**
- Modify: `ez-adam/src/codegen/mod.rs`

**Interfaces:**
- Extends `generate_cell_decl` to append a `filter |_: T| ...` clause for `F64`/`I64` cells with a clamp min and/or max set, using `cel-std`'s `clamp`/`min`/`max` functions.

- [ ] **Step 1: Write the failing tests**

Add to `ez-adam/src/codegen/mod.rs`'s `tests` module:

```rust
    #[test]
    fn generates_a_clamp_filter_with_both_bounds() {
        let mut doc = Document::new("demo");
        add_cell(
            &mut doc,
            "width_pixels",
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(0),
                    max: Some(100),
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter |_: i64| clamp(_, 0i64, 100i64);\n}\n"
        );
    }

    #[test]
    fn generates_a_clamp_filter_with_only_a_minimum() {
        let mut doc = Document::new("demo");
        add_cell(
            &mut doc,
            "width_pixels",
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(0),
                    max: None,
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: i64 filter |_: i64| max(_, 0i64);\n}\n"
        );
    }

    #[test]
    fn generates_a_clamp_filter_with_only_a_maximum() {
        let mut doc = Document::new("demo");
        add_cell(
            &mut doc,
            "width_pixels",
            CellType::F64 {
                clamp: crate::model::cell::ClampRange {
                    min: None,
                    max: Some(100.0),
                },
            },
        );
        let out = generate_adm2(&doc);
        assert_eq!(
            out,
            "sheet demo {\n    cell width_pixels: f64 filter |_: f64| min(_, 100.0);\n}\n"
        );
    }

    #[test]
    fn omits_the_filter_clause_when_no_clamp_bounds_are_set() {
        let mut doc = Document::new("demo");
        add_cell(&mut doc, "width_pixels", CellType::i64());
        let out = generate_adm2(&doc);
        assert_eq!(out, "sheet demo {\n    cell width_pixels: i64;\n}\n");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p ez-adam --lib codegen::tests::generates_a_clamp_filter_with_both_bounds`
Expected: FAIL (no filter clause generated).

- [ ] **Step 3: Implement**

Replace `generate_cell_decl` in `ez-adam/src/codegen/mod.rs` with:

```rust
fn generate_cell_decl(cell: &Cell) -> String {
    let ty = type_name(&cell.ty);
    match clamp_filter_clause(&cell.ty) {
        Some(filter) => format!("cell {}: {} {};", cell.name, ty, filter),
        None => format!("cell {}: {};", cell.name, ty),
    }
}

fn clamp_filter_clause(ty: &CellType) -> Option<String> {
    match ty {
        // `{:?}` (not `{}`) for f64 bounds: `f64::Display` drops the
        // trailing `.0` for whole numbers (`100.0` prints as `100`), which
        // risks the literal being lexed as an integer, not a float —
        // `f64::Debug` always includes a decimal point.
        CellType::F64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => None,
            (Some(min), None) => Some(format!("filter |_: f64| max(_, {min:?})")),
            (None, Some(max)) => Some(format!("filter |_: f64| min(_, {max:?})")),
            (Some(min), Some(max)) => Some(format!("filter |_: f64| clamp(_, {min:?}, {max:?})")),
        },
        // Explicit `i64` suffixes: bare integer literals are not
        // guaranteed to default to `i64` (the one confirmed example in
        // this codebase, `0i32`/`1i32` in `begin/examples/toy_example.adm2`,
        // suffixes every typed integer literal), so an unsuffixed `100`
        // risks a type mismatch against an `i64` filter parameter.
        CellType::I64 { clamp } => match (clamp.min, clamp.max) {
            (None, None) => None,
            (Some(min), None) => Some(format!("filter |_: i64| max(_, {min}i64)")),
            (None, Some(max)) => Some(format!("filter |_: i64| min(_, {max}i64)")),
            (Some(min), Some(max)) => {
                Some(format!("filter |_: i64| clamp(_, {min}i64, {max}i64)"))
            }
        },
        CellType::Bool | CellType::Text => None,
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ez-adam --lib codegen::tests`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/mod.rs
git commit -m "feat(ez-adam): generate clamp filter clauses"
```

---

### Task 17: `codegen` — conditional groups, plus `.adm2` round-trip validation

**Files:**
- Modify: `ez-adam/src/codegen/mod.rs`
- Create: `ez-adam/tests/adm2_round_trip.rs`

**Interfaces:**
- Extends `generate_adm2` to emit `conditional <expr> { <literal> => {...} ... _ => {...} }` for every conditional group.
- Adds an integration test parsing generated `.adm2` text via `adam_lang::AdamParser` to confirm it's syntactically valid.

- [ ] **Step 1: Write the failing test**

Add to `ez-adam/src/codegen/mod.rs`'s `tests` module:

```rust
    #[test]
    fn generates_a_conditional_group_with_bool_condition() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");
        add_conditional_from_bool_cells(&mut doc, vec![flag], group, Point::new(0.0, 20.0));

        let out = generate_adm2(&doc);
        // Branches are generated in the order `add_conditional_from_bool_cells`
        // enumerated them: `combo` counts up from 0, so the all-`false`
        // combination (combo == 0) comes first, `true` (combo == 1) second.
        assert_eq!(
            out,
            "sheet demo {\n    cell constrain_proportions: bool;\n    cell width_pixels: i64;\n    cell height_pixels: i64;\n    conditional constrain_proportions {\n        false => {\n        }\n        true => {\n            relationship {\n                width_pixels := height_pixels * 2;\n                height_pixels := ;\n            }\n        }\n        _ => {\n        }\n    }\n}\n"
        );
    }

    #[test]
    fn generates_a_conditional_group_with_a_multi_cell_tuple_condition() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag_a = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
        let flag_b = add_cell(&mut doc, "lock_aspect", CellType::Bool);
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        add_conditional_from_bool_cells(
            &mut doc,
            vec![flag_a, flag_b],
            group,
            Point::new(0.0, 20.0),
        );

        let out = generate_adm2(&doc);
        assert!(out.contains("conditional (constrain_proportions, lock_aspect) {\n"));
        // combo 0..4: (false,false), (true,false), (false,true), (true,true) —
        // bit i of combo selects cells[i]'s value.
        assert!(out.contains("        (false, false) => {\n        }\n"));
        assert!(out.contains("        (true, false) => {\n        }\n"));
        assert!(out.contains("        (false, true) => {\n        }\n"));
        assert!(out.contains("        (true, true) => {\n            relationship {\n"));
    }

    #[test]
    fn generates_a_conditional_group_with_a_formula_condition() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::conditionals::{add_branch, add_conditional_with_formula, toggle_enabled_group};

        let mut doc = Document::new("demo");
        let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let cond = add_conditional_with_formula(
            &mut doc,
            vec![aspect],
            "aspect_ratio > 2.0",
            Point::new(0.0, 20.0),
        );
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        toggle_enabled_group(&mut doc, cond, 0, group);

        let out = generate_adm2(&doc);
        assert!(out.contains("conditional aspect_ratio > 2.0 {\n"));
        assert!(out.contains("        true => {\n            relationship {\n"));
        assert!(out.contains("        _ => {\n        }\n"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p ez-adam --lib codegen::tests::generates_a_conditional_group_with_bool_condition`
Expected: FAIL (conditional groups aren't generated yet; also the wrapped `group` is currently still emitted as a stray top-level block, since Task 14 already excludes conditional-owned groups from top-level — confirm the failure is specifically "no `conditional` block in output").

- [ ] **Step 3: Implement**

Add to `ez-adam/src/codegen/mod.rs`, and call it from `generate_adm2` after the top-level relationship-group loop:

```rust
    for (_, cond) in doc.conditional_groups_in_order() {
        out.push_str("    ");
        out.push_str(&generate_conditional_block(doc, cond));
        out.push('\n');
    }
```

```rust
use crate::model::conditional_group::{CellValueLiteral, ConditionExpr, ConditionalGroup};

fn generate_conditional_block(doc: &Document, cond: &ConditionalGroup) -> String {
    let mut s = String::from("conditional ");
    s.push_str(&condition_expr_text(doc, &cond.condition));
    s.push_str(" {\n");
    for branch in &cond.branches {
        s.push_str("        ");
        s.push_str(&branch_literal_text(&branch.values));
        s.push_str(" => {\n");
        for &group_id in &branch.enabled_groups {
            s.push_str("            ");
            s.push_str(&generate_relationship_block(
                doc,
                &doc.relationship_groups[group_id],
                "            ",
            ));
            s.push('\n');
        }
        s.push_str("        }\n");
    }
    s.push_str("        _ => {\n");
    for &group_id in &cond.default {
        s.push_str("            ");
        s.push_str(&generate_relationship_block(
            doc,
            &doc.relationship_groups[group_id],
            "            ",
        ));
        s.push('\n');
    }
    s.push_str("        }\n");
    s.push_str("    }");
    s
}

fn condition_expr_text(doc: &Document, condition: &ConditionExpr) -> String {
    match condition {
        ConditionExpr::Cells(cells) => cell_names_text(doc, cells),
        ConditionExpr::Formula { expr, .. } => expr.clone(),
    }
}

fn cell_names_text(doc: &Document, cells: &[crate::model::cell::CellId]) -> String {
    let names: Vec<&str> = cells.iter().map(|c| doc.cells[*c].name.as_str()).collect();
    if names.len() == 1 {
        names[0].to_string()
    } else {
        format!("({})", names.join(", "))
    }
}

fn branch_literal_text(values: &[CellValueLiteral]) -> String {
    let literals: Vec<String> = values.iter().map(literal_text).collect();
    if literals.len() == 1 {
        literals[0].clone()
    } else {
        format!("({})", literals.join(", "))
    }
}

fn literal_text(value: &CellValueLiteral) -> String {
    match value {
        CellValueLiteral::Bool(b) => b.to_string(),
        CellValueLiteral::I64(n) => format!("{n}i64"),
        CellValueLiteral::Text(s) => format!("{s:?}"),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p ez-adam --lib codegen::tests`
Expected: 10 passed (7 from Task 16 + 3 new: bool condition, multi-cell tuple condition, formula condition).

- [ ] **Step 5: Write the round-trip integration test**

Create `ez-adam/tests/adm2_round_trip.rs`:

```rust
//! Confirms `generate_adm2`'s output is syntactically valid `.adm2` source,
//! for every construct it can emit.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use ez_adam::codegen::generate_adm2;
use ez_adam::model::cell::CellType;
use ez_adam::model::document::Document;
use ez_adam::model::geometry::Point;
use ez_adam::ops::cells::{add_cell, add_cell_node, set_output};
use ez_adam::ops::conditionals::add_conditional_from_bool_cells;
use ez_adam::ops::relationships::{create_relationship, set_member_formula};

fn assert_parses(adm2_text: &str) {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = AdamParser::new(TypeRegistry::new(), lookup);
    let result = parser.parse_str(adm2_text);
    assert!(result.is_ok(), "failed to parse:\n{adm2_text}\n\nerror: {:?}", result.err());
}

#[test]
fn a_document_with_every_construct_generates_valid_adm2() {
    let mut doc = Document::new("resize");

    let width = add_cell(
        &mut doc,
        "width_pixels",
        CellType::I64 {
            clamp: ez_adam::model::cell::ClampRange {
                min: Some(0),
                max: Some(4096),
            },
        },
    );
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
    set_output(&mut doc, width, true);

    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 0.0));
    let aspect_node = add_cell_node(&mut doc, aspect, Point::new(20.0, 0.0));

    let r1 = create_relationship(&mut doc, width_node, height_node, Point::new(5.0, 5.0));
    ez_adam::ops::relationships::add_member(&mut doc, r1, aspect_node);
    // Every member needs a non-empty formula: an empty RHS (the sketch's
    // "[ ]" placeholder for an as-yet-unfilled-in formula) is valid
    // intermediate editor state, but isn't valid CEL syntax, so it can't
    // appear in text this test actually parses.
    set_member_formula(&mut doc, r1, width_node, "aspect_ratio * height_pixels");
    set_member_formula(&mut doc, r1, height_node, "aspect_ratio / width_pixels");
    set_member_formula(&mut doc, r1, aspect_node, "width_pixels / height_pixels");

    let flag = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    add_conditional_from_bool_cells(&mut doc, vec![flag], r1, Point::new(0.0, 40.0));

    let adm2_text = generate_adm2(&doc);
    assert_parses(&adm2_text);
}

#[test]
fn a_bare_cell_only_document_generates_valid_adm2() {
    let mut doc = Document::new("empty_ish");
    add_cell(&mut doc, "a", CellType::f64());
    let adm2_text = generate_adm2(&doc);
    assert_parses(&adm2_text);
}
```

- [ ] **Step 6: Run the integration test**

Run: `cargo test -p ez-adam --test adm2_round_trip`
Expected: 2 passed.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add ez-adam/src/codegen/mod.rs ez-adam/tests/adm2_round_trip.rs
git commit -m "feat(ez-adam): generate conditional groups; add .adm2 round-trip test"
```

---

### Task 18: `persistence` — JSON save/load

**Files:**
- Create: `ez-adam/src/persistence.rs`
- Modify: `ez-adam/src/lib.rs`

**Interfaces:**
- Produces: `persistence::to_json(doc: &Document) -> String`, `persistence::from_json(text: &str) -> Result<Document, serde_json::Error>`.

- [ ] **Step 1: Write the failing tests**

Create `ez-adam/src/persistence.rs`:

```rust
//! Save/load [`Document`]s as JSON — `ez-adam`'s native document format.
//! `.adm2` export ([`crate::codegen::generate_adm2`]) is a separate,
//! one-way operation; it is never read back in.

use crate::model::document::Document;

/// Serializes `doc` to pretty-printed JSON.
#[must_use]
pub fn to_json(doc: &Document) -> String {
    serde_json::to_string_pretty(doc).expect("Document always serializes")
}

/// Deserializes a `Document` from JSON text produced by [`to_json`].
///
/// # Errors
///
/// Returns an error if `text` is not valid JSON, or does not match
/// [`Document`]'s current shape. Only
/// [`crate::model::document::CURRENT_FORMAT_VERSION`] is currently
/// supported — no migration path exists yet for older versions.
pub fn from_json(text: &str) -> Result<Document, serde_json::Error> {
    serde_json::from_str(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::geometry::Point;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::{create_relationship, set_member_formula};

    #[test]
    fn round_trips_a_document_with_cells_and_a_relationship() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        set_member_formula(&mut doc, group, a_node, "height_pixels * 2");

        let json = to_json(&doc);
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }

    #[test]
    fn from_json_rejects_malformed_json() {
        assert!(from_json("not json").is_err());
    }

    #[test]
    fn round_trips_a_document_with_a_formula_conditional_and_a_text_cell() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::cells::set_restrict;
        use crate::ops::conditionals::{add_branch, add_conditional_with_formula, toggle_enabled_group};

        let mut doc = Document::new("demo");
        let label = add_cell(&mut doc, "label", CellType::Text);
        set_restrict(&mut doc, label, Some("_ != \"\"".to_string()));
        let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
        let a = add_cell(&mut doc, "width_pixels", CellType::i64());
        let b = add_cell(&mut doc, "height_pixels", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let cond = add_conditional_with_formula(
            &mut doc,
            vec![aspect],
            "aspect_ratio > 2.0",
            Point::new(0.0, 20.0),
        );
        add_branch(&mut doc, cond, vec![CellValueLiteral::Text("wide".to_string())]);
        toggle_enabled_group(&mut doc, cond, 0, group);

        let json = to_json(&doc);
        let back = from_json(&json).unwrap();
        assert_eq!(doc, back);
    }
}
```

- [ ] **Step 2: Wire up the module**

Add to `ez-adam/src/lib.rs`:

```rust
pub mod persistence;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p ez-adam --lib persistence::tests`
Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add ez-adam/src/lib.rs ez-adam/src/persistence.rs
git commit -m "feat(ez-adam): add JSON persistence"
```

---

### Task 19: End-to-end capstone test, and full workspace verification

**Files:**
- Create: `ez-adam/tests/end_to_end.rs`

**Interfaces:**
- Consumes: every public function from Tasks 1–18.
- Produces: no new production code — this task is verification only.

- [ ] **Step 1: Write the end-to-end test**

Create `ez-adam/tests/end_to_end.rs`, replicating the "Property Model Visualization" sketch's `width_pixels`/`height_pixels`/`aspect_ratio`/`constrain_proportions` example:

```rust
//! End-to-end capstone: build the sketch's own example document through
//! `ops`, persist and reload it, and confirm it still generates valid
//! `.adm2`.

use adam_lang::{AdamParser, TypeRegistry};
use cel_parser::OpLookup;
use ez_adam::codegen::generate_adm2;
use ez_adam::model::cell::CellType;
use ez_adam::model::document::Document;
use ez_adam::model::geometry::Point;
use ez_adam::ops::cells::{add_cell, add_cell_node, set_output};
use ez_adam::ops::conditionals::add_conditional_from_bool_cells;
use ez_adam::ops::relationships::{add_member, create_relationship, set_member_formula};
use ez_adam::persistence::{from_json, to_json};

fn parses_as_adm2(adm2_text: &str) -> bool {
    let mut lookup = OpLookup::new();
    cel_std::install(&mut lookup);
    let mut parser = AdamParser::new(TypeRegistry::new(), lookup);
    parser.parse_str(adm2_text).is_ok()
}

fn build_resize_sheet() -> Document {
    let mut doc = Document::new("resize");

    let width = add_cell(&mut doc, "width_pixels", CellType::i64());
    let height = add_cell(&mut doc, "height_pixels", CellType::i64());
    let aspect = add_cell(&mut doc, "aspect_ratio", CellType::f64());
    set_output(&mut doc, width, true);
    set_output(&mut doc, height, true);

    let width_node = add_cell_node(&mut doc, width, Point::new(0.0, 0.0));
    let aspect_node = add_cell_node(&mut doc, aspect, Point::new(20.0, 0.0));
    let r1 = create_relationship(&mut doc, width_node, aspect_node, Point::new(10.0, 0.0));
    let height_node = add_cell_node(&mut doc, height, Point::new(10.0, 10.0));
    add_member(&mut doc, r1, height_node);
    set_member_formula(&mut doc, r1, width_node, "aspect_ratio * height_pixels");
    set_member_formula(&mut doc, r1, height_node, "aspect_ratio / width_pixels");
    set_member_formula(&mut doc, r1, aspect_node, "width_pixels / height_pixels");

    let constrain = add_cell(&mut doc, "constrain_proportions", CellType::Bool);
    add_conditional_from_bool_cells(&mut doc, vec![constrain], r1, Point::new(0.0, 30.0));

    doc
}

#[test]
fn the_resize_sheet_generates_valid_adm2() {
    let doc = build_resize_sheet();
    let adm2_text = generate_adm2(&doc);
    assert!(parses_as_adm2(&adm2_text), "generated:\n{adm2_text}");
}

#[test]
fn the_resize_sheet_survives_a_save_and_load_round_trip() {
    let doc = build_resize_sheet();
    let reloaded = from_json(&to_json(&doc)).unwrap();
    assert_eq!(doc, reloaded);
    assert_eq!(generate_adm2(&doc), generate_adm2(&reloaded));
}
```

- [ ] **Step 2: Run the new tests**

Run: `cargo test -p ez-adam --test end_to_end`
Expected: 2 passed.

- [ ] **Step 3: Run the full check suite for the crate**

Run, in order:
```bash
cargo fmt --all -- --check
cargo build -p ez-adam
cargo test -p ez-adam
cargo test --doc -p ez-adam
cargo clippy -p ez-adam --all-targets -- -D warnings
```
Expected: all succeed with zero warnings.

- [ ] **Step 4: Run the full workspace check suite** (confirms `ez-adam`'s addition to `[workspace] members` didn't break anything else)

Run, in order:
```bash
cargo build --workspace --exclude begin
cargo test --workspace --exclude begin
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
```
Expected: all succeed with zero warnings.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add ez-adam/tests/end_to_end.rs
git commit -m "test(ez-adam): add end-to-end capstone test for Phase 1"
```

---

## Deferred to the follow-up UI plan

- The Dioxus desktop app shell, canvas rendering, toolbar, and side panel (design spec §4).
- `rfd`-based native open/save dialogs wired to `persistence::to_json`/`from_json` and an `.adm2` export action wired to `codegen::generate_adm2`.
- Live diagnostics rendering for `validation::validate_cel_expression`'s `Err` case via `annotate-snippets`, matching `begin`'s `SourcePanel`.

## Deferred pending upstream work

- `Cell.restrict` codegen — blocked on <https://github.com/stlab/cel-rs/issues/146> (adam-lang has no boolean-rejecting cell filter syntax yet).
