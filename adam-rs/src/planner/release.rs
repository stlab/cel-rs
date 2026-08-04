//! Chooses which cells are sources by greedily releasing cells in descending strength
//! order, checking at each step whether a matching + acyclic assignment still exists
//! with that cell (and every previously released cell) forbidden from being claimed.

use std::cmp::Reverse;
use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

use super::digraph::is_acyclic;
use super::matching::Assignment;

/// Finds the strength-optimal acyclic assignment: an [`Assignment`] where the set of
/// cells left unclaimed (sources) is lexicographically maximal in descending strength
/// order among all assignments whose induced digraph is acyclic.
///
/// Processes cells in descending strength order; for each currently-claimed cell,
/// tentatively adds it to the forbidden set and re-solves. If a matching still exists
/// and its induced digraph is acyclic, the release is kept; otherwise the cell remains
/// claimed. This single mechanism handles both ordinary strength-based method
/// selection (an uncontested relationship's choice of which cell to leave exogenous)
/// and cyclic ("diamond") resolution uniformly -- both are just instances of "does
/// releasing this cell still admit a valid acyclic assignment".
///
/// Returns `None` if no acyclic assignment exists at all (a genuine algebraic loop, or
/// no assignment exists whatsoever).
///
/// - Complexity: O(C · solve) where C = cells and `solve` is [`Assignment::solve`]'s
///   cost -- each cell triggers at most one full re-solve attempt. This omits two
///   further per-cell costs not folded into `solve`: [`is_acyclic`]'s own traversal of
///   the candidate's digraph, and cloning `released` (an O(C)-sized `HashSet`) to build
///   each `candidate_released` -- together closer to O(C²) for this part alone.
pub(crate) fn resolve(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Option<Assignment> {
    let mut released: HashSet<CellId> = HashSet::new();
    let mut current = Assignment::solve(relationships, active, &released)?;

    let mut cells_sorted: Vec<CellId> = cells.keys().collect();
    cells_sorted.sort_by_key(|&id| Reverse(cells[id].strength));

    for cell in cells_sorted {
        if !current.claimed.contains_key(&cell) {
            released.insert(cell);
            continue;
        }

        let mut candidate_released = released.clone();
        candidate_released.insert(cell);

        if let Some(candidate) = Assignment::solve(relationships, active, &candidate_released)
            && is_acyclic(&candidate, relationships)
        {
            released = candidate_released;
            current = candidate;
        }
    }

    is_acyclic(&current, relationships).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn no_assignment_returns_none() {
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
        assert!(resolve(&sheet.cells, &sheet.relationships, &active).is_none());
    }

    #[test]
    fn genuinely_unsolvable_cycle_returns_none() {
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
        assert!(resolve(&sheet.cells, &sheet.relationships, &active).is_none());
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
        // begin/examples/diamond.adm2). resolve() must still find a valid, acyclic
        // assignment -- not return None.
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
    }
}
