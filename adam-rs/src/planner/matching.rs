//! Bipartite/hypergraph matching: assigns each active relationship one of its methods
//! such that no two relationships claim the same cell as a pure (non-self-referencing)
//! output, optionally forbidding specific cells from being claimed by anyone at all.

use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::{
    cell::CellId,
    relationship::{Method, RelationshipData, RelationshipId},
};

/// Returns the cells `method` writes but does not read: the cells that must not
/// already be determined for the method to be eligible, and that become claimed by
/// whichever relationship selects it.
///
/// Self-referencing cells (present in both `inputs` and `outputs`) are excluded: they
/// are read at their pre-execution value, so a self-referencing method places no
/// exclusive claim on them.
///
/// - Complexity: O(K²) where K = cells per method (`inputs.contains` scans linearly).
pub(crate) fn pure_outputs(method: &Method) -> HashSet<CellId> {
    method
        .outputs
        .iter()
        .filter(|o| !method.inputs.contains(o))
        .copied()
        .collect()
}

/// Records enough information to undo one mutation to an `Assignment`, for `try_assign`'s backtracking.
enum Change {
    Assigned(RelationshipId, Option<usize>),
    Claimed(CellId, Option<RelationshipId>),
}

/// One method chosen per active relationship, and which relationship currently claims
/// each pure-output cell.
pub(crate) struct Assignment {
    pub(crate) chosen: HashMap<RelationshipId, usize>,
    pub(crate) claimed: HashMap<CellId, RelationshipId>,
}

impl Assignment {
    /// Finds an assignment of one method per relationship in `active` such that no cell
    /// in `forbidden` is claimed as a pure output by anyone, and no two relationships
    /// claim the same cell.
    ///
    /// Relationships are considered in `relationships`' natural (insertion-stable)
    /// order restricted to `active`, so the result is deterministic across calls with
    /// the same inputs.
    ///
    /// Returns `None` if no such assignment exists for any combination of method
    /// choices.
    ///
    /// - Complexity: O(R² · M · K) worst case (R = active relationships, M = methods
    ///   per relationship, K = cells per method): each relationship's assignment search
    ///   may recursively displace up to R-1 others, each doing an O(M·K) scan.
    pub(crate) fn solve(
        relationships: &SlotMap<RelationshipId, RelationshipData>,
        active: &HashSet<RelationshipId>,
        forbidden: &HashSet<CellId>,
    ) -> Option<Self> {
        let mut this = Assignment {
            chosen: HashMap::new(),
            claimed: HashMap::new(),
        };
        let order: Vec<RelationshipId> = relationships.keys().filter(|r| active.contains(r)).collect();
        for rel_id in order {
            if this.chosen.contains_key(&rel_id) {
                continue; // already assigned as a side effect of an earlier displacement
            }
            let mut visited = HashSet::new();
            let mut trail = Vec::new();
            if !this.try_assign(rel_id, relationships, &mut visited, &mut trail, forbidden) {
                return None;
            }
        }
        Some(this)
    }

    /// Attempts to find (and commit) a method for `rel_id` whose pure outputs avoid
    /// `forbidden`, recursively displacing other relationships' claims via augmenting
    /// search when a candidate method's outputs are already claimed. `visited` prevents
    /// re-entering a relationship already being displaced earlier in this same search.
    ///
    /// - Complexity: O(M · (R + K)) per call, recursively bounded by the number of
    ///   distinct relationships in `visited` (at most R).
    fn try_assign(
        &mut self,
        rel_id: RelationshipId,
        relationships: &SlotMap<RelationshipId, RelationshipData>,
        visited: &mut HashSet<RelationshipId>,
        trail: &mut Vec<Change>,
        forbidden: &HashSet<CellId>,
    ) -> bool {
        if !visited.insert(rel_id) {
            return false;
        }

        let rel = &relationships[rel_id];
        for (method_idx, method) in rel.methods.iter().enumerate() {
            let outputs = pure_outputs(method);
            if !outputs.is_disjoint(forbidden) {
                continue;
            }

            let mark = trail.len();
            // While resolving blockers below, nobody (including a displaced blocker)
            // may reclaim one of THIS method's own target outputs -- they're reserved
            // for `rel_id` for the duration of this attempt.
            let mut inner_forbidden = forbidden.clone();
            inner_forbidden.extend(outputs.iter().copied());

            let blockers: HashSet<RelationshipId> = outputs
                .iter()
                .filter_map(|c| self.claimed.get(c).copied())
                .filter(|&r| r != rel_id)
                .collect();

            let mut ok = true;
            for blocker in blockers {
                if visited.contains(&blocker) {
                    // Already resolved (or currently being resolved further up the call
                    // stack) as a side effect of displacing a different blocker in this
                    // same attempt.
                    continue;
                }
                if let Some(&old_idx) = self.chosen.get(&blocker) {
                    let old_outputs = pure_outputs(&relationships[blocker].methods[old_idx]);
                    self.clear_assignment(blocker, trail);
                    for c in old_outputs {
                        if self.claimed.get(&c) == Some(&blocker) {
                            self.clear_claim(c, trail);
                        }
                    }
                }
                if !self.try_assign(blocker, relationships, visited, trail, &inner_forbidden) {
                    ok = false;
                    break;
                }
            }

            if ok {
                for &c in &outputs {
                    self.set_claim(c, rel_id, trail);
                }
                self.set_assignment(rel_id, method_idx, trail);
                return true;
            }

            self.undo(trail, mark);
        }
        false
    }

