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

use slotmap::{Key, SlotMap};

use crate::cell::{CellData, CellId};
use crate::error::{Error, ErrorSite};
use crate::relationship::{RelationshipData, RelationshipId};

use super::{PlanStep, Seeds, matching::pure_outputs};

/// Bundles the immutable planner state one seed-construction pass reuses.
struct SeedBuildContext<'a> {
    claimant: &'a HashMap<CellId, RelationshipId>,
    active: &'a HashSet<RelationshipId>,
    elimination_order: &'a [CellId],
    cells: &'a SlotMap<CellId, CellData>,
    relationships: &'a SlotMap<RelationshipId, RelationshipData>,
}

/// Stores the mutable memoization and DFS path state for one seed-construction pass.
struct SeedTraversal {
    seeds: Seeds,
    path_cells: Vec<CellId>,
    path_relationships: Vec<RelationshipId>,
    path_indices: HashMap<CellId, usize>,
}

/// Lexicographic tie-break data for one sibling seed fold.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeedFoldOrderKey {
    strongest_input: u64,
    selected_method_signature: Vec<u64>,
    relationship_signature: Vec<Vec<u64>>,
}

/// Replays the planner's elimination state to compute the seed for each self-referencing
/// input `execution_order` will read.
///
/// A cell absent from the result reads its own `source` during execution (the common
/// case: no other relationship pushes it). Only a cell that is both claimed by a
/// self-referencing method this round *and* produced by some other incident relationship
/// gets an entry. For each such cell, sibling `target`-producing methods are selected by
/// replaying the planner's elimination order against that relationship's pure-output
/// claims, never by relationship or method declaration order.
///
/// - Precondition: `execution_order`'s `PlanStep::Method` steps name valid method indices
///   in `relationships`.
/// - Precondition: `elimination_order` is the exact cell order the planner's release pass
///   evaluated for tentative elimination this round; `build_seeds` consumes that
///   elimination state verbatim when selecting sibling seed methods.
///
/// # Errors
///
/// - `Error::Conflict` — a sibling relationship that can seed a self-referencing cell
///   has no method compatible with the planner's elimination state, or two sibling seed
///   folds are still structurally indistinguishable after comparing primary strength,
///   selected method signature, and full relationship signature.
/// - `Error::SeedCycle` — sibling seed dependencies form a non-self cycle instead of
///   falling back to any revisited cell's `source` value.
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

    let context = SeedBuildContext {
        claimant: &claimant,
        active: &active,
        elimination_order,
        cells,
        relationships,
    };
    let mut traversal = SeedTraversal {
        seeds: HashMap::new(),
        path_cells: Vec::new(),
        path_relationships: Vec::new(),
        path_indices: HashMap::new(),
    };
    for &cell in &self_ref_cells {
        compute_seed(cell, &context, &mut traversal)?;
    }
    Ok(traversal.seeds)
}

