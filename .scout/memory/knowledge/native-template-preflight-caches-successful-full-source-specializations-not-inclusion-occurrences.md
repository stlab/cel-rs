---
title: "Native template preflight caches successful full-source specializations, not inclusion occurrences"
entity_type: discovery
confidence: 0.97
created: 2026-10-27T23:46:24Z
last_accessed: 2026-10-27T23:46:24Z
last_reinforced: 2026-10-27T23:46:24Z
access_count: 1
sources:
  - "adam-rs/src/template/instantiate.rs:366"
  - "adam-rs/src/template/instantiate.rs:421"
  - "docs/superpowers/2026-10-26-parameterized-sheets-handoff.md"
tags: ["parameterized-sheets", "preflight", "dag", "source-specialization"]
tier: knowledge
---

# Native template preflight caches successful full-source specializations, not inclusion occurrences

Parameterized-sheet source preflight must traverse distinct (TemplateId, complete source-mask) specializations, not unfold the inclusion DAG into occurrences. A compact doubled empty DAG previously caused exponential checking/compilation work. The call-local completion cache in adam-rs/src/template/instantiate.rs:366 and its lookup at :421 record only successfully completed checks after deriving the full mask, including private sources and actual bindings. Do not cache active frames or failures: errors must retain the actual failing inclusion ancestry. Do not apply this deduplication to live expansion: repeated inclusions require distinct runtime occurrences and fresh resources. Distinct complete masks can themselves be exponentially numerous; complexity contracts explicitly preserve that limit. Fixed and regression-tested in PR #265, commit 908ad9e.
