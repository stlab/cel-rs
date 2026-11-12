---
title: "LexLexer compounds preserve first-punctuation spans and group-local adjacency"
entity_type: pattern
confidence: 1.00
created: 2026-11-07T02:18:15Z
last_accessed: 2026-11-07T02:18:15Z
last_reinforced: 2026-11-07T02:18:15Z
access_count: 1
sources:
  - "cel-parser/src/lex_lexer.rs:123-134"
  - "cel-parser/src/lex_lexer.rs:207-224"
  - "cel-parser/src/lex_lexer.rs:247-250"
  - "cel-parser/src/error.rs:108-130"
tags: ["lexer", "source-spans", "parity"]
tier: knowledge
---

# LexLexer compounds preserve first-punctuation spans and group-local adjacency

LexLexer combines operators only from joint punctuation within the same group. A combined token retains the first punctuation's span rather than the full compound spelling; group delimiters retain the group's span. Preserve these rules when changing lexing or comparing diagnostics with another token producer. A decoded-text adapter that uses full-operator byte spans must label that difference and normalize ranges deliberately rather than claim strict span parity. Source: cel-parser/src/lex_lexer.rs:123-134,207-224,247-250; source-span byte conversion is cel-parser/src/error.rs:108-130.
