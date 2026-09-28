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
//! candidate values: `X`'s seed is what `X` would settle to under every relationship
//! incident to it *except* its own claimant, evaluated from `source` values. In the
//! `a<=b<=c` chain, `b`'s claimant (the `b<=c` relationship) is set aside and `b` is
//! seeded by the `a<=b` relationship's `b`-producing method (`b := max(a, b)`), giving
//! `b`'s aspiration from `a`; the claimant then tightens it. Because the seed is rebuilt
//! from `source` every round, it is never stale and never accumulates -- see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.
//!
//! One sibling fold is gated by relative *strength* (a cheap field comparison, not a
//! candidate-value comparison): when `a<=b`'s claimant this round is itself `a` (not
//! `b`), folding `a<=b`'s `b`-producing method to seed some other cell `x` would use `a`'s
//! `source` as if it were authoritative -- but if `a` was written *before* `x`, `a`'s
//! `source` is exactly the stale declaration this module exists to avoid re-introducing.
//! `compute_seed` skips such a fold unless the self-referenced input genuinely outranks
//! `x` in strength; see its doc comment.

use std::any::Any;
use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::cell::{CellData, CellId};
use crate::error::{Error, ErrorSite};
use crate::relationship::{RelationshipData, RelationshipId};

use super::{PlanStep, Seeds, matching::pure_outputs};

