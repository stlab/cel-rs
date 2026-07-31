# Conditional Branch Junction Nodes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a conditional's branch (named or default) holds more than one relationship, route its control edges through an invisible junction node (`conditional → branch-node → relationship`) instead of separate direct edges, so the graph view visually groups them as one branch.

**Architecture:** A new `NodeKind::Branch` variant in `begin/src/bridge.rs` represents the junction node. `to_graph_data` gains a shared helper that emits either the junction node + two-hop links (≥2 relationships) or today's direct link (0 or 1 relationships), called once for each named branch and once for the default relationships. `begin/assets/graph.js` treats `Branch` nodes as zero-size points in the force simulation, bounding-box, and edge-geometry code, and suppresses the dot end-marker where a control link's target is a junction node.

**Tech Stack:** Rust (`adam_rs::Sheet`, `serde`), D3.js v7 (force simulation, SVG).

## Global Constraints

- `cargo fmt --all` must be run before committing (enforced by pre-commit hook).
- `cargo build --workspace` and `cargo test --workspace` must produce zero compiler warnings (not just pass clippy).
- Lint with all three invocations: `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`, `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`, `cargo clippy -p begin --all-targets -- -D warnings`.
- Every function needs a contract-style `///` doc comment (Summary; Preconditions as `- Precondition:` bullets with `debug_assert!` in the body, not prose, for violations; Postconditions as `- Postcondition:` bullets when not implied by the summary; `- Complexity:` bullet whenever not O(1)).
- Unit tests are derived from the contract and public interface only — do not encode implementation details.
- No UI change is considered complete until actually rendered and inspected — see Task 5 and `begin/CLAUDE.md`.

---

### Task 1: `NodeKind::Branch` + junction routing for named branches

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Produces: `NodeKind::Branch` variant; `fn branch_node_id(id: ConditionalId, branch: Option<usize>) -> String`; `fn push_branch_links(nodes: &mut Vec<NodeData>, links: &mut Vec<LinkData>, cond_id_str: &str, cond_id: ConditionalId, branch_index: Option<usize>, branch_active: bool, rels: &[RelationshipId])`. Task 2 calls `push_branch_links` for the default-relationships case.

- [ ] **Step 1: Write the failing tests**

Add these two test helpers and two tests near the existing `sheet_with_conditional`/`sheet_with_forced_conditional` helpers (around `begin/src/bridge.rs:481-526`), inside the existing `#[cfg(test)] mod tests` block:

```rust
    fn sheet_with_multi_relationship_branch() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, c, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel1, rel2])], vec![])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_omits_branch_node_for_single_relationship_branch() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            !data.nodes.iter().any(|n| n.kind == NodeKind::Branch),
            "expected no Branch node when every branch has at most one relationship"
        );
    }

    #[test]
    fn to_graph_data_routes_multi_relationship_branch_through_branch_node() {
        let (sheet, labels) = sheet_with_multi_relationship_branch();
        let data = to_graph_data(&sheet, &labels);

        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        let branch_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Branch)
            .map(|n| n.id.clone())
            .expect("expected a Branch node");

        assert!(
            data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)
                && l.source == cond_id
                && l.target == branch_id
                && l.branch_index == Some(0)
                && l.branch_active == Some(true)),
            "expected a Control link from the conditional to the branch node"
        );

        let rel_ids: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .collect();
        assert_eq!(rel_ids.len(), 2);
        for rel_id in rel_ids {
            assert!(
                data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)
                    && l.source == branch_id
                    && l.target == rel_id
                    && l.branch_index == Some(0)
                    && l.branch_active == Some(true)),
                "expected a Control link from the branch node to relationship {rel_id}"
            );
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --workspace to_graph_data_omits_branch_node_for_single_relationship_branch to_graph_data_routes_multi_relationship_branch_through_branch_node`
Expected: compile error — `NodeKind::Branch` does not exist yet.

- [ ] **Step 3: Add the `NodeKind::Branch` variant**

In `begin/src/bridge.rs`, modify the `NodeKind` enum (currently at lines 138-147):

```rust
/// Node kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A value cell — rendered as a `<rect>`.
    Cell,
    /// A multi-way constraint — rendered as a `<circle>`.
    Relationship,
    /// A conditional switch — rendered as a diamond (rotated `<rect>`).
    Conditional,
    /// An invisible junction node grouping a branch's relationships when a
    /// branch (or the default) holds more than one; rendered as a zero-size point.
    Branch,
}
```

- [ ] **Step 4: Add `branch_node_id` and `push_branch_links`**

