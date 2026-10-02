//! Prepares immutable propagation artifacts and retains one eligible plan per phase.
//!
//! Propagation takes eligibility out of the sheet, borrows prepared artifacts while
//! staging fresh values, and restores eligibility only on success. Structural mutations
//! drop both entries and shared signatures; writes independently drop claiming phases.
//! The last successful prepared snapshot remains available for display through either.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::planner::{
    Plan, PlanStep, SeedEvaluationCache, SeedRecipes, SeedSignatures, SourceCertificate,
    evaluate_seeds,
};
use crate::{CellId, Error, FilterViolation, RelationshipId, RequirementId};

use super::{PlanProvenance, PropagationStage, Sheet};

/// Identifies the independent unconditional and final-active planning phases.
#[derive(Clone, Copy)]
enum PlanPhase {
    Unconditional,
    Main,
}

/// Owns immutable structural artifacts for one selected phase and active set.
///
/// Values and seed evaluation state never survive a propagation through this object.
pub(super) struct PreparedPlan {
    pub(super) plan: Plan,
    pub(super) active: HashSet<RelationshipId>,
    pub(super) certificate: SourceCertificate,
    provenance: PlanProvenance,
    prerequisite_steps: Vec<PlanStep>,
    seeds: SeedRecipes,
    signatures: Rc<SeedSignatures>,
    diagnostic_outputs: HashSet<CellId>,
}

/// Retains one eligible prepared plan and pending post-processing certification.
pub(super) struct CachedPlan {
    prepared: Rc<PreparedPlan>,
    needs_certificate_check: bool,
}

/// Counts actual solver, preparation, signature, and certificate work per sheet.
#[cfg(test)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct ReuseStats {
    pre_plans: usize,
    main_plans: usize,
    preparations: usize,
    signature_builds: usize,
    certificate_checks: usize,
}

impl Sheet {
    /// Drops structural eligibility and signatures without changing completed display state.
    ///
    /// - Complexity: O(A) when the final owner drops prepared artifacts of total size A.
    pub(super) fn invalidate_prepared_plans(&mut self) {
        self.pre_plan_cache = None;
        self.main_plan_cache = None;
        self.seed_signatures = None;
    }

    /// Independently rejects plans that claim a successfully written cell.
    ///
    /// Released-source promotion preserves certificates without a priority scan.
    ///
    /// - Precondition: `cell` is a live, writeable cell with a validated value type.
    /// - Complexity: Expected O(1) membership work, plus O(A) for dropped artifacts.
    pub(super) fn invalidate_after_write(&mut self, cell: CellId) {
        debug_assert!(self.cells.contains_key(cell));
        debug_assert_ne!(self.cells[cell].kind, crate::cell::CellKind::Out);
        for cache in [&mut self.pre_plan_cache, &mut self.main_plan_cache] {
            if cache
                .as_ref()
                .is_some_and(|entry| !entry.prepared.certificate.is_released(cell))
            {
                *cache = None;
            }
        }
    }

