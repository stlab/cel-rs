---
title: "LSP isolates dependency groups before whole-collection module checks"
entity_type: discovery
confidence: 0.97
created: 2026-10-27T04:21:58Z
last_accessed: 2026-10-27T04:21:58Z
last_reinforced: 2026-10-27T04:21:58Z
access_count: 1
sources:
  - "adam-lsp/src/modules.rs:41-65"
  - "adam-lsp/src/modules.rs:83-145"
  - "adam-lsp/src/modules.rs:158-264"
  - "adam-lsp/src/modules.rs:273-285"
  - "adam-lsp/tests/modules.rs:180-374"
tags: ["lsp", "module-checking", "diagnostics", "configuration", "source-ownership"]
tier: knowledge
---

# LSP isolates dependency groups before whole-collection module checks

The core check_module analyzes the whole supplied collection and stops before body checking on resolution/signature failure. Therefore an LSP adapter cannot merely drop its parse-error gate: a malformed definition or unresolved importer can still erase unrelated semantic errors if the adapter initializes all URIs to empty replacements. The transport now derives undirected connected components from the existing structural AST's explicit imports and qualified component uses (including inactive/default branches), using current actual-URI overlays. It checks each group through real ModuleSet/check_module, retains rejected-source diagnostics, and publishes explicit incomplete-check status for inserted files in a failed group. Independent groups retain their actual errors or clean results. Fatal tokenization yields no outgoing structure but retains its syntax error; valid importers still supply incoming edges. Grouping is tooling isolation, not a second semantic checker or solver.

Configuration has a separate invariant: only initialization may default missing module options to source-only mode. Refresh requires an explicit moduleSources array (empty means deliberate removal) or moduleSourcesError. Invalid/missing/null/unrelated refreshes retain the last ownership mapping and report a visible blocking error until a valid snapshot recovers. Never silently switch registered files to standalone checking.

Evidence: adam-lsp/src/modules.rs:41-65,83-145,158-264,273-285; adam-lsp/tests/modules.rs:180-374. Reviewed regressions cover independent resolution/signature/body errors, malformed revisions, recovery, explicit removal and URI-overlay edge changes.

Related: [[module-checking-selects-registered-source-identity-and-checks-the-whole-collection.md|Whole-collection checking]] explains why the LSP isolates groups. [[lsp-source-spans-require-original-text-utf-16-projection-in-both-diagnostic-paths.md|UTF-16 diagnostic projection]] covers the resulting labels' transport coordinates.
