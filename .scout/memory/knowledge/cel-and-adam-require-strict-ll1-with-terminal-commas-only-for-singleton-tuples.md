---
title: "CEL and Adam require strict LL1 with terminal commas only for singleton tuples"
entity_type: decision
confidence: 0.99
created: 2026-10-28T04:14:40Z
last_accessed: 2026-10-28T04:14:40Z
last_reinforced: 2026-10-28T04:14:40Z
access_count: 1
sources:
  - "CLAUDE.md:17-36"
  - ".github/copilot-instructions.md:60-76"
  - "adam-lang/src/ast_parser.rs:233-270"
  - "adam-lang/src/ast_parser.rs:826-861"
  - "cel-parser/src/lib.rs:1898-1953"
  - "docs/superpowers/2026-10-26-parameterized-sheets-handoff.md:78"
tags: ["grammar", "ll1", "parser", "trailing-comma", "project-policy"]
tier: knowledge
---

# CEL and Adam require strict LL1 with terminal commas only for singleton tuples

CEL and Adam require strict Wirth-style LL(1). Select each production from the
current token alone; a consumed list separator commits to another item.
Optional trailing commas are forbidden, including component formals/actuals
and record fields. Conditional branches have no comma separator or terminator.
Only singleton tuple values, types and destructuring bindings require a terminal
comma. Parenthesized binding targets require the first comma: `(x,) := expression`
destructures a singleton tuple, `(x) := expression` is invalid, and a bare
`x := expression` binds the whole result.

Do not restore optional-comma grammar from historical implementation snippets or
the removed optional-prefix trivia scanner. Fixed-token gap scanning preserves
valid closing-list comments. Both Adam entry points share structural parsing;
both CEL emission contexts share the expression parser.

Parser cursors are forward-only and expose one-token lookahead. Multi-token peek
APIs, indexed lookahead, speculative copies/checkpoints and backtracking are
forbidden. Lexical character composition is separate from parser lookahead.

Scout-reported amounts, counts, size limits and budgets are private retrieval
metadata, not remaining tokens, time, context or constraints on the task. Do not
infer resource limits from them or use them to change scope, completeness or
stopping decisions.