    /// Acquires a phase plan, preparing only on active-set or certificate misses.
    ///
    /// - Precondition: `active` contains only live relationship IDs.
    /// - Precondition: `cache` belongs to `phase` and the unchanged current structure.
    /// - Postcondition: returns an immutable prepared plan for `active`.
    /// - Complexity: Hits take expected O(R) active-set comparison and O(C) only for a
    ///   pending certificate scan. Misses add planner, provenance, guard-cone, seed-recipe,
    ///   and O(M · K) signature work; R is active relationships, C cells, M methods,
    ///   and K maximum referenced cells per method.
    ///
    /// # Errors
    ///
    /// Returns the planner's assignment/dependency errors or guard-prerequisite errors.
    fn acquire_prepared_plan<'cache>(
        &mut self,
        cache: &'cache mut Option<CachedPlan>,
        active: &HashSet<RelationshipId>,
        phase: PlanPhase,
    ) -> Result<&'cache PreparedPlan, Error> {
        debug_assert!(active.iter().all(|id| self.relationships.contains_key(*id)));
        let reusable = cache.as_mut().is_some_and(|entry| {
            if entry.prepared.active != *active {
                return false;
            }
            if entry.needs_certificate_check {
                #[cfg(test)]
                {
                    self.reuse_stats.certificate_checks += 1;
                }
                if !entry.prepared.certificate.is_valid(&self.cells) {
                    return false;
                }
                entry.needs_certificate_check = false;
            }
            true
        });
        if !reusable {
            #[cfg(test)]
            match phase {
                PlanPhase::Unconditional => self.reuse_stats.pre_plans += 1,
                PlanPhase::Main => self.reuse_stats.main_plans += 1,
            }
            let plan = crate::planner::plan(&self.cells, &self.relationships, active)?;
            let mut claimed = HashSet::new();
            for &step in &plan.execution_order {
                if let PlanStep::Method(relationship, index) = step {
                    claimed.extend(
                        self.relationships[relationship].methods[index]
                            .outputs
                            .iter()
                            .copied(),
                    );
                }
            }
            let released = self
                .cells
                .keys()
                .filter(|id| !claimed.contains(id))
                .collect();
            let certificate =
                SourceCertificate::new(&self.cells, &plan.elimination_order, released);
            let provenance = self.plan_provenance(&plan);
            let prerequisite_steps = match phase {
                PlanPhase::Unconditional => self.guard_prerequisite_steps(&plan.execution_order)?,
                PlanPhase::Main => Vec::new(),
            };
            let seed_steps = match phase {
                PlanPhase::Unconditional => &prerequisite_steps,
                PlanPhase::Main => &plan.execution_order,
            };
            let seeds = SeedRecipes::new(
                &plan.execution_order,
                seed_steps,
                &self.cells,
                &self.relationships,
            );
            let signatures = Rc::clone(self.seed_signatures.get_or_insert_with(|| {
                #[cfg(test)]
                {
                    self.reuse_stats.signature_builds += 1;
                }
                Rc::new(SeedSignatures::new(&self.relationships))
            }));
            #[cfg(test)]
            {
                self.reuse_stats.preparations += 1;
            }
            *cache = Some(CachedPlan {
                prepared: Rc::new(PreparedPlan {
                    plan,
                    active: active.clone(),
                    certificate,
                    provenance,
                    prerequisite_steps,
                    seeds,
                    signatures,
                    diagnostic_outputs: claimed,
                }),
                needs_certificate_check: false,
            });
        }
        Ok(&cache.as_ref().expect("acquired plan exists").prepared)
    }

    /// Executes fresh staged values against independent eligible phase artifacts.
    ///
    /// Validation precedes changed-state clearing. Staged errors occur before commit;
    /// requirement errors occur after commit. Completed display state is published only
    /// after all diagnostics complete, and eligibility is restored by the caller only
    /// on success.
    ///
    /// - Complexity: Includes [`Sheet::propagate`]'s planning and evaluation costs;
    ///   eligible hits omit structural preparation and scan priorities only when pending.
    ///
    /// # Errors
    ///
    /// Returns validation, planning, seed, prerequisite, execution, or requirement errors
    /// with the same commit boundary as [`Sheet::propagate`].
    pub(super) fn propagate_with_caches(
        &mut self,
        pre: &mut Option<CachedPlan>,
        main: &mut Option<CachedPlan>,
    ) -> Result<(), Error> {
        self.validate()?;
        self.clear_changed();
        let mut stage = PropagationStage::default();
        let mut source_filter_violations = Vec::new();
        let mut seed_evaluation_cache = SeedEvaluationCache::default();

        let pre_plan = if self.conditionals.is_empty() {
            None
        } else {
            let active = self
                .relationships
                .keys()
                .filter(|id| !self.conditional_relationships.contains(id))
                .collect();
            let prepared = self.acquire_prepared_plan(pre, &active, PlanPhase::Unconditional)?;
            let seeds = {
                let source = |id| stage.seed_source(&self.cells, id);
                evaluate_seeds(
                    &prepared.seeds,
                    &prepared.signatures,
                    &self.cells,
                    &self.relationships,
                    &source,
                    &mut seed_evaluation_cache,
                )?
            };
            self.execute_plan_staged(
                &prepared.prerequisite_steps,
                &seeds,
                &prepared.plan.forced_outputs,
                &prepared.provenance,
                &mut stage,
                &mut source_filter_violations,
            )?;
            Some(prepared)
        };

        let active = self.build_active_set_staged(&stage)?;
        let prepared = self.acquire_prepared_plan(main, &active, PlanPhase::Main)?;
        if let Some(prepared_pre) = pre_plan {
            for &step in &prepared_pre.prerequisite_steps {
                if !self.prerequisite_step_is_compatible(
                    &prepared_pre.provenance,
                    &prepared.provenance,
                    step,
                    &stage,
                ) {
                    return Err(Error::Conflict {
                        sites: self.conflict_sites_for_step(
                            step,
                            &prepared_pre.provenance,
                            &prepared.provenance,
                        ),
                    });
                }
            }
        }
        let seeds = {
            let source = |id| stage.seed_source(&self.cells, id);
            evaluate_seeds(
                &prepared.seeds,
                &prepared.signatures,
                &self.cells,
                &self.relationships,
                &source,
                &mut seed_evaluation_cache,
            )?
        };
        self.execute_plan_staged(
            &prepared.plan.execution_order,
            &seeds,
            &prepared.plan.forced_outputs,
            &prepared.provenance,
            &mut stage,
            &mut source_filter_violations,
        )?;

        let previously_derived = self.commit_stage(stage);
        let strengths_changed = self.post_process_strengths(&prepared.plan.execution_order);
        for id in previously_derived {
            if let Some(cell) = self.cells.get_mut(id)
                && cell.derived.is_none()
                && !cell.changed
            {
                cell.changed = true;
                self.changed_cells.push(id);
            }
        }

        let mut last_requirement_violations: HashMap<CellId, Vec<RequirementId>> = HashMap::new();
        for (requirement_id, requirement) in &self.requirements {
            let inputs: Vec<&dyn Any> = requirement
                .inputs
                .iter()
                .map(|&id| self.cells[id].effective())
                .collect();
            let holds = (requirement.function)(&inputs).map_err(|error| Error::MethodFailed {
                error,
                sites: vec![],
            })?;
            if !holds {
                last_requirement_violations
                    .entry(requirement.cell)
                    .or_default()
                    .push(requirement_id);
            }
        }
        self.last_requirement_violations = last_requirement_violations;

        let mut last_filter_violations: HashMap<CellId, FilterViolation> =
            source_filter_violations.into_iter().collect();
        for &cell_id in &prepared.diagnostic_outputs {
            let Some(filter) = self.cells[cell_id].filter.as_ref() else {
                continue;
            };
            let args: Vec<&dyn Any> = filter
                .args
                .iter()
                .map(|&a| self.cells[a].effective())
                .collect();
            let current = self.cells[cell_id].effective();
            match (filter.function)(current, &args) {
                Ok(conformed) => {
                    let cell = &self.cells[cell_id];
                    if conformed.as_ref().type_id() != cell.type_id {
                        last_filter_violations.insert(
                            cell_id,
                            FilterViolation::Failed(anyhow::anyhow!(
                                "filter returned a value of a different type than the cell"
                            )),
                        );
                    } else if !(cell.eq_fn)(conformed.as_ref(), current) {
                        last_filter_violations.insert(cell_id, FilterViolation::NotConformed);
                    }
                }
                Err(error) => {
                    last_filter_violations.insert(cell_id, FilterViolation::Failed(error));
                }
            }
        }
        self.last_filter_violations = last_filter_violations;

        if strengths_changed {
            for entry in [pre.as_mut(), main.as_mut()].into_iter().flatten() {
                entry.needs_certificate_check = true;
            }
        }
        self.last_plan = Some(Rc::clone(
            &main.as_ref().expect("main plan exists").prepared,
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        CellId, CellKind, Error, ErrorSite, Filter, FilterViolation, MatchExpr, Method,
        RelationshipId, Requirement, Sheet,
    };

    /// Propagates with fresh planning in both phases, retaining the previous display snapshot.
    ///
    /// - Complexity: The same as uncached `Sheet::propagate`.
    /// # Errors
    /// Returns the propagation error without changing its commit boundary.
    fn propagate_forced_replan(sheet: &mut Sheet) -> Result<(), Error> {
        sheet.pre_plan_cache = None;
        sheet.main_plan_cache = None;
        sheet.propagate()
    }

    /// Returns a fixture cell's comparable current filter diagnostic.
    ///
    /// - Complexity: O(error message length).
    fn filter_status(sheet: &Sheet, cell: CellId) -> Option<String> {
        sheet
            .filter_violation(cell)
            .map(|violation| match violation {
                FilterViolation::NotConformed => "not-conformed".to_owned(),
                FilterViolation::Failed(error) => format!("failed:{error}"),
            })
    }

    /// Returns violated-requirement membership in attachment order for a fixture cell.
    ///
    /// - Complexity: O(Q²), where Q is the cell's requirement count.
    fn requirement_status(sheet: &Sheet, cell: CellId) -> Vec<bool> {
        sheet
            .cell_requirements(cell)
            .expect("registered fixture cell")
            .iter()
            .map(|requirement| {
                sheet
                    .violated_requirements(cell)
                    .any(|id| id == *requirement)
            })
            .collect()
    }

    /// Asserts equivalent completed state for corresponding integer fixture cells.
    ///
    /// - Complexity: O(C² + R²K + sum(Q²) + diagnostic message lengths).
    ///   C counts fixture cells, R relationships, K method outputs, and Q requirements per cell.
    fn assert_same_snapshot(
        automatic: &Sheet,
        automatic_ids: &[CellId],
        forced: &Sheet,
        forced_ids: &[CellId],
    ) {
        assert_eq!(automatic_ids.len(), forced_ids.len());
        for (&left, &right) in automatic_ids.iter().zip(forced_ids) {
            assert_eq!(
                automatic.read::<i32>(left).unwrap(),
                forced.read::<i32>(right).unwrap()
            );
            assert_eq!(
                automatic.source::<i32>(left).unwrap(),
                forced.source::<i32>(right).unwrap()
            );
            assert_eq!(automatic.is_source(left), forced.is_source(right));
            assert_eq!(automatic.is_forced(left), forced.is_forced(right));
            assert_eq!(
                automatic.changed().any(|id| id == left),
                forced.changed().any(|id| id == right),
            );
            assert_eq!(
                automatic.cell_requirements_valid(left),
                forced.cell_requirements_valid(right)
            );
            assert_eq!(
                requirement_status(automatic, left),
                requirement_status(forced, right)
            );
            assert_eq!(filter_status(automatic, left), filter_status(forced, right));
        }
        let choices = |sheet: &Sheet| {
            sheet
                .relationships()
                .map(|relationship| {
                    (
                        sheet.selected_method(relationship),
                        sheet.is_relationship_forced(relationship),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(choices(automatic), choices(forced));
    }

    /// Returns comparable error sites using fixture declaration ordinals, not slot-map keys.
    ///
    /// Selected-method cycles normalize their unspecified starting node by rotation,
    /// preserving traversal direction. Other diagnostics preserve their specified order.
    ///
    /// - Precondition: Every error site belongs to the fixture.
    /// - Complexity: O(S(C + R)), for S sites, C cells, and R relationships.
    fn error_sites(sheet: &Sheet, error: &Error) -> Vec<(u8, usize, usize)> {
        let cell = |id| {
            debug_assert!(sheet.cells.contains_key(id));
            sheet.cells().position(|candidate| candidate == id).unwrap()
        };
        let relationship = |id| {
            debug_assert!(sheet.relationships.contains_key(id));
            sheet
                .relationships()
                .position(|candidate| candidate == id)
                .unwrap()
        };
        let mut sites: Vec<_> = error
            .sites()
            .iter()
            .map(|site| match *site {
                ErrorSite::Cell(id) => (0, cell(id), 0),
                ErrorSite::Relationship(id) => (1, relationship(id), 0),
                ErrorSite::Method(id, index) => (2, relationship(id), index),
                ErrorSite::MethodIndex(index) => (3, index, 0),
            })
            .collect();
        if matches!(error, Error::Cycle { .. })
            && let Some((start, _)) = sites.iter().enumerate().min_by_key(|(_, site)| *site)
        {
            sites.rotate_left(start);
        }
        sites
    }

    /// Propagates a pair and compares error variants, messages, and ordered diagnostic sites.
    ///
    /// - Complexity: Propagation plus O(S(C + R) + error message lengths).
    fn assert_same_result(automatic: &mut Sheet, forced: &mut Sheet) {
        match (automatic.propagate(), propagate_forced_replan(forced)) {
            (Ok(()), Ok(())) => {}
            (Err(left), Err(right)) => {
                assert_eq!(
                    std::mem::discriminant(&left),
                    std::mem::discriminant(&right)
                );
                assert_eq!(left.to_string(), right.to_string());
                assert_eq!(error_sites(automatic, &left), error_sites(forced, &right));
            }
            (left, right) => panic!("different propagation results: {left:?}, {right:?}"),
        }
    }

    /// Asserts corresponding guards' values, aspirations, changed membership, and source status.
    ///
    /// - Complexity: O(C + diagnostic message lengths), for C changed cells.
    fn assert_same_guard(left: &Sheet, a: CellId, right: &Sheet, b: CellId) {
        assert_eq!(
            left.read::<bool>(a).unwrap(),
            right.read::<bool>(b).unwrap()
        );
        assert_eq!(
            left.source::<bool>(a).unwrap(),
            right.source::<bool>(b).unwrap()
        );
        assert_eq!(left.is_source(a), right.is_source(b));
        assert_eq!(left.is_forced(a), right.is_forced(b));
        assert_eq!(
            left.changed().any(|id| id == a),
            right.changed().any(|id| id == b)
        );
        assert_eq!(filter_status(left, a), filter_status(right, b));
    }

    /// Checks current derived guard staging across preserved and flipped branches.
    #[test]
    fn differential_derived_boolean_guard() {
        let build = || {
            let mut sheet = Sheet::new();
            let input = sheet.add_source(true);
            let guard = sheet.add_cell(false);
            let value = sheet.add_source(8_i32);
            let output = sheet.add_cell(0_i32);
            sheet
                .add_relationship(vec![Method::from_fn_1_1(input, guard, |x: &bool| Ok(*x))])
                .unwrap();
            let branch = sheet
                .add_relationship(vec![Method::from_fn_1_1(value, output, |x: &i32| Ok(*x))])
                .unwrap();
            sheet
                .add_conditional(
                    MatchExpr::cell(guard),
                    vec![(vec![true], vec![branch])],
                    vec![],
                )
                .unwrap();
            (sheet, [value, output], [input, guard])
        };
        let (mut automatic, ids, guards) = build();
        let (mut forced, other, other_guards) = build();
        for _ in 0..3 {
            assert_same_result(&mut automatic, &mut forced);
        }
        let initial = automatic.reuse_stats;
        for (enabled, value) in [(true, 11_i32), (false, 12), (false, 13), (true, 14)] {
            automatic.write(guards[0], enabled).unwrap();
            forced.write(other_guards[0], enabled).unwrap();
            automatic.write(ids[0], value).unwrap();
            forced.write(other[0], value).unwrap();
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            for (&left, &right) in guards.iter().zip(&other_guards) {
                assert_same_guard(&automatic, left, &forced, right);
            }
            assert_eq!(
                *automatic.read::<i32>(ids[1]).unwrap(),
                if enabled { value } else { 0 }
            );
            // Main demotion can cross a conservative source prefix in either phase.
            // Settle that certificate before measuring the next unchanged branch.
            for _ in 0..3 {
                assert_same_result(&mut automatic, &mut forced);
                assert_same_snapshot(&automatic, &ids, &forced, &other);
            }
        }
        let settled = automatic.reuse_stats;
        automatic.write(guards[0], true).unwrap();
        forced.write(other_guards[0], true).unwrap();
        automatic.propagate().unwrap();
        propagate_forced_replan(&mut forced).unwrap();
        assert_same_snapshot(&automatic, &ids, &forced, &other);
        assert_eq!(automatic.reuse_stats.pre_plans, settled.pre_plans);
        assert_eq!(automatic.reuse_stats.main_plans, settled.main_plans);
        assert_eq!(
            automatic.reuse_stats.signature_builds,
            initial.signature_builds
        );
    }

    /// Checks both identity directions, filtered aspirations, and requirement diagnostics.
    #[test]
    fn differential_identity_write_sequence() {
        let (mut automatic, a, b, _) = identity_sheet();
        let (mut forced, x, y, _) = identity_sheet();
        for (sheet, cells) in [(&mut automatic, [a, b]), (&mut forced, [x, y])] {
            sheet
                .add_filter(
                    cells[1],
                    Filter::from_fn_0(|value: &i32| Ok((*value).min(6))),
                )
                .unwrap();
            sheet
                .add_requirement(
                    cells[0],
                    None,
                    Requirement::from_fn_1(cells[0], |value: &i32| Ok(*value >= 0)),
                )
                .unwrap();
        }
        assert_same_result(&mut automatic, &mut forced);
        for (target, value) in [(1_usize, 5_i32), (1, 8), (0, 3), (1, -2), (0, 0)] {
            automatic.write([a, b][target], value).unwrap();
            forced.write([x, y][target], value).unwrap();
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert_same_snapshot(&automatic, &[a, b], &forced, &[x, y]);
        }
        assert!(automatic.reuse_stats.main_plans < forced.reuse_stats.main_plans);
    }

    /// Builds the unconditional identity and conditional forced claimant phase fixture.
    fn phase_sheet() -> (Sheet, [CellId; 3], CellId) {
        let (mut sheet, a, b, _) = identity_sheet();
        let source = sheet.add_source(7_i32);
        let guard = sheet.add_source(true);
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(source, a, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![true], vec![branch])],
                vec![],
            )
            .unwrap();
        (sheet, [a, b, source], guard)
    }

    /// Checks settled certificates, cross-phase demotion, and phase-local write eligibility.
    #[test]
    fn differential_phase_local_releases_and_branch_writes() {
        let (mut automatic, ids, guard) = phase_sheet();
        let (mut forced, other, other_guard) = phase_sheet();
        assert_same_result(&mut automatic, &mut forced);
        let initial = automatic.reuse_stats;
        // Main claims/demotes a, crossing the initial unconditional source prefix.
        assert_same_result(&mut automatic, &mut forced);
        assert_eq!(automatic.reuse_stats.pre_plans, initial.pre_plans + 1);
        assert_eq!(automatic.reuse_stats.main_plans, initial.main_plans);
        assert_same_snapshot(&automatic, &ids, &forced, &other);
        assert_same_result(&mut automatic, &mut forced);
        let settled = automatic.reuse_stats;
        assert_same_result(&mut automatic, &mut forced);
        assert_eq!(automatic.reuse_stats, settled);
        let released = ids[..2]
            .iter()
            .position(|id| {
                automatic
                    .pre_plan_cache
                    .as_ref()
                    .unwrap()
                    .prepared
                    .certificate
                    .is_released(*id)
            })
            .unwrap();
        assert!(!automatic.is_source(ids[released]));
        automatic.write(ids[released], 11_i32).unwrap();
        forced.write(other[released], 11_i32).unwrap();
        assert_same_result(&mut automatic, &mut forced);
        assert_eq!(automatic.reuse_stats.pre_plans, settled.pre_plans);
        assert_eq!(automatic.reuse_stats.main_plans, settled.main_plans + 1);
        assert_same_snapshot(&automatic, &ids, &forced, &other);
        for value in [true, false, false, true, true] {
            let before = automatic.reuse_stats;
            let previous = *automatic.read::<bool>(guard).unwrap();
            automatic.write(guard, value).unwrap();
            forced.write(other_guard, value).unwrap();
            assert_same_result(&mut automatic, &mut forced);
            assert_eq!(automatic.reuse_stats.pre_plans, before.pre_plans);
            assert_eq!(
                automatic.reuse_stats.main_plans,
                before.main_plans + usize::from(previous != value)
            );
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            assert_same_guard(&automatic, guard, &forced, other_guard);
        }
    }

    /// Applies one actual structural change, retaining all new integer cells in fixture order.
    ///
    /// - Precondition: `mutation` is in 0..8 and IDs contain the identity fixture.
    /// - Complexity: The selected public mutation's documented complexity.
    fn mutate_fixture(sheet: &mut Sheet, ids: &mut Vec<CellId>, mutation: usize) {
        debug_assert!(mutation < 8);
        debug_assert!(ids.len() >= 3);
        debug_assert!(ids.iter().all(|id| sheet.cells.contains_key(*id)));
        match mutation {
            0 => ids.push(sheet.add_cell(4_i32)),
            1 => ids.push(sheet.add_source(4_i32)),
            2 => {
                sheet
                    .add_relationship(vec![Method::from_fn_1_1(ids[0], ids[0], |x: &i32| Ok(*x))])
                    .unwrap();
            }
            3 => {
                let branch = sheet.relationships().next().unwrap();
                sheet
                    .add_conditional(
                        MatchExpr::cell(ids[2]),
                        vec![(vec![0_i32], vec![branch])],
                        vec![],
                    )
                    .unwrap();
            }
            4 => sheet
                .add_filter(ids[1], Filter::from_fn_0(|x: &i32| Ok((*x).min(0))))
                .unwrap(),
            5 => {
                sheet
                    .add_requirement(
                        ids[0],
                        None,
                        Requirement::from_fn_1(ids[0], |x: &i32| Ok(*x >= 0)),
                    )
                    .unwrap();
            }
            6 | 7 => {
                let requirements = if mutation == 7 {
                    vec![
                        (
                            Some("check"),
                            Requirement::from_fn_1(ids[2], |x: &i32| Ok(*x >= 0)),
                        ),
                        (
                            Some("check"),
                            Requirement::from_fn_1(ids[2], |x: &i32| Ok(*x >= 0)),
                        ),
                    ]
                } else {
                    vec![]
                };
                let result = sheet.add_out(
                    Method::from_fn_1_1(ids[1], ids[2], |x: &i32| Ok(*x)),
                    requirements,
                );
                if mutation == 7 {
                    assert!(matches!(result, Err(Error::InvalidRequirement)));
                    assert_eq!(sheet.cell_requirements(ids[2]).unwrap().len(), 1);
                } else {
                    result.unwrap();
                }
                assert_eq!(sheet.cell_kind(ids[2]), Some(CellKind::Out));
                assert_eq!(sheet.relationships().count(), 2);
            }
            _ => unreachable!(),
        }
    }

    /// Checks every successful structural surface and partially failing out attachment.
    #[test]
    fn differential_structural_mutations_and_rejected_operations() {
        for mutation in 0..8 {
            let (mut automatic, a, b, _) = identity_sheet();
            let (mut forced, x, y, _) = identity_sheet();
            let mut ids = vec![a, b, automatic.add_cell(0_i32)];
            let mut other = vec![x, y, forced.add_cell(0_i32)];
            for _ in 0..3 {
                assert_same_result(&mut automatic, &mut forced);
            }
            let before = automatic.reuse_stats;
            mutate_fixture(&mut automatic, &mut ids, mutation);
            mutate_fixture(&mut forced, &mut other, mutation);
            automatic.validate().unwrap();
            forced.validate().unwrap();
            assert_same_result(&mut automatic, &mut forced);
            assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
            assert_eq!(
                automatic.reuse_stats.signature_builds,
                before.signature_builds + 1
            );
            assert!(automatic.reuse_stats.preparations > before.preparations);
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            for _ in 0..3 {
                assert_same_result(&mut automatic, &mut forced);
            }
            let settled = automatic.reuse_stats;
            for (sheet, cells) in [(&mut automatic, &ids), (&mut forced, &other)] {
                assert!(matches!(
                    sheet.write(cells[1], false),
                    Err(Error::TypeMismatch { .. })
                ));
                assert!(matches!(
                    sheet.write(CellId::default(), 0_i32),
                    Err(Error::InvalidId)
                ));
                assert!(matches!(
                    sheet.add_relationship(vec![]),
                    Err(Error::InvalidMethod { .. })
                ));
                assert!(matches!(
                    sheet.add_conditional::<i32>(
                        MatchExpr::cell(cells[1]),
                        vec![(vec![], vec![])],
                        vec![]
                    ),
                    Err(Error::InvalidConditional { .. })
                ));
                assert!(matches!(
                    sheet.add_out(
                        Method::from_fn_1_1(cells[1], cells[1], |x: &i32| Ok(*x)),
                        vec![]
                    ),
                    Err(Error::InvalidCellKind { .. })
                ));
                if mutation == 4 {
                    assert!(matches!(
                        sheet.add_filter(cells[1], Filter::from_fn_0(|x: &i32| Ok(*x))),
                        Err(Error::InvalidFilter)
                    ));
                }
                if mutation >= 6 {
                    assert!(matches!(
                        sheet.write(cells[2], 0_i32),
                        Err(Error::InvalidCellKind { .. })
                    ));
                }
                if mutation == 7 {
                    assert!(matches!(
                        sheet.add_requirement(
                            cells[2],
                            Some("check"),
                            Requirement::from_fn_1(cells[2], |_: &i32| Ok(true))
                        ),
                        Err(Error::InvalidRequirement)
                    ));
                }
            }
            assert_same_result(&mut automatic, &mut forced);
            assert_eq!(automatic.reuse_stats, settled);
            assert_same_snapshot(&automatic, &ids, &forced, &other);
        }
    }

    /// Checks structural cycle errors cannot execute an old eligible plan or replace its display.
    #[test]
    fn differential_structural_cycles_preserve_completed_display() {
        for dependency in [false, true] {
            let build = || {
                let mut sheet = Sheet::new();
                let a = sheet.add_cell(0_i32);
                let b = sheet.add_cell(1_i32);
                let relationship = sheet
                    .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
                    .unwrap();
                (sheet, [a, b], relationship)
            };
            let (mut automatic, ids, relationship) = build();
            let (mut forced, other, reference_relationship) = build();
            for _ in 0..3 {
                automatic.propagate().unwrap();
                propagate_forced_replan(&mut forced).unwrap();
            }
            automatic.write(ids[0], 3_i32).unwrap();
            forced.write(other[0], 3_i32).unwrap();
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert!(automatic.changed().any(|id| id == ids[1]));
            let completed = automatic.last_plan.as_ref().unwrap().clone();
            let before = automatic.reuse_stats;
            for (sheet, cells, existing) in [
                (&mut automatic, ids, relationship),
                (&mut forced, other, reference_relationship),
            ] {
                if dependency {
                    sheet
                        .add_conditional(
                            MatchExpr::cell(cells[1]),
                            vec![(vec![3_i32], vec![existing])],
                            vec![],
                        )
                        .unwrap();
                } else {
                    sheet
                        .add_relationship(vec![Method::from_fn_1_1(
                            cells[1],
                            cells[0],
                            |x: &i32| Ok(*x),
                        )])
                        .unwrap();
                }
            }
            let left = automatic.propagate().unwrap_err();
            let right = propagate_forced_replan(&mut forced).unwrap_err();
            if dependency {
                assert!(matches!(left, Error::DependencyCycle { .. }));
                // Validation errors precede clearing the last successful changed set.
                assert!(automatic.changed().any(|id| id == ids[1]));
                assert_eq!(automatic.reuse_stats.main_plans, before.main_plans);
            } else {
                assert!(matches!(left, Error::Cycle { .. }));
                assert_eq!(automatic.changed().count(), 0);
                assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
            }
            assert_eq!(
                std::mem::discriminant(&left),
                std::mem::discriminant(&right)
            );
            assert_eq!(left.to_string(), right.to_string());
            assert_eq!(error_sites(&automatic, &left), error_sites(&forced, &right));
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            assert_eq!(*automatic.read::<i32>(ids[0]).unwrap(), 3);
            assert_eq!(*automatic.read::<i32>(ids[1]).unwrap(), 3);
            assert!(std::rc::Rc::ptr_eq(
                automatic.last_plan.as_ref().unwrap(),
                &completed
            ));
            assert!(automatic.pre_plan_cache.is_none());
            assert!(automatic.main_plan_cache.is_none());
        }
    }

    /// Builds a pure input-controlled method or requirement failure fixture.
    fn failure_sheet(requirement: bool) -> (Sheet, [CellId; 3], RelationshipId) {
        let mut sheet = Sheet::new();
        let input = sheet.add_source(1_i32);
        let staged = sheet.add_cell(0_i32);
        let output = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(input, staged, |x: &i32| Ok(*x))])
            .unwrap();
        let relationship = sheet
            .add_relationship(vec![Method::from_fn_1_1(staged, output, move |x: &i32| {
                if !requirement && *x == 7 {
                    Err(anyhow::anyhow!("method probe"))
                } else {
                    Ok(*x)
                }
            })])
            .unwrap();
        if requirement {
            sheet
                .add_requirement(
                    output,
                    None,
                    Requirement::from_fn_1(input, |value: &i32| {
                        if *value == 7 {
                            Err(anyhow::anyhow!("requirement probe"))
                        } else {
                            Ok(*value >= 0)
                        }
                    }),
                )
                .unwrap();
        }
        (sheet, [input, staged, output], relationship)
    }

    /// Checks fresh staging, pre/post-commit errors, completed display preservation, and recovery.
    #[test]
    fn differential_pure_failures_and_recovery() {
        for requirement in [false, true] {
            let (mut automatic, ids, relationship) = failure_sheet(requirement);
            let (mut forced, other, _) = failure_sheet(requirement);
            for _ in 0..3 {
                assert_same_result(&mut automatic, &mut forced);
            }
            let completed = automatic.last_plan.as_ref().unwrap().clone();
            let before = automatic.reuse_stats;
            automatic.write(ids[0], 7_i32).unwrap();
            forced.write(other[0], 7_i32).unwrap();
            let error = automatic.propagate().unwrap_err();
            let reference = propagate_forced_replan(&mut forced).unwrap_err();
            assert!(matches!(error, Error::MethodFailed { .. }));
            assert_eq!(error.to_string(), reference.to_string());
            assert_eq!(
                error_sites(&automatic, &error),
                error_sites(&forced, &reference)
            );
            assert_eq!(automatic.reuse_stats.main_plans, before.main_plans);
            assert!(std::rc::Rc::ptr_eq(
                automatic.last_plan.as_ref().unwrap(),
                &completed
            ));
            assert!(automatic.pre_plan_cache.is_none());
            assert!(automatic.main_plan_cache.is_none());
            assert_eq!(automatic.selected_method(relationship), Some(0));
            assert!(automatic.is_relationship_forced(relationship));
            for &cell in &ids[1..] {
                assert_eq!(
                    *automatic.read::<i32>(cell).unwrap(),
                    if requirement { 7 } else { 1 }
                );
                assert!(automatic.is_forced(cell));
                assert!(!automatic.is_source(cell));
            }
            assert_eq!(automatic.changed().count(), if requirement { 2 } else { 0 });
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            for value in [3_i32, -1, 4] {
                automatic.write(ids[0], value).unwrap();
                forced.write(other[0], value).unwrap();
                assert_same_result(&mut automatic, &mut forced);
                assert_same_snapshot(&automatic, &ids, &forced, &other);
                assert_eq!(*automatic.read::<i32>(ids[2]).unwrap(), value);
                if requirement {
                    assert_eq!(requirement_status(&automatic, ids[2]), vec![value < 0]);
                }
            }
            assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
            assert_eq!(automatic.reuse_stats.preparations, before.preparations + 1);
            assert_eq!(
                automatic.reuse_stats.signature_builds,
                before.signature_builds
            );
        }
    }

    /// Checks post-commit requirement errors preserve the previous, different method assignment.
    #[test]
    fn differential_requirement_error_retains_previous_direction() {
        let (mut automatic, a, b, relationship) = identity_sheet();
        let (mut forced, x, y, _) = identity_sheet();
        for (sheet, ids) in [(&mut automatic, [a, b]), (&mut forced, [x, y])] {
            for name in ["first", "second"] {
                sheet
                    .add_requirement(
                        ids[0],
                        Some(name),
                        Requirement::from_fn_1(ids[0], |value: &i32| {
                            if *value == 7 {
                                Err(anyhow::anyhow!("requirement probe"))
                            } else {
                                Ok(*value >= 0)
                            }
                        }),
                    )
                    .unwrap();
            }
        }
        for _ in 0..3 {
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
        }
        assert_eq!(automatic.selected_method(relationship), Some(1));
        let completed = automatic.last_plan.as_ref().unwrap().clone();
        automatic.write(a, 7_i32).unwrap();
        forced.write(x, 7_i32).unwrap();
        let left = automatic.propagate().unwrap_err();
        let right = propagate_forced_replan(&mut forced).unwrap_err();
        assert!(matches!(left, Error::MethodFailed { .. }));
        assert_eq!(left.to_string(), right.to_string());
        assert_eq!(error_sites(&automatic, &left), error_sites(&forced, &right));
        assert_same_snapshot(&automatic, &[a, b], &forced, &[x, y]);
        assert_eq!(*automatic.read::<i32>(b).unwrap(), 7);
        assert_eq!(*automatic.source::<i32>(b).unwrap(), 7);
        assert!(automatic.changed().any(|id| id == b));
        assert_eq!(automatic.selected_method(relationship), Some(1));
        assert!(automatic.is_source(b));
        assert!(!automatic.is_source(a));
        assert!(std::rc::Rc::ptr_eq(
            automatic.last_plan.as_ref().unwrap(),
            &completed
        ));
        let before = automatic.reuse_stats;
        for value in [-1_i32, 3] {
            automatic.write(a, value).unwrap();
            forced.write(x, value).unwrap();
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert_same_snapshot(&automatic, &[a, b], &forced, &[x, y]);
            assert_eq!(requirement_status(&automatic, a), vec![value < 0; 2]);
        }
        assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(automatic.reuse_stats.preparations, before.preparations + 1);
        assert_eq!(automatic.selected_method(relationship), Some(0));
    }

    /// Attaches the two self-referencing directions of an integer inequality.
    ///
    /// - Complexity: The same as attaching a two-method relationship.
    fn add_inequality(sheet: &mut Sheet, low: CellId, high: CellId) {
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([low, high], low, |a: &i32, b: &i32| Ok((*a).min(*b))),
                Method::from_fn_2_1([low, high], high, |a: &i32, b: &i32| Ok((*a).max(*b))),
            ])
            .unwrap();
    }

    /// Builds a recursive inequality chain or a strength-ordered sibling seed fold.
    fn seed_sheet(siblings: bool) -> (Sheet, Vec<CellId>) {
        let mut sheet = Sheet::new();
        let ids = if siblings {
            [4_i32, 5, 3, 10]
                .map(|value| sheet.add_cell(value))
                .to_vec()
        } else {
            [0_i32, 10, 20].map(|value| sheet.add_cell(value)).to_vec()
        };
        if siblings {
            add_inequality(&mut sheet, ids[0], ids[3]);
            add_inequality(&mut sheet, ids[1], ids[0]);
            add_inequality(&mut sheet, ids[0], ids[2]);
        } else {
            add_inequality(&mut sheet, ids[0], ids[1]);
            add_inequality(&mut sheet, ids[1], ids[2]);
        }
        (sheet, ids)
    }

    /// Checks current recursive and sibling seeds, self-reference invalidation, and spring-back.
    #[test]
    fn differential_recursive_and_sibling_seed_writes() {
        for siblings in [false, true] {
            let (mut automatic, ids) = seed_sheet(siblings);
            let (mut forced, other) = seed_sheet(siblings);
            for _ in 0..3 {
                assert_same_result(&mut automatic, &mut forced);
                assert_same_snapshot(&automatic, &ids, &forced, &other);
            }
            let settled = automatic.reuse_stats;
            assert_same_result(&mut automatic, &mut forced);
            assert_eq!(automatic.reuse_stats, settled);
            let released = ids.iter().position(|id| automatic.is_source(*id)).unwrap();
            for value in [5_i32, if siblings { 10 } else { 20 }] {
                automatic.write(ids[released], value).unwrap();
                forced.write(other[released], value).unwrap();
                automatic.propagate().unwrap();
                propagate_forced_replan(&mut forced).unwrap();
                assert_same_snapshot(&automatic, &ids, &forced, &other);
                assert_eq!(automatic.reuse_stats.main_plans, settled.main_plans);
                if !siblings && value == 5 {
                    assert_eq!(*automatic.read::<i32>(ids[1]).unwrap(), 5);
                    assert_eq!(*automatic.source::<i32>(ids[1]).unwrap(), 10);
                }
            }
            // Reuse without a write still evaluates seeds from current sources.
            let writes = [
                (0_usize, 100_i32),
                (0, 0),
                (1, 100),
                (0, 10),
                (0, 9),
                (0, 120),
                (0, 50),
                (1, 24),
                (1, 23),
                (2, 100),
                (0, -2),
            ];
            for (target, value) in writes {
                let claimed = !automatic.is_source(ids[target]);
                let before = automatic.reuse_stats;
                automatic.write(ids[target], value).unwrap();
                forced.write(other[target], value).unwrap();
                assert_same_result(&mut automatic, &mut forced);
                assert_same_snapshot(&automatic, &ids, &forced, &other);
                if claimed {
                    assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
                }
                assert_eq!(
                    automatic.reuse_stats.signature_builds,
                    settled.signature_builds
                );
                if !siblings && target == 0 && value == 0 {
                    assert_eq!(*automatic.read::<i32>(ids[1]).unwrap(), 10);
                    assert_eq!(*automatic.read::<i32>(ids[2]).unwrap(), 20);
                }
            }
        }
    }

    /// Builds a filtered integer guard with an input-controlled filter diagnostic.
    fn filtered_guard_sheet() -> (Sheet, [CellId; 3]) {
        let mut sheet = Sheet::new();
        let guard = sheet.add_source(1_i32);
        let input = sheet.add_source(8_i32);
        let output = sheet.add_cell(0_i32);
        sheet
            .add_filter(
                guard,
                Filter::from_fn_0(|value: &i32| {
                    if *value == 7 {
                        Err(anyhow::anyhow!("filter probe"))
                    } else {
                        Ok((*value).clamp(0, 1))
                    }
                }),
            )
            .unwrap();
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(input, output, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![1_i32], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet
            .add_filter(output, Filter::from_fn_0(|value: &i32| Ok((*value).min(4))))
            .unwrap();
        (sheet, [guard, input, output])
    }

    /// Checks cached filtered guards select branches from current conformance and diagnostics.
    #[test]
    fn differential_filtered_guard_diagnostics() {
        let (mut automatic, ids) = filtered_guard_sheet();
        let (mut forced, other) = filtered_guard_sheet();
        for _ in 0..3 {
            assert_same_result(&mut automatic, &mut forced);
        }
        let initial = automatic.reuse_stats;
        for value in [2_i32, 7, 0, -1, 1, 1] {
            automatic.write(ids[0], value).unwrap();
            forced.write(other[0], value).unwrap();
            assert_same_result(&mut automatic, &mut forced);
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            let diagnostic = match value {
                7 => Some("failed:filter probe".to_owned()),
                // Released sources conform successfully; only failure is diagnostic.
                _ => None,
            };
            assert_eq!(filter_status(&automatic, ids[0]), diagnostic);
            assert_eq!(
                filter_status(&automatic, ids[2]),
                if *automatic.read::<i32>(ids[2]).unwrap() == 8 {
                    Some("not-conformed".to_owned())
                } else {
                    None
                }
            );
        }
        assert_eq!(automatic.reuse_stats.pre_plans, initial.pre_plans);
        assert_eq!(
            automatic.reuse_stats.signature_builds,
            initial.signature_builds
        );
    }

    /// Builds the restaged guard seed fixture, optionally with a recursive seed dependency.
    ///
    /// - Complexity: O(1), with at most seven cells and seven relationships.
    fn restaged_sheet(recursive: bool) -> (Sheet, Vec<CellId>) {
        let mut sheet = Sheet::new();
        let mode = sheet.add_cell(0_i32);
        let staged = sheet.add_cell(0_i32);
        let guard = sheet.add_cell(0_i32);
        let sink = sheet.add_cell(0_i32);
        let output = sheet.add_cell(0_i32);
        let mut ids = vec![mode, staged, guard, sink, output];
        sheet
            .add_relationship(vec![
                Method::from_fn_1_1(mode, staged, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(staged, mode, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(guard, guard, |x: &i32| Ok(*x))])
            .unwrap();
        let seed_input = if recursive {
            let inner = sheet.add_cell(0_i32);
            let inner_sink = sheet.add_cell(0_i32);
            ids.extend([inner, inner_sink]);
            sheet
                .add_relationship(vec![Method::from_fn_1_1(inner, inner, |x: &i32| Ok(*x))])
                .unwrap();
            sheet
                .add_relationship(vec![
                    Method::from_fn_2_1([staged, inner_sink], inner, |x: &i32, _: &i32| {
                        x.checked_add(10).ok_or_else(|| anyhow::anyhow!("overflow"))
                    }),
                    Method::from_fn_2_1([staged, inner], inner_sink, |_: &i32, x: &i32| Ok(*x)),
                ])
                .unwrap();
            inner
        } else {
            staged
        };
        sheet
            .add_relationship(vec![
                Method::from_fn_2_1([seed_input, sink], guard, |x: &i32, _: &i32| {
                    x.checked_add(1).ok_or_else(|| anyhow::anyhow!("overflow"))
                }),
                Method::from_fn_2_1([seed_input, guard], sink, |_: &i32, x: &i32| Ok(*x)),
            ])
            .unwrap();
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(mode, output, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![1_i32, 11], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet
            .add_conditional::<i32>(MatchExpr::cell(staged), vec![], vec![])
            .unwrap();
        // Keep staged released during warm-up; writing mode later changes its producer.
        sheet.write(staged, 0_i32).unwrap();
        (sheet, ids)
    }

    /// Checks cached prerequisite provenance rejects restaged direct/recursive seed inputs.
    #[test]
    fn differential_restaged_seed_provenance_and_recovery() {
        for recursive in [false, true] {
            let (mut automatic, ids) = restaged_sheet(recursive);
            let (mut forced, other) = restaged_sheet(recursive);
            for _ in 0..3 {
                automatic.propagate().unwrap();
                propagate_forced_replan(&mut forced).unwrap();
                assert_same_snapshot(&automatic, &ids, &forced, &other);
            }
            let completed = automatic.last_plan.as_ref().unwrap().clone();
            let settled = automatic.reuse_stats;
            assert_same_result(&mut automatic, &mut forced);
            assert_eq!(automatic.reuse_stats, settled);
            // Warm propagation demotes the sibling sink; restore its explicit pressure
            // so the guard seed traverses the input whose producer will be restaged.
            automatic.write(ids[3], 0_i32).unwrap();
            forced.write(other[3], 0_i32).unwrap();
            if recursive {
                automatic.write(ids[6], 0_i32).unwrap();
                forced.write(other[6], 0_i32).unwrap();
            }
            automatic.write(ids[0], 5_i32).unwrap();
            forced.write(other[0], 5_i32).unwrap();
            let before_values = ids
                .iter()
                .map(|id| *automatic.read::<i32>(*id).unwrap())
                .collect::<Vec<_>>();
            let result = automatic.propagate();
            let reference = propagate_forced_replan(&mut forced);
            assert!(
                result.is_err(),
                "recursive={recursive}, automatic={result:?}, forced={reference:?}"
            );
            let left = result.unwrap_err();
            let right = reference.unwrap_err();
            assert!(matches!(left, Error::Conflict { .. }));
            assert_eq!(error_sites(&automatic, &left), error_sites(&forced, &right));
            assert_eq!(left.to_string(), right.to_string());
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            assert_eq!(
                ids.iter()
                    .map(|id| *automatic.read::<i32>(*id).unwrap())
                    .collect::<Vec<_>>(),
                before_values
            );
            assert_eq!(automatic.changed().count(), 0);
            assert!(std::rc::Rc::ptr_eq(
                automatic.last_plan.as_ref().unwrap(),
                &completed
            ));
            assert!(automatic.pre_plan_cache.is_none());
            assert!(automatic.main_plan_cache.is_none());
            let before = automatic.reuse_stats;
            automatic.write(ids[1], 0_i32).unwrap();
            forced.write(other[1], 0_i32).unwrap();
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert_same_snapshot(&automatic, &ids, &forced, &other);
            assert_eq!(automatic.reuse_stats.pre_plans, before.pre_plans + 1);
            assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
        }
    }

    /// Builds the conditional seed cycle fixture with a successful inactive branch.
    fn cycle_sheet() -> (Sheet, [CellId; 7]) {
        let mut sheet = Sheet::new();
        let c = sheet.add_cell(-1_i32);
        let d = sheet.add_cell(-2_i32);
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(10_i32);
        let source = sheet.add_cell(7_i32);
        sheet
            .add_relationship(vec![Method::from_fn_2_1(
                [a, source],
                a,
                |_: &i32, x: &i32| Ok(*x),
            )])
            .unwrap();
        let mut siblings = Vec::new();
        for (target, input, fallback) in [(a, b, c), (b, a, d)] {
            let methods = [(target, 1_usize), (fallback, 2)].map(|(output, argument)| {
                Method::new(
                    vec![target, input, fallback],
                    vec![output],
                    vec![std::any::TypeId::of::<i32>(); 3],
                    vec![std::any::TypeId::of::<i32>()],
                    move |args| {
                        Ok(vec![Box::new(
                            *args[argument].downcast_ref::<i32>().unwrap(),
                        )])
                    },
                )
            });
            siblings.push(sheet.add_relationship(methods.into()).unwrap());
        }
        let mode = sheet.add_source(0_i32);
        let selected = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(mode, selected, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(selected),
                vec![(vec![1_i32], siblings)],
                vec![],
            )
            .unwrap();
        (sheet, [a, b, c, d, source, mode, selected])
    }

    /// Checks exact cycle provenance, discarded prerequisite staging, display retention, and recovery.
    #[test]
    fn differential_seed_cycle_after_warm_hits() {
        let (mut automatic, ids) = cycle_sheet();
        let (mut forced, other) = cycle_sheet();
        for _ in 0..3 {
            automatic.propagate().unwrap();
            propagate_forced_replan(&mut forced).unwrap();
            assert_same_snapshot(&automatic, &ids, &forced, &other);
        }
        let completed = automatic.last_plan.as_ref().unwrap().clone();
        let settled = automatic.reuse_stats;
        assert_same_result(&mut automatic, &mut forced);
        assert_eq!(automatic.reuse_stats, settled);
        automatic.write(ids[5], 1_i32).unwrap();
        forced.write(other[5], 1_i32).unwrap();
        let left = automatic.propagate().unwrap_err();
        let right = propagate_forced_replan(&mut forced).unwrap_err();
        assert!(matches!(left, Error::SeedCycle { .. }));
        assert_eq!(left.to_string(), right.to_string());
        assert_eq!(error_sites(&automatic, &left), error_sites(&forced, &right));
        assert_eq!(
            error_sites(&automatic, &left),
            vec![(0, 2, 0), (1, 1, 0), (0, 3, 0), (1, 2, 0)]
        );
        assert_eq!(*automatic.read::<i32>(ids[6]).unwrap(), 0);
        assert_eq!(automatic.changed().count(), 0);
        assert!(std::rc::Rc::ptr_eq(
            automatic.last_plan.as_ref().unwrap(),
            &completed
        ));
        assert_same_snapshot(&automatic, &ids, &forced, &other);
        let before = automatic.reuse_stats;
        automatic.write(ids[5], 0_i32).unwrap();
        forced.write(other[5], 0_i32).unwrap();
        automatic.propagate().unwrap();
        propagate_forced_replan(&mut forced).unwrap();
        assert_same_snapshot(&automatic, &ids, &forced, &other);
        assert_eq!(automatic.reuse_stats.pre_plans, before.pre_plans + 1);
        assert_eq!(automatic.reuse_stats.main_plans, before.main_plans + 1);
    }

    /// Creates an identity relationship whose later-added cell initially wins.
    fn identity_sheet() -> (Sheet, CellId, CellId, RelationshipId) {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(1_i32);
        let relationship = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |x: &i32| Ok(*x)),
                Method::from_fn_1_1(b, a, |x: &i32| Ok(*x)),
            ])
            .unwrap();
        (sheet, a, b, relationship)
    }

    /// Checks repeat reuse and independent invalidation on source versus claimed writes.
    #[test]
    fn identity_reuses_until_a_claimed_cell_is_written() {
        let (mut sheet, a, b, _) = identity_sheet();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        assert_eq!(initial.main_plans, 1);
        assert_eq!(initial.pre_plans, 0);
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        assert_eq!(sheet.reuse_stats.preparations, initial.preparations);
        assert_eq!(sheet.reuse_stats.signature_builds, initial.signature_builds);
        sheet.write(b, 9_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 9);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        sheet.write(a, 4_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 4);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(sheet.reuse_stats.signature_builds, initial.signature_builds);
    }

    /// Checks that empty sheets prepare only once and never need a certificate scan.
    #[test]
    fn empty_sheet_reuses_without_pre_plan_or_certificate_check() {
        let mut sheet = Sheet::new();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats, initial);
        assert_eq!(initial.main_plans, 1);
        assert_eq!(initial.pre_plans, 0);
    }

    /// Checks that a filter reclamp does not turn a released source into a claimant.
    #[test]
    fn filtered_released_source_preserves_reuse() {
        let (mut sheet, a, b, _) = identity_sheet();
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok((*x).min(5))))
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        assert!(sheet.is_source(b));
        sheet.write(b, 9_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(a).unwrap(), 5);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        assert_eq!(sheet.reuse_stats.pre_plans, 0);
    }

    /// Checks that structural invalidation leaves the completed display snapshot intact.
    #[test]
    fn newly_added_cell_is_a_source_in_previous_display_snapshot() {
        let (mut sheet, a, _, relationship) = identity_sheet();
        sheet.propagate().unwrap();
        let selected = sheet.selected_method(relationship);
        let initial = sheet.reuse_stats;
        let new_cell = sheet.add_cell(3_i32);
        assert!(sheet.is_source(new_cell));
        assert!(!sheet.is_source(a));
        assert_eq!(sheet.selected_method(relationship), selected);
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            initial.signature_builds + 1
        );
    }

    /// Checks that only actual strength changes schedule a single certificate scan.
    #[test]
    fn settled_strengths_do_not_repeat_certificate_scans() {
        let (mut sheet, _, _, _) = identity_sheet();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.certificate_checks, 0);
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.certificate_checks, 1);
        let settled = sheet.reuse_stats;
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats, settled);
    }

    /// Checks pre-plan reuse across branch changes with a freshly evaluated derived guard.
    #[test]
    fn branch_changes_replace_only_main_plan_and_share_signatures() {
        let mut sheet = Sheet::new();
        let input = sheet.add_source(true);
        let value = sheet.add_source(7_i32);
        let output = sheet.add_cell(0_i32);
        let guard = sheet.add_cell(false);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(input, guard, |x: &bool| Ok(*x))])
            .unwrap();
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(value, output, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![true], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        assert_eq!(initial.pre_plans, 1);
        assert_eq!(initial.main_plans, 1);
        assert_eq!(initial.preparations, 2);
        assert_eq!(initial.signature_builds, 1);
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.preparations, initial.preparations);
        sheet.write(input, false).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.pre_plans, initial.pre_plans);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(sheet.reuse_stats.signature_builds, 1);
        assert_eq!(sheet.selected_method(branch), None);
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 0);
        assert!(sheet.changed().any(|id| id == output));
        sheet.write(input, true).unwrap();
        sheet.write(value, 11_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 11);
        assert_eq!(sheet.reuse_stats.pre_plans, initial.pre_plans);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 2);
        assert_eq!(sheet.reuse_stats.signature_builds, 1);
    }

    /// Checks invalidation of only the phase that claims an explicitly written cell.
    #[test]
    fn claimed_main_write_preserves_unconditional_cache() {
        let mut sheet = Sheet::new();
        let guard = sheet.add_source(true);
        let input = sheet.add_source(7_i32);
        let output = sheet.add_cell(0_i32);
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(input, output, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![true], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        sheet.write(output, 9_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
        assert_eq!(sheet.reuse_stats.pre_plans, initial.pre_plans);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(sheet.reuse_stats.signature_builds, 1);
    }

    /// Checks that main-plan demotion invalidates a pre-plan certificate, not the solver flag alone.
    #[test]
    fn crossed_unconditional_certificate_replans_only_unconditional_phase() {
        let (mut sheet, a, b, identity) = identity_sheet();
        let input = sheet.add_source(7_i32);
        let guard = sheet.add_source(true);
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(input, a, |x: &i32| Ok(*x))])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![true], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);
        assert_eq!(sheet.selected_method(identity), Some(0));
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.pre_plans, initial.pre_plans + 1);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        assert_eq!(sheet.reuse_stats.preparations, initial.preparations + 1);
        assert_eq!(sheet.reuse_stats.certificate_checks, 2);
        assert_eq!(sheet.reuse_stats.signature_builds, 1);
    }

    /// Checks that a rejected mutation does not drop prepared plans or signatures.
    #[test]
    fn rejected_writes_and_attachments_preserve_reuse() {
        let (mut sheet, a, b, _) = identity_sheet();
        sheet
            .add_filter(b, Filter::from_fn_0(|x: &i32| Ok(*x)))
            .unwrap();
        sheet.propagate().unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        assert!(matches!(
            sheet.write(b, false),
            Err(Error::TypeMismatch { .. })
        ));
        assert!(matches!(
            sheet.write(CellId::default(), 0_i32),
            Err(Error::InvalidId)
        ));
        assert!(matches!(
            sheet.add_filter(b, Filter::from_fn_0(|x: &i32| Ok(*x))),
            Err(Error::InvalidFilter)
        ));
        assert!(matches!(
            sheet.add_relationship(vec![]),
            Err(Error::InvalidMethod { .. })
        ));
        assert!(matches!(
            sheet.add_requirement(a, None, Requirement::from_fn_1(a, |_: &i32| Ok(false))),
            Err(Error::InvalidRequirement)
        ));
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats, initial);
    }

    /// Checks invalidation even when add_out returns an error after mutating the graph.
    #[test]
    fn partially_failing_add_out_drops_structural_eligibility() {
        let mut sheet = Sheet::new();
        let input = sheet.add_source(7_i32);
        let output = sheet.add_cell(0_i32);
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        let result = sheet.add_out(
            Method::from_fn_1_1(input, output, |x: &i32| Ok(*x)),
            vec![
                (
                    Some("valid"),
                    Requirement::from_fn_1(output, |x: &i32| Ok(*x > 0)),
                ),
                (
                    Some("valid"),
                    Requirement::from_fn_1(output, |x: &i32| Ok(*x > 0)),
                ),
            ],
        );
        assert!(matches!(result, Err(Error::InvalidRequirement)));
        assert_eq!(sheet.cell_kind(output), Some(CellKind::Out));
        assert_eq!(sheet.cell_requirements(output).unwrap().len(), 1);
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 7);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            initial.signature_builds + 1
        );
        let settled = sheet.reuse_stats;
        assert!(matches!(
            sheet.write(output, 0_i32),
            Err(Error::InvalidCellKind { .. })
        ));
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, settled.main_plans);
    }

    /// Checks staged failure discards eligibility while preserving the successful display.
    #[test]
    fn cached_method_failure_preserves_values_and_display_then_replans() {
        let mut sheet = Sheet::new();
        let input = sheet.add_source(1_i32);
        let output = sheet.add_cell(0_i32);
        let relationship = sheet
            .add_relationship(vec![Method::from_fn_1_1(input, output, |x: &i32| {
                anyhow::ensure!(*x >= 0, "negative input");
                Ok(*x)
            })])
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        sheet.write(input, -1_i32).unwrap();
        assert!(matches!(sheet.propagate(), Err(Error::MethodFailed { .. })));
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 1);
        assert_eq!(sheet.changed().count(), 0);
        assert_eq!(sheet.selected_method(relationship), Some(0));
        assert!(sheet.is_forced(output));
        assert_eq!(sheet.forced_cells().collect::<Vec<_>>(), vec![output]);
        assert!(sheet.is_relationship_forced(relationship));
        assert_eq!(
            sheet.forced_relationships().collect::<Vec<_>>(),
            vec![relationship]
        );
        sheet.write(input, 3_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 3);
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(sheet.reuse_stats.signature_builds, initial.signature_builds);
    }

    /// Checks that requirement errors still occur after committing a cached plan's values.
    #[test]
    fn cached_requirement_failure_commits_but_does_not_publish_new_display() {
        let (mut sheet, a, b, relationship) = identity_sheet();
        sheet
            .add_requirement(
                a,
                None,
                Requirement::from_fn_1(b, |x: &i32| {
                    anyhow::ensure!(*x >= 0, "negative input");
                    Ok(true)
                }),
            )
            .unwrap();
        sheet.propagate().unwrap();
        let initial = sheet.reuse_stats;
        sheet.write(b, -1_i32).unwrap();
        assert!(matches!(sheet.propagate(), Err(Error::MethodFailed { .. })));
        assert_eq!(*sheet.read::<i32>(a).unwrap(), -1);
        assert!(sheet.changed().any(|id| id == a));
        assert_eq!(sheet.selected_method(relationship), Some(1));
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans);
        sheet.write(b, 3_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 1);
        assert_eq!(sheet.reuse_stats.signature_builds, initial.signature_builds);
        sheet.write(a, -2_i32).unwrap();
        assert!(matches!(sheet.propagate(), Err(Error::MethodFailed { .. })));
        assert_eq!(*sheet.read::<i32>(b).unwrap(), -2);
        assert_eq!(sheet.selected_method(relationship), Some(1));
        assert!(sheet.is_source(b));
        assert!(!sheet.is_source(a));
        assert_eq!(sheet.reuse_stats.main_plans, initial.main_plans + 2);
    }

    /// Checks successful structural attachments rebuild plans and signatures before reuse.
    #[test]
    fn actual_structural_mutations_rebuild_shared_artifacts() {
        let (mut sheet, a, b, _) = identity_sheet();
        sheet.propagate().unwrap();
        let mut before = sheet.reuse_stats;
        let guard = sheet.add_source(true);
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            before.signature_builds + 1
        );

        before = sheet.reuse_stats;
        let output = sheet.add_cell(0_i32);
        let branch = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, output, |x: &i32| Ok(*x))])
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(*sheet.read::<i32>(output).unwrap(), 1);

        before = sheet.reuse_stats;
        sheet
            .add_conditional(
                MatchExpr::cell(guard),
                vec![(vec![true], vec![branch])],
                vec![],
            )
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.pre_plans, before.pre_plans + 1);
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            before.signature_builds + 1
        );

        before = sheet.reuse_stats;
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &i32| Ok((*x).min(0))))
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.pre_plans, before.pre_plans + 1);
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            before.signature_builds + 1
        );
        assert!(matches!(
            sheet.filter_violation(a),
            Some(crate::FilterViolation::NotConformed)
        ));

        before = sheet.reuse_stats;
        sheet
            .add_requirement(a, None, Requirement::from_fn_1(a, |x: &i32| Ok(*x >= 0)))
            .unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.pre_plans, before.pre_plans + 1);
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans + 1);
        assert_eq!(
            sheet.reuse_stats.signature_builds,
            before.signature_builds + 1
        );
        sheet.write(b, 0_i32).unwrap();
        before = sheet.reuse_stats;
        sheet.propagate().unwrap();
        assert_eq!(sheet.reuse_stats.main_plans, before.main_plans);
        assert!(sheet.filter_violation(a).is_none());
        assert!(sheet.cell_requirements_valid(a));
    }
}
