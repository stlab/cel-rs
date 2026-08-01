# Conditional Branch Junction Nodes

**Date:** 2026-07-30
**Branch:** `worktree-begin-link-condtional-branches`

## Goal

Today the `begin` graph draws one direct `Control` edge from a conditional node to each
relationship in a branch. When a branch holds more than one relationship, those edges are
visually indistinguishable from edges belonging to different branches — there's no way to see
at a glance that several relationships are grouped under the same branch. Add a visual grouping
so that multi-relationship branches read as a single fan-out from a shared point.

## Approach

Introduce an invisible, zero-size **junction node** per branch (named or default) that has two
or more relationships. Control links for that branch route through it:
`conditional → branch-node → relationship`, instead of `conditional → relationship` directly.
Branches (or the default) with 0 or 1 relationships are unaffected — they keep today's direct
edge. The junction node is a real node in the D3 force simulation (so the fan-out spreads
naturally) but has no visible shape, no new SVG layer, and no interaction handlers.

### Approaches considered

1. **Server-side junction node (chosen).** `bridge.rs` emits a new `NodeKind::Branch` node
   and routes control links through it. `graph.js` gives it zero collision radius and never
   draws a shape for it.
2. **Client-side synthesis in `graph.js` only**, leaving `GraphData` untouched and grouping
   `Control` links by `branch_index` in JS before feeding the simulation. Rejected: `update()`
   already relies on `data.nodes`/`data.links` being the authoritative 1:1 source for stable
   ids and structural-change detection; synthesizing extra nodes client-side duplicates that
   bookkeeping and moves "what counts as a branch" (an `adam_rs` concept) into the view layer,
   away from `bridge.rs`, which is the established single source of truth for translating sheet
   state into display data.
3. **Reuse `NodeKind::Relationship` with a `virtual: true` flag** instead of a new enum variant.
   Rejected: a "relationship" that isn't one is confusing, and every switch on `NodeKind`
   (styling, dimming, collision radius) would need an extra flag check anyway — a real new
   variant costs no more code and is clearer.

## Section 1 — Data model (`begin/src/bridge.rs`)

### `NodeKind`

```rust
pub enum NodeKind {
    Cell,
    Relationship,
    Conditional,
    /// An invisible junction node grouping a branch's relationships when a
    /// branch (or the default) holds more than one; rendered as a zero-size point.
    Branch,
}
```

### Node ID scheme

```rust
fn branch_node_id(cond_id: ConditionalId, branch: Option<usize>) -> String {
    match branch {
        Some(b) => format!("br{}_{}", cond_id.data().as_ffi(), b),
        None => format!("br{}_def", cond_id.data().as_ffi()),
    }
}
```

`Branch` nodes carry empty `label`/`value`, matching `Conditional`/`Relationship` nodes today.

### `to_graph_data` changes

Refactor the named-branch loop and the default-relationships block to share one helper that,
given `(cond_id, branch_index: Option<usize>, is_active: bool, rels: &[RelationshipId])`:

- If `rels.len() <= 1`: emit exactly today's behavior — a single direct `Control` link
  `conditional → relationship` (or nothing, if `rels` is empty), with `branch_index`/
  `branch_active` set as today.
- If `rels.len() >= 2`: push one `Branch` `NodeData`, one `Control` link
  `conditional → branch-node` carrying `branch_index`/`branch_active` for this branch, and one
  `Control` link `branch-node → relationship` per relationship in `rels`, carrying the *same*
  `branch_index`/`branch_active` as the trunk link — so the whole path colors/dims as one unit.

Call this helper once per named branch (`branch_index: Some(i)`) and once for the default
relationships (`branch_index: None`).

### Docs and tests

