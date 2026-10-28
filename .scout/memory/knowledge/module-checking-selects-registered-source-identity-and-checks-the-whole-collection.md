---
title: "Module checking selects registered source identity and checks the whole collection"
entity_type: decision
confidence: 0.97
created: 2026-10-27T02:09:16Z
last_accessed: 2026-10-27T02:09:16Z
last_reinforced: 2026-10-27T02:09:16Z
access_count: 1
sources:
  - "adam-lang/src/module/analysis.rs:20-64"
  - "adam-lang/src/module/analysis.rs:241-265"
  - "adam-lang/tests/module_analysis.rs:133-233"
tags: ["parameterized-sheets", "module-checking", "source-identity", "ownership", "nonexecution"]
tier: knowledge
---

# Module checking selects registered source identity and checks the whole collection

The parameterized-sheet checker takes `check_module(&ModulePath, &ModuleSet, &TypeRegistry, &OpLookup)`, not an unassociated `ast::Module`: the registered collection owns the unique AST/source identity. A valid selected path checks every registered definition, including unrelated unused definitions; unknown selection produces a source-free diagnostic. Static analysis emits/checks retained expressions but never executes host defaults, initializers, methods, filters, requirements, or match expressions. Actual whole-graph planning and guard-independence validation remain construction-time; analysis success is not a solvability guarantee. The borrowed checked plans must be transferred to owning immutable recipes before capturing them in repeatable `'static` factories; never retain references into a consumed ModuleSet or clone live execution state.

Evidence: `adam-lang/src/module/analysis.rs:20-64` public contract and entry; `adam-lang/src/module/analysis.rs:241-265` borrowed Checked representation and analyzer; `adam-lang/tests/module_analysis.rs:133-233` nonexecution tests with positive callback controls. Design rationale: a path-selected checker avoids guessing which registered source an external AST replaces.

Related: [[issue-29-means-typed-parameterized-sheet-composition-not-equation-inference.md|Parameterized-sheet design]] defines composition semantics. [[lsp-isolates-dependency-groups-before-whole-collection-module-checks.md|LSP dependency isolation]] scopes collections for diagnostics. [[conditional-construction-errors-need-native-identity-context-to-retain-the-failed-child-source.md|Conditional construction provenance]] covers failures deferred until instantiation.
