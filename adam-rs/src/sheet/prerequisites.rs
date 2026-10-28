//! Selected unconditional-plan steps needed to resolve conditional guards.

use std::collections::{HashMap, HashSet};

use crate::{conditional::MatchSource, error::Error, planner::PlanStep, sheet::Sheet};

impl Sheet {
    /// Returns the selected unconditional-plan cone needed to evaluate every conditional guard.
    ///
    /// The returned steps retain `order`'s topological order and omit selected methods and
    /// filter reclamps outside the guards' transitive prerequisite cone. A method contributes
    /// its non-self inputs, and a filter contributes its argument cells, so a guard observes
    /// the selected producers and filter ordering that establish its value.
    ///
    /// - Precondition: `order` is the execution order from the complete unconditional plan.
    /// - Postcondition: every returned step is in `order`, and every selected guard input has
    ///   its transitive selected producer steps included.
    /// - Complexity: O(V + E) time and space where V is the number of selected plan steps and
    ///   cells, and E is the number of their method outputs, method inputs, and filter arguments.
    ///
    pub(crate) fn guard_prerequisite_steps(
        &self,
        order: &[PlanStep],
    ) -> Result<Vec<PlanStep>, Error> {
        let mut producer = HashMap::with_capacity(order.len());
        for &step in order {
            match step {
                PlanStep::Method(relationship, method_index) => {
                    for &output in &self.relationships[relationship].methods[method_index].outputs {
                        producer.insert(output, step);
                    }
                }
                PlanStep::FilterReclamp(cell) => {
                    producer.insert(cell, step);
                }
            }
        }

        let mut pending = Vec::new();
        for conditional in self.conditionals.values() {
            match &conditional.source {
                MatchSource::Cell(cell) => pending.push(*cell),
                MatchSource::Expr(expression) => {
                    pending.extend(expression.inputs.iter().copied());
                }
            }
        }

        let mut seen_cells = HashSet::with_capacity(producer.len());
        let mut needed_steps = HashSet::with_capacity(order.len());
        while let Some(cell) = pending.pop() {
            if !seen_cells.insert(cell) {
                continue;
            }
            let Some(step) = producer.get(&cell).copied() else {
                continue;
            };
            if !needed_steps.insert(step) {
                continue;
            }
            match step {
                PlanStep::Method(relationship, method_index) => {
                    let method = &self.relationships[relationship].methods[method_index];
                    let outputs: HashSet<_> = method.outputs.iter().copied().collect();
                    pending.extend(
                        method
                            .inputs
                            .iter()
                            .copied()
                            .filter(|input| !outputs.contains(input)),
                    );
                }
                PlanStep::FilterReclamp(cell) => {
                    pending.extend(
                        self.cells[cell]
                            .filter
                            .as_ref()
                            .expect("planned filter reclamps have an attached filter")
                            .args
                            .iter()
                            .copied(),
                    );
                }
            }
        }

        Ok(order
            .iter()
            .copied()
            .filter(|step| needed_steps.contains(step))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filter, MatchExpr, Method};

    /// Omits all selected steps when there are no guards.
    #[test]
    fn empty_guard_cone_omits_unrelated_steps() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &i32| Ok(*v))])
            .unwrap();
        assert!(
            sheet
                .guard_prerequisite_steps(&[PlanStep::Method(rel, 0)])
                .unwrap()
                .is_empty()
        );
        assert!(sheet.guard_prerequisite_steps(&[]).unwrap().is_empty());
    }

    /// Includes transitive filter arguments and a multi-output producer once, in plan order.
    #[test]
    fn prerequisite_cone_retains_order_and_deduplicates_producers() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(7_i32);
        let b = sheet.add_cell(0_i32);
        let c = sheet.add_cell(0_i32);
        let filtered = sheet.add_cell(20_i32);
        let unrelated = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::new(
                vec![a],
                vec![b, c],
                vec![std::any::TypeId::of::<i32>()],
                vec![std::any::TypeId::of::<i32>(); 2],
                |values| {
                    let value = *values[0].downcast_ref::<i32>().unwrap();
                    Ok(vec![Box::new(value), Box::new(value)])
                },
            )])
            .unwrap();
        sheet
            .add_filter(
                filtered,
                Filter::from_fn_2([b, c], |v: &i32, lo: &i32, hi: &i32| {
                    Ok((*v).clamp(*lo, *hi))
                }),
            )
            .unwrap();
        let unused = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, unrelated, |v: &i32| Ok(*v))])
            .unwrap();
        sheet
            .add_conditional::<i32>(
                MatchExpr::from_fn_2([filtered, c], |v: &i32, _: &i32| Ok(*v)),
                vec![],
                vec![],
            )
            .unwrap();
        let order = [
            PlanStep::Method(unused, 0),
            PlanStep::Method(rel, 0),
            PlanStep::FilterReclamp(filtered),
        ];
        assert_eq!(sheet.guard_prerequisite_steps(&order).unwrap(), order[1..]);
    }

    /// Does not recursively require a self-referencing producer's own output.
    #[test]
    fn self_input_does_not_add_a_prerequisite_cycle() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(7_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, a, |v: &i32| Ok((*v).max(0)))])
            .unwrap();
        sheet
            .add_conditional::<i32>(MatchExpr::cell(a), vec![], vec![])
            .unwrap();
        let order = [PlanStep::Method(rel, 0)];
        assert_eq!(sheet.guard_prerequisite_steps(&order).unwrap(), order);
    }
}
