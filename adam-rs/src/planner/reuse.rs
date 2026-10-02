//! Certifies that changed cell priorities preserve a prepared plan's released sources.
//!
//! Construct a certificate from a plan's complete elimination order and the cells
//! absent from every selected method's outputs. Query membership without replanning,
//! then check changed strengths only while cells and the planning structure are unchanged.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use slotmap::SlotMap;

use crate::cell::{CellData, CellId};

/// Records the source prefixes and stable tie order of one completed planning pass.
///
/// `original_order` contains every cell exactly once; `released` contains only
/// cells from that order, and `tie_ordinals` records the original `cells.keys()`
/// enumeration, independent of strength.
///
/// - Complexity: O(C) storage for C cells.
pub(crate) struct SourceCertificate {
    original_order: Vec<CellId>,
    released: HashSet<CellId>,
    tie_ordinals: HashMap<CellId, usize>,
}

impl SourceCertificate {
    /// Captures a plan's complete release order, released sources, and stable ties.
    ///
    /// - Precondition: `original_order` is the complete elimination order of a
    ///   plan for `cells`, with every cell occurring exactly once.
    /// - Precondition: `released` is that plan's source set, excluding all selected
    ///   method outputs, including self-referencing outputs.
    /// - Complexity: Expected O(C) time and O(C) storage for C cells.
    pub(crate) fn new(
        cells: &SlotMap<CellId, CellData>,
        original_order: &[CellId],
        released: HashSet<CellId>,
    ) -> Self {
        let tie_ordinals: HashMap<_, _> = cells
            .keys()
            .enumerate()
            .map(|(ordinal, cell)| (cell, ordinal))
            .collect();
        debug_assert_eq!(original_order.len(), cells.len());
        debug_assert_eq!(
            original_order.iter().copied().collect::<HashSet<_>>().len(),
            cells.len()
        );
        debug_assert!(
            original_order
                .iter()
                .all(|cell| tie_ordinals.contains_key(cell))
        );
        debug_assert!(released.iter().all(|cell| tie_ordinals.contains_key(cell)));
        Self {
            original_order: original_order.to_vec(),
            released,
            tie_ordinals,
        }
    }

    /// Returns whether the captured plan released `cell`, including filtered sources.
    ///
    /// Cells absent from the captured plan return false.
    ///
    /// - Complexity: Expected O(1) time and O(1) additional space.
    pub(crate) fn is_released(&self, cell: CellId) -> bool {
        self.released.contains(&cell)
    }

    /// Returns whether `cell` existed when the captured plan was constructed.
    ///
    /// Cells created after the captured plan return false.
    ///
    /// - Complexity: Expected O(1) time and O(1) additional space.
    pub(crate) fn contains_cell(&self, cell: CellId) -> bool {
        self.tie_ordinals.contains_key(&cell)
    }

