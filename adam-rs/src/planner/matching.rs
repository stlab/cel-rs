//! Bipartite/hypergraph matching: assigns each active relationship one of its methods
//! such that no two relationships claim the same cell as an output (self-referencing
//! outputs included), optionally forbidding specific cells from being claimed by anyone
//! at all.

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
    Visited(RelationshipId),
}

/// One method chosen per active relationship, and which relationship currently claims
/// each output cell (self-referencing outputs included).
pub(crate) struct Assignment {
    pub(crate) chosen: HashMap<RelationshipId, usize>,
    pub(crate) claimed: HashMap<CellId, RelationshipId>,
}

impl Assignment {
    /// Finds an assignment of one method per relationship in `active` such that no cell
    /// in `forbidden` is claimed as an output by anyone, and no two relationships
    /// claim the same cell.
    ///
    /// Relationships are considered in `relationships`' natural (insertion-stable)
    /// order restricted to `active`, so the result is deterministic across calls with
    /// the same inputs, with one caveat: whenever a single candidate method has more
    /// than one simultaneous blocker, they are resolved in `HashSet<RelationshipId>`
    /// iteration order, which is not sorted. Feasibility and soundness do not depend on
    /// this order -- only, potentially, the tie-break of *which* valid assignment is
    /// found is not guaranteed deterministic across builds/runs when more than one
    /// exists.
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
        let order: Vec<RelationshipId> = relationships
            .keys()
            .filter(|r| active.contains(r))
            .collect();
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

    /// Attempts to find (and commit) a method for `rel_id` whose outputs avoid
    /// `forbidden`, recursively displacing other relationships' claims via augmenting
    /// search when a candidate method's outputs are already claimed. `visited` prevents
    /// re-entering a relationship already being displaced (or exhausted) elsewhere in
    /// this search; the entry is undoable via the same `trail` mechanism as `chosen`/
    /// `claimed`, so a relationship marked visited while being unsuccessfully displaced
    /// during one candidate method of `rel_id` is un-poisoned before the next candidate
    /// method is tried -- it must not stay marked past the specific attempt that
    /// introduced it, or a later attempt would wrongly treat it as already resolved
    /// even though its claim was restored by `undo` and never actually vacated. `rel_id`
    /// itself is the one exception: its own entry is pushed before this function's
    /// per-candidate-method loop begins (and therefore before that loop's own `mark`),
    /// so it remains marked for this call's entire duration, preventing self-recursion.
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
        trail.push(Change::Visited(rel_id));

