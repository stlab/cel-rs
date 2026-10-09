---
title: "DynamicSequence and DynamicArray Value Persistence"
entity_type: concept
confidence: 0.75
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "cel-runtime/src/dynamic_sequence.rs:487-535"
  - "cel-runtime/src/dynamic_array.rs:777-810"
relationships:
  - type: applies_to
    target: "Four-Layer Stack Runtime Architecture"
tags: ["cel-runtime", "dynamic-sequence", "dynamic-array", "tuples", "arrays", "value-persistence"]
tier: knowledge
source: inferred
---

# DynamicSequence and DynamicArray Value Persistence

cel-runtime segments use RawStack for temporary evaluation storage. DynamicSequence and DynamicArray provide owned, type-erased tuple and array values that can outlive a segment evaluation:
- DynamicSequence (cel-runtime/src/dynamic_sequence.rs:487-535): Owned, type-erased tuple storing heterogeneous values within a dedicated RawStack buffer with element shape descriptors (shape: Vec<SequenceElement>, max_align). Converts to and from concrete Rust tuples via the TupleSequence trait.
- DynamicArray (cel-runtime/src/dynamic_array.rs:777-810): Owned, type-erased homogeneous array backed by a Vec-compatible heap allocation (ptr, len, capacity, element: ArrayElementType). Supports zero-copy conversion from concrete vectors via DynamicArray::try_from_vec.

Future changes to tuple or array representation must maintain memory alignment and drop safety across these two persistent types.

Aliases / also known as: DynamicSequence, DynamicArray, type-erased tuple, dynamic array, cel-runtime values, value persistence
