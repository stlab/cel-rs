# Conditionals Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render property-model conditionals in the `begin` D3 force graph as diamond-shaped nodes with dashed control lines, and add a two-branch conditional to the demo sheet.

**Architecture:** Four layers of change from the bottom up — Sheet public API, bridge serialization, demo sheet, JavaScript rendering. Each layer is independently testable. The demo sheet wires a `p` cell (i32) to a conditional that links the `a×b=c` and `d×e=f` constraint systems. JavaScript renders the conditional as a rotated diamond with color-coded dashed control lines to each branch's relationships; inactive branch relationships are dimmed.

**Tech Stack:** Rust (property-model crate, begin crate), Serde JSON, Dioxus, D3 v7, plain JavaScript, CSS.

## Global Constraints

- All Rust code must pass `cargo clippy --workspace --exclude begin -- -D warnings`.
- Every public Rust function must have a `///` doc comment in contract style.
- Preconditions go in `/// - Precondition:` bullets; complexity goes in `/// - Complexity:` bullet if not O(1).
- Tests derive from the public contract only — no implementation-reading.
- `cargo fmt --all` must pass before any commit.
- JavaScript is plain ES5-compatible (no `let`/`const`/arrow-in-var, match the existing `graph.js` style). Arrow functions inside D3 callbacks are OK where they already appear.
- Working directory for all commands: `d:\repos\github.com\stlab\cel-rs\.claude\worktrees\conditionals-display`

---

## File Map

| File | Change |
|------|--------|
| `property-model/src/sheet.rs` | Add 6 public accessor methods before `propagate_without_replan` |
| `begin/src/bridge.rs` | New `NodeKind::Conditional`, `LinkKind` enum, updated `LinkData`, remove `GroupData`/`groups`, update `to_graph_data` |
| `begin/src/app.rs` | Replace `make_demo_sheet` with `a×b=c`, `d×e=f`, conditional on `p` |
| `begin/assets/graph.js` | New constants, layers, conditional node, control link, dimming logic |
| `begin/assets/graph.css` | `.node-conditional`, `.link-control` |

---

## Task 1: Sheet Accessor Methods

**Files:**
- Modify: `property-model/src/sheet.rs` (insert before `propagate_without_replan` at line ~669, add tests in the `#[cfg(test)]` block)

**Interfaces:**
- Consumes: existing `Sheet`, `ConditionalData`, `Branch` internals
- Produces:
  - `Sheet::conditionals(&self) -> impl Iterator<Item = ConditionalId> + '_`
  - `Sheet::conditional_match_cell(&self, id: ConditionalId) -> Option<CellId>`
  - `Sheet::conditional_branch_count(&self, id: ConditionalId) -> Option<usize>`
  - `Sheet::conditional_branch_relationships(&self, id: ConditionalId, branch: usize) -> Option<&[RelationshipId]>`
  - `Sheet::conditional_default_relationships(&self, id: ConditionalId) -> Option<&[RelationshipId]>`
  - `Sheet::conditional_active_branch(&self, id: ConditionalId) -> Option<usize>`

- [ ] **Step 1: Write the failing tests**

Add to `property-model/src/sheet.rs` inside the `#[cfg(test)]` block (after the last test):

```rust
    // ── Conditional accessor tests ─────────────────────────────────────────

    fn sheet_with_two_branch_conditional() -> (Sheet, ConditionalId) {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(0_i32);

        let rel0 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &i32| Ok(*v))])
            .unwrap();
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, a, |v: &i32| Ok(*v))])
            .unwrap();

        let cid = sheet
            .add_conditional(
                p,
                vec![(vec![0_i32], vec![rel0]), (vec![1_i32], vec![rel1])],
                vec![],
            )
            .unwrap();
        (sheet, cid)
    }

    fn sheet_with_default_conditional() -> (Sheet, ConditionalId, RelationshipId) {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let p = sheet.add_cell(99_i32); // no branch matches → default

        let rel_default = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &i32| Ok(*v))])
            .unwrap();

        let cid = sheet
            .add_conditional::<i32>(p, vec![], vec![rel_default])
            .unwrap();
        (sheet, cid, rel_default)
    }

    #[test]
    fn conditionals_returns_registered_id() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert!(sheet.conditionals().any(|id| id == cid));
    }

    #[test]
    fn conditionals_empty_on_new_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditionals().count(), 0);
    }

    #[test]
    fn conditional_match_cell_returns_correct_cell() {
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let cid = sheet.add_conditional::<i32>(p, vec![], vec![]).unwrap();
        assert_eq!(sheet.conditional_match_cell(cid), Some(p));
    }

    #[test]
    fn conditional_match_cell_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditional_match_cell(ConditionalId::default()), None);
    }

    #[test]
    fn conditional_branch_count_returns_correct_count() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert_eq!(sheet.conditional_branch_count(cid), Some(2));
    }

    #[test]
    fn conditional_branch_count_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditional_branch_count(ConditionalId::default()), None);
    }

    #[test]
    fn conditional_branch_relationships_returns_correct_rels() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        let rels0 = sheet.conditional_branch_relationships(cid, 0).unwrap();
        let rels1 = sheet.conditional_branch_relationships(cid, 1).unwrap();
        assert_eq!(rels0.len(), 1);
        assert_eq!(rels1.len(), 1);
        assert_ne!(rels0[0], rels1[0]);
    }

    #[test]
    fn conditional_branch_relationships_returns_none_for_out_of_bounds() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert!(sheet.conditional_branch_relationships(cid, 2).is_none());
    }

    #[test]
    fn conditional_branch_relationships_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(
            sheet
                .conditional_branch_relationships(ConditionalId::default(), 0)
                .is_none()
        );
    }

    #[test]
    fn conditional_default_relationships_returns_correct_rels() {
        let (sheet, cid, rel_default) = sheet_with_default_conditional();
        let rels = sheet.conditional_default_relationships(cid).unwrap();
        assert_eq!(rels, [rel_default]);
    }

    #[test]
    fn conditional_default_relationships_empty_when_no_default() {
        let (sheet, cid) = sheet_with_two_branch_conditional();
        assert_eq!(
            sheet.conditional_default_relationships(cid).unwrap(),
            &[]
        );
    }

    #[test]
    fn conditional_default_relationships_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(
            sheet
                .conditional_default_relationships(ConditionalId::default())
                .is_none()
        );
    }

    #[test]
    fn conditional_active_branch_returns_matching_branch_index() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cell(cid).unwrap();
        sheet.write(p, 0_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), Some(0));
        sheet.write(p, 1_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), Some(1));
    }

    #[test]
    fn conditional_active_branch_returns_none_when_no_branch_matches() {
        let (mut sheet, cid) = sheet_with_two_branch_conditional();
        let p = sheet.conditional_match_cell(cid).unwrap();
        sheet.write(p, 99_i32).unwrap();
        assert_eq!(sheet.conditional_active_branch(cid), None);
    }

    #[test]
    fn conditional_active_branch_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert_eq!(sheet.conditional_active_branch(ConditionalId::default()), None);
    }
```

