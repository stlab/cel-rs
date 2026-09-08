//! Value-aware source selection for self-referencing blocks: for a connected component of
//! relationships containing at least one self-referencing method, chooses which cell(s)
//! stay literal sources by concretely executing every structurally valid candidate and
//! comparing violated stays, instead of by strength alone. A component with no
//! self-referencing method is untouched, left to [`super::release`]'s existing
//! strength-lexicographic algorithm. See
//! `docs/superpowers/specs/2026-09-07-adam-rs-value-aware-self-ref-planning-design.md` for
//! the full design rationale and literature grounding.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    relationship::{RelationshipData, RelationshipId},
};

use super::digraph::{self, Node};
use super::matching::{self, Assignment};
use super::scc;

/// Returns `true` if any of `rel`'s methods is self-referencing (some cell appears in
/// both its `inputs` and `outputs`).
///
/// - Complexity: O(inputs × outputs) per method.
pub(crate) fn has_self_reference(rel: &RelationshipData) -> bool {
    rel.methods
        .iter()
        .any(|m| m.inputs.iter().any(|i| m.outputs.contains(i)))
}

/// Partitions `active` into its connected components under cell-sharing: two
/// relationships are in the same component iff they share a cell, transitively. A
/// self-referencing chain overlapping a functional diamond via a shared cell always
/// lands in one component — see the design doc's rationale for why no separate cascade
/// step is needed on top of this.
///
/// - Precondition: every relationship in `active` is present in `relationships`, and
///   every cell reachable from them is present in `cells`.
///
/// - Complexity: O(R + sum of adjacency sizes) via breadth-first search.
pub(crate) fn partition_components(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    active: &HashSet<RelationshipId>,
) -> Vec<HashSet<RelationshipId>> {
    let mut visited: HashSet<RelationshipId> = HashSet::new();
    let mut components = Vec::new();

    for &start in active {
        if visited.contains(&start) {
            continue;
        }
        let mut component = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited.insert(start);

        while let Some(rel_id) = queue.pop_front() {
            component.insert(rel_id);
            for &cell_id in &relationships[rel_id].adj {
                for &neighbor in &cells[cell_id].adj {
                    if active.contains(&neighbor) && visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        components.push(component);
    }
    components
}

/// Executes `assignment`'s chosen methods against `cells`' current values, without
/// mutating `cells`. Returns the sorted-descending strengths of every self-referencing
/// output whose tentative value differs from that cell's own `source` (a violated
/// stay), or `None` if any method returns `Err`, produces the wrong number of outputs,
/// or produces an output whose runtime type doesn't match its destination cell's
/// registered type.
///
/// Mirrors `Sheet::execute_plan`'s input rule exactly: a self-referencing input always
/// reads the cell's real `source`, never a tentative value from this same scoring pass;
/// every other input reads a prior method's tentative output if this pass already
/// produced one, else the cell's real current `effective()` value. A purely functional
/// (non-self-referencing) output is executed, since a downstream method may depend on
/// it, but never scored — only a self-referencing output has a stay to violate.
///
/// - Precondition: `assignment`'s induced digraph (`digraph::build_digraph`) is acyclic.
///
/// - Complexity: O(R · K) where R = relationships in `assignment.chosen`, K = cells per
///   method.
fn score_candidate(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    assignment: &Assignment,
) -> Option<Vec<u64>> {
    let adj = digraph::build_digraph(assignment, relationships);
    let mut components = scc::tarjan_scc(&adj);
    components.reverse();

    let mut overlay: HashMap<CellId, Box<dyn Any>> = HashMap::new();
    let mut violated: Vec<u64> = Vec::new();

    for component in components {
        debug_assert_eq!(component.len(), 1, "assignment must already be acyclic");
        let Node::Relationship(rel_id) = component[0] else {
            continue;
        };
        let method_idx = assignment.chosen[&rel_id];
        let method = &relationships[rel_id].methods[method_idx];

        let inputs: Vec<&dyn Any> = method
            .inputs
            .iter()
            .map(|&id| {
                if method.outputs.contains(&id) {
                    cells[id].source.as_ref()
                } else {
                    overlay
                        .get(&id)
                        .map(|v| v.as_ref())
                        .unwrap_or_else(|| cells[id].effective())
                }
            })
            .collect();

        let outputs = (method.function)(&inputs).ok()?;
        if outputs.len() != method.outputs.len() {
            return None;
        }
        if method
            .outputs
            .iter()
            .zip(&outputs)
            .any(|(&output_id, value)| value.as_ref().type_id() != cells[output_id].type_id)
        {
            // A method's declared `output_types` is checked against the destination
            // cell's registered type at `add_relationship` time, but that can't verify
            // what a type-erased closure actually produces at runtime for a given
            // input -- exactly the condition `Sheet::execute_plan` treats as
            // `Error::TypeMismatch`. Scoring this candidate as worst-case (rather than
            // inserting the mismatched value into `overlay`) keeps a downstream method
            // in this same candidate from downcasting it and panicking during planning,
            // before `execute_plan` ever gets a chance to fail gracefully.
            return None;
        }

        for (&output_id, value) in method.outputs.iter().zip(outputs) {
            if method.inputs.contains(&output_id) {
                let cell = &cells[output_id];
                if !(cell.eq_fn)(value.as_ref(), cell.source.as_ref()) {
                    violated.push(cell.strength);
                }
            }
            overlay.insert(output_id, value);
        }
    }

    violated.sort_unstable_by(|a, b| b.cmp(a));
    Some(violated)
}

/// Chooses the value-aware optimal assignment for one connected component containing at
/// least one self-referencing method: enumerates every structurally valid acyclic
/// assignment for `component` ([`matching::Assignment::solve_acyclic_all`]), scores each
/// by concretely executing it against `cells`' current values ([`score_candidate`]), and
/// returns the one whose violated self-referencing stays, sorted descending, are
/// lexicographically smallest — breaking ties by today's rule of lexicographically
/// largest sorted source-set strengths. A candidate whose execution errors is scored as
/// worst-case (`vec![u64::MAX]`) rather than excluded outright, so it can still win the
/// tie-break fallback if every candidate errors.
///
/// - Precondition: every relationship in `component` is present in `relationships`.
///
/// - Postcondition: returns `None` iff no structurally valid acyclic assignment exists
///   for `component` at all, mirroring [`super::release::ReleaseFailure`] (the caller
///   distinguishes `NoAssignment` from `NoAcyclicAssignment` the same way
///   `release::resolve` already does for the non-self-referencing path).
///
/// - Complexity: O(N · R·K) to score N candidates found by
///   [`matching::Assignment::solve_acyclic_all`] (R = relationships in `component`, K =
///   cells per method), on top of that function's own exponential-in-R worst-case
///   search.
pub(crate) fn resolve_component(
    cells: &SlotMap<CellId, CellData>,
    relationships: &SlotMap<RelationshipId, RelationshipData>,
    component: &HashSet<RelationshipId>,
) -> Option<Assignment> {
    let candidates =
        matching::Assignment::solve_acyclic_all(relationships, component, &HashSet::new());

    let source_cells: HashSet<CellId> = component
        .iter()
        .flat_map(|&rel_id| relationships[rel_id].adj.iter().copied())
        .collect();

    candidates
        .into_iter()
        .map(|assignment| {
            let violated =
                score_candidate(cells, relationships, &assignment).unwrap_or(vec![u64::MAX]);
            let mut source_strengths: Vec<u64> = source_cells
                .iter()
                .filter(|c| !assignment.claimed.contains_key(c))
                .map(|&c| cells[c].strength)
                .collect();
            source_strengths.sort_unstable_by(|a, b| b.cmp(a));
            (violated, Reverse(source_strengths), assignment)
        })
        .min_by(|(v1, s1, _), (v2, s2, _)| v1.cmp(v2).then_with(|| s1.cmp(s2)))
        .map(|(_, _, assignment)| assignment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, Sheet};

    #[test]
    fn has_self_reference_true_for_a_method_with_overlapping_inputs_and_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], a, |x: &i32, y: &i32| {
                Ok((*x).min(*y))
            })])
            .unwrap();

        assert!(has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn has_self_reference_false_for_a_purely_functional_relationship() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();

        assert!(!has_self_reference(&sheet.relationships[rel]));
    }

    #[test]
    fn partition_components_merges_relationships_sharing_a_cell() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 1);
        assert_eq!(components[0], active);
    }

    #[test]
    fn partition_components_keeps_disjoint_relationships_in_separate_components() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let d = sheet.add_cell(0_i32);
        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(c, d, |x: &i32| Ok(*x))])
            .unwrap();
        let active: HashSet<_> = [rel1, rel2].into_iter().collect();

        let components = partition_components(&sheet.cells, &sheet.relationships, &active);

        assert_eq!(components.len(), 2);
        let comp1: HashSet<_> = [rel1].into_iter().collect();
        let comp2: HashSet<_> = [rel2].into_iter().collect();
        assert!(
            (components[0] == comp1 && components[1] == comp2)
                || (components[0] == comp2 && components[1] == comp1)
        );
    }

    #[test]
    fn resolve_component_prefers_the_consistent_edit_over_the_higher_strength_cell() {
        // Issue #182: a<=b<=c. Writing a=25 then c=40 is jointly consistent (any
        // b in [25,40] satisfies a<=b<=c), so a's edit must survive even though c's
        // strength (written last) is higher than a's.
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

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source"
        );
        assert_eq!(assignment.claimed[&b], rel1);
        assert_eq!(assignment.claimed[&c], rel2);
    }

    #[test]
    fn resolve_component_matches_todays_choice_when_all_candidates_honor_their_stays() {
        // The discriminating case from issue #182: a=11, b=20 (declared), c=100 already
        // satisfy a<=b<=c, so every candidate source violates nothing -- the tie-break
        // must still pick c (highest strength), matching today's behavior exactly.
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
        sheet.write(a, 11_i32).unwrap();
        sheet.write(c, 0_i32).unwrap();
        sheet.write(c, 100_i32).unwrap();

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&c),
            "c must remain the literal source, matching today's behavior"
        );
        assert_eq!(assignment.claimed[&a], rel1);
        assert_eq!(assignment.claimed[&b], rel2);
    }

    #[test]
    fn resolve_component_treats_a_candidate_execution_error_as_worst_case() {
        // rel's method claiming q always errors; its only alternative (claiming p)
        // always succeeds and honors p's stay. This verifies that a candidate whose
        // execution errors is neither panicked on nor incorrectly preferred over a
        // succeeding one -- it must be scored as worst-case, regardless of q's higher
        // strength.
        let mut sheet = Sheet::new();
        let p = sheet.add_cell(5_i32);
        let q = sheet.add_cell(50_i32);
        sheet.write(q, 50_i32).unwrap();
        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([p, q], p, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::new(
                    vec![p, q],
                    vec![q],
                    vec![std::any::TypeId::of::<i32>(), std::any::TypeId::of::<i32>()],
                    vec![std::any::TypeId::of::<i32>()],
                    |_args| Err(anyhow::anyhow!("boom")),
                ),
            ])
            .unwrap();

        let component: HashSet<_> = [rel].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert_eq!(assignment.claimed[&p], rel);
        assert!(!assignment.claimed.contains_key(&q));
    }

    #[test]
    fn resolve_component_does_not_panic_on_a_mistyped_functional_output() {
        // rel2 declares an i32 output type (passing add_relationship's static check
        // against the cell's registered type) but its closure actually returns a String
        // at runtime -- add_relationship cannot catch this, since it only validates the
        // *declared* type, not what a type-erased closure actually produces. rel3 reads
        // rel2's output as a plain i32 input and downcasts it. Every candidate for this
        // component runs rel2 (it has only one method) then rel3, regardless of which
        // side of rel1's self-referencing pair wins -- so if score_candidate doesn't
        // reject a type-mismatched output before it reaches a downstream consumer, this
        // panics during planning, before execute_plan ever gets a chance to fail
        // gracefully with Error::TypeMismatch.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let y = sheet.add_cell(10_i32);
        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([x, y], y, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();

        let p = sheet.add_cell(0_i32);
        let i32_ty = std::any::TypeId::of::<i32>();
        let rel2 = sheet
            .add_relationship(vec![Method::new(
                vec![y],
                vec![p],
                vec![i32_ty],
                vec![i32_ty],
                |_args| Ok(vec![Box::new("oops".to_string()) as Box<dyn Any>]),
            )])
            .unwrap();

        let z = sheet.add_cell(0_i32);
        let rel3 = sheet
            .add_relationship(vec![Method::from_fn_1_1(p, z, |v: &i32| Ok(*v + 1))])
            .unwrap();

        let component: HashSet<_> = [rel1, rel2, rel3].into_iter().collect();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_component(&sheet.cells, &sheet.relationships, &component)
        }));

        assert!(
            result.is_ok(),
            "resolve_component must not panic on a mistyped functional output"
        );
    }

    #[test]
    fn resolve_component_generalizes_to_a_four_cell_chain() {
        // a<=b<=c<=d via 3 relationships. Writing a=25 (b,c,d untouched since creation)
        // then d=999 makes d the highest strength; today's plain algorithm would
        // (wrongly) keep d as the sole source, overwriting a's edit via the chain
        // (a would end up min'd down to b=20). The value-aware choice must instead keep
        // a as the source (sacrificing only b's untouched, lowest-strength stay).
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(10_i32);
        let b = sheet.add_cell(20_i32);
        let c = sheet.add_cell(30_i32);
        let d = sheet.add_cell(40_i32);
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
        let rel3 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([c, d], c, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([c, d], d, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        sheet.write(a, 25_i32).unwrap();
        sheet.write(d, 999_i32).unwrap();

        let component: HashSet<_> = [rel1, rel2, rel3].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&a),
            "a must remain the literal source, sacrificing only b's untouched stay"
        );
    }

    #[test]
    fn resolve_component_never_scores_a_functional_output_as_a_violated_stay() {
        // x=5 <= y=10 already holds, so rel1 (the self-referencing x<=y pair) carries
        // zero true violations no matter which side it claims. rel2 is a purely
        // functional (non-self-referencing) pair over (y, p) whose two methods are
        // deliberately NOT inverses of each other: method0 (p := y - 5) reproduces p's
        // stored value (5) exactly, but method1 (y := p + 3) does NOT reproduce y's
        // stored value (10) -- it computes 8. That asymmetry means only the candidate
        // where rel2 claims y (leaving p as the source) has a mismatching functional
        // output; the other two candidates' functional outputs match exactly.
        //
        // A correct scorer never counts either mismatch (rel2's output is never
        // self-referencing), so all three candidates tie at zero real violations and p
        // (created last, highest strength) wins the tie-break. If a functional output
        // were wrongly scored instead, the p-remains-source candidate's spurious
        // violation (attached to y's strength) would beat the other two candidates'
        // zero violations, and p would wrongly end up claimed rather than remaining
        // the source -- so this test is verified to fail under that mutation, unlike the
        // version it replaces.
        let mut sheet = Sheet::new();
        let x = sheet.add_cell(5_i32);
        let y = sheet.add_cell(10_i32);
        let p = sheet.add_cell(5_i32);

        let rel1 = sheet
            .add_relationship(vec![
                Method::from_fn_2_1([x, y], x, |x: &i32, y: &i32| Ok((*x).min(*y))),
                Method::from_fn_2_1([x, y], y, |x: &i32, y: &i32| Ok((*x).max(*y))),
            ])
            .unwrap();
        // Purely functional (non-self-referencing) pair sharing y with rel1; its two
        // methods are intentionally not inverses of each other (see comment above).
        let rel2 = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(y, p, |y: &i32| Ok(*y - 5)),
                Method::from_fn_1_1(p, y, |p: &i32| Ok(*p + 3)),
            ])
            .unwrap();

        let component: HashSet<_> = [rel1, rel2].into_iter().collect();
        let assignment = resolve_component(&sheet.cells, &sheet.relationships, &component)
            .expect("a valid acyclic assignment exists");

        assert!(
            !assignment.claimed.contains_key(&p),
            "p must remain the literal source"
        );
        assert_eq!(assignment.claimed[&x], rel1);
        assert_eq!(assignment.claimed[&y], rel2);
    }
}