    /// Records `rel`'s previous method choice in `trail` before assigning it to `idx`.
    fn set_assignment(&mut self, rel: RelationshipId, idx: usize, trail: &mut Vec<Change>) {
        trail.push(Change::Assigned(rel, self.chosen.insert(rel, idx)));
    }

    /// Records `rel`'s current method choice in `trail` before removing it.
    fn clear_assignment(&mut self, rel: RelationshipId, trail: &mut Vec<Change>) {
        trail.push(Change::Assigned(rel, self.chosen.remove(&rel)));
    }

    /// Records `cell`'s previous claim (if any) in `trail` before claiming it for `rel`.
    fn set_claim(&mut self, cell: CellId, rel: RelationshipId, trail: &mut Vec<Change>) {
        trail.push(Change::Claimed(cell, self.claimed.insert(cell, rel)));
    }

    /// Records `cell`'s current claim in `trail` before removing it.
    fn clear_claim(&mut self, cell: CellId, trail: &mut Vec<Change>) {
        trail.push(Change::Claimed(cell, self.claimed.remove(&cell)));
    }

    /// Reverts every change recorded in `trail` since `mark`.
    fn undo(&mut self, trail: &mut Vec<Change>, mark: usize) {
        while trail.len() > mark {
            match trail.pop().expect("loop condition checked len > mark") {
                Change::Assigned(rel, Some(idx)) => {
                    self.chosen.insert(rel, idx);
                }
                Change::Assigned(rel, None) => {
                    self.chosen.remove(&rel);
                }
                Change::Claimed(cell, Some(r)) => {
                    self.claimed.insert(cell, r);
                }
                Change::Claimed(cell, None) => {
                    self.claimed.remove(&cell);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn single_relationship_single_method_is_assigned() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen[&rel], 0);
        assert_eq!(assignment.claimed[&b], rel);
        assert!(!assignment.claimed.contains_key(&a));
    }

    #[test]
    fn two_relationships_wanting_the_same_only_output_is_infeasible() {
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
        assert!(Assignment::solve(&sheet.relationships, &active, &HashSet::new()).is_none());
    }

    #[test]
    fn diamond_relationships_admit_a_feasible_assignment() {
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
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len(), "no two relationships may claim the same cell");
    }

    #[test]
    fn self_referencing_output_does_not_conflict_with_a_different_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |x: &i32| Ok((*x).min(0)))])
            .unwrap();
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        let active: HashSet<_> = [r1, r2].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen.len(), 2);
        assert!(!assignment.claimed.contains_key(&a));
        assert_eq!(assignment.claimed[&b], r2);
    }

    #[test]
    fn multi_output_method_claims_all_its_outputs_when_forced() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell("a".to_string());
        let b = sheet.add_cell("b".to_string());
        let c = sheet.add_cell("ab".to_string());
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([a, b], c, |x: &String, y: &String| Ok(x.clone() + y)),
                Method::new(
                    vec![c],
                    vec![a, b],
                    vec![std::any::TypeId::of::<String>()],
                    vec![std::any::TypeId::of::<String>(), std::any::TypeId::of::<String>()],
                    |args| {
                        let z = args[0].downcast_ref::<String>().unwrap();
                        let mut chars = z.chars();
                        let first = chars.next().unwrap_or_default().to_string();
                        let rest = chars.collect::<String>();
                        Ok(vec![Box::new(first), Box::new(rest)])
                    },
                ),
            ])
            .unwrap();
        let active: HashSet<_> = [rel].into_iter().collect();

        let unconstrained = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(unconstrained.chosen[&rel], 0);
        assert_eq!(unconstrained.claimed[&c], rel);

        let mut forbidden = HashSet::new();
        forbidden.insert(c);
        let constrained = Assignment::solve(&sheet.relationships, &active, &forbidden).unwrap();
        assert_eq!(constrained.chosen[&rel], 1);
        assert_eq!(constrained.claimed[&a], rel);
        assert_eq!(constrained.claimed[&b], rel);
        assert!(!constrained.claimed.contains_key(&c));
    }

    #[test]
    fn blocker_displacement_falls_back_when_the_cascade_cannot_complete() {
        // R1's only method wants `x`, currently claimed by R2's default method; R2's
        // only alternative wants `y`, currently claimed by R3's default method; R3's
        // only alternative claims `q`, which is free. Exercises multi-level blocker
        // resolution without corrupting state on the way to the final assignment.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let x = sheet.add_cell(0_i32);
        let q = sheet.add_cell(0_i32);
        let y = sheet.add_cell(0_i32);
        let s = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(p, x, |v: &i32| Ok(*v))])
            .unwrap();
        // R2's two methods both reference {q, x, y} -- method 0 ignores y, method 1
        // ignores x.
        let r2 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([q, y], x, |v: &i32, _y: &i32| Ok(*v)),
                Method::from_fn_2_1([q, x], y, |v: &i32, _x: &i32| Ok(*v)),
            ])
            .unwrap();
        // R3's two methods both reference {s, q, y} -- method 0 ignores q, method 1
        // ignores y.
        let r3 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([s, q], y, |v: &i32, _q: &i32| Ok(*v)),
                Method::from_fn_2_1([s, y], q, |v: &i32, _y: &i32| Ok(*v)),
            ])
            .unwrap();
        let active: HashSet<_> = [r1, r2, r3].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
        assert_eq!(assignment.chosen.len(), 3);
        let unique: HashSet<_> = assignment.claimed.values().collect();
        assert_eq!(unique.len(), assignment.claimed.len());
    }
}