Update the `to_graph_data` doc comment and the `NodeKind`/`LinkData` field docs to describe the
junction behavior. Add contract-style unit tests alongside the existing conditional tests
(added at [bridge.rs:732-841](../../../begin/src/bridge.rs#L732-L841)):

- A `Branch` node is emitted iff a named branch has ≥2 relationships.
- A `Branch` node is emitted iff the default has ≥2 relationships.
- Control links for a ≥2-relationship branch route through the branch node, and both hops carry
  matching `branch_index`/`branch_active`.
- A branch/default with 0 or 1 relationships emits no `Branch` node and behaves exactly as
  before (regression coverage for the existing tests in this area).

## Section 2 — Rendering (`begin/assets/graph.js`)

`Branch` nodes have no visible shape, so they need no new SVG layer or join — only enough
handling in the existing force/geometry code so the simulation and edge drawing behave
correctly:

- **Collision force**: in the `d3.forceCollide().radius(...)` callback, add
  `if (d.kind === 'Branch') return 0;`.
- **Link distance**: give the two junction-related hops (`conditional→branch`,
  `branch→relationship`) half of today's `LINK_DISTANCE` (40px vs. 80px) via
  `d3.forceLink().distance(function(d) { ... })`, keyed on whether either link endpoint is a
  `Branch` node — otherwise splitting one edge into two would double the visual distance from
  conditional to relationship for grouped branches relative to ungrouped ones.
- **`computeBBox`**: add a `Branch` case with `hw = hh = 0` (contributes just its point plus the
  existing `FIT_MARGIN`, no phantom padding).
- **`ticked()` control-link geometry**: the per-target radius calc (currently
  `t.kind === 'Conditional' ? COND_COLLIDE_R : REL_R`) needs a `Branch` case returning `0`, and
  the dot-marker padding (`+ NODE_STROKE_WIDTH / 2 + CONTROL_DOT_RADIUS`) must be skipped when
  the target is a `Branch` node, so the trunk edge terminates exactly at the junction point.
- **Dot marker**: currently every control-link line unconditionally gets
  `.attr('marker-end', 'url(#dot)')`. Change this to look up the target node's kind via the
  existing `nodeMap` (same pattern already used for the arrowhead lookup at
  [graph.js:503-510](../../../begin/assets/graph.js#L503-L510)) and only add the dot marker when
  the target is *not* a `Branch` node. This makes the trunk edge a plain dashed line with no cap,
  and only the leaf edges (branch→relationship) get the dot — so the whole path reads as one
  continuous dashed control edge with a single dot at its true endpoint.
- **`branch_active` styling**: no change — it already colors per-link from `d.branch_active`,
  and since trunk and leaf links for a branch carry the same value, the whole path colors
  consistently with no extra code.
- No CSS changes — nothing new is drawn.

## Section 3 — Testing & verification

- Contract-style unit tests in `bridge.rs` per the above.
- `cargo test --workspace`, `cargo test --doc --workspace`.
- `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`,
  `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`,
  `cargo clippy -p begin --all-targets -- -D warnings`.
- Visual verification via the `verifying-begin-ui` skill: confirm a 2+-relationship branch shows
  an invisible elbow with correctly dashed edges and a single dot at each relationship; confirm
  single-relationship branches and the default branch (both single- and multi-relationship
  forms) render as before/as designed; confirm active/inactive coloring applies consistently
  across the trunk and leaf edges of a branch; confirm pan/zoom "Fit" bounds aren't visibly
  thrown off by the new node.

## Files to change

| File | Change |
|------|--------|
| `begin/src/bridge.rs` | `NodeKind::Branch`, `branch_node_id`, refactored branch/default loop in `to_graph_data`, doc updates, new tests |
| `begin/assets/graph.js` | Collision radius, link distance, bbox, control-link geometry, and dot-marker handling for `Branch` nodes |

## Out of scope

- Visually distinguishing the trunk edge (conditional→branch) from leaf edges (branch→relationship) — both use identical styling so the branch reads as one continuous edge.
- Junction nodes for branches/default with 0 or 1 relationships — direct edges are kept for those.
- Any change to the `Conditional` node's own rendering, the inspector, or non-conditional graph behavior.