Add these two functions after `cond_node_id` (currently at `begin/src/bridge.rs:220-222`), before `to_graph_data`:

```rust
/// Returns the stable node ID for the junction node of one branch (or the default) of a
/// conditional: `"br{ffi}_{branch}"` for a named branch, `"br{ffi}_def"` for the default.
fn branch_node_id(id: ConditionalId, branch: Option<usize>) -> String {
    match branch {
        Some(b) => format!("br{}_{}", id.data().as_ffi(), b),
        None => format!("br{}_def", id.data().as_ffi()),
    }
}

/// Pushes control links (and, when needed, a junction node) for one branch — named or
/// default — of a conditional.
///
/// - Postcondition: when `rels.len() >= 2`, pushes one `Branch` node, one
///   `conditional → branch` control link, and one `branch → relationship` control link per
///   entry in `rels`, all sharing `branch_index`/`branch_active`. When `rels.len() <= 1`,
///   pushes at most one direct `conditional → relationship` control link (none if `rels` is
///   empty), matching the pre-junction-node behavior.
fn push_branch_links(
    nodes: &mut Vec<NodeData>,
    links: &mut Vec<LinkData>,
    cond_id_str: &str,
    cond_id: ConditionalId,
    branch_index: Option<usize>,
    branch_active: bool,
    rels: &[RelationshipId],
) {
    if rels.len() >= 2 {
        let bnode_id = branch_node_id(cond_id, branch_index);
        nodes.push(NodeData {
            id: bnode_id.clone(),
            kind: NodeKind::Branch,
            label: String::new(),
            value: String::new(),
        });
        links.push(LinkData {
            source: cond_id_str.to_string(),
            target: bnode_id.clone(),
            kind: LinkKind::Control,
            branch_index,
            branch_active: Some(branch_active),
        });
        for &rel_id in rels {
            links.push(LinkData {
                source: bnode_id.clone(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                branch_index,
                branch_active: Some(branch_active),
            });
        }
    } else {
        for &rel_id in rels {
            links.push(LinkData {
                source: cond_id_str.to_string(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                branch_index,
                branch_active: Some(branch_active),
            });
        }
    }
}
```

- [ ] **Step 5: Route the named-branch loop through `push_branch_links`**

In `to_graph_data`, replace the named-branches loop (currently at `begin/src/bridge.rs:325-340`):

```rust
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
```

with:

```rust
        // Control links for named branches
        let branch_count = sheet.conditional_branch_count(cond_id).unwrap_or(0);
        for branch in 0..branch_count {
            let is_active = active_branch == Some(branch);
            if let Some(rels) = sheet.conditional_branch_relationships(cond_id, branch) {
                push_branch_links(
                    &mut nodes,
                    &mut links,
                    &node_id,
                    cond_id,
                    Some(branch),
                    is_active,
                    rels,
                );
            }
        }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --workspace to_graph_data_omits_branch_node_for_single_relationship_branch to_graph_data_routes_multi_relationship_branch_through_branch_node`
Expected: both PASS.

- [ ] **Step 7: Run the full bridge.rs test suite to check for regressions**

Run: `cargo test --workspace --lib bridge::`
Expected: all existing tests still PASS (in particular `to_graph_data_emits_control_link_for_branch_relationship` and `to_graph_data_active_branch_control_link_is_active`, which exercise the single-relationship-branch path this refactor must preserve).

- [ ] **Step 8: Format and commit**

```bash
cargo fmt --all
git add begin/src/bridge.rs
git commit -m "feat(begin): route multi-relationship named branches through a junction node"
```

---

### Task 2: Junction routing for the default relationships + doc updates

**Files:**
- Modify: `begin/src/bridge.rs`

**Interfaces:**
- Consumes: `push_branch_links` from Task 1 (same signature, called with `branch_index: None`).

- [ ] **Step 1: Write the failing test**

Add this helper and test alongside the ones from Task 1:

