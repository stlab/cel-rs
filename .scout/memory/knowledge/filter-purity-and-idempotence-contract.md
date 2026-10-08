---
title: "Filter Purity and Idempotence Contract"
entity_type: observation
confidence: 0.80
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "adam-rs/src/filter.rs:42-85"
  - "adam-rs/src/sheet.rs:751"
  - "adam-rs/src/relationship.rs:23-36"
relationships:
  - type: applies_to
    target: "Sheet Constraint Planning and Method Selection Flow"
tags: ["adam-rs", "filter", "method", "idempotence", "purity", "constraints"]
tier: knowledge
source: inferred
---

# Filter Purity and Idempotence Contract

adam-rs enforces strict contracts on Filter and Method callbacks:
- Filter (adam-rs/src/filter.rs:42-85): Idempotent, per-cell domain constraint with optional dynamic arguments attached via Sheet::add_filter (adam-rs/src/sheet.rs:751).
- Purity contract: All Filter and Method callbacks must be pure, deterministic functions of their inputs and immutable captured constants with no externally observable side effects.
- Idempotence contract: Filters and self-referencing methods must be idempotent. Ordinary methods require purity and determinism, not idempotence.
- Range-bound evaluators also obey the purity contract; the filter's conforming callback must be idempotent.

Aliases / also known as: Filter contract, filter idempotence, Method purity, constraint invariants, filter callbacks
