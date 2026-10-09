---
title: "Architecture Overview"
entity_type: concept
confidence: 0.80
created: 2026-10-08T18:51:41Z
last_accessed: 2026-10-08T18:52:53Z
last_reinforced: 2026-10-08T18:51:41Z
access_count: 2
sources:
  - "cel-runtime/src/lib.rs:1-35"
  - "cel-parser/src/lib.rs:555-615"
  - "adam-rs/src/sheet.rs:320-370"
  - "adam-lang/src/parser.rs:215-235"
relationships:
  - type: relates_to
    target: "Ez-adam Document Model and Node Placements Separation"
  - type: relates_to
    target: "Clippy and Feature Flag Validation Matrix"
  - type: relates_to
    target: "Contract-Style Documentation and Precondition Invariants"
  - type: relates_to
    target: "Git Worktree and No-Direct-Commit Policy"
  - type: relates_to
    target: "Vendored Spectrum Bundle Maintenance via xtask"
tags: ["architecture", "cel-rs", "adam-rs", "cel-runtime", "cel-parser", "overview"]
tier: knowledge
source: inferred
---

# Architecture Overview

The cel-rs repository is a property model and CEL expression evaluation system organized across modular Rust crates:
- cel-runtime: Core stack-based expression evaluator layered into RawStack, RawSegment, DynSegment, and Segment<Args, Stack>.
- cel-parser: Recursive descent parser generic over ParserContext, emitting into executable DynSegment bytecode (DynSegmentContext) or span-carrying ASTs (AstContext).
- cel-rs-macros: Compile-time CEL validation procedural macros.
- cel-std: Built-in standard library operations for CEL.
- adam-rs: Multi-way constraint graph engine managing cells, directional methods, filters, conditionals, and planner release/seed resolution.
- adam-lang: DSL parser (.adm2 files) compiling into live adam-rs Sheets (AdamParser) or abstract syntax trees (AdamAstParser).
- adam-lsp: Language server providing real-time diagnostics, type validation, and formatting for .adm2 documents.
- ez-adam: Visual graph editor document model separating logical cells from canvas node placements.
- begin: Desktop-first Dioxus property-model development environment with interactive graph inspection.
- xtask: Workspace automation commands (build-js, fetch-assets).

First-to-read entry points: cel-runtime/src/lib.rs:1, cel-parser/src/lib.rs:555, adam-rs/src/sheet.rs:320, adam-lang/src/parser.rs:215.

Aliases / also known as: cel-rs, cel_rs, property model system, adam architecture, adam workspace, cel-rs workspace