```rust
    fn sheet_with_multi_relationship_default() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, c, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional(p, vec![], vec![rel1, rel2])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_routes_multi_relationship_default_through_branch_node() {
        let (sheet, labels) = sheet_with_multi_relationship_default();
        let data = to_graph_data(&sheet, &labels);

        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        let branch_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Branch)
            .map(|n| n.id.clone())
            .expect("expected a Branch node for the default relationships");

        assert!(
            data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)
                && l.source == cond_id
                && l.target == branch_id
                && l.branch_index.is_none()
                && l.branch_active == Some(true)),
            "expected a Control link from the conditional to the default branch node"
        );

        let rel_ids: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .collect();
        assert_eq!(rel_ids.len(), 2);
        for rel_id in rel_ids {
            assert!(
                data.links.iter().any(|l| matches!(l.kind, LinkKind::Control)
                    && l.source == branch_id
                    && l.target == rel_id
                    && l.branch_index.is_none()
                    && l.branch_active == Some(true)),
                "expected a Control link from the default branch node to relationship {rel_id}"
            );
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --workspace to_graph_data_routes_multi_relationship_default_through_branch_node`
Expected: FAIL — no `Branch` node is emitted yet for the default case (the default block still emits direct links).

- [ ] **Step 3: Route the default-relationships block through `push_branch_links`**

Replace the default-relationships block in `to_graph_data` (currently at `begin/src/bridge.rs:342-354`):

```rust
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
```

with:

```rust
        // Control links for default relationships
        let default_active = active_branch.is_none();
        if let Some(default_rels) = sheet.conditional_default_relationships(cond_id) {
            push_branch_links(
                &mut nodes,
                &mut links,
                &node_id,
                cond_id,
                None,
                default_active,
                default_rels,
            );
        }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --workspace to_graph_data_routes_multi_relationship_default_through_branch_node`
Expected: PASS.

- [ ] **Step 5: Update doc comments to describe the junction node**

Update the `to_graph_data` doc comment (currently at `begin/src/bridge.rs:230-235`):

```rust
/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// Constraint links: when a plan is cached (`sheet.selected_method` returns `Some`) links are
/// directed (inputs → relationship → outputs) and [`GraphData::arrows`] is `true`. Otherwise
/// all cells adjacent to the relationship are emitted as undirected source→relationship edges.
///
/// Conditional nodes: for each conditional, emits one `Conditional` node, one `Constraint` link
/// from the match cell to the conditional node, and one `Control` link per relationship in each
/// branch/default. When a branch (or the default) holds more than one relationship, its control
/// links route through an intermediate `Branch` junction node (`conditional → branch →
/// relationship`) instead of a direct edge, so the branch's relationships visually group
/// together; branches with 0 or 1 relationships keep a direct edge. Control links carry
/// `branch_index` and `branch_active` for rendering, shared identically across both hops of a
/// junction-routed branch.
///
/// - Complexity: O(c + r + e + cond·b·k) where c = cells, r = relationships, e = adjacency pairs,
///   cond = conditionals, b = branches per conditional, k = keys per branch.
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
```

Update the `LinkData` doc comment (currently at `begin/src/bridge.rs:171-175`):

```rust
/// A single edge in the D3 graph.
///
/// When [`GraphData::arrows`] is `false` constraint edges are undirected; when `true`
/// they are directed from `source` to `target`. Control edges are always directed — from a
/// conditional node to a relationship, or, when a branch has more than one relationship, from
/// the conditional to an intermediate `Branch` node and from that node to each relationship —
/// and styled by `branch_index` and `branch_active`.
```

- [ ] **Step 6: Run the full bridge.rs test suite to check for regressions**

Run: `cargo test --workspace --lib bridge::`
Expected: all tests PASS, including `to_graph_data_forced_relationships_field_contains_forced_relationship` and the other `sheet_with_forced_conditional`-based tests (default relationships there are empty, so they must still emit no `Branch` node).

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --all
git add begin/src/bridge.rs
git commit -m "feat(begin): route multi-relationship default branch through a junction node"
```

---

### Task 3: Force simulation + bounding box handling for `Branch` nodes

**Files:**
- Modify: `begin/assets/graph.js`

**Interfaces:**
- Consumes: nodes with `kind === 'Branch'` now present in `data.nodes`/`data.links` from Tasks 1-2.

**Note:** This project has no JS test runner (no Node/Playwright in this environment — see `begin/CLAUDE.md`). These edits are verified together with Task 4's edits via the visual check in Task 5, not in isolation.

- [ ] **Step 1: Give `Branch` nodes zero collision radius**

In `buildGraph`, modify the `collide` force (currently at `begin/assets/graph.js:269-273`):

```javascript
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return CELL_COLLIDE_R;
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                return REL_COLLIDE_R;
            }));
```

to:

```javascript
            .force('collide', d3.forceCollide().radius(function (d) {
                if (d.kind === 'Cell') return CELL_COLLIDE_R;
                if (d.kind === 'Conditional') return COND_COLLIDE_R;
                if (d.kind === 'Branch') return 0;
                return REL_COLLIDE_R;
            }));
```