/// Computes the seed value for every self-referencing input `execution_order` will read.
///
/// A cell absent from the result reads its own `source` during execution (the common
/// case: no other relationship pushes it). Only a cell that is both claimed by a
/// self-referencing method this round *and* produced by some other incident relationship
/// gets an entry.
///
/// - Precondition: `execution_order`'s `PlanStep::Method` steps name valid method indices
///   in `relationships`.
/// - Precondition: `elimination_order` is the exact cell order the planner's release pass
///   evaluated for tentative elimination this round.
///
/// # Errors
///
/// - `Error::Conflict` — a sibling relationship that can seed a self-referencing cell
///   has no method compatible with the planner's elimination state.
/// - `Error::SeedCycle` — sibling seed dependencies form a non-self cycle.
///
/// - Complexity: O(S · A · M · K²) where S = self-referencing claimed cells, A =
///   relationships incident to each, M = methods per sibling relationship, K = cells per
///   method; the recursion visits each cell once (memoized).
pub(crate) fn build_seeds(
    execution_order: &[PlanStep],
    elimination_order: &[CellId],
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Seeds, Error> {
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
    let mut path_cells: Vec<CellId> = Vec::new();
    let mut path_relationships: Vec<RelationshipId> = Vec::new();
    let mut path_indices: HashMap<CellId, usize> = HashMap::new();
    for &cell in &self_ref_cells {
        compute_seed(
            cell,
            &claimant,
            &active,
            elimination_order,
            cells,
            relationships,
            &mut seeds,
            &mut path_cells,
            &mut path_relationships,
            &mut path_indices,
        )?;
    }
    Ok(seeds)
}

/// Populates `seeds[x]` with `x`'s aspiration, computed by folding every incident
/// relationship other than `x`'s claimant through its elimination-compatible
/// `x`-producing method, ordered by the strongest non-`x` input's strength from weakest
/// to strongest, seeded from `x`'s `source` and each other input's own seed
/// (recursively). Leaves `x` absent when no such relationship exists (its seed is just
/// `source`) or when every candidate method errors or mistypes its output.
///
/// Only relationships in `active` (those the current plan actually runs) count as
/// siblings: an inactive conditional branch that happens to name `x` must not seed it.
///
/// `path_cells`/`path_relationships` record the active seed dependency path. A sibling
/// input that reaches a cell already on that path reports `Error::SeedCycle` naming the
/// participating cells and sibling relationships in traversal order.
///
/// A sibling method is skipped (contributes nothing) when `x` itself holds a live
/// explicit strength (a `write()`/`add_cell` not yet superseded by a later claim) *and*
/// some other input `y` to that method is claimed this round, self-referencingly, by
/// that same sibling relationship, with `y`'s strength not exceeding `x`'s. Folding that
/// method would use `y`'s stale `source` to override `x`'s own explicit edit even though
/// `y` is not the stronger party, and the relationship's own chosen method this round
/// already computes `y` from `x` (not the reverse). Gating on `x`'s *own* strength being
/// explicit (rather than comparing `y` against whatever value `x` currently holds,
/// explicit or not) matters because a purely derived `x` has no live explicit edit to
/// protect, and its post-round strength is only an execution-order tie-break, not a
/// genuine priority signal -- treating it as one here would incorrectly suppress the
/// aspiration fold for the ordinary case this module exists to handle. This is the same
/// relative-strength test `release::resolve` uses elsewhere, not a comparison of
/// candidate values. Among the sibling methods that survive that gate, weaker
/// relationships fold first and the strongest surviving influence folds last so the
/// final seed respects strength rather than relationship insertion order.
///
/// # Errors
///
/// - `Error::Conflict` — some sibling relationship that can seed `x` has no surviving
///   `x`-producing method after replaying the planner's elimination state.
/// - `Error::SeedCycle` — a sibling seed dependency reaches a currently visiting cell.
fn compute_seed(
    x: CellId,
    claimant: &HashMap<CellId, RelationshipId>,
    active: &HashSet<RelationshipId>,
    elimination_order: &[CellId],
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    seeds: &mut Seeds,
    path_cells: &mut Vec<CellId>,
    path_relationships: &mut Vec<RelationshipId>,
    path_indices: &mut HashMap<CellId, usize>,
) -> Result<(), Error> {
    if seeds.contains_key(&x) {
        return Ok(());
    }
    debug_assert!(!path_indices.contains_key(&x));
    path_indices.insert(x, path_cells.len());
    path_cells.push(x);

    let own_claimant = claimant.get(&x).copied();
    let mut sibling_methods: Vec<(RelationshipId, usize, u64)> = cells[x]
        .adj
        .iter()
        .filter(|&&rel_id| active.contains(&rel_id) && Some(rel_id) != own_claimant)
        .filter_map(|&rel_id| {
            match select_seed_method(rel_id, x, elimination_order, relationships) {
                Ok(Some(idx)) => {
                    let strongest_input = relationships[rel_id].methods[idx]
                        .inputs
                        .iter()
                        .filter(|&&input| input != x)
                        .map(|&input| cells[input].strength)
                        .max()
                        .unwrap_or(0);
                    Some(Ok((rel_id, idx, strongest_input)))
                }
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            }
        })
        .collect::<Result<Vec<_>, Error>>()?;
    sibling_methods.sort_by_key(|&(_, _, strongest_input)| strongest_input);

    for &(rel_id, method_idx, _) in &sibling_methods {
        for &input in &relationships[rel_id].methods[method_idx].inputs {
            if input != x {
                if let Some(&cycle_start) = path_indices.get(&input) {
                    let sites =
                        seed_cycle_sites(path_cells, path_relationships, cycle_start, rel_id, x);
                    path_indices.remove(&x);
                    path_cells.pop();
                    return Err(Error::SeedCycle { sites });
                }
                path_relationships.push(rel_id);
                let result = compute_seed(
                    input,
                    claimant,
                    active,
                    elimination_order,
                    cells,
                    relationships,
                    seeds,
                    path_cells,
                    path_relationships,
                    path_indices,
                );
                path_relationships.pop();
                if let Err(err) = result {
                    path_indices.remove(&x);
                    path_cells.pop();
                    return Err(err);
                }
            }
        }
    }

    let mut accumulated: Option<Box<dyn Any>> = None;
    for &(rel_id, method_idx, _) in &sibling_methods {
        let method = &relationships[rel_id].methods[method_idx];

        let has_weaker_self_referenced_input = cells[x].has_explicit_strength()
            && method.inputs.iter().any(|&input| {
                input != x
                    && claimant.get(&input) == Some(&rel_id)
                    && cells[input].strength <= cells[x].strength
            });
        if has_weaker_self_referenced_input {
            continue;
        }

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

    path_indices.remove(&x);
    path_cells.pop();
    if let Some(value) = accumulated {
        seeds.insert(x, value);
    }
    Ok(())
}

/// Returns `Error::SeedCycle` sites for a revisited path cell.
///
/// - Precondition: `cycle_start < path_cells.len()`.
/// - Precondition: `path_cells.len() == path_relationships.len() + 1`.
/// - Complexity: O(n) where n is the number of reported cycle members.
fn seed_cycle_sites(
    path_cells: &[CellId],
    path_relationships: &[RelationshipId],
    cycle_start: usize,
    closing_relationship: RelationshipId,
    current: CellId,
) -> Vec<ErrorSite> {
    debug_assert!(cycle_start < path_cells.len());
    debug_assert_eq!(path_cells.len(), path_relationships.len() + 1);
    debug_assert_eq!(path_cells.last().copied(), Some(current));

    let mut sites = Vec::with_capacity((path_cells.len() - cycle_start) * 2);
    sites.push(ErrorSite::Cell(path_cells[cycle_start]));
    for index in cycle_start..path_relationships.len() {
        sites.push(ErrorSite::Relationship(path_relationships[index]));
        sites.push(ErrorSite::Cell(path_cells[index + 1]));
    }
    sites.push(ErrorSite::Relationship(closing_relationship));
    sites
}

/// Selects `rel_id`'s unique `target`-producing seed method compatible with the
/// planner's elimination order.
///
/// Starts from every method in `rel_id` whose declared outputs contain `target`, then
/// replays elimination across the relationship's other referenced cells in
/// `elimination_order`. Eliminating a cell removes any candidate method that would still
/// purely claim that cell under the matching layer's semantics ([`pure_outputs`]).
///
/// Returns `Ok(None)` when `rel_id` has no `target`-producing method.
///
/// # Errors
///
/// - `Error::Conflict` — `rel_id` could seed `target`, but the elimination replay leaves
///   either no candidate method or more than one surviving candidate.
///
/// - Complexity: O(E · M · K²) where E = `rel_id`'s non-`target` referenced cells, M =
///   methods in the relationship, K = cells per method.
fn select_seed_method(
    rel_id: RelationshipId,
    target: CellId,
    elimination_order: &[CellId],
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> Result<Option<usize>, Error> {
    let rel = &relationships[rel_id];
    let mut candidates: Vec<usize> = rel
        .methods
        .iter()
        .enumerate()
        .filter_map(|(idx, method)| method.outputs.contains(&target).then_some(idx))
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }

    for &eliminated in elimination_order
        .iter()
        .filter(|&&cell| cell != target && rel.adj.contains(&cell))
    {
        candidates.retain(|&idx| !pure_outputs(&rel.methods[idx]).contains(&eliminated));
        if candidates.is_empty() {
            return Err(Error::Conflict {
                sites: vec![ErrorSite::Relationship(rel_id)],
            });
        }
    }

    match candidates.as_slice() {
        [survivor] => Ok(Some(*survivor)),
        _ => Err(Error::Conflict {
            sites: vec![ErrorSite::Relationship(rel_id)],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    use crate::{ErrorSite, Method, Sheet};

    #[test]
    fn build_seeds_reports_conflict_when_elimination_rejects_every_sibling_method() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(7_i32);
        let i32_type = TypeId::of::<i32>();

        let sibling = sheet
            .add_relationship(vec![
                Method::new(
                    vec![b],
                    vec![x, a],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(11_i32), Box::new(13_i32)]),
                ),
                Method::new(
                    vec![a],
                    vec![x, b],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(17_i32), Box::new(19_i32)]),
                ),
                Method::new(
                    vec![x],
                    vec![a, b],
                    vec![i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(23_i32), Box::new(29_i32)]),
                ),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(
                x,
                x,
                |value: &i32| Ok(*value + 1),
            )])
            .unwrap();

        let err = build_seeds(
            &[PlanStep::Method(sibling, 2), PlanStep::Method(claimant, 0)],
            &[a, b, x],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict {
                sites
            } if sites == vec![ErrorSite::Relationship(sibling)]
        ));
    }

    #[test]
    fn build_seeds_reports_conflict_when_multiple_sibling_methods_survive() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let a = sheet.add_cell(5_i32);
        let b = sheet.add_cell(7_i32);
        let i32_type = TypeId::of::<i32>();

        let sibling = sheet
            .add_relationship(vec![
                Method::new(
                    vec![x, b],
                    vec![x, a],
                    vec![i32_type, i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(11_i32), Box::new(13_i32)]),
                ),
                Method::new(
                    vec![x, a],
                    vec![x, b],
                    vec![i32_type, i32_type],
                    vec![i32_type, i32_type],
                    |_| Ok(vec![Box::new(17_i32), Box::new(19_i32)]),
                ),
            ])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(
                x,
                x,
                |value: &i32| Ok(*value + 1),
            )])
            .unwrap();

        let err = build_seeds(
            &[PlanStep::Method(sibling, 0), PlanStep::Method(claimant, 0)],
            &[x],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict {
                sites
            } if sites == vec![ErrorSite::Relationship(sibling)]
        ));
    }
}
