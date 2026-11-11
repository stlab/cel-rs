# Conditional forcing and remembered priority

## Purpose and scope

This design resolves the remaining value and priority contract in #219 for
authors of `adam-rs` sheets and hosts that replay accepted edits. A cell can
temporarily become a forced output when a conditional activates a relationship.
When that forcing ends, its remembered value and priority must participate in
the next plan without requiring another edit.

The design covers conditional forcing, including forcing propagated through
active relationships, and its interaction with self-referencing methods,
ordinary reversible relationships, caching, and recovery. The broader 2D
seedfill acceptance work remains in #186.

## Existing behavior and decision

An ordinary reversible relationship publishes its result into the losing
cell's source value and assigns it a derived priority. When that relationship
deactivates, the cell retains the last derived value rather than reviving an
earlier independent value. This behavior remains unchanged. In particular,
this design does not introduce a continuously updated hypothetical value or
restore an older value after ordinary reversible derivation.

A conditionally forced output uses a derived override, leaving its underlying
source value available for release. Existing self-referencing methods also
retain their source aspirations and reconstruct seeds during propagation. The
runtime currently demotes the priority of a conditionally forced output; its
previous priority is not explicitly restored on release. This design changes
that priority behavior.

## Value and priority contract

- On the transition into conditional forcing, retain the cell's priority as it
  stood immediately before forcing. If ordinary derivation already demoted the
  cell, remember that derived priority, not an earlier write priority.
- While forcing remains active, the forced relationship determines the
  effective value. Upstream changes update that effective value without
  replacing the stored source aspiration or its remembered priority.
- A successful explicit write while forced replaces the source aspiration and
  the remembered priority with that write's actual recency. Subsequent writes
  to other cells keep their normal relative ordering; release does not promote
  the remembered edit above newer edits.
- When a cell was conditionally forced in the preceding successful
  propagation, the unconditional pre-plan uses its remembered priority to
  compute conditional guards. This matches the pre-plan's source-value view,
  even before the runtime knows whether the current branches will release the
  cell. The final plan uses remembered priority wherever the cell is no longer
  forced.
- When conditional forcing ends, the stored aspiration and remembered priority
  participate in the first plan that can use the cell as a source. The runtime
  must not select that plan using the temporary forced-output priority and
  restore the priority only afterward.
- If another active relationship still forces the cell, the cell remains
  forced. Changes among overlapping or cascaded forcing relationships must not
  overwrite the remembered priority with a temporary derived priority.
- If the released cell is subsequently selected as an ordinary reversible
  relationship's output, ordinary publication and derived-priority rules apply.
  Release does not guarantee that the cell becomes a source if the complete
  set of constraints chooses another valid assignment.
- A self-referencing method continues to seed from its source aspiration and
  honor an explicit edit's priority. Conditional forcing and subsequent release
  must not erase that edit or turn the seed into the forced effective value.

Out cells remain non-writable. Existing checked errors and the requirement to
discard a sheet after a fallible mutating operation returns an error remain
unchanged. No observation or recovery behavior is promised for a failed sheet.

## Approaches

The selected approach retains remembered priority independently from the
priority used for a forced output. It supports repeated forced propagations and
writes during forcing without assigning a new priority on release. The
implementation must preserve the priority in effect at the forcing transition,
including an already-derived priority; retaining only each cell's last explicit
write is insufficient.

An alternative snapshots the active priority at the transition into forcing and
restores it on exit. That can satisfy the same contract, but transition and
overlap handling must avoid overwriting the snapshot. The implementation plan
should choose the representation after tracing planning, conditional
classification, and cache invalidation; the observable contract does not
require an exposed memory API.

Keeping the current demotion for forced cells was rejected because release
would lose the cell's earlier priority. Preserving old aspirations for every
ordinary reversible output was also rejected because it changes the existing
ordinary-relationship contract.

## Integration constraints

The planner remains value-blind and follows its existing strength and
forced-output rules. Determine the released priority early enough to affect
source selection in the release propagation. Cached plans and forced replanning
must produce the same values, source classification, and relationship
direction across activation, repeated forced updates, overlapping forcing,
explicit writes, and release. A change to remembered or effective priority
must invalidate or revalidate any affected cache certificate.

The existing static guard-independence rule and pre-plan/final-plan
compatibility check remain in force. If remembered-priority pre-planning
selects a guard prerequisite incompatible with the active final plan,
propagation reports the existing conflict; it must not silently reuse the
incompatible result or commit a partial plan.

Host recovery must replay accepted edits with their original ordering and
reconstruct the same observable release behavior. `source_cells` recordings
continue to capture effective source values rather than a full aspiration or
priority history; this design does not extend their existing contract.

## Verification

Contract-derived tests must distinguish values and priority through public
observations such as `read`, `source`, `is_source`, and subsequent relationship
direction. Cover:

1. A conditional forces a previously edited cell, upstream inputs change
   repeatedly, and release restores its source value without a new edit.
2. A released cell competes with another cell in a reversible relationship.
   The selected direction reflects their write order from before forcing.
3. Writes while forced and writes to other cells in both relative orders retain
   their actual recency on release, including a same-value write.
4. Overlapping and cascaded forcing preserve memory until the final forcing
   cause ends. Repeated propagation does not change it.
5. A cell already demoted by ordinary reversible derivation is forced and
   released without reviving an older explicit priority or value.
6. Self-referencing spring-back and explicit priority survive conditional
   forcing and release, alongside existing ordinary-reversible behavior.
7. An unconditional reversible relationship involving a previously forced cell
   computes a guard for an independent conditional, without violating guard
   independence. The pre-plan uses remembered priority when choosing its
   direction and guard value. If that prerequisite is incompatible with the
   final plan, propagation reports the existing conflict instead of committing.
8. Normal cached execution and forced replanning agree on the same sequences.
   Accepted host recovery preserves the observable result of release.

Tests must follow the public error contract and must not inspect a sheet after
a failed fallible mutation.