- [ ] **Step 2: Halve the link distance for junction hops**

In the same `buildGraph` function, modify the `link` force (currently at `begin/assets/graph.js:264-265`):

```javascript
        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(LINK_DISTANCE))
```

to:

```javascript
        simulation = d3.forceSimulation()
            .force('link', d3.forceLink().id(function (d) { return d.id; }).distance(function (d) {
                var sKind = typeof d.source === 'object' ? d.source.kind : null;
                var tKind = typeof d.target === 'object' ? d.target.kind : null;
                return (sKind === 'Branch' || tKind === 'Branch') ? LINK_DISTANCE / 2 : LINK_DISTANCE;
            }))
```

- [ ] **Step 3: Exclude `Branch` nodes from bounding-box padding**

In `computeBBox`, modify the per-node size switch (currently at `begin/assets/graph.js:85-89`):

```javascript
            var hw, hh;
            if (n.kind === 'Cell') { hw = CELL_W / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else { hw = REL_R; hh = REL_R; }
```

to:

```javascript
            var hw, hh;
            if (n.kind === 'Cell') { hw = CELL_W / 2; hh = CELL_H / 2; }
            else if (n.kind === 'Conditional') { hw = COND_COLLIDE_R; hh = COND_COLLIDE_R; }
            else if (n.kind === 'Branch') { hw = 0; hh = 0; }
            else { hw = REL_R; hh = REL_R; }
```

- [ ] **Step 4: Commit**

```bash
git add begin/assets/graph.js
git commit -m "feat(begin): exclude Branch junction nodes from collision/bbox sizing"
```

---

### Task 4: Control-link geometry and dot-marker suppression for `Branch` targets

**Files:**
- Modify: `begin/assets/graph.js`

**Interfaces:**
- Consumes: `nodeMap` (a `Map<string, node>` already built in `update()`, in scope before the `controlLinkLayer` join).

**Note:** Same as Task 3 — no isolated JS test; verified visually in Task 5.

- [ ] **Step 1: Fix `linkEndpoints`'s `edgePt` for `Branch` nodes**

A `Branch` node has no visible boundary, so an edge touching it (as either source or target) should end exactly at its `(x, y)`, not offset by a circle radius. Modify `edgePt` inside `linkEndpoints` (currently at `begin/assets/graph.js:65-71`):

```javascript
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
```

to:

```javascript
    function linkEndpoints(d) {
        var s = d.source, t = d.target;
        function edgePt(node, ox, oy) {
            if (node.kind === 'Cell') return cellEdgePoint(ox, oy, node.x, node.y);
            if (node.kind === 'Branch') return { x: node.x, y: node.y };
            var r = node.kind === 'Conditional' ? COND_COLLIDE_R : REL_R;
            return circleEdgePoint(ox, oy, node.x, node.y, r);
        }
        var srcPt = edgePt(s, t.x, t.y);
        var tgtPt = edgePt(t, s.x, s.y);
        return { x1: srcPt.x, y1: srcPt.y, x2: tgtPt.x, y2: tgtPt.y };
    }
```

This matters for the trunk edge's source point (a `Conditional`, unaffected) and, crucially, for a leaf edge's *source* point where the source is now a `Branch` node — without this fix, `linkEndpoints` would offset that line's start away from the junction point by `REL_R` (16px), leaving a visible gap.

- [ ] **Step 2: Zero the target-radius padding when a control link's target is a `Branch` node**

In `ticked()`, modify the control-link geometry block (currently at `begin/assets/graph.js:485-493`):

```javascript
        controlLinkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            var t = d.target;
            var tgtR = (t.kind === 'Conditional' ? COND_COLLIDE_R : REL_R) + NODE_STROKE_WIDTH / 2 + CONTROL_DOT_RADIUS;
            var tgtPt = circleEdgePoint(d.source.x, d.source.y, t.x, t.y, tgtR);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', tgtPt.x).attr('y2', tgtPt.y);
        });
```

to:

```javascript
        controlLinkLayer.selectAll('line').each(function (d) {
            var ep = linkEndpoints(d);
            var t = d.target;
            var tgtR = t.kind === 'Branch'
                ? 0
                : (t.kind === 'Conditional' ? COND_COLLIDE_R : REL_R) + NODE_STROKE_WIDTH / 2 + CONTROL_DOT_RADIUS;
            var tgtPt = circleEdgePoint(d.source.x, d.source.y, t.x, t.y, tgtR);
            d3.select(this)
                .attr('x1', ep.x1).attr('y1', ep.y1)
                .attr('x2', tgtPt.x).attr('y2', tgtPt.y);
        });
```

