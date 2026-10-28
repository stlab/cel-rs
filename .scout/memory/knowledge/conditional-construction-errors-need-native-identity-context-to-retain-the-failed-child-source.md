---
title: "Conditional construction errors need native identity context to retain the failed child source"
entity_type: discovery
confidence: 0.98
created: 2026-10-27T02:49:25Z
last_accessed: 2026-10-27T02:49:33Z
last_reinforced: 2026-10-27T02:49:33Z
access_count: 2
sources:
  - "adam-rs/src/template/error.rs:146-163"
  - "adam-rs/src/template/instantiate.rs:108-128"
  - "adam-rs/src/template/instantiate.rs:508-522"
  - "adam-lang/src/composition.rs:1572-1609"
  - "adam-lang/tests/module_compile.rs:533-590"
tags: ["composition", "conditional", "provenance", "error-ownership", "source-identity"]
tier: knowledge
---

# Conditional construction errors need native identity context to retain the failed child source

Deferred conditional-key or match construction can fail after static emission succeeds. A bare ConditionalFactory/Runtime error has no cell slot, so a language adapter cannot identify the failed imported child's item from it. Returning the outer definition plus all reachable uses loses the actual child source and falsely suggests a failure path. Native instantiation now wraps conditional constructor failures in TemplateError::ConditionalConstruction { template, conditional, inclusions, source }: the local conditional registration ordinal and actual root-to-child inclusion slots identify the failed definition/item and repeated call site. The language adapter resolves these identities into owning SourceLabels and preserves the boxed original error and its concrete Error::source chain. Context allocation occurs only on failure. Language SourceId and native handle ownership use Rc and are not Send+Sync: keep context in Box<TemplateError>/ModuleCause rather than forcing it through anyhow or replacing it with display text. A cross-file regression fails in the second conditional, excludes completed/unexecuted sibling calls, drops all owners and still resolves the child span and concrete KeyFailure.

Evidence: adam-rs/src/template/error.rs:146-163; adam-rs/src/template/instantiate.rs:108-128 and :508-522; adam-lang/src/composition.rs:1572-1609; adam-lang/tests/module_compile.rs:533-590.

Related: [[deep-native-template-ownership-requires-iterative-rc-inclusion-teardown.md|Native template ownership]] covers recipe teardown independently of retained error identities.
