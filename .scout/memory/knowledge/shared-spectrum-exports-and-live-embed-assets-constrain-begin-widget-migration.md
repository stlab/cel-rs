---
title: "Shared Spectrum exports and live-embed assets constrain Begin widget migration"
entity_type: discovery
confidence: 0.99
created: 2026-11-05T21:58:41Z
last_accessed: 2026-11-05T22:11:59Z
last_reinforced: 2026-11-05T22:11:59Z
access_count: 2
sources:
  - "ez-adam/src/ui/side_panel.rs:17"
  - "adam-web-ui/src/lib.rs:12"
  - "ez-adam/src/ui/app.rs:435-441"
  - "begin/src/app.rs:332"
  - "xtask/src/live_book_assets.rs:1-7"
  - "adam-lang-book-live/src/lib.rs:99-132"
  - "xtask/src/live_book_assets.rs:161-173"
  - "xtask/src/live_book_assets.rs:208-212"
  - "adam-lang-book/book-src/theme/adam-live-bootstrap.js:23-29"
tags: ["dioxus", "spectrum", "shared-assets"]
tier: knowledge
---

# Shared Spectrum exports and live-embed assets constrain Begin widget migration

Shared widget changes require coordinating all consumers, not only Begin. EZ Adam's document-only side panel imports adam_web_ui::spectrum controls (ez-adam/src/ui/side_panel.rs:17), publicly exported at adam-web-ui/src/lib.rs:12; its root loads a separate Spectrum bundle (ez-adam/src/ui/app.rs:435-441). The book live root renders SheetInspector and SpTheme (adam-lang-book-live/src/lib.rs:99-132). Book and slides share prepare_live_assets, which requires and copies begin/assets/swc.js plus inspector/graph assets (xtask/src/live_book_assets.rs:161-173,208-212); browser dependencyScripts unconditionally loads swc.js (adam-lang-book/book-src/theme/adam-live-bootstrap.js:23-29). A component migration must update callers, themes, bootstrap, staging and associated tests together before removing old exports or bundles. This coupling does not require adding runtime Sheet ownership to EZ Adam.