`circleEdgePoint(..., 0)` degenerates to the node's own `(x, y)` with no offset, so the trunk edge terminates exactly at the junction point.

- [ ] **Step 3: Suppress the dot marker when the target is a `Branch` node**

In `update()`, modify the `controlLinkLayer` join (currently at `begin/assets/graph.js:339-349`):

```javascript
        controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('marker-end', 'url(#dot)')
            .style('stroke', function (d) { return d.branch_active ? null : INACTIVE_STROKE; });
```

to:

```javascript
        controlLinkLayer.selectAll('line')
            .data(controlLinks, function (d) {
                var src = typeof d.source === 'object' ? d.source.id : d.source;
                var tgt = typeof d.target === 'object' ? d.target.id : d.target;
                return src + '-' + tgt;
            })
            .join('line')
            .attr('class', 'link-control')
            .attr('stroke-dasharray', '5 3')
            .attr('marker-end', function (d) {
                var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
                var tgtNode = nodeMap.get(tgtId);
                return (tgtNode && tgtNode.kind === 'Branch') ? null : 'url(#dot)';
            })
            .style('stroke', function (d) { return d.branch_active ? null : INACTIVE_STROKE; });
```

This makes the trunk edge (conditional → branch) a plain dashed line with no cap, and keeps the dot only on leaf edges (branch → relationship) — so the whole path reads as one continuous dashed control edge with a single dot at its true endpoint.

**No change needed:** the inactive-relationship dimming block just below (`begin/assets/graph.js:370-397`) builds `controlledRelIds`/`activeRelIds` from every control link's target id, which now sometimes includes a branch-node id. That id never matches any actual relationship node id, so it's an inert extra entry in the sets — `isInactiveRel` is only ever queried with real relationship ids, so this needs no code change.

- [ ] **Step 4: Commit**

```bash
git add begin/assets/graph.js
git commit -m "feat(begin): route control-link geometry and dot markers around Branch junction nodes"
```

---

### Task 5: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Run the full test suite**

Run: `cargo test --workspace`
Expected: PASS, zero compiler warnings in the output (read the full output, not just the pass/fail summary — `-D warnings` in clippy doesn't catch everything a plain build/test compile warns about).

Run: `cargo test --doc --workspace`
Expected: PASS.

- [ ] **Step 2: Run all three clippy invocations**

Run:
```bash
cargo clippy --workspace --exclude begin --all-targets -- -D warnings
cargo clippy -p begin --no-default-features --all-targets -- -D warnings
cargo clippy -p begin --all-targets -- -D warnings
```
Expected: all three exit clean with no warnings.

- [ ] **Step 3: Visually verify with the existing demo sheet**

The demo sheet (`begin/assets/demo.adm2`) already has exactly the shapes needed, no edits required:
- `p = 0`: named branch with **one** relationship (`f ↔ c`) — must render as a direct edge, unchanged from before this feature.
- `p = 1`: named branch with **two** relationships (`f ↔ c × 2`, and `c → g × 10`) — must render through a junction node.
- default (`p` anything else): **one** relationship (`f → c`) — must render as a direct edge, unchanged.

Use the `verifying-begin-ui` skill to serve `begin` and inspect the rendered graph:
1. Load the app with the default demo (`p = 0`). Confirm the conditional's two branches (`0` and default) each still show a single direct dashed edge straight from the diamond to their relationship circle — no junction point, no visual change from before this feature.
2. Set `p` to `1` via the Inspector. Confirm branch `1`'s two relationships (the `f/c` relationship and `g`'s relationship) now both connect through one shared invisible elbow point coming off the diamond, instead of two edges that appear to originate independently from the diamond.
3. Confirm the dot end-marker appears only where each dashed line meets an actual relationship circle — not floating at the junction elbow itself.
4. Confirm branch coloring/dimming still behaves correctly: with `p = 1` active, branch 1's edges (both the trunk and both leaves) should render in the "active" style, and branch 0's edge should render in the inactive style (and vice versa when `p` is set back to `0`).
5. Confirm "Fit" / pan-zoom bounds still frame the graph reasonably (no oversized empty margin caused by the new node).

- [ ] **Step 4: Record the outcome**

If any visual check in Step 3 fails, return to Task 3/4 to fix the specific geometry or styling issue before considering this plan complete. Once all checks pass, the feature is done — no further commit is needed for this task (it's verification-only).
