---
title: "CEL Closure Grouping Preserves Precedence Grammar"
entity_type: decision
confidence: 1.00
created: 2026-10-29T00:43:34Z
last_accessed: 2026-10-29T00:45:28Z
last_reinforced: 2026-10-29T00:45:28Z
access_count: 2
sources:
  - "cel-parser/src/lib.rs:45-70"
  - "cel-parser/src/lib.rs:1308-1319"
tags: ["cel", "grammar", "closures", "ll1", "precedence", "design"]
tier: knowledge
---

# CEL Closure Grouping Preserves Precedence Grammar

CEL keeps operator precedence in its Wirth-style grammar, not in a separate token-recognition grammar plus binding table. `expression = closure_expression | range_expression`; the range/operator ladder reaches `primary_expression`, which excludes closures. Bare closures occupy complete-expression positions (root, arguments, tuple/array elements, branch and closure bodies); operator operands, cast inputs and postfix receivers require grouping. A closure body remains a complete expression, so `|| 1 + 2` returns `3`, while `(|| 1)()` invokes the grouped closure. Choosing `|` or `||` at the expression entry keeps FIRST sets disjoint and removes the need for four operand/tail states. Generic ParserContext emission and direct single-pass segment construction do not require treating closures as primaries. Keep module grammar, production contracts, formatter grouping and CEL book consistent when changing syntax.
