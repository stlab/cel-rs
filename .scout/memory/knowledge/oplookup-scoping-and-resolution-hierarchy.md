---
title: "OpLookup Scoping and Resolution Hierarchy"
entity_type: concept
confidence: 0.80
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "cel-parser/src/op_table.rs:1669-1715"
relationships:
  - type: applies_to
    target: "CELParser and ParserContext Dual Emission"
tags: ["cel-parser", "op-lookup", "operator-resolution", "scope", "builtins"]
tier: knowledge
source: inferred
---

# OpLookup Scoping and Resolution Hierarchy

cel-parser's OpLookup (cel-parser/src/op_table.rs:1669-1715) manages operator and function dispatch across a strict hierarchical scope chain:
1. Dynamic user scopes (scopes: Vec<ScopeFn>): Checked in LIFO order via push_scope. Custom operator definitions take precedence over built-ins.
2. Library scopes: Checked after user scopes via push_library_scope.
3. Tuple-shaped operator signatures (tuple_signatures: Vec<TupleOpSignature>): Matched by element TypeId sequences via register_tuple_op.
4. Built-in scopes (BuiltinScope): Static phf_map table matching primitive operators by arity and operand TypeId.

When extending the operator set or integrating domain functions, register custom scopes or tuple signatures on OpLookup before passing it to CELParser.

Aliases / also known as: OpLookup, operator lookup, scope resolution, push_scope, built-in ops, operator table
