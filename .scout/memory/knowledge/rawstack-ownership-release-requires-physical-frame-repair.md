---
title: "RawStack Ownership Release Requires Physical Frame Repair"
entity_type: pattern
confidence: 0.98
created: 2026-10-28T21:11:53Z
last_accessed: 2026-10-28T21:11:53Z
last_reinforced: 2026-10-28T21:11:53Z
access_count: 1
sources:
  - "cel-runtime/src/raw_stack.rs:169"
  - "cel-runtime/src/raw_stack.rs:769"
  - "cel-runtime/src/container_execution.rs:437"
  - "cel-runtime/src/dyn_segment.rs:895"
tags: ["runtime", "ownership", "unsafe", "unwinding"]
tier: knowledge
source: RT-04 root cause and accepted fix 5ba0a5d; scoped runtime spec/quality re-review passed.
relationships:
  - type: relates_to
    target: "CEL cleanup belongs to live values, not typed instruction plans"
---

# RawStack Ownership Release Requires Physical Frame Repair

RawStack typed destruction derives a value's address from the current byte-buffer length, rather than a saved owner offset. RawStack::release_top removes ownership metadata without truncating the bytes. Releasing a suffix while retaining an owned prefix can therefore redirect that prefix's destructor into the suffix bytes.

release_top is unsafe. A caller must restore the retained owners' physical length, values, and padding before owner-based operations or destruction, including unwinding. If metadata is released before physical repair, perform the repair immediately, without intervening assertions, allocation, callbacks, or other potentially panicking operations. Container result transfers and dynamic array collapse complete release/truncation before subsequent effects. Typed pop instead restores the physical prefix before releasing its matching metadata.

When no owners remain, extra unowned bytes do not themselves make destruction invalid. Leaking a released resource is distinct from reusing an abandoned transferred or destroyed representation; the latter can violate memory safety. Regression coverage should use compile-time safe-call rejection and valid completed-transfer normal/unwind tests, never execute an invalid owning-pointer specimen.