The test file already imports `crate::{Error, Method, Sheet, cell::CellId, relationship::RelationshipId}`. Add `ConditionalId` to this import line:

```rust
use crate::{ConditionalId, Error, Method, Sheet, cell::CellId, relationship::RelationshipId};
```

- [ ] **Step 2: Run tests to confirm they fail**

```
cargo test --workspace conditional_active_branch
```

Expected: compile errors — `Sheet` has no method `conditionals`, `conditional_match_cell`, etc.

- [ ] **Step 3: Implement the 6 accessor methods**

Insert the following block in `property-model/src/sheet.rs` immediately before the `propagate_without_replan` method (before the `///` doc comment at line ~669):

```rust
    /// Iterates all live conditional IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of conditionals.
    pub fn conditionals(&self) -> impl Iterator<Item = ConditionalId> + '_ {
        self.conditionals.keys()
    }

    /// Returns the match cell for conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_match_cell(&self, id: ConditionalId) -> Option<CellId> {
        self.conditionals.get(id).map(|c| c.cell)
    }

    /// Returns the number of named branches in conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_branch_count(&self, id: ConditionalId) -> Option<usize> {
        self.conditionals.get(id).map(|c| c.branches.len())
    }

    /// Returns the relationship IDs for branch `branch` of conditional `id`.
    ///
    /// Returns `None` if `id` is not a live conditional, or `branch` is out of bounds.
    pub fn conditional_branch_relationships(
        &self,
        id: ConditionalId,
        branch: usize,
    ) -> Option<&[RelationshipId]> {
        self.conditionals
            .get(id)?
            .branches
            .get(branch)
            .map(|b| b.relationships.as_slice())
    }

    /// Returns the default relationship IDs for conditional `id`.
    ///
    /// These relationships are active when no named branch key matches the match cell.
    /// Returns `None` if `id` is not a live conditional in this sheet.
    pub fn conditional_default_relationships(
        &self,
        id: ConditionalId,
    ) -> Option<&[RelationshipId]> {
        self.conditionals.get(id).map(|c| c.default.as_slice())
    }

    /// Returns the index of the currently matching branch for conditional `id`.
    ///
    /// Evaluates branch keys against the match cell's current value in definition order;
    /// returns the index of the first matching branch. Returns `None` if no branch key
    /// matches (the default branch is active) or if `id` is not a live conditional.
    ///
    /// - Complexity: O(B·K) where B = branches, K = keys per branch.
    pub fn conditional_active_branch(&self, id: ConditionalId) -> Option<usize> {
        let cond = self.conditionals.get(id)?;
        let cell = &self.cells[cond.cell];
        let eq_fn = cell.eq_fn;
        let value = cell.value.as_ref();
        cond.branches
            .iter()
            .enumerate()
            .find(|(_, branch)| branch.keys.iter().any(|key| eq_fn(value, key.as_ref())))
            .map(|(i, _)| i)
    }
```

- [ ] **Step 4: Run tests to confirm they pass**

```
cargo test --workspace conditionals
```

Expected: all new tests PASS. Zero pre-existing failures.

- [ ] **Step 5: Run clippy and format**

