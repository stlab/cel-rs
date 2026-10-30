---
title: "Conditional Evaluation Failure Must Not Activate The Default Branch"
entity_type: pattern
confidence: 0.99
created: 2026-10-29T01:40:49Z
last_accessed: 2026-10-29T01:40:49Z
last_reinforced: 2026-10-29T01:40:49Z
access_count: 1
sources:
  - "adam-web-ui/src/graph/data.rs:347-357"
  - "adam-web-ui/src/graph/data.rs:379-405"
  - "begin/assets/graph.js:888-897"
  - "begin/assets/graph.js:970-971"
tags: ["graph", "conditionals", "error-handling", "ui"]
tier: knowledge
---

# Conditional Evaluation Failure Must Not Activate The Default Branch

Graph branch activity distinguishes an evaluation failure from a successfully selected default. Preserve the outer optional evaluation result when computing default activity: `active_branch.map(|active| active.is_none())` yields unknown for failure, rather than treating failure as a missing named match and activating the default. Every control link has `branch_active: None` after a conditional evaluation error; a successful named/default decision yields explicit Boolean activity.

The JavaScript consumer marks targets active only for truthy `branch_active` and gives unknown control links the inactive presentation. Keep the Rust producer and this consumer consistent; flattening away the failure state can make a failed sheet look successfully evaluated. The regression changes a valid conditional divisor to zero and requires both control links to remain unknown. Actual Begin browser verification also confirmed that one active/one inactive link becomes two inactive links after the failure.

Sources: `adam-web-ui/src/graph/data.rs:347-357`, `adam-web-ui/src/graph/data.rs:379-405`, `begin/assets/graph.js:888-897`, `begin/assets/graph.js:970-971`.
