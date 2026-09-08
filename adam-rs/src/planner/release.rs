//! Chooses which cells are sources.
//!
//! [`resolve`] partitions the active relationship set into connected components
//! ([`super::stay::partition_components`]). A component with no self-referencing method
//! is resolved by [`resolve_plain`]: greedily releasing cells in descending strength
//! order, checking at each step whether a matching + acyclic assignment still exists
//! with that cell (and every previously released cell) forbidden from being claimed. A
//! component containing at least one self-referencing method is instead resolved by
//! [`super::stay::resolve_component`], which compares candidates by the concrete values
//! they would produce rather than by strength alone — plain strength-lexicographic
//! release is not value-safe once a method can read one of its own outputs; see
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md`.
//!
//! This module has no visibility into a filter's dynamic-argument dependencies —
//! `digraph::add_filter_edges` adds those edges to the digraph only *after* `resolve` has
//! already finished searching (see
//! `docs/superpowers/specs/2026-08-25-adam-rs-filter-revalidation-design.md` §3).
//! `resolve`'s acyclicity guarantee therefore holds only for the relationship-only
//! subgraph; `plan()` re-checks acyclicity once more after filter edges are added,
//! returning `Error::FilterCycle` (distinct from this module's own `Error::Cycle`)
//! if that combined graph turns out cyclic. Generalizing `resolve` itself to search
//! around filter edges is tracked as issue #153.

use std::cmp::Reverse;
use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

use super::matching::Assignment;
use super::stay;

/// Why [`resolve`] could not find a strength-optimal acyclic assignment.
#[derive(Debug)]
pub(crate) enum ReleaseFailure {
    /// No method assignment exists at all for `active`, acyclic or not -- e.g. two
    /// relationships whose only methods both claim the same cell.
    NoAssignment,
    /// A method assignment exists, but every one of them is cyclic: a genuine
    /// algebraic loop with no external input, regardless of cell strength.
    NoAcyclicAssignment,
}

/// Finds the optimal acyclic assignment for `active`, dispatching each connected
/// component ([`stay::partition_components`]) to whichever algorithm applies: a
/// component with no self-referencing method is resolved by [`resolve_plain`]
/// (unchanged strength-lexicographic release); a component containing at least one is
/// resolved by [`stay::resolve_component`] (value-aware release). The two never
/// interact, since components are disjoint by construction, so their results merge
/// directly.
///
/// Failure precedence is computed across *every* component before returning, matching
/// the single monolithic pre-partition algorithm's semantics: since components are
/// cell-disjoint, a full assignment over `active` exists iff one exists for every
/// individual component, so [`ReleaseFailure::NoAssignment`] takes precedence over
/// [`ReleaseFailure::NoAcyclicAssignment`] even when a *different* component is the one
/// that failed acyclicity — checking only the first failing component (in partition
/// order) would report a less severe failure than the sheet as a whole actually has.
///
/// # Errors
///
/// - [`ReleaseFailure::NoAssignment`] — no method assignment exists at all for some
///   component, cyclic or not.
/// - [`ReleaseFailure::NoAcyclicAssignment`] — a method assignment exists for every
///   component, but at least one component admits no acyclic assignment.
///
/// - Complexity: see [`resolve_plain`] and [`stay::resolve_component`]; components are
///   independent, so their costs add rather than multiply.
pub(crate) fn resolve(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Assignment, ReleaseFailure> {
    let mut plain: HashSet<RelationshipId> = HashSet::new();
    let mut value_aware: Vec<HashSet<RelationshipId>> = Vec::new();
    for component in stay::partition_components(cells, relationships, active) {
        if component
            .iter()
            .any(|&rel_id| stay::has_self_reference(&relationships[rel_id]))
        {
            value_aware.push(component);
        } else {
            plain.extend(component);
        }
    }

    let plain_result = resolve_plain(cells, relationships, &plain);
    let component_results: Vec<Result<Assignment, ReleaseFailure>> = value_aware
        .iter()
        .map(|component| {
            stay::resolve_component(cells, relationships, component).ok_or_else(|| {
                if Assignment::solve(relationships, component, &HashSet::new()).is_some() {
                    ReleaseFailure::NoAcyclicAssignment
                } else {
                    ReleaseFailure::NoAssignment
                }
            })
        })
        .collect();

    let any_no_assignment = matches!(plain_result, Err(ReleaseFailure::NoAssignment))
        || component_results
            .iter()
            .any(|r| matches!(r, Err(ReleaseFailure::NoAssignment)));
    if any_no_assignment {
        return Err(ReleaseFailure::NoAssignment);
    }

    let mut assignment = plain_result?;
    for result in component_results {
        let component_assignment = result?;
        assignment.chosen.extend(component_assignment.chosen);
        assignment.claimed.extend(component_assignment.claimed);
    }

    Ok(assignment)
}

/// Finds the strength-optimal acyclic assignment for a relationship set containing no
/// self-referencing method: an [`Assignment`] where the set of cells left unclaimed
/// (sources) is lexicographically maximal in descending strength order among all
/// assignments whose induced digraph is acyclic.
///
/// Processes every cell in descending strength order, tentatively adding it to the
/// forbidden set and searching for an assignment that is both valid (no double claims)
/// and acyclic with that cell -- and every previously accepted release -- forbidden from
/// being claimed ([`Assignment::solve_acyclic`]). The release is kept only when such an
/// assignment exists. This single mechanism handles both ordinary strength-based method
/// selection (an uncontested relationship's choice of which cell to leave exogenous)
/// and cyclic ("diamond") resolution uniformly -- both are just instances of "does
/// releasing this cell still admit a valid acyclic assignment".
///
/// Every cell is re-checked this way, even one that happens not to be claimed by the
/// current best assignment: a cell being currently unclaimed is an artifact of
/// `solve_acyclic`'s deterministic method-choice order, not proof that leaving it a
/// source is compatible with releasing every higher-strength cell still to come, so it
/// cannot be adopted as released without the same check every other cell gets.
///
/// # Errors
///
/// - [`ReleaseFailure::NoAssignment`] -- no method assignment exists at all, cyclic or
///   not.
/// - [`ReleaseFailure::NoAcyclicAssignment`] -- a method assignment exists, but none of
///   them is acyclic.
///
/// - Complexity: O(C · `solve_acyclic`) where C = cells -- each cell triggers one
///   `solve_acyclic` attempt, itself exponential in the number of active relationships
///   in the worst case (see its own doc comment).
fn resolve_plain(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Result<Assignment, ReleaseFailure> {
    let mut released: HashSet<CellId> = HashSet::new();
    let Some(mut current) = Assignment::solve_acyclic(relationships, active, &released) else {
        return Err(
            if Assignment::solve(relationships, active, &released).is_some() {
                ReleaseFailure::NoAcyclicAssignment
            } else {
                ReleaseFailure::NoAssignment
            },
        );
    };

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for cell in cells_sorted {
        let mut candidate_released = released.clone();
        candidate_released.insert(cell);

        if let Some(candidate) =
            Assignment::solve_acyclic(relationships, active, &candidate_released)
        {
            released = candidate_released;
            current = candidate;
        }
    }

    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn no_assignment_returns_no_assignment_failure() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let out = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, out, |x: &i32| Ok(*x))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, out, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        assert!(matches!(
            resolve(&sheet.cells, &sheet.relationships, &active),
            Err(ReleaseFailure::NoAssignment)
        ));
    }

    #[test]
    fn genuinely_unsolvable_cycle_returns_no_acyclic_assignment_failure() {
        // x = f(y); y = g(x), each with only one method and no other cell involved:
        // no acyclic assignment exists no matter which cell is released.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        assert!(matches!(
            resolve(&sheet.cells, &sheet.relationships, &active),
            Err(ReleaseFailure::NoAcyclicAssignment)
        ));
    }

    #[test]
    fn strength_prefers_the_higher_strength_cell_as_source() {
        // Triangle a,b,c: a and b are written (higher strength) after c is added, so
        // a and b must remain sources and c must be derived, regardless of method
        // iteration order.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        sheet.write(a, 2.0).unwrap();
        sheet.write(b, 3.0).unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();
        assert_eq!(assignment.claimed[&c], rel);
        assert!(!assignment.claimed.contains_key(&a));
        assert!(!assignment.claimed.contains_key(&b));
    }

    #[test]
    fn diamond_collision_pattern_resolves_instead_of_failing() {
        // R1{a,b,c}, R2{b,c,d}: a and d outrank b and c (the collision pattern from
        // begin/examples/diamond.adm2). {a, d} can never both be sources for this
        // structure, so the strength-optimal resolution keeps d (strength 24, the
        // higher of the two) and sacrifices a, promoting c (the next-highest
        // remaining cell) as the other source instead of b.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let c = sheet.add_cell(0.0_f64);
        let d = sheet.add_cell(0.0_f64);
        let r1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], d, |x: &f64, y: &f64| Ok(x * y)),
                Method::from_fn_2_1([b, d], c, |x: &f64, y: &f64| Ok(y / x)),
                Method::from_fn_2_1([c, d], b, |x: &f64, y: &f64| Ok(y / x)),
            ])
            .unwrap();
        sheet.write(a, 3.0).unwrap();
        sheet.write(d, 24.0).unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active)
            .expect("a valid acyclic assignment exists for this structure");
        assert_eq!(assignment.chosen.len(), 2);
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len());

        assert!(!assignment.claimed.contains_key(&d), "d must stay a source");
        assert!(
            assignment.claimed.contains_key(&a),
            "a cannot coexist with d as a source: must be claimed (derived)"
        );
        assert!(
            !assignment.claimed.contains_key(&c),
            "c outranks b among the remaining candidates: must be the other source"
        );
        assert!(
            assignment.claimed.contains_key(&b),
            "b must be claimed (derived)"
        );
    }

    #[test]
    fn resolve_prefers_the_consistent_edit_over_the_higher_strength_cell() {
        // The issue #182 shape, exercised through the public resolve() dispatch (not
        // stay::resolve_component directly) to confirm the partition/merge wiring
        // itself routes a self-referencing component to the value-aware path.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([a, b], b, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([b, c], b, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([b, c], c, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 25_i32).unwrap();
        sheet.write(c, 40_i32).unwrap();

        let active: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source"
        );
    }

    #[test]
    fn resolve_merges_independent_plain_and_value_aware_components() {
        // A self-referencing pair (x,y) fully disjoint from a functional diamond
        // (p,q,r): each must be resolved by its own algorithm and merged without
        // interference.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let y = sheet.add_cell(10_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([x, y], y, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();

        let p = sheet.add_cell(0.0_f64);
        let q = sheet.add_cell(0.0_f64);
        let r = sheet.add_cell(0.0_f64);
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([p, q], r, |a: &f64, b: &f64| Ok(a * b)),
                Method::from_fn_2_1([p, r], q, |a: &f64, b: &f64| Ok(b / a)),
                Method::from_fn_2_1([q, r], p, |a: &f64, b: &f64| Ok(b / a)),
            ])
            .unwrap();
        sheet.write(p, 2.0).unwrap();
        sheet.write(q, 3.0).unwrap();

        let active: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve(&sheet.cells, &sheet.relationships, &active).unwrap();

        assert_eq!(assignment.chosen.len(), 2);
        assert_eq!(
            assignment.claimed[&r], rel2,
            "the functional diamond must still pick r as the derived cell"
        );
    }

    #[test]
    fn resolve_reports_no_assignment_when_any_component_lacks_one_even_if_another_only_fails_acyclically()
     {
        // A plain component that's a genuine algebraic loop (x=f(y); y=g(x), single
        // method each, no self-reference) -- NoAcyclicAssignment on its own, mirroring
        // genuinely_unsolvable_cycle_returns_no_acyclic_assignment_failure -- alongside a
        // disjoint self-referencing component whose two relationships both insist on
        // claiming the same cell with no alternative method -- NoAssignment on its own.
        // The aggregate failure must be NoAssignment, matching the pre-partition
        // monolithic algorithm's semantics (a full assignment exists iff every disjoint
        // component has one), not NoAcyclicAssignment just because resolve_plain --
        // checked first -- only sees its own component's cyclic-only failure.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(y, x, |v: &i32| Ok(*v + 1))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(x, y, |v: &i32| Ok(*v + 1))])
            .unwrap();

        let a = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).max(0)))])
            .unwrap();

        let active: HashSet<_> = sheet.relationships().collect();
        let result = resolve(&sheet.cells, &sheet.relationships, &active);

        assert!(matches!(result, Err(ReleaseFailure::NoAssignment)));
    }
}
