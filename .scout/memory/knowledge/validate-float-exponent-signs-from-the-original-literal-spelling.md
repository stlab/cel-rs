---
title: "Validate float exponent signs from the original literal spelling"
entity_type: discovery
confidence: 0.70
created: 2026-11-18T00:22:38Z
last_accessed: 2026-11-18T00:22:38Z
last_reinforced: 2026-11-18T00:22:38Z
access_count: 1
sources:
  - "cel-parser/src/lex_lexer.rs:185"
  - "cel-parser/src/lex_lexer.rs:385"
  - "cel-parser/src/lib.rs:394"
tags: ["lexer", "literals", "syn", "grammar"]
tier: knowledge
---

# Validate float exponent signs from the original literal spelling

Syn accepts exponent signs after underscores, but CEL rejects them: the exponent grammar places an optional sign immediately after `e` or `E`, before underscores. `normalize_literal` checks the original spelling, preserving spans, and both `LexLexer` and the direct literal-compilation path call it. Keep both entry points aligned when changing float lexing; `1e_+2f64` is invalid while `1e+_2f64` is valid.
