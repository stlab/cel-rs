---
title: "Adam Forced Classification Is Not Free-Source Feasibility"
entity_type: pattern
confidence: 0.97
created: 2026-10-28T22:35:47Z
last_accessed: 2026-10-29T03:25:15Z
last_reinforced: 2026-10-29T03:25:15Z
access_count: 2
sources:
  - "adam-rs/src/planner/matching.rs:227"
  - "adam-rs/src/sheet.rs:2222"
  - "adam-rs/src/planner.rs:73"
  - "adam-rs/src/planner.rs:91"
  - "adam-rs/src/sheet.rs:1146"
  - "adam-rs/src/sheet.rs:1197"
  - "adam-rs/src/sheet/cached.rs:64"
  - "adam-rs/src/sheet/cached.rs:359"
  - "adam-rs/tests/contract_regressions.rs:290"
tags: ["adam-rs", "planner", "contributors", "source-feasibility", "component-isolation"]
tier: knowledge
source: Verified contributor/planner contracts, complete active-component remediation delta, package tests and final workspace validation at 42ab7e8.
votes_up: 1
votes_down: 0
last_voted: 2026-10-29T03:25:18Z
---

# Adam Forced Classification Is Not Free-Source Feasibility

Sheet::is_forced reads the completed plan's pure-output forced classification. It is not an exact predicate for whether an active graph can leave a cell unclaimed. A mandatory self-referencing method can claim a cell in every feasible assignment while that cell's editable stored aspiration still influences its seed; such a cell need not appear in the pure-output forced set.

Do not use !is_forced as exact free-source eligibility or make those self-reference aspirations uneditable. planner::can_be_source asks the existing acyclic exclusive-assignment solver to forbid the candidate cell as an output. Contributor traversal needs this distinction when deciding whether a non-current source's dynamic filter arguments can be authoritative dependencies; diagnostic-only filters on method outputs are not value dependencies.

At contributing_cells, restrict exact search to the candidate's complete undirected input/output relationship component in the last successful prepared active set. Include every method's declared cells, shared output claims and self-reference; selected-method or directed-upstream traversal is insufficient. The last successful assignment witnesses feasibility in every independent component, so component-local feasibility is equivalent to global feasibility at this caller. This reduction is not valid for an arbitrary graph without that witness.

The witness survives writes, appended declarations and failed propagation because registered methods are immutable and append-only, and last_plan is published only on successful propagation. Filter current adjacency through prepared.active so newly registered relationships or changed conditional selection do not enter the query early. Filter arguments and potential conditional contributors still use the current registered graph.

Exact search remains potentially exponential inside the connected component, and the solver still scans global relationship storage. Do not restore global combinatorial amplification from unrelated reversible components or replace exact feasibility with forced classification. No persistent feasibility cache is needed for component isolation.
