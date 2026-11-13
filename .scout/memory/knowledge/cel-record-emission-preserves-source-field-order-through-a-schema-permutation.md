---
title: "CEL record emission preserves source field order through a schema permutation"
entity_type: pattern
confidence: 0.99
created: 2026-11-07T00:57:06Z
last_accessed: 2026-11-07T00:57:06Z
last_reinforced: 2026-11-07T00:57:06Z
access_count: 1
sources:
  - "cel-parser/src/registered_ops.rs:41-50"
  - "cel-parser/src/registered_ops.rs:69-90"
  - "cel-parser/src/registered_ops.rs:176-255"
tags: ["cel", "records", "emission", "evaluation-order"]
tier: knowledge
---

# CEL record emission preserves source field order through a schema permutation

CEL record construction separates expression evaluation order from canonical field layout. RecordEmitter receives a source-position-to-schema-position permutation and an ambient stack byte offset. The record lookup validates unknown, duplicate, missing, and mismatched complete field types before invoking the emitter. Preserve source-order field effects; reorder destinations with the validated mapping rather than reordering expression execution. Evidence: cel-parser/src/registered_ops.rs:41-50,69-90,176-255.
