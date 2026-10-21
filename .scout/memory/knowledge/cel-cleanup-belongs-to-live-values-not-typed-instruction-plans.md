---
title: "CEL cleanup belongs to live values, not typed instruction plans"
entity_type: decision
confidence: 0.98
created: 2026-10-20T22:04:31Z
last_accessed: 2026-10-20T22:04:31Z
last_reinforced: 2026-10-20T22:04:31Z
access_count: 1
sources:
  - "cel-runtime/src/raw_stack.rs:11"
  - "cel-runtime/src/raw_stack.rs:186"
  - "cel-runtime/src/raw_stack.rs:308"
  - "cel-runtime/src/raw_segment.rs:383"
  - "cel-runtime/src/canonical_operation.rs:39"
  - "cel-runtime/src/array.rs:34"
  - "cel-runtime/src/array.rs:64"
  - "cel-runtime/src/container_execution.rs:250"
  - "cel-runtime/src/container_execution.rs:404"
  - "cel-runtime/src/container_execution.rs:417"
  - "cel-runtime/src/container_execution.rs:453"
  - "cel-runtime/src/container_execution.rs:475"
tags: ["cel-runtime", "ownership", "unwinding", "allocation", "value-type", "unsafe"]
tier: knowledge
source: User-approved redesign implemented in the uncommitted worktree-simple-straighten working tree during this session; verify merge status before assuming these APIs exist on another branch.
---

# CEL cleanup belongs to live values, not typed instruction plans

## Design decision
Ordinary typed CEL instructions already know operand/result representations; retaining recursive `ValueType` plans and ambient cleanup graphs in each callback is unnecessary. `OperationPlan` is now transient emission-time validation, and typed emit0–emit3 capture only actual user callback state, not the plan (`cel-runtime/src/canonical_operation.rs:39`). Existing `ValueType` Arc trees remain allowed for builder stack information and genuinely erased ownership/layout; this change does not flatten the descriptor representation.

## Ownership invariants
- `RawStack` owns live values through a flat destructor-function-pointer ledger. Separate raw-slot side metadata supplies descriptors only when destruction is erased (`cel-runtime/src/raw_stack.rs:11`). Track logical depth as well as byte offsets: multiple owned ZSTs can share a byte boundary.
- Safe owned `push<T>` requires `T: 'static`: a nongeneric owning stack cannot encode shorter borrowed destructor lifetimes (`cel-runtime/src/raw_stack.rs:186`).
- Raw initialization registers an owner only after the initializer succeeds. Partial initialization needs its own guard; byte rollback alone cannot destroy initialized children (`cel-runtime/src/raw_stack.rs:308`).
- Raw byte moves/truncation do not infer ownership transfer. Release source ledger entries exactly once before truncation (`cel-runtime/src/container_execution.rs:250`, `:417`, `:475`). After successful erased writes/clones, bind destruction with `own_top` (`cel-runtime/src/container_execution.rs:404`, `:453`). Missing either step causes leaks or double destruction.
- Segment-level execution guards clean owned suffixes on errors/panics, preserving the caller prefix. Scope metadata records the byte origin and initial owned input count; fragments must not inherit a root origin accidentally (`cel-runtime/src/raw_segment.rs:383`).
- `RawArray` itself owns member destruction: typed arrays bind concrete drop glue, while erased arrays retain optional element layout/context. `Array<T>` delegates destruction to its header; do not also recursively destroy the same array payload externally (`cel-runtime/src/array.rs:34`, `:64`). Inline typed tuples remain `CStackList`; erased tuple destruction context travels with live stack ownership/`OwnedValue` rather than a new tuple header.

Implementation provenance: these changes were uncommitted in worktree-simple-straighten when recorded; inspect the current branch before applying API-specific advice.