    /// Returns whether every rejected cell remains below its released source prefix.
    ///
    /// Priorities compare descending strength, breaking ties by ascending original
    /// cell enumeration ordinal. A true result certifies reuse of the original
    /// source set; false requires replanning but does not prove the plan changed.
    ///
    /// - Precondition: The cells and planning structure are unchanged since
    ///   construction; only cell values and strengths may have changed.
    /// - Complexity: Expected O(C) time and O(1) additional space for C cells.
    pub(crate) fn is_valid(&self, cells: &SlotMap<CellId, CellData>) -> bool {
        debug_assert_eq!(cells.len(), self.tie_ordinals.len());
        debug_assert!(
            cells
                .keys()
                .all(|cell| self.tie_ordinals.contains_key(&cell))
        );
        let mut weakest = None;
        for &cell in &self.original_order {
            let priority = (cells[cell].strength, Reverse(self.tie_ordinals[&cell]));
            if self.released.contains(&cell) {
                weakest =
                    Some(weakest.map_or(priority, |previous| std::cmp::min(previous, priority)));
            } else if weakest.is_some_and(|source_priority| priority >= source_priority) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::{Plan, PlanStep, SourceCertificate, plan};
    use crate::{CellId, Filter, Method, Sheet};

    /// Checks that a rejected cell may weaken but cannot cross its source prefix.
    #[test]
    fn certificate_accepts_demotions_and_rejects_crossed_source_prefixes() {
        let mut sheet = Sheet::new();
        let source = sheet.add_cell(0_i32);
        let rejected = sheet.add_cell(0_i32);
        sheet.cells[source].strength = 20;
        sheet.cells[rejected].strength = 10;
        let certificate =
            SourceCertificate::new(&sheet.cells, &[source, rejected], HashSet::from([source]));

        sheet.cells[rejected].strength = 5;
        assert!(certificate.is_valid(&sheet.cells));
        sheet.cells[source].strength = 4;
        assert!(!certificate.is_valid(&sheet.cells));
    }

    /// Checks the empty certificate's vacuous validity.
    #[test]
    fn certificate_accepts_empty_cells() {
        let sheet = Sheet::new();
        let certificate = SourceCertificate::new(&sheet.cells, &[], HashSet::new());
        assert!(certificate.is_valid(&sheet.cells));
    }

    /// Checks that unrestricted source sets impose no priority comparisons.
    #[test]
    fn certificate_accepts_all_released_and_no_released_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet.cells[a].strength = 20;
        sheet.cells[b].strength = 10;
        let all = SourceCertificate::new(&sheet.cells, &[a, b], HashSet::from([a, b]));
        let none = SourceCertificate::new(&sheet.cells, &[a, b], HashSet::new());

        sheet.cells[b].strength = 30;
        assert!(all.is_valid(&sheet.cells));
        assert!(none.is_valid(&sheet.cells));
        assert!(all.is_released(a));
        assert!(all.is_released(b));
        assert!(!none.is_released(a));
        assert!(!none.is_released(b));
    }

    /// Checks each rejected cell against its own preceding released prefix.
    #[test]
    fn certificate_checks_multiple_released_prefixes() {
        let mut sheet = Sheet::new();
        let cells = [(); 5].map(|()| sheet.add_cell(0_i32));
        for (&cell, strength) in cells.iter().zip([50, 40, 30, 20, 10]) {
            sheet.cells[cell].strength = strength;
        }
        let [a, b, c, d, e] = cells;
        let certificate = SourceCertificate::new(&sheet.cells, &cells, HashSet::from([a, c, e]));

        sheet.cells[b].strength = 35;
        assert!(certificate.is_valid(&sheet.cells));
        sheet.cells[d].strength = 31;
        assert!(!certificate.is_valid(&sheet.cells));
        sheet.cells[d].strength = 20;
        sheet.cells[a].strength = 15;
        assert!(!certificate.is_valid(&sheet.cells));
    }

    /// Checks source promotions, including reordering two released cells.
    #[test]
    fn certificate_accepts_source_promotions() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        sheet.cells[a].strength = 30;
        sheet.cells[b].strength = 20;
        sheet.cells[c].strength = 10;
        let certificate = SourceCertificate::new(&sheet.cells, &[a, b, c], HashSet::from([a, b]));

        sheet.cells[b].strength = 40;
        assert!(certificate.is_valid(&sheet.cells));
        sheet.cells[c].strength = 35;
        assert!(!certificate.is_valid(&sheet.cells));
        sheet.cells[c].strength = 10;
        sheet.cells[a].strength = 50;
        assert!(certificate.is_valid(&sheet.cells));
        sheet.cells[c].strength = 45;
        assert!(!certificate.is_valid(&sheet.cells));
    }

    /// Checks equal priorities against slot enumeration rather than sorted order.
    #[test]
    fn certificate_breaks_strength_ties_by_original_cell_ordinal() {
        for source_created_first in [true, false] {
            let mut sheet = Sheet::new();
            let first = sheet.add_cell(0_i32);
            let second = sheet.add_cell(0_i32);
            let (source, rejected) = if source_created_first {
                (first, second)
            } else {
                (second, first)
            };
            sheet.cells[source].strength = 20;
            sheet.cells[rejected].strength = 10;
            let certificate =
                SourceCertificate::new(&sheet.cells, &[source, rejected], HashSet::from([source]));

            sheet.cells[source].strength = 10;
            assert_eq!(certificate.is_valid(&sheet.cells), source_created_first);
        }
    }

    /// Checks plan-local membership after a later cell is added.
    #[test]
    fn certificate_membership_recognizes_cells_created_after_the_plan() {
        let mut sheet = Sheet::new();
        let source = sheet.add_cell(0_i32);
        let rejected = sheet.add_cell(0_i32);
        let certificate =
            SourceCertificate::new(&sheet.cells, &[rejected, source], HashSet::from([source]));
        let later = sheet.add_cell(0_i32);

        assert!(certificate.contains_cell(source));
        assert!(certificate.contains_cell(rejected));
        assert!(!certificate.contains_cell(later));
        assert!(certificate.is_released(source));
        assert!(!certificate.is_released(rejected));
        assert!(!certificate.is_released(later));
    }