```
cargo fmt --all && cargo clippy --workspace --exclude begin -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 6: Commit**

```
git add property-model/src/sheet.rs
git commit -m "feat(property-model): add conditional accessor methods to Sheet"
```

---

## Task 2: Bridge Update

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes (from Task 1):
  - `Sheet::conditionals() -> impl Iterator<Item = ConditionalId>`
  - `Sheet::conditional_match_cell(ConditionalId) -> Option<CellId>`
  - `Sheet::conditional_branch_count(ConditionalId) -> Option<usize>`
  - `Sheet::conditional_branch_relationships(ConditionalId, usize) -> Option<&[RelationshipId]>`
  - `Sheet::conditional_default_relationships(ConditionalId) -> Option<&[RelationshipId]>`
  - `Sheet::conditional_active_branch(ConditionalId) -> Option<usize>`
- Produces:
  - `NodeKind::Conditional` — new variant, serializes as `"Conditional"`
  - `LinkKind` — enum with `Constraint` and `Control` variants
  - `LinkData { source, target, kind: LinkKind, branch_index: Option<usize>, branch_active: Option<bool> }`
  - `GraphData` without `groups` field
  - `to_graph_data(sheet, labels) -> GraphData` — emits conditional nodes, constraint link from match cell to conditional node, and control links from conditional node to branch/default relationships

- [ ] **Step 1: Write the failing tests**

Add a test helper and new tests inside the `#[cfg(test)]` block at the bottom of `begin/src/bridge.rs`:

```rust
    fn sheet_with_conditional() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        // Two-method relationship placed in branch 0.
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |v: &f64| Ok(*v)),
                Method::from_fn_1_1(b, a, |v: &f64| Ok(*v)),
            ])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_emits_conditional_node() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.nodes.iter().any(|n| n.kind == NodeKind::Conditional),
            "expected a Conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_constraint_link_from_match_cell_to_conditional() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Constraint) && l.target == cond_id),
            "expected a Constraint link targeting the conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_control_link_for_branch_relationship() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)),
            "expected at least one Control link"
        );
    }

    #[test]
    fn to_graph_data_active_branch_control_link_is_active() {
        // p = 0: branch 0 matches → branch_active should be true
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let active_control = data
            .links
            .iter()
            .find(|l| matches!(l.kind, LinkKind::Control) && l.branch_index == Some(0));
        assert!(active_control.is_some(), "expected a Control link for branch 0");
        assert_eq!(active_control.unwrap().branch_active, Some(true));
    }

    #[test]
    fn to_graph_data_no_groups_field() {
        // GraphData must not have a `groups` field — verified by the struct definition.
        // This test exists to document the removal of GroupData.
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        // Serialize and confirm "groups" key is absent.
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("\"groups\""), "GraphData must not contain groups");
    }
```

- [ ] **Step 2: Run tests to confirm they fail**

```
cargo test -p begin to_graph_data_emits_conditional_node
```

Expected: compile errors — `NodeKind::Conditional` and `LinkKind` do not exist.

- [ ] **Step 3: Implement the bridge changes**

Replace the contents of `begin/src/bridge.rs` with the following (the `Labels` / `CellMeta` / `WriteStrFn` top section is unchanged; paste it in full):

