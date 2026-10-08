---
title: "Adam-lang Dual Parsing Pipeline"
entity_type: concept
confidence: 0.80
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "adam-lang/src/parser.rs:216-235"
  - "adam-lang/src/ast_parser.rs:15-40"
  - "adam-lang/src/type_registry.rs:130-151"
relationships:
  - type: applies_to
    target: "Architecture Overview"
  - type: relates_to
    target: "Adam-lsp Server Loop and Diagnostic Publishing Flow"
tags: ["adam-lang", "adam-parser", "ast-parser", "type-registry", "adm2"]
tier: knowledge
source: inferred
---

# Adam-lang Dual Parsing Pipeline

adam-lang provides two distinct parsers for .adm2 DSL files:
- AdamParser (adam-lang/src/parser.rs:216-235): Compiles DSL text directly into a live adam_rs::Sheet wrapped in ParsedSheet. Resolves type names to concrete Rust TypeIds via TypeRegistry (adam-lang/src/type_registry.rs:130-151) and compiles method formulas into bytecode using an embedded CELParser.
- AdamAstParser (adam-lang/src/ast_parser.rs:15-40): Pure syntactic parser that builds an abstract syntax tree (ast::Sheet) without requiring a TypeRegistry or OpLookup. Used for linting, syntax formatting (adam-fmt), and tooling that operates prior to type resolution.

Aliases / also known as: AdamParser, AdamAstParser, adm2 parsing, TypeRegistry, live sheet vs ast sheet, adam parser pipeline
