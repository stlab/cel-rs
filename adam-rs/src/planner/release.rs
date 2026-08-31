//! Chooses which cells are sources by greedily releasing cells in descending strength
//! order, checking at each step whether a matching + acyclic assignment still exists
//! with that cell (and every previously released cell) forbidden from being claimed.
//!
//! This module has no visibility into a filter's dynamic-argument dependencies —
//! `digraph::add_filter_edges` adds those edges to the digraph only *after*
//! `resolve` has already finished searching (see
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

/// Finds the strength-optimal acyclic assignment: an [`Assignment`] where the set of
/// cells left unclaimed (sources) is lexicographically maximal in descending strength
/// order among all assignments whose induced digraph is acyclic.
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
/// - [`ReleaseFailure::NoAssignment`] — no method assignment exists at all, cyclic or
///   not.
/// - [`ReleaseFailure::NoAcyclicAssignment`] — a method assignment exists, but none of
///   them is acyclic.
///
/// - Complexity: O(C · `solve_acyclic`) where C = cells -- each cell triggers one
///   `solve_acyclic` attempt, itself exponential in the number of active relationships
///   in the worst case (see its own doc comment).
pub(crate) fn resolve(
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
}