```rust
//! Serialization bridge from [`property_model::Sheet`] to D3-ready JSON.
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] and [`RelationshipId`] keys. [`to_graph_data`] serializes a
//! [`Sheet`] and its [`Labels`] into a [`GraphData`] value ready for JSON encoding.

use indexmap::IndexMap;
use property_model::{CellId, ConditionalId, Error, RelationshipId, Sheet};
use serde::Serialize;
use slotmap::Key;

/// Type-erased write closure: parses a string and writes it to a cell.
pub type WriteStrFn = Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>;

/// Display and write metadata for a single cell.
pub struct CellMeta {
    /// Human-readable cell name shown in the graph and inspector.
    pub label: String,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type mismatch.
    pub write_str: WriteStrFn,
}

/// Associates human-readable labels and type-erased closures with stable sheet IDs.
pub struct Labels {
    /// Cells in insertion order (preserves sidebar ordering).
    pub cells: IndexMap<CellId, CellMeta>,
}

impl Labels {
    /// Creates an empty label set.
    pub fn new() -> Self {
        Self {
            cells: IndexMap::new(),
        }
    }

    /// Registers display metadata for a cell of type `T`.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with.
    /// - Precondition: `T` matches the type registered with `Sheet::add_cell` for `id`.
    pub fn add_cell<T>(&mut self, id: CellId, label: &str)
    where
        T: std::any::Any + std::fmt::Display + std::str::FromStr + 'static,
        T::Err: std::fmt::Display,
    {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                display: Box::new(move |sheet| {
                    sheet
                        .read::<T>(id)
                        .map(|v| format!("{}", v))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(move |sheet, s| {
                    let value = s
                        .parse::<T>()
                        .map_err(|e| Error::MethodFailed(anyhow::anyhow!("parse error: {}", e)))?;
                    sheet.write(id, value)
                }),
            },
        );
    }
}

impl Default for Labels {
    /// Returns `Labels::new()`.
    fn default() -> Self {
        Self::new()
    }
}

/// Node kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A value cell — rendered as a `<rect>`.
    Cell,
    /// A multi-way constraint — rendered as a `<circle>`.
    Relationship,
    /// A conditional switch — rendered as a diamond (rotated `<rect>`).
    Conditional,
}

/// A single node in the D3 graph.
#[derive(Serialize, Clone, PartialEq)]
pub struct NodeData {
    /// Stable string ID: `"c{ffi}"` for cells, `"r{ffi}"` for relationships, `"cond{ffi}"` for conditionals.
    pub id: String,
    pub kind: NodeKind,
    /// Cell label (e.g. `"a"`); empty string for relationships and conditionals.
    pub label: String,
    /// Current cell value as a display string; empty string for relationships and conditionals.
    pub value: String,
}

/// Link kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// A regular constraint edge (cell ↔ relationship, or match cell → conditional node).
    Constraint,
    /// A control edge from a conditional node to a branch relationship.
    Control,
}

/// A single edge in the D3 graph.
///
/// When [`GraphData::arrows`] is `false` constraint edges are undirected; when `true`
/// they are directed from `source` to `target`. Control edges are always directed
/// (conditional node → relationship) and styled by `branch_index` and `branch_active`.
#[derive(Serialize, Clone, PartialEq)]
pub struct LinkData {
    pub source: String,
    pub target: String,
    pub kind: LinkKind,
    /// Branch index for `Control` links; `None` for `Constraint` links and default-branch control links.
    pub branch_index: Option<usize>,
    /// `true` if this branch is currently active; `None` for `Constraint` links.
    pub branch_active: Option<bool>,
}

/// Complete graph snapshot ready for JSON serialization and delivery to D3.
#[derive(Serialize, Clone, PartialEq)]
pub struct GraphData {
    pub nodes: Vec<NodeData>,
    pub links: Vec<LinkData>,
    /// Stable IDs of cells that changed during the last `propagate()` call.
    pub changed: Vec<String>,
    /// `true` when at least one relationship has a cached plan and constraint links are directed
    /// where plans exist; `false` when no plan has been computed.
    pub arrows: bool,
}

fn cell_node_id(id: CellId) -> String {
    format!("c{}", id.data().as_ffi())
}

fn rel_node_id(id: RelationshipId) -> String {
    format!("r{}", id.data().as_ffi())
}

fn cond_node_id(id: ConditionalId) -> String {
    format!("cond{}", id.data().as_ffi())
}

/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// Constraint links: when a plan is cached (`sheet.selected_method` returns `Some`) links are
/// directed (inputs → relationship → outputs) and [`GraphData::arrows`] is `true`. Otherwise
/// all cells adjacent to the relationship are emitted as undirected source→relationship edges.
///
/// Conditional nodes: for each conditional, emits one `Conditional` node, one `Constraint` link
/// from the match cell to the conditional node, and one `Control` link per relationship in each
/// branch/default. Control links carry `branch_index` and `branch_active` for rendering.
///
/// - Complexity: O(c + r + e + cond·b·k) where c = cells, r = relationships, e = adjacency pairs,
///   cond = conditionals, b = branches per conditional, k = keys per branch.
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let mut arrows = false;

    // Cell nodes
    for id in sheet.cells() {
        let (label, value) = labels
            .cells
            .get(&id)
            .map(|m| (m.label.clone(), (m.display)(sheet)))
            .unwrap_or_default();
        nodes.push(NodeData {
            id: cell_node_id(id),
            kind: NodeKind::Cell,
            label,
            value,
        });
    }

    // Relationship nodes and constraint links
    for id in sheet.relationships() {
        nodes.push(NodeData {
            id: rel_node_id(id),
            kind: NodeKind::Relationship,
            label: String::new(),
            value: String::new(),
        });

        if let Some(method_idx) = sheet.selected_method(id) {
            arrows = true;
            if let Some(inputs) = sheet.method_inputs(id, method_idx) {
                for &cell_id in inputs {
                    links.push(LinkData {
                        source: cell_node_id(cell_id),
                        target: rel_node_id(id),
                        kind: LinkKind::Constraint,
                        branch_index: None,
                        branch_active: None,
                    });
                }
            }
            if let Some(outputs) = sheet.method_outputs(id, method_idx) {
                for &cell_id in outputs {
                    links.push(LinkData {
                        source: rel_node_id(id),
                        target: cell_node_id(cell_id),
                        kind: LinkKind::Constraint,
                        branch_index: None,
                        branch_active: None,
                    });
                }
            }
        } else if let Some(adj) = sheet.relationship_adj(id) {
            for &cell_id in adj {
                links.push(LinkData {
                    source: cell_node_id(cell_id),
                    target: rel_node_id(id),
                    kind: LinkKind::Constraint,
                    branch_index: None,
                    branch_active: None,
                });
            }
        }
    }

    // Conditional nodes and control links
    for cond_id in sheet.conditionals() {
        let node_id = cond_node_id(cond_id);
        nodes.push(NodeData {
            id: node_id.clone(),
            kind: NodeKind::Conditional,
            label: String::new(),
            value: String::new(),
        });

        // Constraint link: match cell → conditional node
        if let Some(match_cell) = sheet.conditional_match_cell(cond_id) {
            links.push(LinkData {
                source: cell_node_id(match_cell),
                target: node_id.clone(),
                kind: LinkKind::Constraint,
                branch_index: None,
                branch_active: None,
            });
        }

        let active_branch = sheet.conditional_active_branch(cond_id);

        // Control links for named branches
        let branch_count = sheet.conditional_branch_count(cond_id).unwrap_or(0);
        for branch in 0..branch_count {
            let is_active = active_branch == Some(branch);
            if let Some(rels) = sheet.conditional_branch_relationships(cond_id, branch) {
                for &rel_id in rels {
                    links.push(LinkData {
                        source: node_id.clone(),
                        target: rel_node_id(rel_id),
                        kind: LinkKind::Control,
                        branch_index: Some(branch),
                        branch_active: Some(is_active),
                    });
                }
            }
        }

        // Control links for default relationships
        let default_active = active_branch.is_none();
        if let Some(default_rels) = sheet.conditional_default_relationships(cond_id) {
            for &rel_id in default_rels {
                links.push(LinkData {
                    source: node_id.clone(),
                    target: rel_node_id(rel_id),
                    kind: LinkKind::Control,
                    branch_index: None,
                    branch_active: Some(default_active),
                });
            }
        }
    }

    let changed = sheet.changed().map(cell_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        arrows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use property_model::{Method, Sheet};

    fn demo_sheet() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(3.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");

        sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();

        (sheet, labels)
    }

    // Separate helper that adds the output cell first so propagation succeeds.
    fn demo_sheet_with_plan() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        // c added first → lowest strength (output by default).
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(3.0_f64);
        labels.add_cell::<f64>(b, "b");

        sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();

        (sheet, labels)
    }

    fn sheet_with_conditional() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |v: &f64| Ok(*v)),
                Method::from_fn_1_1(b, a, |v: &f64| Ok(*v)),
            ])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_produces_correct_node_counts() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert_eq!(
            data.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Cell)
                .count(),
            3
        );
        assert_eq!(
            data.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Relationship)
                .count(),
            1
        );
    }

    #[test]
    fn to_graph_data_produces_correct_link_count() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert_eq!(data.links.len(), 3);
    }

    #[test]
    fn to_graph_data_cell_nodes_have_labels() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        let cell_labels: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Cell)
            .map(|n| n.label.as_str())
            .collect();
        assert!(cell_labels.contains(&"a"));
        assert!(cell_labels.contains(&"b"));
        assert!(cell_labels.contains(&"c"));
    }

    #[test]
    fn to_graph_data_relationship_nodes_have_empty_labels() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        for node in data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
        {
            assert!(node.label.is_empty());
        }
    }

    #[test]
    fn to_graph_data_changed_contains_changed_cell_ids() {
        let (mut sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();
        sheet.write(a_id, 2.0_f64).unwrap();
        sheet.write(b_id, 3.0_f64).unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.changed.is_empty());
    }

    #[test]
    fn to_graph_data_arrows_false_before_propagate() {
        let (sheet, labels) = demo_sheet_with_plan();
        let data = to_graph_data(&sheet, &labels);
        assert!(!data.arrows);
    }

    #[test]
    fn to_graph_data_arrows_true_after_propagate() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);
        assert!(data.arrows);
    }

    #[test]
    fn to_graph_data_directed_input_links_target_relationship() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let rel_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .unwrap();

        let to_rel: Vec<_> = data
            .links
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::Constraint) && l.target == rel_id)
            .collect();
        assert_eq!(to_rel.len(), 2);
    }

    #[test]
    fn to_graph_data_directed_output_links_source_relationship() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let rel_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .unwrap();

        let from_rel: Vec<_> = data
            .links
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::Constraint) && l.source == rel_id)
            .collect();
        assert_eq!(from_rel.len(), 1);
    }

    #[test]
    fn display_closure_returns_value_string() {
        let (sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "2");
    }

    #[test]
    fn write_str_closure_parses_and_writes() {
        let (mut sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        assert!((&labels.cells[&a_id].write_str)(&mut sheet, "5.0").is_ok());
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "5");
    }

    #[test]
    fn to_graph_data_emits_conditional_node() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.nodes.iter().any(|n| n.kind == NodeKind::Conditional),
            "expected a Conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_constraint_link_from_match_cell_to_conditional() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Constraint) && l.target == cond_id),
            "expected a Constraint link targeting the conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_control_link_for_branch_relationship() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)),
            "expected at least one Control link"
        );
    }

    #[test]
    fn to_graph_data_active_branch_control_link_is_active() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let active_control = data
            .links
            .iter()
            .find(|l| matches!(l.kind, LinkKind::Control) && l.branch_index == Some(0));
        assert!(active_control.is_some(), "expected a Control link for branch 0");
        assert_eq!(active_control.unwrap().branch_active, Some(true));
    }

    #[test]
    fn to_graph_data_no_groups_field() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let json = serde_json::to_string(&data).unwrap();
        assert!(!json.contains("\"groups\""), "GraphData must not contain groups");
    }
}
```

