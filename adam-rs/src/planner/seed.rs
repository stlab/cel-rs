//! Seedfill: reconstructs, for each self-referencing input a plan will read, the value
//! that cell *aspires* to before its own claiming relationship tightens it.
//!
//! A self-referencing method `X := f(X, ..)` reads `X`'s own value as one input. Its
//! `source` is frozen at the last explicit `write`, which goes stale the moment another
//! relationship pushes `X` around (e.g. `a <= b` forcing `b` up to match `a`): `b`'s
//! `source` still reads its original declaration even though the chain has settled it
//! elsewhere. Reading `source` directly then discards a consistent edit; reading the
//! previous round's `derived` value instead re-introduces a shrinking accumulator.
//!
//! Seedfill computes the right value structurally, without the planner ever comparing
//! two values: `X`'s seed is what `X` would settle to under every relationship incident
//! to it *except* its own claimant, evaluated from `source` values. In the `a<=b<=c`
//! chain, `b`'s claimant (the `b<=c` relationship) is set aside and `b` is seeded by the
//! `a<=b` relationship's `b`-producing method (`b := max(a, b)`), giving `b`'s aspiration
//! from `a`; the claimant then tightens it. Because the seed is rebuilt from `source`
//! every round, it is never stale and never accumulates -- see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.

use std::any::Any;
use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::cell::{CellData, CellId};
use crate::relationship::{RelationshipData, RelationshipId};

use super::{PlanStep, Seeds};

/// Computes the seed value for every self-referencing input `execution_order` will read.
///
/// A cell absent from the result reads its own `source` during execution (the common
/// case: no other relationship pushes it). Only a cell that is both claimed by a
/// self-referencing method this round *and* produced by some other incident relationship
/// gets an entry.
///
/// - Precondition: `execution_order`'s `PlanStep::Method` steps name valid method indices
///   in `relationships`.
///
/// - Complexity: O(S · A · K) where S = self-referencing claimed cells, A = relationships
///   incident to each, K = cells per method; the recursion visits each cell once
///   (memoized).
pub(crate) fn build_seeds(
    execution_order: &[PlanStep],
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Seeds {
    let mut claimant: HashMap<CellId, RelationshipId> = HashMap::new();
    let mut active: HashSet<RelationshipId> = HashSet::new();
    let mut self_ref_cells: Vec<CellId> = Vec::new();
    for step in execution_order {
        let PlanStep::Method(rel_id, method_idx) = *step else {
            continue;
        };
        active.insert(rel_id);
        let method = &relationships[rel_id].methods[method_idx];
        for &output in &method.outputs {
            claimant.insert(output, rel_id);
            if method.inputs.contains(&output) {
                self_ref_cells.push(output);
            }
        }
    }

    let mut seeds: Seeds = HashMap::new();
    let mut visiting: HashSet<CellId> = HashSet::new();
    for &cell in &self_ref_cells {
        compute_seed(
            cell,
            &claimant,
            &active,
            cells,
            relationships,
            &mut seeds,
            &mut visiting,
        );
    }
    seeds
}

/// Populates `seeds[x]` with `x`'s aspiration, computed by folding every incident
/// relationship other than `x`'s claimant through its `x`-producing method, in
/// `relationships`' natural order, seeded from `x`'s `source` and each other input's own
/// seed (recursively). Leaves `x` absent when no such relationship exists (its seed is
/// just `source`) or when every candidate method errors or mistypes its output.
///
/// Only relationships in `active` (those the current plan actually runs) count as
/// siblings: an inactive conditional branch that happens to name `x` must not seed it.
///
/// `visiting` guards against a cyclic seed dependency (a cell reachable from itself
/// through the incident-relationship graph): a cell already on the stack contributes its
/// `source` value rather than recursing forever.
fn compute_seed(
    x: CellId,
    claimant: &HashMap<CellId, RelationshipId>,
    active: &HashSet<RelationshipId>,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    seeds: &mut Seeds,
    visiting: &mut HashSet<CellId>,
) {
    if seeds.contains_key(&x) || !visiting.insert(x) {
        return;
    }

    let own_claimant = claimant.get(&x).copied();
    let sibling_methods: Vec<(RelationshipId, usize)> = cells[x]
        .adj
        .iter()
        .filter(|&&rel_id| active.contains(&rel_id) && Some(rel_id) != own_claimant)
        .filter_map(|&rel_id| {
            relationships[rel_id]
                .methods
                .iter()
                .position(|m| m.outputs.contains(&x))
                .map(|idx| (rel_id, idx))
        })
        .collect();

    for &(rel_id, method_idx) in &sibling_methods {
        for &input in &relationships[rel_id].methods[method_idx].inputs {
            if input != x {
                compute_seed(
                    input,
                    claimant,
                    active,
                    cells,
                    relationships,
                    seeds,
                    visiting,
                );
            }
        }
    }

    let mut accumulated: Option<Box<dyn Any>> = None;
    for &(rel_id, method_idx) in &sibling_methods {
        let method = &relationships[rel_id].methods[method_idx];
        let produced = {
            let inputs: Vec<&dyn Any> = method
                .inputs
                .iter()
                .map(|&input| {
                    if input == x {
                        accumulated
                            .as_deref()
                            .unwrap_or_else(|| cells[x].source.as_ref())
                    } else {
                        seeds
                            .get(&input)
                            .map(|value| value.as_ref())
                            .unwrap_or_else(|| cells[input].source.as_ref())
                    }
                })
                .collect();
            (method.function)(&inputs)
                .ok()
                .filter(|outputs| outputs.len() == method.outputs.len())
                .and_then(|mut outputs| {
                    let pos = method.outputs.iter().position(|&o| o == x)?;
                    Some(outputs.swap_remove(pos))
                })
                .filter(|value| value.as_ref().type_id() == cells[x].type_id)
        };
        if produced.is_some() {
            accumulated = produced;
        }
    }

    visiting.remove(&x);
    if let Some(value) = accumulated {
        seeds.insert(x, value);
    }
}
