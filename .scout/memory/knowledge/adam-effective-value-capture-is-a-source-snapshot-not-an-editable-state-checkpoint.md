---
title: "Adam effective-value capture is a source snapshot, not an editable-state checkpoint"
entity_type: decision
confidence: 1.00
created: 2026-11-04T21:23:53Z
last_accessed: 2026-11-04T21:23:53Z
last_reinforced: 2026-11-04T21:23:53Z
access_count: 1
sources:
  - "adam-rs/src/sheet.rs:2222"
  - "adam-rs/src/lib.rs:157"
relationships:
  - type: relates_to
    target: "Adam Forced Classification Is Not Free-Source Feasibility"
tags: ["adam-rs", "capture", "playback", "source-cells", "invariants"]
tier: knowledge
source: Public capture contracts in adam-rs/src/sheet.rs and adam-rs/src/lib.rs.
---

# Adam effective-value capture is a source snapshot, not an editable-state checkpoint

Use `Sheet::source_cells(&[])` for whole-sheet capture, not just `is_source()`: capture includes both unclaimed cells and selected method outputs that also appear in that method's inputs. Self-referencing inputs can influence evaluation even while claimed. The iterator excludes pure derived outputs and `Out` cells, yields each cell once, and borrows the last successful plan. Whole-sheet queries allocate nothing (`adam-rs/src/sheet.rs:2222`).

Pass a nonempty slice of result cells to capture only their contributing snapshot sources. Trace selected producer paths, conforming filter arguments, and relevant conditional selectors; union overlapping cones without duplicates. Do not reuse the broader widget-relevance `contributing_cells` query: alternative directions can name derived cells that must not become high-priority replay writes. Scoped traversal uses owned work sets but never copies cell values (`adam-rs/src/sheet.rs:2222`).

Capture after successful propagation and before further edits, reading effective `read<T>()` values rather than `source<T>()` aspirations. Replay writes every captured value in any order as one batch before propagation. Reproduction of the requested results requires a settled assignment satisfying relationships/filters, a compatible graph, and matching uncaptured inputs. Crucially, every directional method must preserve an already-satisfied assignment; individual idempotence and the runtime's shared-cell-set validation alone are insufficient. Self-referencing corrections and filters must also be idempotent (`adam-rs/src/lib.rs:157`).

This reconstructs current effective values, not original aspirations, priority history, diagnostics, or later spring-back. Clients own their post-construction baseline, change/relevance filtering, stable schema identities, and checked typed codecs. Do not persist live `CellId`s or add serialization traits to runtime type erasure for this feature. An omitted input becomes replay context that must match (`adam-rs/src/lib.rs:167`).