- [ ] **Step 4: Run tests**

```
cargo test -p begin
```

Expected: all tests PASS. `to_graph_data_groups_is_empty` no longer exists (removed in the full rewrite above). All new tests PASS.

- [ ] **Step 5: Run clippy and format**

```
cargo fmt --all && cargo clippy --workspace --exclude begin -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 6: Commit**

```
git add begin/src/bridge.rs
git commit -m "feat(begin): add Conditional node and Control link to bridge"
```

---

## Task 3: Demo Sheet Update

**Files:**
- Modify: `begin/src/app.rs`

**Interfaces:**
- Consumes: existing `Sheet::add_relationship`, `Sheet::add_conditional`, `Method::from_fn_2_1`, `Method::from_fn_1_1`
- Produces: updated `make_demo_sheet` returning a sheet with cells `a`, `b`, `c`, `d`, `e`, `f`, `p` and a two-branch conditional

- [ ] **Step 1: Replace `make_demo_sheet` in `begin/src/app.rs`**

The function currently runs from line ~17 to ~57. Replace it entirely:

```rust
/// Builds the demo sheet.
///
/// Two independent bidirectional constraint systems (`a × b = c` and `d × e = f`)
/// are linked by a conditional on `p`:
/// - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
/// - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active.
/// - Any other `p`: the two systems are independent.
///
/// Cells `c` and `f` are added first (lowest strength) so they are natural outputs
/// within each system. `propagate()` is called once to compute initial values;
/// `clear_changed()` resets pulse state so no cells flash on startup.
pub fn make_demo_sheet() -> (Sheet, Labels) {
    let mut sheet = Sheet::new();
    let mut labels = Labels::new();

    // c and f added first → lowest strength (natural outputs).
    let c = sheet.add_cell(0.0_f64);
    let f = sheet.add_cell(0.0_f64);

    // a, b, d, e, p added later → higher strength (natural sources).
    let a = sheet.add_cell(2.0_f64);
    let b = sheet.add_cell(3.0_f64);
    let d = sheet.add_cell(4.0_f64);
    let e = sheet.add_cell(5.0_f64);
    let p = sheet.add_cell(0_i32);

    // a × b = c (three bidirectional methods)
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    // d × e = f (three bidirectional methods)
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([d, e], f, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([e, f], d, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([d, f], e, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    // Branch p=0: c = f (bidirectional)
    let rel_eq = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(f, c, |v: &f64| Ok(*v)),
            Method::from_fn_1_1(c, f, |v: &f64| Ok(*v)),
        ])
        .unwrap();

    // Branch p=1: c = f × 2  ↔  f = c / 2 (bidirectional)
    let rel_double = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(f, c, |v: &f64| Ok(v * 2.0)),
            Method::from_fn_1_1(c, f, |v: &f64| Ok(v / 2.0)),
        ])
        .unwrap();

    sheet
        .add_conditional(
            p,
            vec![
                (vec![0_i32], vec![rel_eq]),
                (vec![1_i32], vec![rel_double]),
            ],
            vec![],
        )
        .unwrap();

    sheet.propagate().unwrap();
    sheet.clear_changed();

    labels.add_cell::<f64>(a, "a");
    labels.add_cell::<f64>(b, "b");
    labels.add_cell::<f64>(c, "c");
    labels.add_cell::<f64>(d, "d");
    labels.add_cell::<f64>(e, "e");
    labels.add_cell::<f64>(f, "f");
    labels.add_cell::<i32>(p, "p");

    (sheet, labels)
}
```

Also update the import at the top of `app.rs` — `Method` is still used, no new imports needed since `add_conditional`'s return value (`ConditionalId`) is discarded via `.unwrap()`.

- [ ] **Step 2: Build to confirm it compiles**

```
cargo build -p begin
```

Expected: compiles without errors. (The `begin` crate uses `--no-default-features` for clippy; a plain build here is sufficient to verify correctness.)

- [ ] **Step 3: Run clippy**

```
cargo clippy -p begin --no-default-features -- -D warnings
```

Expected: no warnings.

- [ ] **Step 4: Commit**

```
git add begin/src/app.rs
git commit -m "feat(begin): update demo sheet with conditional on p linking c and f systems"
```

---

## Task 4: JavaScript and CSS Rendering

**Files:**
- Modify: `begin/assets/graph.js`
- Modify: `begin/assets/graph.css`

**Interfaces:**
- Consumes (from Task 2): JSON with `nodes[].kind === "Conditional"`, `links[].kind === "Constraint"|"Control"`, `links[].branch_index: number|null`, `links[].branch_active: boolean|null`
- Produces: rendered diamond nodes, dashed color-coded control lines, dimmed inactive relationship circles

There is no automated test framework for the JavaScript. Verification is manual: run the `begin` app and inspect the graph.

- [ ] **Step 1: Add CSS for new elements**

Append to `begin/assets/graph.css`:

```css
.node-conditional {
    fill: #fffff0;
    stroke: #555;
    stroke-width: 1.5;
    cursor: default;
}

