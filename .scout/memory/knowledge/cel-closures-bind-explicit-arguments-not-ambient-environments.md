---
title: "CEL closures bind explicit arguments, not ambient environments"
entity_type: pattern
confidence: 0.99
created: 2026-11-07T00:35:13Z
last_accessed: 2026-11-07T00:35:13Z
last_reinforced: 2026-11-07T00:35:13Z
access_count: 1
sources:
  - "cel-runtime/src/dyn_closure.rs:1-4"
  - "cel-runtime/src/dyn_closure.rs:438-486"
  - "cel-runtime/src/dyn_closure.rs:526-547"
  - "cel-runtime/src/dyn_segment.rs:1149-1162"
tags: ["cel", "closures", "prepared-execution", "ownership"]
tier: knowledge
---

# CEL closures bind explicit arguments, not ambient environments

CEL closures contain compiled code plus a declared signature, with no implicit captured environment. `DynClosure::typed` validates one result, exact declared positional reads, and clone capabilities. Its body must require neither preloaded arguments nor a nonzero ambient stack origin; zero-origin fragments remain valid. Preserve these checks when changing closure compilation or prepared invocation: instruction captures/client callbacks do not authorize borrowing an outer CEL evaluation environment. Evidence: cel-runtime/src/dyn_closure.rs:1-4,438-486,526-547; cel-runtime/src/dyn_segment.rs:1149-1162.
