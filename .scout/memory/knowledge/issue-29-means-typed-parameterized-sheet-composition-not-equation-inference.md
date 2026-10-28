---
title: "Issue 29 means typed parameterized-sheet composition, not equation inference"
entity_type: decision
confidence: 0.99
created: 2026-10-26T23:33:25Z
last_accessed: 2026-10-26T23:33:25Z
last_reinforced: 2026-10-26T23:33:25Z
access_count: 1
sources:
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:5"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:8"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:123"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:145"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:170"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:198"
  - "docs/superpowers/specs/2026-10-25-parameterized-sheets-design.md:231"
tags: ["adam-lang", "adam-rs", "parameterized-sheets", "composition", "issue-29", "approved-design"]
tier: knowledge
---

# Issue 29 means typed parameterized-sheet composition, not equation inference

The user-approved #29 design replaces its original equation-inference scope with explicitly parameterized reusable sheets; this is an approved design, not a claim that implementation is complete. `sheet helper(a: f64, b: f64)` defines a component; `relationship helper(a: low, b: high);` binds existing caller cells by name. Native construction must work without parsing CEL/Adam text. Expansion behaves like depth-first inline-at-use construction into ONE planning graph, preserving bound-cell identities and priorities. Module resolution uses client-supplied collections, not filesystem IO. Crucial source-specialization invariant: if ANY mapped output of a method is Source, remove the ENTIRE method; never project away individual outputs. Reject an invocation when a constituent relationship loses all directions, even in an inactive branch. Conditional uses must be transitively relationship-only; pruning does not make a stateful component eligible. Out-cell arguments, language generics, inferred equations and general conditional component state remain deferred to #261–#264. See approved design and implementation plan before continuing work.

Related: [[module-checking-selects-registered-source-identity-and-checks-the-whole-collection.md|Module checking]] records the static-analysis boundary for this design.
