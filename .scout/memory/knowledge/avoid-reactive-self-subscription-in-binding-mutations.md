---
title: "Avoid reactive self-subscription in binding mutations"
entity_type: discovery
confidence: 0.70
created: 2026-11-08T23:23:18Z
last_accessed: 2026-11-08T23:23:18Z
last_reinforced: 2026-11-08T23:23:18Z
access_count: 1
sources:
  - "adam-web-ui/src/binding/dioxus.rs:207"
  - "adam-web-ui/src/binding/dioxus.rs:625"
  - "adam-slides/tests/slides.spec.mjs:251"
tags: ["dioxus", "bindings", "slider", "reactivity"]
tier: knowledge
---

# Avoid reactive self-subscription in binding mutations

SheetHandle::instance tracks the publication revision for reactive views. CellBinding mutation and reporting paths must check the state identity without tracking: the upstream slider calls submit from a reactive movement effect, so subscribing that effect to the revision published by its own submission causes a runaway transaction loop and freezes pointer dragging. Exercise real pointer down/move/up in browser regressions; editing the slider's numeric textbox does not cover this path.