        let rel = &relationships[rel_id];
        for (method_idx, method) in rel.methods.iter().enumerate() {
            let outputs: HashSet<CellId> = method.outputs.iter().copied().collect();
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
                    let old_outputs: HashSet<CellId> = relationships[blocker].methods[old_idx]
                        .outputs
                        .iter()
                        .copied()
                        .collect();
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

            self.undo(trail, mark, visited);
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

    /// Reverts every change recorded in `trail` since `mark`, including any
    /// `visited` markings introduced after `mark` (so a relationship that was marked
    /// visited while being unsuccessfully displaced during the undone attempt becomes
    /// eligible for a fresh displacement attempt again).
    fn undo(
        &mut self,
        trail: &mut Vec<Change>,
        mark: usize,
        visited: &mut HashSet<RelationshipId>,
    ) {
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
                Change::Visited(rel) => {
                    visited.remove(&rel);
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
        assert_eq!(
            unique.len(),
            assignment.claimed.len(),
            "no two relationships may claim the same cell"
        );
    }

    #[test]
    fn self_referencing_output_is_claimed_and_does_not_conflict_with_a_different_relationship() {
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
        assert_eq!(assignment.claimed[&a], r1);
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
                    vec![
                        std::any::TypeId::of::<String>(),
                        std::any::TypeId::of::<String>(),
                    ],
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

        let unconstrained =
            Assignment::solve(&sheet.relationships, &active, &HashSet::new()).unwrap();
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

    /// Asserts that `assignment` is internally consistent: for every relationship's
    /// chosen method, every cell that method outputs must be claimed by that same
    /// relationship. A stale or incorrectly-skipped blocker resolution can silently
    /// desynchronize `claimed` from `chosen` without ever violating the weaker
    /// "no two relationships claim the same cell" check other tests use (since the
    /// desync manifests as a claim silently pointing to the *wrong* relationship, not
    /// as two relationships both claiming the same cell in `claimed`).
    fn assert_chosen_claims_consistent(
        assignment: &Assignment,
        relationships: &SlotMap<RelationshipId, RelationshipData>,
    ) {
        for (&rel, &idx) in &assignment.chosen {
            for &cell in &relationships[rel].methods[idx].outputs {
                assert_eq!(
                    assignment.claimed.get(&cell),
                    Some(&rel),
                    "cell {cell:?} is output by {rel:?}'s chosen method but claimed by a \
                     different relationship (or unclaimed)"
                );
            }
        }
    }

    #[test]
    fn undone_blocker_displacement_does_not_leave_stale_visited_poisoning() {
        // Regression test: `try_assign`'s shared `visited` set must be rolled back
        // (via the same `trail`/`undo` mechanism as `chosen`/`claimed`) when a
        // candidate method's blocker resolution fails, not just left to accumulate for
        // the rest of the call. Before that fix, a relationship marked `visited` while
        // being unsuccessfully displaced during one candidate method stayed
        // "poisoned" for the rest of the caller's candidate-method loop: a later
        // candidate encountering the same relationship as a blocker would then treat
        // it as "already resolved" (via `if visited.contains(&blocker) { continue; }`)
        // even though `undo` had restored its original claim -- silently overwriting
        // that still-valid claim instead of properly re-displacing it.
        //
        // R_B has two methods: B0 (default) claims {u, v}; B1 claims {u} only (frees
        // v, using v as a plain input instead).
        // R_A has two methods: A0 wants u; A1 wants v.
        //
        // Displacing R_B for A0 (forbidding u) fails: B1 also outputs u, so no
        // candidate of R_B can free u -- this is the "first displacement fails"
        // attempt, and it marks R_B (and, transitively, nothing else) visited before
        // failing and fully undoing.
        //
        // A1 (wanting v) then encounters R_B as a blocker again. With the fix, R_B is
        // no longer stuck in `visited` and is freshly re-displaced: B1 is tried, and
        // since it doesn't output v, it succeeds, freeing v for R_A. The resulting
        // assignment is valid and mutually consistent.
        //
        // Before the fix, R_B's stale `visited` entry caused R_A's second attempt to
        // skip re-displacing it entirely, silently claiming v for R_A while R_B's
        // unchanged, still-active chosen method (B0) also genuinely outputs v --
        // exactly the double-write this invariant check catches.
        let mut sheet = Sheet::new();
        let p_b = sheet.add_cell(0_i32);
        let u = sheet.add_cell(0_i32);
        let v = sheet.add_cell(0_i32);
        let i32_ty = std::any::TypeId::of::<i32>();

        let r_b = sheet
            .add_relationship(vec![
                Method::new(
                    vec![p_b],
                    vec![u, v],
                    vec![i32_ty],
                    vec![i32_ty, i32_ty],
                    |args| {
                        let p = *args[0].downcast_ref::<i32>().unwrap();
                        Ok(vec![Box::new(p), Box::new(p)])
                    },
                ),
                Method::new(
                    vec![p_b, v],
                    vec![u],
                    vec![i32_ty, i32_ty],
                    vec![i32_ty],
                    |args| {
                        let p = *args[0].downcast_ref::<i32>().unwrap();
                        Ok(vec![Box::new(p)])
                    },
                ),
            ])
            .unwrap();

        let r_a = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(v, u, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(u, v, |x: &i32| Ok(*x)),
            ])
            .unwrap();

        let active: HashSet<_> = [r_b, r_a].into_iter().collect();
        let assignment = Assignment::solve(&sheet.relationships, &active, &HashSet::new())
            .expect("a valid assignment exists: R_B falls back to freeing v while keeping u");

        assert_chosen_claims_consistent(&assignment, &sheet.relationships);
        assert_eq!(
            assignment.chosen[&r_b], 1,
            "R_B must fall back to B1 to free v for R_A"
        );
        assert_eq!(
            assignment.chosen[&r_a], 1,
            "R_A must fall back to A1 (wants v)"
        );
        assert_eq!(assignment.claimed[&u], r_b);
        assert_eq!(assignment.claimed[&v], r_a);
    }
}