    /// Derives literal sources from all selected outputs, including self-references.
    ///
    /// - Complexity: O(C + O), for C cells and O selected method outputs.
    fn released_sources(sheet: &Sheet, plan: &Plan) -> HashSet<CellId> {
        let outputs: HashSet<_> = plan
            .execution_order
            .iter()
            .filter_map(|step| match step {
                PlanStep::Method(relationship, index) => Some(
                    sheet.relationships[*relationship].methods[*index]
                        .outputs
                        .iter()
                        .copied(),
                ),
                PlanStep::FilterReclamp(_) => None,
            })
            .flatten()
            .collect();
        sheet
            .cells
            .keys()
            .filter(|cell| !outputs.contains(cell))
            .collect()
    }

    /// Checks that a self-referencing claimant is not a literal released source.
    #[test]
    fn certificate_excludes_self_referencing_method_outputs() {
        let mut sheet = Sheet::new();
        let cell = sheet.add_cell(0_i32);
        let relationship = sheet
            .add_relationship(vec![Method::from_fn_1_1(cell, cell, |value: &i32| {
                Ok(*value)
            })])
            .unwrap();
        let original = plan(
            &sheet.cells,
            &sheet.relationships,
            &HashSet::from([relationship]),
        )
        .unwrap();
        let released = released_sources(&sheet, &original);
        assert!(released.is_empty());
        let certificate =
            SourceCertificate::new(&sheet.cells, &original.elimination_order, released);
        assert!(certificate.contains_cell(cell));
        assert!(!certificate.is_released(cell));
        sheet.cells[cell].strength = 100;
        assert!(certificate.is_valid(&sheet.cells));
    }

    /// Checks that a filter reclamp does not remove a released source.
    #[test]
    fn certificate_keeps_filtered_sources_released() {
        let mut sheet = Sheet::new();
        let source = sheet.add_cell(0_i32);
        sheet
            .add_filter(source, Filter::from_fn_0(|value: &i32| Ok(*value)))
            .unwrap();
        let original = plan(&sheet.cells, &sheet.relationships, &HashSet::new()).unwrap();
        assert_eq!(
            original.execution_order,
            vec![PlanStep::FilterReclamp(source)]
        );
        let released = released_sources(&sheet, &original);
        assert_eq!(released, HashSet::from([source]));
        let certificate =
            SourceCertificate::new(&sheet.cells, &original.elimination_order, released);
        assert!(certificate.is_released(source));
        assert!(certificate.is_valid(&sheet.cells));
    }

    /// Checks accepted priority permutations against independent greedy replanning.
    #[test]
    fn certificate_preserves_real_plans_for_all_strict_three_cell_permutations() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let relationship = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |value: &i32| Ok(*value)),
                Method::from_fn_1_1(b, a, |value: &i32| Ok(*value)),
            ])
            .unwrap();
        let active = HashSet::from([relationship]);
        let permutations = [
            [30, 20, 10],
            [30, 10, 20],
            [20, 30, 10],
            [10, 30, 20],
            [20, 10, 30],
            [10, 20, 30],
        ];
        let mut accepted = 0_usize;
        let mut rejected = 0_usize;
        for baseline in permutations {
            for (cell, strength) in [a, b, c].into_iter().zip(baseline) {
                sheet.cells[cell].strength = strength;
            }
            let original = plan(&sheet.cells, &sheet.relationships, &active).unwrap();
            let released = released_sources(&sheet, &original);
            assert_eq!(
                released,
                HashSet::from([if baseline[0] > baseline[1] { a } else { b }, c])
            );
            let certificate =
                SourceCertificate::new(&sheet.cells, &original.elimination_order, released.clone());
            assert!(certificate.is_valid(&sheet.cells));
            for candidate in permutations {
                for (cell, strength) in [a, b, c].into_iter().zip(candidate) {
                    sheet.cells[cell].strength = strength;
                }
                if certificate.is_valid(&sheet.cells) {
                    accepted += 1;
                    let fresh = plan(&sheet.cells, &sheet.relationships, &active).unwrap();
                    assert_eq!(fresh.execution_order, original.execution_order);
                    assert_eq!(released_sources(&sheet, &fresh), released);
                } else {
                    rejected += 1;
                }
            }
        }
        assert!(accepted > 6);
        assert!(rejected > 0);
    }
}
