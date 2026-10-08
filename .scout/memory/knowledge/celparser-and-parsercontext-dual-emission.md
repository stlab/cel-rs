---
title: "CELParser and ParserContext Dual Emission"
entity_type: concept
confidence: 0.80
created: 2026-10-08T18:52:02Z
last_accessed: 2026-10-08T18:52:02Z
last_reinforced: 2026-10-08T18:52:02Z
access_count: 1
sources:
  - "cel-parser/src/lib.rs:601-615"
  - "cel-parser/src/parser_context.rs:513-525"
  - "cel-parser/src/ast.rs:329-347"
relationships:
  - type: applies_to
    target: "Architecture Overview"
  - type: relates_to
    target: "OpLookup Scoping and Resolution Hierarchy"
tags: ["cel-parser", "parser-context", "ast-context", "dyn-segment-context", "cel-expression"]
tier: knowledge
source: inferred
---

# CELParser and ParserContext Dual Emission

cel-parser's recursive-descent parser (Parser<C: ParserContext>, cel-parser/src/lib.rs:601-615) is generic over the context it emits into:
- DynSegmentContext (cel-parser/src/parser_context.rs:513-525): Emits directly into cel_runtime::DynSegment bytecode, performing immediate operator resolution and runtime type verification. The common alias CELParser is defined as Parser<DynSegmentContext>.
- AstContext (cel-parser/src/ast.rs:329-347): Builds an un-evaluated, span-carrying Expr syntax tree without resolving types or consulting OpLookup. Never fails on semantic grounds; downstream consumers (adam-lang, adam-lsp, adam-fmt) perform deferred semantic checks and type resolution.

When adding grammar constructs or expressions, ensure both DynSegmentContext and AstContext implementations in parser_context.rs and ast.rs remain in parity.

Aliases / also known as: ParserContext, CELParser, AstContext, DynSegmentContext, parser emission, dual emission, cel parser ast
