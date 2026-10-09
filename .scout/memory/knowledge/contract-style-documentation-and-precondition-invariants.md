---
title: "Contract-Style Documentation and Precondition Invariants"
entity_type: pattern
confidence: 0.85
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "cel-parser/src/lib.rs:555-600"
  - "adam-rs/src/sheet.rs:320-330"
relationships:
  - type: applies_to
    target: "Architecture Overview"
tags: ["conventions", "documentation", "contract", "preconditions", "debug-assert"]
tier: knowledge
source: inferred
---

# Contract-Style Documentation and Precondition Invariants

The workspace enforces strict documentation and precondition conventions for all functions:
- Contract format: /// doc comments written in contract style starting with a present-tense summary ending in a period.
- Sections: Non-obvious preconditions (- Precondition:), observable postconditions (- Postcondition:), # Errors, # Safety (for unsafe functions), and - Complexity: (mandatory for any operation that is not O(1)).
- Enforcement: State non-obvious preconditions explicitly and check them with debug_assert! in debug builds. Do not specify the consequences of violating a precondition or test precondition violations.
- Unit test convention: Tests must derive strictly from public contracts and postconditions, never from implementation details.

Aliases / also known as: contract documentation, doc conventions, debug_assert preconditions, contract-style comments, testing convention
