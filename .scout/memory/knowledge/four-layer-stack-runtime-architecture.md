---
title: "Four-Layer Stack Runtime Architecture"
entity_type: concept
confidence: 0.80
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "cel-runtime/src/raw_stack.rs:6-35"
  - "cel-runtime/src/raw_segment.rs:12-35"
  - "cel-runtime/src/dyn_segment.rs:680-730"
  - "cel-runtime/src/segment.rs:28-70"
relationships:
  - type: applies_to
    target: "Architecture Overview"
  - type: relates_to
    target: "DynamicSequence and DynamicArray Value Persistence"
tags: ["cel-runtime", "raw-stack", "raw-segment", "dyn-segment", "segment", "type-safety"]
tier: knowledge
source: inferred
---

# Four-Layer Stack Runtime Architecture

cel-runtime implements an expression evaluator structured in four distinct layers of increasing type safety:
1. RawStack (cel-runtime/src/raw_stack.rs:6-35): Low-level, byte-aligned unsafe stack over RawVec. Values are stored as raw bytes respecting the stack's base alignment (with_base_alignment).
2. RawSegment (cel-runtime/src/raw_segment.rs:12-35): Operation list containing type-erased closure storage and per-operation droppers.
3. DynSegment (cel-runtime/src/dyn_segment.rs:680-730): Runtime type-checked segment tracking stack_ids (Vec<StackInfo>), argument type IDs, and base stack offset. Validates operand types when composing operations.
4. Segment<Args, Stack> (cel-runtime/src/segment.rs:28-70): Compile-time zero-cost phantom wrapper tracking argument types (Args: IntoList) and type stack (Stack: List) using cons-cell heterogeneous lists (CStackList / CNil).

Operations are appended via opN (e.g. op1, op2) or fallible variants opNr (e.g. op1r, op2r) returning std::result::Result.

Aliases / also known as: four-layer stack, cel-runtime layers, RawStack, RawSegment, DynSegment, Segment, c_stack_list, type-level stack
