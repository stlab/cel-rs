# v1.0 Group 4 - Language Surface Completion Review

Status as of 2026-10-23: the implementations are merged in #253 and #255 and
present at reviewed HEAD `844ef48`. This review verified the existing contracts,
closed #110, #10, and #111 with evidence, and updated the milestone plan.
No product code changed.

## What's done

- #110: arrays support typed literals and recursive composition. The postfix
  grammar accepts named receiver calls and properties
  (`cel-parser/src/lib.rs:1632`). The runtime context dispatches receiver-first
  through `OpLookup::lookup_member`, independently of ordinary value bindings
  (`cel-parser/src/parser_context.rs:663`, `cel-parser/src/op_table.rs:1931`).
- #10: arbitrary closure-valued expressions are callable
  (`cel-parser/src/op_table.rs:2029`). Contract tests exercise immediate,
  higher-order, returned, tuple-projected, and conditional callees, plus
  incompatible signatures (`cel-parser/tests/closure_invocation.rs:8`,
  `:127`, `:146`). Native and interpreted callables share typed container
  representations (`cel-runtime/tests/callable_values.rs:29`).
- #111: the opt-in Euclid library supplies points, vectors, sizes, rectangles,
  boxes, angles, and affine transforms. Its public installation contract and
  examples live in `cel-euclid/src/lib.rs`. Geometry operations register exact
  signatures (`cel-euclid/src/geometry.rs:127`); transform operations include
  composition, mapping, checked inverse, and checked outer bounds
  (`cel-euclid/src/transforms.rs:226`). The executable Adam pan/zoom example
  verifies the motivating scale-limit and translation-clamping policy
  (`cel-euclid/tests/adam_examples.rs:493`).

Each issue now has an explanation of its implementation provenance, supported
boundary, and fresh verification before its completed closure.

## Supported boundaries

Functions require typed parameters and do not capture enclosing lexical state.
General invocation, deferred by the original closure design, is now implemented;
this review does not claim capturing closures or every possible future function
capability.

Euclid was an explicit alternative in #111. Its general `Transform2D` covers
translation plus uniform scale without introducing `kurbo::TranslateScale`.
Geometry uses `f64` and `UnknownUnit`; hosts explicitly install the library.
Graph-specific fitting and clamping remain CEL sheet policy. Neither core crate
depends on geometry.

## Verification

The following command passed during this review, including doctests and optional
Adam geometry integration:

```sh
cargo test -p cel-runtime -p cel-parser -p cel-euclid --features cel-euclid/adam --quiet
```

This was a completion review of merged code, not a new implementation. Existing
contract tests supplied the behavioral evidence; no redundant code or tests were
added. No UI changed, and no new rendered verification or full-workspace gate is
claimed.

## What's left

- Group 2: #186 remains open for its broader seedfill acceptance, including the
  deferred rectangle-containment spring-back/reversal coverage.
- Group 5: #29 remains open for equation-style relationship declarations.

## Follow-up: #152

Closed #152 on 2026-10-23 after verifying the automatic plan-reuse implementation
merged in #245. `Sheet::propagate` owns phase-local reuse and replanning decisions;
active-set and strength-certificate checks control eligibility, and writes to
claimed cells invalidate the affected phase. Fresh verification passed all
24 cache tests:

```sh
cargo test -p adam-rs --lib sheet::cached::tests --quiet
```

Group 3 merged in #247 on 2026-10-07, closing #194 and #191. Groups 1, 3, 4, 6,
and 7 are complete. Subsequent v1.0 scope additions are #258 (requirement-label
syntax review) and #259 (language-versioning and upgrade-tooling plan), recorded
as Groups 8 and 9 in the milestone plan. The milestone's remaining open issues
are #186, #29, #258, and #259.
