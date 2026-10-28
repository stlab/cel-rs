---
title: "LSP source spans require original-text UTF-16 projection in both diagnostic paths"
entity_type: discovery
confidence: 0.99
created: 2026-10-27T05:01:12Z
last_accessed: 2026-10-27T05:01:12Z
last_reinforced: 2026-10-27T05:01:12Z
access_count: 1
sources:
  - "adam-lsp/src/diagnostics.rs:88"
  - "adam-lsp/src/diagnostics.rs:101"
  - "adam-lsp/src/diagnostics.rs:125"
  - "adam-lsp/src/modules.rs:388"
  - "adam-lsp/tests/modules.rs:180"
  - "adam-lsp/tests/modules.rs:268"
tags: ["adam-lsp", "unicode", "diagnostics", "source-ownership", "parameterized-sheets"]
tier: knowledge
---

# LSP source spans require original-text UTF-16 projection in both diagnostic paths

Adam/CEL SourceSpan columns count Unicode scalar characters, while LSP columns count UTF-16 code units. An ASCII/BMP approximation produces wrong offsets after a non-BMP character even when that character appears only in a preceding comment. Both standalone and registered-module diagnostics must project against their original owning source text with the shared source_range helper, including related-information labels. Do not copy raw columns or project imported spans against the caller's text. The standalone projection was corrected during the parameterized-sheet final audit. Parsed lexical failures retain their existing point spans; conversion must not manufacture token lengths. Evidence: adam-lsp/src/diagnostics.rs:88,101,125; adam-lsp/src/modules.rs:388; real source-owned cross-file and standalone transport regressions at adam-lsp/tests/modules.rs:180,268.
