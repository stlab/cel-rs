# CEL Euclid implementation handoff

## Completed

The language stage is implemented and independently reviewed on
`worktree-euclid-library`, through commit `00111378`.

- Persistent shared operation registrations compose overloads across installers.
- Named receiver properties and calls preserve ordinary call and tuple semantics.
- Registered records support explicit fields in arbitrary written order.
- Predictive brace restrictions preserve CEL and embedded Adam control flow.
- Registration metadata informs checking, diagnostics, AST consumers, and books.
- Formatter fixes preserve closure receiver semantics and grouped comments.

Targeted suites, downstream checks, strict documentation, formatting, and
all-target lint checks passed for the affected language crates. The final
whole-workspace gate remains due after the library and graph stages.

The library stage is implemented and independently reviewed through `f5b48309`.
`cel-euclid` exposes the seven concrete geometry aliases, named records, the
documented operation table, checked primitives, and optional Adam registration.
Executable CEL-only crop/straighten and pan/zoom examples run in configured
web, LSP, and book environments. The crop preserves source aspiration and
checks fitting and fixed-overlay presentation precision explicitly.

A prerequisite adds registered closure parameter annotations and checked
general closure invocation, including recursive tuple intermediates.
Independent review also required rejecting ambient-dependent closure bodies.

## Remaining

Migrate the live graph policy into Adam, implement its lifecycle-owned bridge,
and verify actual rendered behavior. The full workspace gate remains pending.

The approved design and three stage plans remain authoritative. No
crop-specific constraint function or Rust fitting helper is permitted: the
example must compose general geometry operations in CEL.

## Implementation rulings

Function markers require `Send + Sync` to preserve scope thread-safety.
If unnecessarily strict, this restricts unusual host marker types; the planned
markers are zero-sized and satisfy the bounds.

Field-name validation precedes constructor emission, not streaming compilation
of field expressions. Invalid parsing discards its segment; the cost is
instructions appended to a segment that is subsequently discarded.

The approved crop composition requires typed immediately invoked closures.
Implementing their missing general-purpose runtime support was necessary
before the example could proceed. If that scope decision is wrong, it adds
broader closure/runtime work than originally anticipated; the prerequisite
and its safety fix received independent review.

## Recovery

Task briefs, evidence, and review ledgers live in git-ignored
`.superpowers\sdd\` stage directories. Resume the first task without a completion
entry rather than repeating committed work. No push or pull request is requested.
