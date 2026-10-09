---
title: "Ez-adam Document Model and Node Placements Separation"
entity_type: observation
confidence: 0.80
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "ez-adam/src/model/cell_node.rs:1-35"
  - "ez-adam/src/ops/cells.rs:15-55"
  - "ez-adam/src/model/cell.rs:70"
relationships:
  - type: applies_to
    target: "Architecture Overview"
tags: ["ez-adam", "document-model", "cell-node", "cell", "canvas", "visual-model"]
tier: knowledge
source: inferred
---

# Ez-adam Document Model and Node Placements Separation

In ez-adam, visual canvas placement is decoupled from logical property-model cell data:
- Cell (ez-adam/src/model/cell.rs:70): Represents a logical property cell with an identity CellId, name, formula, CellType, and optional restriction expression.
- CellNode (ez-adam/src/model/cell_node.rs:21-35): Represents a visual canvas placement of a CellId at a specific Point.
- Key architectural invariant: Multiple CellNode placements can reference the same CellId ("two instances of the same value in the graph"). Modifying cell properties via ops::cells (e.g. set_name, set_restrict) updates the shared Cell data, which reflects across all canvas placements for that cell.

Aliases / also known as: ez-adam model, CellNode, Cell, Document, visual graph placements, multi-placement cells, ez-adam ops
