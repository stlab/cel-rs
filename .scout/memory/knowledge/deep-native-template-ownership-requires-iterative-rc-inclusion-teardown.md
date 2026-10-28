---
title: "Deep native template ownership requires iterative Rc inclusion teardown"
entity_type: discovery
confidence: 0.99
created: 2026-10-27T00:06:34Z
last_accessed: 2026-10-27T00:06:34Z
last_reinforced: 2026-10-27T00:06:34Z
access_count: 1
sources:
  - "adam-rs/src/template.rs:223-256"
  - "adam-rs/tests/templates.rs:374"
  - "adam-rs/tests/templates.rs:397"
  - "adam-rs/tests/templates.rs:424"
tags: ["adam-rs", "templates", "ownership", "rc", "deep-nesting", "root-cause"]
tier: knowledge
source: written
---

# Deep native template ownership requires iterative Rc inclusion teardown

Native SheetTemplate inclusion graphs own children through Rc. Explicit traversal stacks for instantiation do NOT prevent process-aborting stack overflow when the last root owner drops a deep chain. Task 2 review reproduced this against a freshly built integration test: 32,768 empty inclusions instantiated successfully, then ordinary sole-root drop aborted on a 2 MiB Rust thread. A test retaining every intermediate owner and dropping roots first hides this separate lifecycle bug. Commit 19cbe10 fixes library-owned inclusion teardown with Drop for SheetTemplate: take its relationship vector as a worklist; Rc::try_unwrap distinguishes a unique child, whose inclusion edges are moved to the worklist before its remaining fields drop, from a shared child, which must remain unchanged. Preserve shared factories, external child owners, repeated DAG edges and live instance independence. Do not remove iterative teardown or replace the ordinary sole-root regression with intermediate-owner retention. This does not change ordinary Rust semantics for user-supplied captured-value destructors.

Related: [[cel-cleanup-belongs-to-live-values-not-typed-instruction-plans.md|CEL value cleanup]] distinguishes live-value destruction from reusable instruction ownership.
