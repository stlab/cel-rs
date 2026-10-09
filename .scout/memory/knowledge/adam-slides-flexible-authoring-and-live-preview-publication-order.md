---
title: "Adam slides flexible authoring and live preview publication order"
entity_type: pattern
confidence: 0.99
created: 2026-10-09T04:51:44Z
last_accessed: 2026-10-09T05:37:53Z
last_reinforced: 2026-10-09T05:37:53Z
access_count: 2
sources:
  - "adam-slides/deck.mjs:12-25"
  - "adam-slides/build.mjs:46-65"
  - "adam-slides/example.css:18-35"
  - "adam-slides/README.md"
  - "adam-slides/build.mjs:37-86"
tags: ["adam-slides", "marp", "live-preview", "authoring", "manifest"]
tier: knowledge
---

# Adam slides flexible authoring and live preview publication order

The tutorial seeds initial content, not authoring constraints. renderDeck expands freely selected/repeated/reordered directives and leaves text-only slides intact; graphs default off and graph=1 enables one (adam-slides/deck.mjs:12-25). Book sources come from the shared live manifest, merged with deck-local .adm2 files under reserved local/ keys, so displayed source and mounted source use identical bytes (adam-slides/build.mjs:46-58).

Important failure boundary: prepareDeck stages the book-only manifest and runtime assets into an owned temporary directory using xtask's optional destination, then calls publishDeck, and removes staging in finally (adam-slides/build.mjs:76-86). publishDeck validates required assets, local names, selectors and all referenced sources BEFORE touching published dist files. Invalid authoring leaves the previous published manifest and Markdown unchanged. Do not stage directly into dist before validation: an earlier implementation overwrote its merged manifest with book-only sources, then failed on a directive/local-name error, breaking local frames or mismatching displayed and mounted source after reload.

Publication order remains essential: write the merged manifest privately, copy the validated runtime assets including that merged manifest to dist/theme, THEN write generated dist/slides.md and emit readiness (adam-slides/build.mjs:59-66). Marp watches that Markdown and immediately rebuilds/reloads the browser. The full VS Code task sequences WASM/dependency/build preparation, source readiness, Marp readiness, then the HTTP server. Preview alone is static; both watchers are required. Node --watch-path is Windows/macOS-only; static builds/browser tests support Linux.

Variable prose uses flexible slide columns. The graph fills remaining frame space instead of a fixed 240px minimum: graph-bearing main has fixed viewport height, the graph flex:1/min-height:0, and the inspector does not shrink (adam-slides/example.css:18-35).
