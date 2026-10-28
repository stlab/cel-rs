---
title: "Module analysis requires output backing defaults even when an output has a writer"
entity_type: discovery
confidence: 0.97
created: 2026-10-27T23:46:30Z
last_accessed: 2026-10-27T23:46:30Z
last_reinforced: 2026-10-27T23:46:30Z
access_count: 1
sources:
  - "adam-lang/src/module/analysis.rs:725"
  - "adam-lang/tests/module_analysis.rs"
  - "docs/superpowers/2026-10-26-parameterized-sheets-handoff.md"
tags: ["module-checking", "outputs", "defaults", "nonexecuting-analysis"]
tier: knowledge
---

# Module analysis requires output backing defaults even when an output has a writer

An adm2 `out` declaration always materializes a default-initialized private backing cell before its writer runs. Therefore a compiled/typed writer does not exempt an output from registry default validation. The nonexecuting module checker uses `compiled.is_none() || kind == Kind::Out` at adam-lang/src/module/analysis.rs:725. Preserve the existing exemption for initialized ordinary cells and sources. Pure analysis must inspect default availability recursively without invoking default constructors, writers, or method factories. This keeps checking and compilation consistent for unused definitions and host types registered without defaults; arrays can still have empty defaults even when their element type lacks a default. Fixed with host callback-counter regressions in PR #265, commit 908ad9e.
