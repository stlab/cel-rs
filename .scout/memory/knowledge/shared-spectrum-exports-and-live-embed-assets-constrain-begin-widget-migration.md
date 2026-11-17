---
title: "Shared controls and live-embed assets constrain widget changes"
entity_type: discovery
confidence: 0.99
created: 2026-11-05T21:58:41Z
last_accessed: 2026-11-05T22:11:59Z
last_reinforced: 2026-11-05T22:11:59Z
access_count: 2
sources:
  - "ez-adam/src/ui/side_panel.rs:17"
  - "adam-web-ui/src/lib.rs:1-13"
  - "ez-adam/src/ui/app.rs:445"
  - "adam-lang-book-live/src/lib.rs:470-477"
  - "xtask/src/live_book_assets.rs:121-147"
  - "xtask/src/live_book_assets.rs:161-220"
  - "xtask/src/live_book_assets.rs:276-295"
  - "adam-lang-book/book-src/theme/adam-live-bootstrap.js:52-59"
  - "adam-lang-book/tests/bootstrap.test.cjs:179-190"
tags: ["dioxus", "spectrum", "shared-assets"]
tier: knowledge
---

# Shared controls and live-embed assets constrain widget changes

Shared widget changes require coordinating all consumers, not only Begin. `adam_web_ui::components` exports model-independent controls and `Theme`. EZ Adam's document-only side panel imports these controls, and both its application root and the book live root render `Theme`.

Book and slides share `prepare_live_assets`, which stages `adam-controls.css` from `components::CONTROL_CSS`, `inspector.css`, graph assets, and the compiled live-example runtime. It does not require or copy `swc.js`; the staging regression test explicitly checks its absence. Browser `dependencyScripts(hasGraphs)` loads `d3.v7.min.js` and `graph.js` only for graph pages and returns no scripts for graph-free pages.

Coordinate shared component exports, callers, themes, bootstrap, staging, and associated tests when changing widgets. Do not restore Spectrum exports, `SpTheme`, or `swc.js` as migration prerequisites. This coupling does not require adding runtime Sheet ownership to EZ Adam.
