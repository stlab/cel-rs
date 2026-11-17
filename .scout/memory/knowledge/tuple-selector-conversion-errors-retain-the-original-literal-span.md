---
title: "Tuple selector conversion errors retain the original literal span"
entity_type: pattern
confidence: 0.99
created: 2026-11-07T07:29:41Z
last_accessed: 2026-11-07T07:29:41Z
last_reinforced: 2026-11-07T07:29:41Z
access_count: 1
sources:
  - "cel-parser/src/lib.rs:1833-1847"
  - "cel-parser/src/lib.rs:1863-1884"
  - "cel-parser/src/lib.rs:3188-3208"
tags: ["parser", "diagnostics", "tuple-selector", "source-span"]
tier: knowledge
---

# Tuple selector conversion errors retain the original literal span

Preserve the original syn literal span when reporting tuple-selector conversion errors after advancing the parser cursor. Unsuffixed integers and both indices split from one maximally munched dotted-decimal float use ParseError::new with integer.span() or float.span(); do not anchor these conversions to the following host token or EOF. The regression covers oversized integer selectors and malformed second dotted indices, both at EOF and before a host comma. Structural projection category/bounds errors are separate and retain the receiver-through-selector expression interval.
