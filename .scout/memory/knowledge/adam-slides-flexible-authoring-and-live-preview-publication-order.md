---
title: "Adam slides flexible authoring and live preview publication order"
entity_type: pattern
confidence: 0.98
created: 2026-10-09T04:51:44Z
last_accessed: 2026-10-09T04:51:44Z
last_reinforced: 2026-10-09T04:51:44Z
access_count: 1
sources:
  - "adam-slides/deck.mjs:12-25"
  - "adam-slides/build.mjs:46-65"
  - "adam-slides/example.css:18-35"
  - "adam-slides/README.md"
tags: ["adam-slides", "marp", "live-preview", "authoring", "manifest"]
tier: knowledge
---

# Adam slides flexible authoring and live preview publication order

The tutorial is initial content, not an authoring constraint. `renderDeck` expands freely selected/repeated/reordered example directives and leaves text-only slides intact; graphs default off and `graph=1` enables one (adam-slides/deck.mjs:12-25). Build source resolution uses the staged book manifest plus deck-local `.adm2` files under reserved `local/` keys, rather than parsing tutorial includes or independently rereading book files (adam-slides/build.mjs:46-58). This makes displayed source and live mounted source identical and rejects references excluded from the live manifest. Crucial watcher publication order: write the completed runtime manifest BEFORE generated `dist/slides.md`, because Marp watches that Markdown and may immediately rebuild/reload the browser; print source readiness only after both writes finish (adam-slides/build.mjs:63-65). The full VS Code serve task sequences WASM/dependency/build preparation, source-watcher readiness, Marp-watcher readiness, then the HTTP server. Preview alone is static; both watchers are required for editing. Node --watch-path is Windows/macOS-only; ordinary static builds and browser tests remain Linux-compatible. For variable prose, the graph fills remaining frame space; a fixed 240px graph minimum caused avoidable scrolling in shorter panes, so graph-bearing main has fixed viewport height and the graph has flex:1/min-height:0 while the inspector does not shrink (adam-slides/example.css:18-35).
