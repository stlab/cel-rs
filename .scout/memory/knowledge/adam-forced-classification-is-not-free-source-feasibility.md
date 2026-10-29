---
title: "Adam Forced Classification Is Not Free-Source Feasibility"
entity_type: pattern
confidence: 0.97
created: 2026-10-28T22:35:47Z
last_accessed: 2026-10-28T22:35:47Z
last_reinforced: 2026-10-28T22:35:47Z
access_count: 1
sources:
  - "adam-rs/src/sheet.rs:2220"
  - "adam-rs/src/planner.rs:82"
  - "adam-rs/src/planner.rs:286"
  - "adam-rs/src/planner/matching.rs:227"
tags: ["adam-rs", "planner", "contributors", "source-feasibility"]
tier: knowledge
source: Accepted Adam runtime contract reconciliation and verified planner/query definitions.
---

# Adam Forced Classification Is Not Free-Source Feasibility

Sheet::is_forced reads the completed plan's pure-output forced classification. It is not an exact predicate for whether an active graph can leave a cell unclaimed. A mandatory self-referencing method can claim a cell in every feasible assignment while that cell's editable stored aspiration still influences its seed; such a cell need not appear in the pure-output forced set.

Do not use !is_forced as exact free-source eligibility or make those self-reference aspirations uneditable. For a statically valid active graph, planner::can_be_source asks the existing acyclic exclusive-assignment solver to forbid the candidate cell as an output. That accounts for all output claims, including self-reference. Contributor traversal needs this distinction when deciding whether a non-current source's dynamic filter arguments can be authoritative dependencies; diagnostic-only filters on method outputs are not value dependencies.

Exact feasibility can require exponential search. Preserve the documented search/partial-graph costs rather than replacing the solver check with the cheaper but inequivalent forced classification.
