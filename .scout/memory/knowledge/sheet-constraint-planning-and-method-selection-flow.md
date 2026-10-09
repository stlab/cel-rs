---
title: "Sheet Constraint Planning and Method Selection Flow"
entity_type: observation
confidence: 0.80
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "adam-rs/src/sheet.rs:326-370"
  - "adam-rs/src/planner.rs:89-135"
  - "adam-rs/src/relationship.rs:23-36"
relationships:
  - type: applies_to
    target: "Architecture Overview"
  - type: relates_to
    target: "Filter Purity and Idempotence Contract"
tags: ["adam-rs", "sheet", "planner", "multi-way-constraints", "method-selection"]
tier: knowledge
source: inferred
---

# Sheet Constraint Planning and Method Selection Flow

adam-rs coordinates multi-way constraint relationships across type-erased cells within a Sheet (adam-rs/src/sheet.rs:326-370):
- Each relationship consists of one or more directional Methods (adam-rs/src/relationship.rs:23-36) specifying inputs, outputs, and pure transformation callbacks.
- Planning (adam-rs/src/planner.rs:89-135): During plan(), the planner determines an execution order:
  1. forced_output_cells calculates fixpoints for cells that can never be sources and identifies relationships with only one viable method.
  2. release::resolve performs tentative source elimination and acyclic assignment search guided by cell strength.
  3. Method selection is purely strength-based and value-blind; self-referencing cycles reconstruct inputs via evaluate_seeds at runtime.
  4. Yields a Plan containing execution_order, or returns Error::Conflict / Error::Cycle when no acyclic plan can be established.

Aliases / also known as: adam planning, Sheet planner, method selection, multi-way constraints, release resolve, constraint propagation
