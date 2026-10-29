---
title: "EZ Document Epoch Must Key An Iterable Workspace Fragment"
entity_type: pattern
confidence: 1.00
created: 2026-10-29T02:14:31Z
last_accessed: 2026-10-29T02:14:31Z
last_reinforced: 2026-10-29T02:14:31Z
access_count: 1
sources:
  - "ez-adam/src/ui/app.rs:49-57"
  - "ez-adam/src/ui/app.rs:60-89"
tags: ["dioxus", "ui", "lifetime", "document-replacement", "keyed-fragment"]
tier: knowledge
---

# EZ Document Epoch Must Key An Iterable Workspace Fragment

EZ replacement clears selection, restores Select, increments a checked epoch and installs the new Document. `editor_workspace` places its epoch key on a singleton iterable fragment containing Canvas and SidePanel. Preserve this fragment boundary: a nested component key, or a root-only key without the iterable fragment, did not retire document-local scopes reliably in the verified Dioxus composition. Retiring those scopes cancels pending drag/connection state and listeners, preventing reused node IDs in a replacement document from receiving old gestures. Production and controlled lifecycle tests share `editor_workspace`; the actual retirement scenario passed in debug and release. An epoch-counter assertion alone does not establish scope retirement.
