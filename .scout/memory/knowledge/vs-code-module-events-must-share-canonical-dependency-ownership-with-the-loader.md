---
title: "VS Code module events must share canonical dependency ownership with the loader"
entity_type: pattern
confidence: 0.98
created: 2026-10-28T05:13:03Z
last_accessed: 2026-10-28T05:13:03Z
last_reinforced: 2026-10-28T05:13:03Z
access_count: 1
sources:
  - "editors/vscode-adam-lang/src/moduleSources.ts:58-165"
  - "editors/vscode-adam-lang/src/extension.ts:54-104"
tags: ["vscode", "modules", "symlinks", "canonical-paths", "cache-invalidation", "source-ownership"]
tier: knowledge
---

# VS Code module events must share canonical dependency ownership with the loader

The extension must share one ModuleSourceLoader instance between initialization, the serialized refresh queue and editor event selection. A fresh stateless load alone cannot retain canonical-target identities for later watcher/save events. Module records keep their configured lexical URIs for LSP wire ownership and overlays; approved canonical paths are read targets and event dependencies, not replacement wire identities. The loader retains canonical dependencies across read failure, resolves later owners despite an earlier owner failure, prunes removed configuration and replaces aliases on successful retargeting. Its event predicate selects lexical/canonical dependencies and ancestor replacement events. If realpath cannot resolve a dependency, it temporarily retries on source events so initially missing paths and deletion/recreation can recover; successful resolution restores precise unrelated-event suppression. Keep containment validation before reading and publish explicit failure snapshots rather than empty successful text. The regression root cause was canonical filesystem reads combined with lexical-only event filtering: allowed internal links loaded successfully while real-target changes left dependency diagnostics stale.
