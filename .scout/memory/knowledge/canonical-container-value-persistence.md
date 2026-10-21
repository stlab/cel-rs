---
title: "Canonical Container Value Persistence"
entity_type: concept
confidence: 0.75
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "cel-runtime/src/array.rs"
  - "cel-runtime/src/owned_value.rs"
  - "cel-runtime/src/value_type.rs"
relationships:
  - type: applies_to
    target: "Four-Layer Stack Runtime Architecture"
tags: ["cel-runtime", "canonical-values", "tuples", "arrays", "value-persistence"]
tier: knowledge
source: inferred
---

# Canonical Container Value Persistence

cel-runtime segments use RawStack for temporary evaluation storage. Canonical arrays and tuple lists also persist outside evaluation:
- `Array<T>` owns a contiguous, Vec-compatible payload. `ArrayOf<E>` supplies the complete recursive element witness, including empty arrays.
- `Cons<H, T>` and `Nil` witness canonical heterogeneous tuple lists.
- `OwnedValue` owns one whole erased canonical value together with its `ValueType`. Checked typed conversion validates the full shape before moving. Optional clone, equality, and debug capabilities remain independent.
- The single erased host bridge imports concrete scalar boxes or complete `OwnedValue` owners. It never translates array or tuple elements to another representation.
- Adam compound cells own canonical values; Inspector labels retain a prepared debug capability and explicitly reject compound edits.

Future changes must preserve alignment, exact-once destruction, complete shape checks, and array payload allocation identity.

Aliases / also known as: canonical tuple, canonical array, erased whole-value owner, cel-runtime values, value persistence
