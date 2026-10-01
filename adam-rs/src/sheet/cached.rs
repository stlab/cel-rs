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
        CellId, CellKind, Error, Filter, MatchExpr, Method, RelationshipId, Requirement, Sheet,
    };

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
