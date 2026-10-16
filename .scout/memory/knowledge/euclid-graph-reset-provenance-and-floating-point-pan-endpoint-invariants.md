---
title: "Euclid graph reset provenance and floating-point pan endpoint invariants"
entity_type: discovery
confidence: 0.98
created: 2026-10-16T04:44:28Z
last_accessed: 2026-10-16T04:44:28Z
last_reinforced: 2026-10-16T04:44:28Z
access_count: 1
sources:
  - "begin/assets/graph.js:545-548"
  - "begin/assets/graph.js:617-624"
  - "begin/assets/graph.js:1227-1231"
  - "adam-web-ui/src/graph/view_constraints.adm2:22-35"
tags: ["euclid", "graph", "adam", "floating-point", "async"]
tier: knowledge
source: "PR #253, published head c0c6f36660838a83de83a3b4884584a70369df90; current euclid-library worktree source verified with narrow native reads because Scout indexes an older graph implementation."
---

# Euclid graph reset provenance and floating-point pan endpoint invariants

Two regression-derived invariants from the Euclid graph migration in PR #253:

1. Reset request provenance must outlive animation state. GraphInstance tracks Fit request objects in a WeakSet independently from resetPending/resetAnimating (begin/assets/graph.js:545-548). applyApproved recognizes those origins and returns without applying a late/repeated reset target unless it is still the pending reset that should start animateFit (begin/assets/graph.js:617-624). resetZoom registers the action's identity before submission (begin/assets/graph.js:1227-1231). Otherwise, a geometry refresh that preserves a queued Fit, followed by a wheel gesture cancelling reset, can let a repeated Fit approval snap the view back. Do not replace provenance tracking with only a transition-active boolean.

2. Decide whether an axis centers or clamps by comparing the actual computed pan endpoints, not an algebraically equivalent scaled-extent predicate. Floating-point rounding at translated fit can invert nominally equal clamp endpoints by one ulp. The sheet centers when viewport - zoom*max >= -zoom*min, otherwise clamps using those same endpoints (adam-web-ui/src/graph/view_constraints.adm2:24-35). Zoom itself clamps to the derived fit minimum and effective maximum (line 22).

Evidence status: UNINDEXED worktree-specific native source reads, anchored to published PR head c0c6f366; Scout's indexed graph snapshot still showed the pre-migration direct-JS reset implementation. These are committed implementation invariants, not the paused uncommitted review fixes.
