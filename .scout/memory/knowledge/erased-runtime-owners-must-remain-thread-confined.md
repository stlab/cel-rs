---
title: "Erased Runtime Owners Must Remain Thread-Confined"
entity_type: decision
confidence: 0.98
created: 2026-10-28T20:36:34Z
last_accessed: 2026-10-28T20:36:34Z
last_reinforced: 2026-10-28T20:36:34Z
access_count: 1
sources:
  - "cel-runtime/src/raw_stack.rs:7-29"
  - "cel-runtime/src/raw_segment.rs:68-90"
relationships:
  - type: relates_to
    target: "Four-Layer Stack Runtime Architecture"
tags: ["cel-runtime", "ownership", "type-erasure", "send", "sync", "thread-confinement"]
tier: knowledge
source: verified code and compile-fail contracts
---

# Erased Runtime Owners Must Remain Thread-Confined

RawStack owns erased live values, and RawSegment owns erased instruction captures. Their registration/storage interfaces do not require those values or captures to implement Send or Sync. Raw byte storage must not accidentally grant these typed owners Send/Sync: moving an erased Rc across threads bypasses Rust's normal safety boundary.

Both actual erased owners carry a zero-sized PhantomData<Rc<()>> marker, keeping them !Send and !Sync without imposing stronger requirements on stored values or adding synchronization. Preserve confinement when refactoring their storage or wrappers. A future parallel-execution design must establish appropriate bounds at every ownership/registration boundary before relaxing this invariant; an unsafe Send implementation on the byte buffer is not sufficient.

Validate the boundary with compile-fail examples that attempt to move an Rc-owning stack/program into thread::spawn, rather than executing an unsafe cross-thread specimen. Keep the restriction on the erased owners rather than indiscriminately restricting untyped byte-storage utilities.

Evidence: cel-runtime/src/raw_stack.rs:7-29 documents live ownership, demonstrates rejected thread transfer, and stores the marker. cel-runtime/src/raw_segment.rs:68-90 documents unrestricted capture types and stores the corresponding marker.
