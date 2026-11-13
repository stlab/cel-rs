---
title: "CEL record fields use typed member operation dispatch"
entity_type: pattern
confidence: 1.00
created: 2026-11-07T02:49:40Z
last_accessed: 2026-11-07T02:49:40Z
last_reinforced: 2026-11-07T02:49:40Z
access_count: 1
sources:
  - "cel-parser/src/lib.rs:1809-1826"
  - "cel-parser/src/parser_context.rs:846-858"
  - "cel-parser/src/op_table.rs:2030-2051"
  - "cel-parser/src/op_table.rs:2081-2095"
  - "cel-parser/src/registered_ops.rs:414-435"
  - "cel-euclid/src/geometry.rs:48-58"
  - "cel-euclid/src/geometry.rs:160-178"
tags: ["records", "member-dispatch", "parity"]
tier: knowledge
---

# CEL record fields use typed member operation dispatch

Named postfix selection calls apply_member with receiver-plus-explicit-argument arity and member=true. Member lookup selects direct operation dispatch, checks eligible scopes then exact registered operand signatures, emits the operation, and annotates its complete result. Concrete record field getters are registered unary callbacks taking their native receiver by value; a port's generic canonical record projection should be documented as an interface/carrier adaptation rather than a same-named Rust primitive. Preserve member lookup and arity semantics when changing record support or ordinary callable-name resolution.