/// Populates `seeds[x]` with `x`'s aspiration, computed by folding every incident
/// relationship other than `x`'s claimant through its elimination-compatible
/// `x`-producing method, ordered by the strongest non-`x` input's strength from weakest
/// to strongest, seeded from `x`'s `source` and each other input's own seed
/// (recursively). Leaves `x` absent when no such relationship exists (its seed is just
/// `source`) or when every candidate method errors or mistypes its output.
///
/// Only relationships in `context.active` (those the current plan actually runs) count as
/// siblings: an inactive conditional branch that happens to name `x` must not seed it.
///
/// `traversal.path_cells`/`traversal.path_relationships` record the active seed
/// dependency path. A sibling input that reaches a cell already on that path reports
/// `Error::SeedCycle` naming the participating cells and sibling relationships in
/// traversal order.
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
/// final seed respects strength rather than relationship insertion order; equal-primary
/// folds break ties by the selected method's ordered signature and then the
/// relationship's full ordered method-signature sequence. If both relationships are
/// still structurally identical under that comparison, propagation rejects the
/// ambiguous shape instead of folding in adjacency order.
///
/// # Errors
///
/// - `Error::Conflict` — some sibling relationship that can seed `x` has no surviving
///   `x`-producing method after replaying the planner's elimination state, or two
///   equal-primary sibling folds remain structurally indistinguishable even after
///   comparing the full ordered method-signature sequence of their relationships.
/// - `Error::SeedCycle` — a sibling seed dependency reaches a currently visiting cell.
fn compute_seed(
    x: CellId,
    context: &SeedBuildContext<'_>,
    traversal: &mut SeedTraversal,
) -> Result<(), Error> {
    if traversal.seeds.contains_key(&x) {
        return Ok(());
    }
    debug_assert!(!traversal.path_indices.contains_key(&x));
    traversal.path_indices.insert(x, traversal.path_cells.len());
    traversal.path_cells.push(x);

    let own_claimant = context.claimant.get(&x).copied();
    let mut sibling_methods: Vec<(RelationshipId, usize, SeedFoldOrderKey)> = context.cells[x]
        .adj
        .iter()
        .filter(|&&rel_id| context.active.contains(&rel_id) && Some(rel_id) != own_claimant)
        .filter_map(|&rel_id| {
            match select_seed_method(rel_id, x, context.elimination_order, context.relationships) {
                Ok(Some(idx)) => {
                    let order_key =
                        seed_fold_order_key(rel_id, idx, x, context.cells, context.relationships);
                    Some(Ok((rel_id, idx, order_key)))
                }
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            }
        })
        .collect::<Result<Vec<_>, Error>>()?;
    sibling_methods.sort_by(|lhs, rhs| lhs.2.cmp(&rhs.2));
    for window in sibling_methods.windows(2) {
        if window[0].2 == window[1].2 {
            return Err(Error::Conflict {
                sites: vec![
                    ErrorSite::Relationship(window[0].0),
                    ErrorSite::Relationship(window[1].0),
                ],
            });
        }
    }

    for &(rel_id, method_idx, _) in &sibling_methods {
        for &input in &context.relationships[rel_id].methods[method_idx].inputs {
            if input != x {
                if let Some(&cycle_start) = traversal.path_indices.get(&input) {
                    let sites = seed_cycle_sites(
                        &traversal.path_cells,
                        &traversal.path_relationships,
                        cycle_start,
                        rel_id,
                        x,
                    );
                    traversal.path_indices.remove(&x);
                    traversal.path_cells.pop();
                    return Err(Error::SeedCycle { sites });
                }
                traversal.path_relationships.push(rel_id);
                let result = compute_seed(input, context, traversal);
                traversal.path_relationships.pop();
                if let Err(err) = result {
                    traversal.path_indices.remove(&x);
                    traversal.path_cells.pop();
                    return Err(err);
                }
            }
        }
    }

    let mut accumulated: Option<Box<dyn Any>> = None;
    for &(rel_id, method_idx, _) in &sibling_methods {
        let method = &context.relationships[rel_id].methods[method_idx];

        let has_weaker_self_referenced_input = context.cells[x].has_explicit_strength()
            && method.inputs.iter().any(|&input| {
                input != x
                    && context.claimant.get(&input) == Some(&rel_id)
                    && context.cells[input].strength <= context.cells[x].strength
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
                            .unwrap_or_else(|| context.cells[x].source.as_ref())
                    } else {
                        traversal
                            .seeds
                            .get(&input)
                            .map(|value| value.as_ref())
                            .unwrap_or_else(|| context.cells[input].source.as_ref())
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
                .filter(|value| value.as_ref().type_id() == context.cells[x].type_id)
        };
        if produced.is_some() {
            accumulated = produced;
        }
    }

    traversal.path_indices.remove(&x);
    traversal.path_cells.pop();
    if let Some(value) = accumulated {
        traversal.seeds.insert(x, value);
    }
    Ok(())
}

/// Returns the deterministic ordering key for one sibling seed fold.
///
/// Equal-primary siblings compare the selected method's ordered signature first, then
/// the relationship's full ordered method-signature sequence. Relationships that still
/// compare equal under this key are structurally indistinguishable to seedfill and are
/// therefore rejected rather than folded in adjacency order.
fn seed_fold_order_key(
    rel_id: RelationshipId,
    method_idx: usize,
    target: CellId,
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
) -> SeedFoldOrderKey {
    let relationship = &relationships[rel_id];
    let method = &relationship.methods[method_idx];
    let strongest_input = method
        .inputs
        .iter()
        .filter(|&&input| input != target)
        .map(|&input| cells[input].strength)
        .max()
        .unwrap_or(0);
    SeedFoldOrderKey {
        strongest_input,
        selected_method_signature: method_content_key(method),
        relationship_signature: relationship
            .methods
            .iter()
            .map(method_content_key)
            .collect(),
    }
}

/// Returns the selected-method component of a deterministic sibling-fold ordering key.
///
/// The key encodes the input and output arities, then the referenced input and output
/// cell IDs in their declared order, so equal-primary sibling folds can compare the
/// selected method before falling back to the whole relationship signature.
///
/// - Complexity: O(k) where k = `method.inputs.len() + method.outputs.len()`.
fn method_content_key(method: &crate::relationship::Method) -> Vec<u64> {
    let mut key = Vec::with_capacity(2 + method.inputs.len() + method.outputs.len());
    key.push(method.inputs.len() as u64);
    key.extend(method.inputs.iter().map(|&input| cell_sort_key(input)));
    key.push(method.outputs.len() as u64);
    key.extend(method.outputs.iter().map(|&output| cell_sort_key(output)));
    key
}

/// Returns a stable numeric sort key for a cell handle within one sheet.
///
/// `slotmap` keys carry both slot and generation; `as_ffi()` exposes that stable payload
/// as an opaque integer suitable for deterministic ordering without relying on debug
/// formatting or relationship insertion order.
fn cell_sort_key(cell: CellId) -> u64 {
    cell.data().as_ffi()
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
/// purely claim that cell under the matching layer's semantics ([`pure_outputs`]). The
/// surviving method is therefore the one consistent with the planner's actual release
/// decisions this round, never simply the first declaration that mentions `target`.
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

    #[test]
    fn build_seeds_breaks_equal_strength_ties_by_ordered_signature() {
        fn ordered_max(
            first: CellId,
            second: CellId,
            third: CellId,
            strongest: CellId,
            output: CellId,
        ) -> Method {
            Method::new(
                vec![first, second, third, strongest],
                vec![output],
                vec![
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                ],
                vec![TypeId::of::<i32>()],
                |args| {
                    let lhs = args[0]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    let rhs = args[1]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    Ok(vec![Box::new((*lhs).max(*rhs))])
                },
            )
        }

        fn ordered_min(
            first: CellId,
            second: CellId,
            third: CellId,
            strongest: CellId,
            output: CellId,
        ) -> Method {
            Method::new(
                vec![first, second, third, strongest],
                vec![output],
                vec![
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                    TypeId::of::<i32>(),
                ],
                vec![TypeId::of::<i32>()],
                |args| {
                    let lhs = args[0]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    let rhs = args[1]
                        .downcast_ref::<i32>()
                        .expect("type checked at add_relationship");
                    Ok(vec![Box::new((*lhs).min(*rhs))])
                },
            )
        }

        fn build_sheet(
            reverse_siblings: bool,
        ) -> (
            Sheet,
            RelationshipId,
            RelationshipId,
            RelationshipId,
            CellId,
        ) {
            let mut sheet = Sheet::new();
            let x = sheet.add_cell(4_i32);
            let a = sheet.add_cell(5_i32);
            let b = sheet.add_cell(3_i32);
            let shared_strongest = sheet.add_cell(0_i32);

            let a_le_x = vec![
                ordered_min(a, x, b, shared_strongest, a),
                ordered_max(a, x, b, shared_strongest, x),
            ];
            let x_le_b = vec![
                ordered_min(b, x, a, shared_strongest, x),
                ordered_max(b, x, a, shared_strongest, b),
            ];
            let claimant = sheet
                .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
                .unwrap();
            let (a_le_x_rel, x_le_b_rel) = if reverse_siblings {
                let x_le_b_rel = sheet.add_relationship(x_le_b).unwrap();
                let a_le_x_rel = sheet.add_relationship(a_le_x).unwrap();
                (a_le_x_rel, x_le_b_rel)
            } else {
                let a_le_x_rel = sheet.add_relationship(a_le_x).unwrap();
                let x_le_b_rel = sheet.add_relationship(x_le_b).unwrap();
                (a_le_x_rel, x_le_b_rel)
            };

            (sheet, a_le_x_rel, x_le_b_rel, claimant, x)
        }

        let (forward_sheet, forward_up, forward_down, forward_claimant, forward_x) =
            build_sheet(false);
        let forward_seeds = build_seeds(
            &[
                PlanStep::Method(forward_up, 0),
                PlanStep::Method(forward_down, 1),
                PlanStep::Method(forward_claimant, 0),
            ],
            &[forward_x],
            &forward_sheet.cells,
            &forward_sheet.relationships,
        )
        .expect("forward build_seeds should succeed");

        let (reversed_sheet, reversed_up, reversed_down, reversed_claimant, reversed_x) =
            build_sheet(true);
        let reversed_seeds = build_seeds(
            &[
                PlanStep::Method(reversed_up, 0),
                PlanStep::Method(reversed_down, 1),
                PlanStep::Method(reversed_claimant, 0),
            ],
            &[reversed_x],
            &reversed_sheet.cells,
            &reversed_sheet.relationships,
        )
        .expect("reversed build_seeds should succeed");

        let forward_seed = forward_seeds[&forward_x]
            .downcast_ref::<i32>()
            .expect("seed should stay typed as i32");
        let reversed_seed = reversed_seeds[&reversed_x]
            .downcast_ref::<i32>()
            .expect("seed should stay typed as i32");
        assert_eq!(*forward_seed, 3);
        assert_eq!(*reversed_seed, 3);
    }

    #[test]
    fn seed_fold_order_key_uses_full_relationship_structure_after_selected_signature() {
        fn selected_method(x: CellId, a: CellId, b: CellId) -> Method {
            Method::new(
                vec![x, a],
                vec![x, b],
                vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
                vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
                |_| Ok(vec![Box::new(0_i32), Box::new(0_i32)]),
            )
        }

        fn inert_method(inputs: Vec<CellId>, outputs: Vec<CellId>) -> Method {
            let output_len = outputs.len();
            let input_len = inputs.len();
            Method::new(
                inputs,
                outputs,
                vec![TypeId::of::<i32>(); input_len],
                vec![TypeId::of::<i32>(); output_len],
                move |_| {
                    Ok((0..output_len)
                        .map(|_| Box::new(0_i32) as Box<dyn Any>)
                        .collect())
                },
            )
        }

        let mut sheet = Sheet::new();
        let x = sheet.add_cell(1_i32);
        let a = sheet.add_cell(9_i32);
        let b = sheet.add_cell(0_i32);

        let first = sheet
            .add_relationship(vec![
                selected_method(x, a, b),
                inert_method(vec![x, b], vec![a, b]),
                inert_method(vec![b], vec![a, x]),
            ])
            .unwrap();
        let second = sheet
            .add_relationship(vec![
                selected_method(x, a, b),
                inert_method(vec![b], vec![a, x]),
                inert_method(vec![x, b], vec![a, b]),
            ])
            .unwrap();

        let first_key = seed_fold_order_key(first, 0, x, &sheet.cells, &sheet.relationships);
        let second_key = seed_fold_order_key(second, 0, x, &sheet.cells, &sheet.relationships);

        assert_eq!(first_key.strongest_input, second_key.strongest_input);
        assert_eq!(
            first_key.selected_method_signature,
            second_key.selected_method_signature
        );
        assert_ne!(
            first_key.relationship_signature,
            second_key.relationship_signature
        );
        assert_ne!(first_key, second_key);
    }

    #[test]
    fn build_seeds_rejects_structurally_identical_equal_primary_siblings() {
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(1_i32);
        let a = sheet.add_cell(9_i32);

        let first = sheet
            .add_relationship(vec![Method::from_fn_2_1(
                [x, a],
                x,
                |value: &i32, _: &i32| Ok(*value + 1),
            )])
            .unwrap();
        let second = sheet
            .add_relationship(vec![Method::from_fn_2_1(
                [x, a],
                x,
                |value: &i32, _: &i32| Ok(*value * 2),
            )])
            .unwrap();
        let claimant = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, x, |value: &i32| Ok(*value))])
            .unwrap();

        let err = build_seeds(
            &[
                PlanStep::Method(first, 0),
                PlanStep::Method(second, 0),
                PlanStep::Method(claimant, 0),
            ],
            &[a, x],
            &sheet.cells,
            &sheet.relationships,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            crate::Error::Conflict { sites }
                if sites
                    == vec![
                        ErrorSite::Relationship(first),
                        ErrorSite::Relationship(second),
                    ]
        ));
    }
}