.link-control {
    stroke-width: 1.5;
    fill: none;
}
```

- [ ] **Step 2: Replace `begin/assets/graph.js` with the updated version**

The full file (all changes annotated with `// NEW` or `// CHANGED`):

```javascript
(function () {
    // Tunable layout constants
    var LINK_DISTANCE = 80;
    var CHARGE_STRENGTH = -300;
    var CELL_W = 60;
    var CELL_H = 36;
    var CELL_RX = 4;
    var REL_R = 16;
    var COND_SIZE = 20;                                   // NEW: diamond half-width/height
    var CELL_COLLIDE_R = 38;
    var REL_COLLIDE_R = 22;
    var COND_COLLIDE_R = COND_SIZE * Math.SQRT2;          // NEW: diamond circumradius
    var PULSE_COLOR = '#f90';
    var PULSE_ON_MS = 200;
    var PULSE_OFF_MS = 400;
    var BRANCH_COLORS = ['#4a90d9', '#e67e22'];           // NEW: branch 0=blue, 1=orange
    var DEFAULT_BRANCH_COLOR = '#888';                    // NEW: default/no-branch control links
    var INACTIVE_OPACITY = 0.25;                          // NEW: dimmed inactive elements

    var svg = null;
    var simulation = null;
    var controlLinkLayer = null;                          // NEW
    var linkLayer = null;
    var cellLayer = null;
    var relLayer = null;
    var condLayer = null;                                 // NEW
    var labelLayer = null;
    var valueLayer = null;
    var nodes = [];
    var links = [];
    var width = 800;
    var height = 600;

    function cellEdgePoint(sx, sy, tx, ty) {
        var dx = tx - sx, dy = ty - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: tx, y: ty };
        var nx = dx / dist, ny = dy / dist;
        var hw = CELL_W / 2, hh = CELL_H / 2;
        var td = Math.abs(nx) > 1e-9 ? hw / Math.abs(nx) : Infinity;
        var ld = Math.abs(ny) > 1e-9 ? hh / Math.abs(ny) : Infinity;
        var d = Math.min(td, ld);
        return { x: tx - nx * d, y: ty - ny * d };
    }

    function circleEdgePoint(sx, sy, cx, cy, r) {
        var dx = cx - sx, dy = cy - sy;
        var dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) return { x: cx, y: cy };
        return { x: cx - dx / dist * r, y: cy - dy / dist * r };
    }

    // CHANGED: handles Cell, Relationship, and Conditional source/target kinds.
    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        function edgePt(node, ox, oy) {
            if (node.kind === 'Cell') return cellEdgePoint(ox, oy, node.x, node.y);
            var r = node.kind === 'Conditional' ? COND_COLLIDE_R : REL_R;
            return circleEdgePoint(ox, oy, node.x, node.y, r);
        }
        var srcPt = edgePt(s, t.x, t.y);
        var tgtPt = edgePt(t, s.x, s.y);
        return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
    }

    function settleSimulation() {
        var n = Math.ceil(Math.log(simulation.alphaMin()) / Math.log(1 - simulation.alphaDecay()));
        simulation.stop().alpha(1).tick(n);
        ticked();
    }

    function init(containerId, data) {
        if (simulation) { simulation.stop(); simulation = null; }
        if (svg) { svg.remove(); svg = null; }
        nodes = [];
        links = [];

        var container = document.getElementById(containerId);
        width = container.clientWidth || width;
        height = container.clientHeight || height;

        svg = d3.select(container)
            .append('svg')
            .attr('width', '100%')
            .attr('height', '100%')
            .attr('viewBox', [0, 0, width, height]);

        var defs = svg.append('defs');
        defs.append('marker')
            .attr('id', 'arrowhead')
            .attr('viewBox', '0 -5 10 10')
            .attr('refX', 10)
            .attr('refY', 0)
            .attr('markerWidth', 8)
            .attr('markerHeight', 8)
            .attr('markerUnits', 'userSpaceOnUse')
            .attr('orient', 'auto')
            .append('path').attr('d', 'M0,-5L10,0L0,5').attr('fill', '#999');

        // Layer z-order: bg → control links → constraint links → cells → rels → conditionals → labels → values
        svg.append('g').attr('class', 'bg-layer');
        controlLinkLayer = svg.append('g').attr('class', 'control-link-layer'); // NEW
        linkLayer = svg.append('g').attr('class', 'link-layer');
        cellLayer = svg.append('g').attr('class', 'cell-layer');
        relLayer = svg.append('g').attr('class', 'rel-layer');
        condLayer = svg.append('g').attr('class', 'cond-layer');               // NEW
        labelLayer = svg.append('g').attr('class', 'label-layer');
        valueLayer = svg.append('g').attr('class', 'value-layer');

        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(LINK_DISTANCE))
            .force('charge', d3.forceManyBody().strength(CHARGE_STRENGTH))
            .force('center', d3.forceCenter(width / 2, height / 2))
            // CHANGED: collision radius handles Conditional nodes.
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return CELL_COLLIDE_R;
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                return REL_COLLIDE_R;
            }));

        simulation.on('tick', ticked);
        update(data);
    }

    function update(data) {
        if (!svg) return;

        var oldNodeIds = new Set(nodes.map(function (n) { return n.id; }));
        var oldLinkSet = new Set(links.map(function (l) {
            var src = typeof l.source === 'object' ? l.source.id : l.source;
            var tgt = typeof l.target === 'object' ? l.target.id : l.target;
            return src + '-' + tgt;
        }));
        var structureChanged = nodes.length !== data.nodes.length
            || links.length !== data.links.length
            || data.nodes.some(function (n) { return !oldNodeIds.has(n.id); })
            || data.links.some(function (l) { return !oldLinkSet.has(l.source + '-' + l.target); });

        var oldNodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        nodes = data.nodes.map(function (n) {
            var existing = oldNodeMap.get(n.id);
            if (existing) {
                existing.kind = n.kind;
                existing.label = n.label;
                existing.value = n.value;
                return existing;
            }
            return Object.assign({}, n);
        });
        var nodeMap = new Map(nodes.map(function (n) { return [n.id, n]; }));
        links = data.links.map(function (l) { return Object.assign({}, l); });

        var changedSet = new Set(data.changed || []);
        var cellNodes = nodes.filter(function (n) { return n.kind === 'Cell'; });
        var relNodes = nodes.filter(function (n) { return n.kind === 'Relationship'; });
        var condNodes = nodes.filter(function (n) { return n.kind === 'Conditional'; }); // NEW
        var constraintLinks = links.filter(function (l) { return l.kind === 'Constraint'; }); // NEW
        var controlLinks = links.filter(function (l) { return l.kind === 'Control'; });         // NEW

        // Constraint links (existing behavior)
        linkLayer.selectAll('line')
            .data(constraintLinks, function (d) {         // CHANGED: constraintLinks only
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link')
            .attr('marker-end', function (d) {
                if (!data.arrows) return null;
                var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                var tgtNode = nodeMap.get(tgtId);
                return tgtNode ? 'url(#arrowhead)' : null;
            });

        // NEW: Control links (dashed, color-coded by branch)
        controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('stroke', function (d) {
                if (d.branch_index === null || d.branch_index === undefined) {
                    return DEFAULT_BRANCH_COLOR;
                }
                return BRANCH_COLORS[d.branch_index % BRANCH_COLORS.length] || DEFAULT_BRANCH_COLOR;
            })
            .attr('stroke-opacity', function (d) {
                return d.branch_active ? 1.0 : INACTIVE_OPACITY;
            });

        // Cell rects
        cellLayer.selectAll('rect')
            .data(cellNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-cell')
            .attr('width', CELL_W)
            .attr('height', CELL_H)
            .attr('rx', CELL_RX);

        // Relationship circles
        relLayer.selectAll('circle')
            .data(relNodes, function (d) { return d.id; })
            .join('circle')
            .attr('class', 'node-relationship')
            .attr('r', REL_R);

        // NEW: Dim inactive relationship circles.
        // A relationship is inactive if any control link targets it but none are active.
        (function () {
            var controlledRelIds = new Set();
            var activeRelIds = new Set();
            controlLinks.forEach(function (l) {
                var tgtId = typeof l.target === 'object' ? l.target.id : l.target;
                controlledRelIds.add(tgtId);
                if (l.branch_active) activeRelIds.add(tgtId);
            });
            relLayer.selectAll('circle').attr('opacity', function (d) {
                return (controlledRelIds.has(d.id) && !activeRelIds.has(d.id))
                    ? INACTIVE_OPACITY : null;
            });
        }());

        // NEW: Conditional diamond nodes (rotated rect)
        condLayer.selectAll('rect')
            .data(condNodes, function (d) { return d.id; })
            .join('rect')
            .attr('class', 'node-conditional')
            .attr('width', COND_SIZE * 2)
            .attr('height', COND_SIZE * 2);

        // Cell name labels
        labelLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-label')
            .text(function (d) { return d.label; });

        // Cell value labels
        valueLayer.selectAll('text')
            .data(cellNodes, function (d) { return d.id; })
            .join('text')
            .attr('class', 'node-value')
            .text(function (d) { return d.value || ''; });

        // Pulse changed cells
        if (changedSet.size > 0) {
            cellLayer.selectAll('rect')
                .filter(function (d) { return changedSet.has(d.id); })
                .transition().duration(PULSE_ON_MS)
                .style('fill', PULSE_COLOR)
                .transition().duration(PULSE_OFF_MS)
                .style('fill', null);
        }

        // Feed ALL links to the simulation (both constraint and control) so D3
        // resolves source/target strings to node objects for ticked().
        simulation.nodes(nodes);
        simulation.force('link').links(links);

        if (structureChanged) {
            settleSimulation();
        } else {
            ticked();
        }
    }

    function ticked() {
        // Constraint links: edge-to-edge so arrowheads land at node boundaries.
        linkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', ep.x2).attr('y2', ep.y2);
        });

        // NEW: Control links: center-to-center (dashed lines, no arrowhead clipping needed).
        controlLinkLayer.selectAll('line').each(function (d) {
            var s = d.source, t = d.target;
            d3.select(this)
                .attr('x1', s.x).attr('y1', s.y)
                .attr('x2', t.x).attr('y2', t.y);
        });

        cellLayer.selectAll('rect')
            .attr('x', function (d) { return d.x - CELL_W / 2; })
            .attr('y', function (d) { return d.y - CELL_H / 2; });

        relLayer.selectAll('circle')
            .attr('cx', function (d) { return d.x; })
            .attr('cy', function (d) { return d.y; });

        // NEW: Conditional diamond: rotated rect centered at (d.x, d.y).
        condLayer.selectAll('rect')
            .attr('transform', function (d) {
                return 'translate(' + d.x + ',' + d.y + ') rotate(45) translate(' + (-COND_SIZE) + ',' + (-COND_SIZE) + ')';
            });

        labelLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y - 4; });

        valueLayer.selectAll('text')
            .attr('x', function (d) { return d.x; })
            .attr('y', function (d) { return d.y + 10; });
    }

    window.beginGraph = { init: init, update: update };
}());
```

- [ ] **Step 3: Build and run the app**

```
cargo build -p begin
```

Then launch with `dx serve` from the `begin/` directory and open the browser. Verify:

1. The graph shows a yellow diamond node (the conditional) connected to the `p` cell by a solid edge.
2. Two dashed control lines run from the diamond to the two relationship circles: one blue (branch 0, `c=f`) and one orange (branch 1, `c=f×2`).
3. With `p=0` (initial value): the blue control line is fully opaque; the orange one and its relationship circle are dimmed.
4. Edit `p` to `1` in the inspector: the orange line becomes opaque and the blue line + its circle become dimmed.
5. Edit `p` to `2` (no branch matches): both control lines and both relationship circles are dimmed; the two constraint systems (`a×b=c` and `d×e=f`) operate independently.
6. The existing `a×b=c` and `d×e=f` constraint arrows still animate correctly on cell edits.
7. No console errors.

- [ ] **Step 4: Commit**

```
git add begin/assets/graph.js begin/assets/graph.css
git commit -m "feat(begin): render conditionals as diamond nodes with control lines in D3 graph"
```
