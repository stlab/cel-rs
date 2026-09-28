//! Selected unconditional-plan steps needed to resolve conditional guards.

use std::collections::{HashMap, HashSet};

use crate::{conditional::MatchSource, error::Error, planner::PlanStep, sheet::Sheet};

impl Sheet {
    /// Returns the selected unconditional steps needed to evaluate every conditional guard.
    ///
    /// The returned steps retain `order`'s topological order and omit selected methods and
    /// filter reclamps outside the guards' transitive prerequisite cone.
    ///
    /// - Precondition: `order` is the execution order from the complete unconditional plan.
    ///
    /// - Complexity: O(V + E) time and space where V is the number of selected plan steps and
    ///   cells, and E is the number of their method outputs, method inputs, and filter arguments.
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
