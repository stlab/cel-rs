# `begin`: Highlight Forced Relationships and Their Edges in the Graph

**Date:** 2026-07-14
**Author:** Sean Parent (with Claude)
**Status:** Approved

## Problem

The earlier "Surface Forced Cells in the UI" work
(`docs/superpowers/specs/2026-07-09-begin-forced-cells-ui-design.md`) gave `begin`'s D3
graph a `forced` highlight for cells that some active relationship's method structure
guarantees will always be overwritten by `propagate()`, plus a `forced-edge` highlight
on every constraint edge touching a forced cell. It explicitly scoped out the
relationship node itself: "No highlighting of the relationship node that produces a
forced cell."

That scope leaves a gap. In the demo sheet (`begin/assets/demo.pm`), setting `p` to `1`
activates a single-method relationship `[c] -> [g]`, which forces `g` — correctly
highlighted today. But the relationship node itself, and the edge from `c` into that
relationship, show no visual indication, even though that whole local structure
(which method runs, which edges carry values) is exactly as determined as `g` is,
independent of cell strength. `c` itself isn't a forced cell (its own value can still
come from elsewhere), so the existing cell-forced logic never lights up that edge.

This spec extends the highlighting to relationships: a relationship is forced when the
planner's method-elimination fixpoint leaves it exactly one viable method — whether
because it only ever had one method, or because sibling methods died from a forced-cell
cascade. Forced relationships get the same highlight as forced cells, and all of their
constraint edges (both inputs and outputs) get the `forced-edge` highlight too.

## Design

### property-model: expose forced relationships

`planner::plan` (`property-model/src/planner.rs`) already computes `alive:
HashMap<RelationshipId, Vec<bool>>` as part of `forced_output_cells`'s fixpoint: a
method is marked dead when its pure output is guaranteed to be produced by a
*different* relationship. A relationship is forced exactly when its `alive` vector has
exactly one `true` entry.

`Plan` (`property-model/src/planner.rs`) gains a field:

```rust
pub(crate) struct Plan {
    pub(crate) execution_order: Vec<(RelationshipId, usize)>,
    pub(crate) forced_outputs: HashSet<CellId>,
    /// Active relationships with exactly one alive method after the forced-output
    /// fixpoint (see `forced_output_cells`) — the planner has no alternative method to
    /// choose for these, regardless of cell strength.
    pub(crate) forced_relationships: HashSet<RelationshipId>,
}
```

Computed in `plan()` right after the `forced_output_cells` call, from the same `alive`
map already in scope:

```rust
let forced_relationships = alive
    .iter()
    .filter(|(_, methods)| methods.iter().filter(|&&is_alive| is_alive).count() == 1)
    .map(|(&rel_id, _)| rel_id)
    .collect();
```

`Sheet` (`property-model/src/sheet.rs`) gains a field mirroring `last_forced`:

```rust
last_forced_relationships: Option<HashSet<RelationshipId>>,
```

Set in `propagate()` alongside `last_forced`, left untouched by
`propagate_without_replan` (same rationale as `last_forced`: it reflects the last full
planning pass). Two new public methods mirror the existing cell-level API exactly:

```rust
/// Returns `true` if `id` had exactly one viable method as of the last successful
/// `propagate()` call — the planner has no alternative regardless of cell strength.
///
/// Returns `false` if no propagation has run yet.
pub fn is_relationship_forced(&self, id: RelationshipId) -> bool

/// Iterates relationships that are forced (see `Sheet::is_relationship_forced`) as of
/// the last `propagate()` call.
///
/// - Complexity: O(n) where n is the number of forced relationships.
pub fn forced_relationships(&self) -> impl Iterator<Item = RelationshipId> + '_
```

### begin: thread it through to the graph

`GraphData` (`begin/src/bridge.rs`) gains a field populated the same way as `forced`:

```rust
/// Stable IDs of relationships forced by the planner (see
/// `property_model::Sheet::is_relationship_forced`); consumers may render them
/// distinctly, along with their constraint edges.
pub forced_relationships: Vec<String>,
```

```rust
let forced_relationships = sheet.forced_relationships().map(rel_node_id).collect();
```

### graph.js / graph.css: highlight relationship nodes and their edges

The existing forced-highlighting IIFE in `graph.js`'s `update()` extends: build a
second set from `data.forced_relationships`, toggle `.forced` on relationship
`<circle>`s in that set, and widen the `forced-edge` predicate so a link is forced when
*either* endpoint is a forced cell *or* a forced relationship:

```javascript
(function () {
    var forcedSet = new Set(data.forced || []);
    var forcedRelSet = new Set(data.forced_relationships || []);
    cellLayer.selectAll('rect')
        .classed('forced', function (d) { return forcedSet.has(d.id); });
    relLayer.selectAll('circle')
        .classed('forced', function (d) { return forcedRelSet.has(d.id); });
    linkLayer.selectAll('line')
        .classed('forced-edge', function (d) {
            var srcId = typeof d.source === 'object' ? d.source.id : d.source;
            var tgtId = typeof d.target === 'object' ? d.target.id : d.target;
            return forcedSet.has(srcId) || forcedSet.has(tgtId)
                || forcedRelSet.has(srcId) || forcedRelSet.has(tgtId);
        });
}());
```

`graph.css` adds one rule, reusing the same purple used for forced cells:

```css
.node-relationship.forced {
    stroke: #8e44ad;
    stroke-width: 3;
}
```

No demo-source change is needed: `begin/assets/demo.pm` already has the `p`/`c`/`g`
scenario this spec targets (a `conditional p { 1i32 => { method [c] -> [g] { c * 10.0 } } }`
block, added by the prior forced-cells work).

## Testing

- `property-model/tests/integration.rs`: tests mirroring
  `is_forced_true_for_single_method_output` and
  `forced_outputs_cascade_through_adjacent_relationship`, asserting
  `Sheet::is_relationship_forced`/`forced_relationships()` for a single-method
  relationship and for a relationship whose sibling method died via cascade; a
  multi-method-relationship counterpart asserting it is *not* forced (mirroring
  `is_forced_false_for_multi_method_relationship`).
- `begin/src/bridge.rs`: tests mirroring `to_graph_data_forced_field_contains_forced_cell`
  and `to_graph_data_forced_field_excludes_cell_when_branch_inactive`, asserting
  `GraphData::forced_relationships` contains/excludes the `[c] -> g` relationship's node
  ID as `p`'s conditional branch activates/deactivates.
- Manual verification: run the app (`dx serve --platform desktop` from `begin/`), set `p`
  to `1`, confirm the `[c] -> g` relationship circle and the `c -> relationship`
  constraint edge turn purple along with `g`; set `p` back to `0` and confirm all three
  revert.

## Out of Scope

- No change to the flood-fill/method-selection algorithm itself — this only surfaces
  information the planner already computes internally (`alive`).
- No new demo source — the existing `p`/`c`/`g` conditional already exercises this.
- No distinct visual treatment for "which method was forced" beyond the existing
  directed-edge rendering (arrows already show the selected method's input/output
  shape); forced relationships reuse the same purple as forced cells rather than
  introducing a new color.
