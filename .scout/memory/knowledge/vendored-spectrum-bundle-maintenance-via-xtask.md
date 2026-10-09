---
title: "Vendored Spectrum Bundle Maintenance via xtask"
entity_type: procedure
confidence: 0.85
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "xtask/src/main.rs:60-101"
  - "begin/src/main.rs:1-30"
relationships:
  - type: applies_to
    target: "Architecture Overview"
tags: ["xtask", "spectrum", "swc-js", "assets", "build-js"]
tier: knowledge
source: inferred
---

# Vendored Spectrum Bundle Maintenance via xtask

begin and ez-adam vendor their Adobe Spectrum Web Components bundles directly in source control at assets/swc.js so developers can build the Rust application without a Node.js toolchain:
- Bundle build command: `cargo xtask build-js` runs `npm ci && npm run build` in begin/ and ez-adam/ (xtask/src/main.rs:60-101).
- Assets download: `cargo xtask fetch-assets` downloads external assets.
- Rule: Never edit assets/swc.js manually. Regenerate it using cargo xtask build-js only when updating Spectrum dependencies in package.json or js/spectrum-entry.js, and commit the updated bundle together with package-lock.json.

Aliases / also known as: xtask build-js, swc.js, spectrum bundle, fetch-assets, vendored assets, xtask maintenance
