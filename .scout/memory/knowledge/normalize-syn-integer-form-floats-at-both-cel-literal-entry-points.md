---
title: "Normalize Syn integer-form floats at both CEL literal entry points"
entity_type: discovery
confidence: 0.70
created: 2026-11-17T23:00:02Z
last_accessed: 2026-11-18T03:00:42Z
last_reinforced: 2026-11-18T03:00:42Z
access_count: 4
sources:
  - "cel-parser/src/lex_lexer.rs:189-194"
  - "cel-parser/src/lex_lexer.rs:387-402"
  - "cel-parser/src/lib.rs:392-394"
  - "cel-parser/src/lib.rs:590-592"
  - "cel-parser/src/lib.rs:616-619"
tags: ["cel-parser", "literals", "syn"]
tier: knowledge
---

# Normalize Syn integer-form floats at both CEL literal entry points

Syn initially classifies decimal `1f32` and `1f64` as `Lit::Int`. CEL normalizes those tokens to `Lit::Float` both in `LexLexer` and when validating or retaining a directly supplied Syn literal. Preserve the original token and span, and leave radix-prefixed integers and other suffixes unchanged.
