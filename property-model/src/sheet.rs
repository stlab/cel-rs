//! The [`Sheet`] owns and manages a property model constraint graph.
//!
//! All cells and relationships are created through the sheet and are
//! destroyed when the sheet is dropped.

use std::any::{Any, TypeId};

use slotmap::SlotMap;

use crate::{
    cell::{CellData, CellId},
    error::Error,
    relationship::{Method, RelationshipData, RelationshipId},
};

/// Owns a complete property model constraint graph.
///
/// Create cells with [`Sheet::add_cell`], define multi-way constraints with
/// [`Sheet::add_relationship`], write input values with [`Sheet::write`],
/// then call [`Sheet::propagate`] to execute the planning pass and update
/// derived cells.
///
/// # Example
///
/// ```rust
/// use property_model::{Sheet, Method};
///
/// let mut sheet = Sheet::new();
/// let a = sheet.add_cell(0_i32);
/// let b = sheet.add_cell(0_i32);
/// sheet.add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))]).unwrap();
/// sheet.write(a, 3_i32).unwrap();
/// assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
/// ```
pub struct Sheet {
    pub(crate) cells: SlotMap<CellId, CellData>,
    pub(crate) relationships: SlotMap<RelationshipId, RelationshipData>,
    pub(crate) changed_cells: Vec<CellId>,
    /// Monotonic counter incremented by both `add_cell` and `write`; cells added
    /// later and cells written later have strictly higher strength, making the
    /// default method-selection direction deterministic.
    next_strength: u64,
    last_plan: Option<Vec<(RelationshipId, usize)>>,
}

impl Sheet {
    /// Creates an empty sheet with no cells or relationships.
    pub fn new() -> Self {
        Sheet {
            cells: SlotMap::with_key(),
            relationships: SlotMap::with_key(),
            changed_cells: Vec::new(),
            next_strength: 0,
            last_plan: None,
        }
    }

    /// Registers a cell with an initial value and returns a stable handle.
    ///
    /// The cell's `TypeId` is fixed at creation time; subsequent `write` and
    /// `read` calls that use a different type will return `Error::TypeMismatch`.
    ///
    /// Each call increments the sheet's internal strength counter so that cells
    /// added later have strictly higher initial strength than cells added earlier.
    /// This makes the default method-selection direction deterministic: cells added
    /// last are treated as sources and cells added first are treated as outputs.
    pub fn add_cell<T: Any + 'static>(&mut self, value: T) -> CellId {
        self.next_strength += 1;
        self.cells.insert(CellData {
            value: Box::new(value),
            type_id: TypeId::of::<T>(),
            strength: self.next_strength,
            changed: false,
            adj: Vec::new(),
        })
    }

    /// Registers a relationship defined by a non-empty list of methods.
    ///
    /// All methods are validated: their declared `TypeId`s must match the
    /// registered cells, inputs and outputs must be disjoint, and each method
    /// must have at least one input and one output. On success the `RelationshipId`
    /// is added to each adjacent cell's adjacency list.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidMethod` — `methods` is empty, a method has no inputs,
    ///   a method has no outputs, or a method's inputs and outputs overlap.
    /// - `Error::InvalidId` — a `CellId` in any method is not found in this sheet.
    /// - `Error::TypeMismatch` — a method's declared `TypeId` does not match the
    ///   cell's registered `TypeId`.
    ///
    /// - Complexity: O(m × c) where m is the total number of methods and c is the
    ///   maximum number of cells per method.
    pub fn add_relationship(&mut self, methods: Vec<Method>) -> Result<RelationshipId, Error> {
        if methods.is_empty() {
            return Err(Error::InvalidMethod);
        }

        for method in &methods {
            if method.inputs.is_empty() || method.outputs.is_empty() {
                return Err(Error::InvalidMethod);
            }

            // inputs ∩ outputs must be empty
            for output in &method.outputs {
                if method.inputs.contains(output) {
                    return Err(Error::InvalidMethod);
                }
            }

            // declared type counts must match cell-id counts
            if method.inputs.len() != method.input_types.len()
                || method.outputs.len() != method.output_types.len()
            {
                return Err(Error::InvalidMethod);
            }

            for (&cell_id, &declared) in method.inputs.iter().zip(method.input_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }

            for (&cell_id, &declared) in method.outputs.iter().zip(method.output_types.iter()) {
                let cell = self.cells.get(cell_id).ok_or(Error::InvalidId)?;
                if cell.type_id != declared {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found: declared,
                    });
                }
            }
        }

        // Collect the union of all adjacent cells in insertion order, deduplicated.
        let mut adj: Vec<CellId> = Vec::new();
        let mut seen: std::collections::HashSet<CellId> = std::collections::HashSet::new();
        for method in &methods {
            for &cell_id in method.inputs.iter().chain(method.outputs.iter()) {
                if seen.insert(cell_id) {
                    adj.push(cell_id);
                }
            }
        }

        let rel_id = self.relationships.insert(RelationshipData {
            methods,
            adj: adj.clone(),
        });

        for cell_id in adj {
            if let Some(cell) = self.cells.get_mut(cell_id)
                && !cell.adj.contains(&rel_id)
            {
                cell.adj.push(rel_id);
            }
        }

        Ok(rel_id)
    }

    /// Writes a value to a cell, incrementing the cell's write-recency strength.
    ///
    /// Each successful `write` increments a global monotonic counter and assigns
    /// the new value to `cell.strength`, so the most-recently-written cell always
    /// has the highest strength.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn write<T: Any + 'static>(&mut self, id: CellId, value: T) -> Result<(), Error> {
        let cell = self.cells.get_mut(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        self.next_strength += 1;
        cell.strength = self.next_strength;
        cell.value = Box::new(value);
        Ok(())
    }

    /// Returns a shared reference to the current value of a cell.
    ///
    /// # Errors
    ///
    /// - `Error::InvalidId` — `id` is not a cell in this sheet.
    /// - `Error::TypeMismatch` — `T` does not match the cell's registered `TypeId`.
    pub fn read<T: Any + 'static>(&self, id: CellId) -> Result<&T, Error> {
        let cell = self.cells.get(id).ok_or(Error::InvalidId)?;
        if cell.type_id != TypeId::of::<T>() {
            return Err(Error::TypeMismatch {
                expected: cell.type_id,
                found: TypeId::of::<T>(),
            });
        }
        Ok(cell.value.downcast_ref::<T>().expect("type checked above"))
    }

    /// Iterates over the cells that were updated during the last `propagate()` call.
    ///
    /// This tracks which cells were written by selected methods; it does not attempt to
    /// compare old/new values for equality.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn changed(&self) -> impl Iterator<Item = CellId> + '_ {
        self.changed_cells.iter().copied()
    }

    /// Clears the changed-cell set and resets each cell's `changed` flag.
    ///
    /// Call after processing the results of `propagate()`.
    ///
    /// - Complexity: O(n) where n is the number of changed cells.
    pub fn clear_changed(&mut self) {
        for id in std::mem::take(&mut self.changed_cells) {
            if let Some(cell) = self.cells.get_mut(id) {
                cell.changed = false;
            }
        }
    }

    /// Iterates all live cell IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of cells.
    pub fn cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.cells.keys()
    }

    /// Iterates all live relationship IDs in the sheet.
    ///
    /// - Complexity: O(n) where n is the number of relationships.
    pub fn relationships(&self) -> impl Iterator<Item = RelationshipId> + '_ {
        self.relationships.keys()
    }

    /// Returns the relationships adjacent to `id`.
    ///
    /// Returns `None` if `id` is not a live cell in this sheet.
    ///
    /// - Complexity: O(1).
    pub fn cell_adj(&self, id: CellId) -> Option<&[RelationshipId]> {
        self.cells.get(id).map(|c| c.adj.as_slice())
    }

    /// Returns the cells adjacent to `id` (union across all methods).
    ///
    /// Returns `None` if `id` is not a live relationship in this sheet.
    ///
    /// - Complexity: O(1).
    pub fn relationship_adj(&self, id: RelationshipId) -> Option<&[CellId]> {
        self.relationships.get(id).map(|r| r.adj.as_slice())
    }

    /// Runs the planning pass and executes the selected methods.
    ///
    /// Clears the changed-cell set from the previous `propagate()` call before planning.
    /// After propagation, call [`Sheet::changed`] to inspect which cells were updated,
    /// and [`Sheet::clear_changed`] when done.
    ///
    /// If `execute_plan` fails, `last_plan` is not updated; `selected_method` will continue
    /// to reflect the last *successful* propagation.
    ///
    /// # Errors
    ///
    /// - `Error::Conflict` — no valid method assignment exists.
    /// - `Error::MethodFailed` — a method's function returned an error, or a method produced
    ///   the wrong number of outputs.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the cell's
    ///   registered type.
    pub fn propagate(&mut self) -> Result<(), Error> {
        self.clear_changed();
        let plan = crate::planner::plan(&self.cells, &self.relationships)?;
        self.execute_plan(&plan.execution_order)?;
        self.last_plan = Some(plan.execution_order);
        Ok(())
    }

    /// Executes `execution_order` without invoking the planner.
    ///
    /// # Errors
    ///
    /// - `Error::MethodFailed` — the method's function returned an error, or the method
    ///   produced a different number of outputs than declared.
    /// - `Error::TypeMismatch` — a method output's runtime type does not match the cell's
    ///   registered type.
    ///
    /// - Complexity: O(R·K) where R is the number of entries and K is the max cells per method.
    fn execute_plan(&mut self, execution_order: &[(RelationshipId, usize)]) -> Result<(), Error> {
        for &(rel_id, method_idx) in execution_order {
            // Gather outputs in a scoped block so the shared borrows on
            // `self.relationships` and `self.cells` are released before the
            // mutable borrow of `self.cells` below.
            let (outputs, output_ids) = {
                let method = &self.relationships[rel_id].methods[method_idx];
                let inputs: Vec<&dyn Any> = method
                    .inputs
                    .iter()
                    .map(|&id| self.cells[id].value.as_ref())
                    .collect();
                let outputs = (method.function)(&inputs).map_err(Error::MethodFailed)?;
                let output_ids = method.outputs.clone();
                (outputs, output_ids)
            };

            if outputs.len() != output_ids.len() {
                return Err(Error::MethodFailed(anyhow::anyhow!(
                    "method produced {} outputs but relationship expects {}",
                    outputs.len(),
                    output_ids.len()
                )));
            }

            for (cell_id, new_value) in output_ids.into_iter().zip(outputs) {
                let cell = &mut self.cells[cell_id];
                let found = new_value.as_ref().type_id();
                if found != cell.type_id {
                    return Err(Error::TypeMismatch {
                        expected: cell.type_id,
                        found,
                    });
                }
                cell.value = new_value;
                if !cell.changed {
                    cell.changed = true;
                    self.changed_cells.push(cell_id);
                }
            }
        }
        Ok(())
    }

    /// Returns the index of the method selected for `rel` in the last propagation.
    ///
    /// Returns `None` if no propagation has run yet or `rel` is not in the cached plan.
    pub fn selected_method(&self, rel: RelationshipId) -> Option<usize> {
        self.last_plan
            .as_ref()?
            .iter()
            .find(|&&(r, _)| r == rel)
            .map(|&(_, idx)| idx)
    }

    /// Returns the input cells of method `idx` in relationship `rel`.
    ///
    /// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
    pub fn method_inputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
        self.relationships
            .get(rel)?
            .methods
            .get(idx)
            .map(|m| m.inputs.as_slice())
    }

    /// Returns the output cells of method `idx` in relationship `rel`.
    ///
    /// Returns `None` if `rel` is not a live relationship or `idx` is out of bounds.
    pub fn method_outputs(&self, rel: RelationshipId, idx: usize) -> Option<&[CellId]> {
        self.relationships
            .get(rel)?
            .methods
            .get(idx)
            .map(|m| m.outputs.as_slice())
    }

    /// Returns `true` if `id` was not written by any selected method in the last propagation.
    ///
    /// Returns `false` if no propagation has run yet (conservatively forces a full re-plan).
    pub fn is_source(&self, id: CellId) -> bool {
        let Some(plan) = &self.last_plan else {
            return false;
        };
        !plan.iter().any(|&(rel_id, method_idx)| {
            self.relationships
                .get(rel_id)
                .and_then(|r| r.methods.get(method_idx))
                .map(|m| m.outputs.contains(&id))
                .unwrap_or(false)
        })
    }

    /// Re-executes the cached plan without invoking the planner.
    ///
    /// - Precondition: Every cell written since the last `propagate()` satisfies
    ///   `is_source(id)`. Violation produces incorrect output values but no panic.
    ///
    /// # Errors
    ///
    /// - `Error::Conflict` — `propagate()` has not yet been called; no plan is cached.
    /// - `Error::MethodFailed` — a method's function returned an error.
    pub fn propagate_without_replan(&mut self) -> Result<(), Error> {
        let Some(execution_order) = self.last_plan.take() else {
            return Err(Error::Conflict);
        };
        self.clear_changed();
        let result = self.execute_plan(&execution_order);
        self.last_plan = Some(execution_order);
        result
    }
}

impl Default for Sheet {
    /// Returns `Sheet::new()`.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, Method, Sheet, cell::CellId, relationship::RelationshipId};
    use std::any::TypeId;

    #[test]
    fn add_cell_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(1_i32);
        let b = sheet.add_cell(2_i32);
        assert_ne!(a, b);
    }

    #[test]
    fn write_read_roundtrip() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(42_i32);
        sheet.write(id, 99_i32).unwrap();
        assert_eq!(*sheet.read::<i32>(id).unwrap(), 99);
    }

    #[test]
    fn write_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.write(id, 1.0_f64),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn read_wrong_type_returns_type_mismatch() {
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        assert!(matches!(
            sheet.read::<f64>(id),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn add_relationship_empty_methods_returns_invalid_method() {
        let mut sheet = Sheet::new();
        assert!(matches!(
            sheet.add_relationship(vec![]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_type_mismatch_returns_error() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Method declares f64 input but cell holds i32.
        let method = Method::from_fn_1_1(a, b, |x: &f64| Ok(*x * 2.0));
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::TypeMismatch { .. })
        ));
    }

    #[test]
    fn add_relationship_overlapping_inputs_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        // Cell `a` appears in both inputs and outputs.
        let method = Method::new(
            vec![a, b],
            vec![a],
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            vec![TypeId::of::<i32>()],
            |args| Ok(vec![Box::new(*args[0].downcast_ref::<i32>().unwrap())]),
        );
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_empty_outputs_returns_invalid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let method = Method::new(
            vec![a],
            vec![], // no outputs
            vec![TypeId::of::<i32>()],
            vec![],
            |_| Ok(vec![]),
        );
        assert!(matches!(
            sheet.add_relationship(vec![method]),
            Err(Error::InvalidMethod)
        ));
    }

    #[test]
    fn add_relationship_returns_distinct_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let c = sheet.add_cell(0_i32);
        let r2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(b, c, |x: &i32| Ok(*x))])
            .unwrap();
        assert_ne!(r1, r2);
    }

    #[test]
    fn changed_is_empty_before_propagate() {
        let sheet = Sheet::new();
        assert_eq!(sheet.changed().count(), 0);
    }

    #[test]
    fn changed_after_propagate_contains_method_outputs() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        let changed: Vec<_> = sheet.changed().collect();
        assert_eq!(changed, vec![b]);
    }

    #[test]
    fn clear_changed_empties_changed_set() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.clear_changed();
        assert_eq!(sheet.changed().count(), 0);
    }

    #[test]
    fn propagate_clears_previous_changed_set() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        sheet.write(a, 3_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate().unwrap();
        let changed: Vec<_> = sheet.changed().collect();
        assert_eq!(changed, vec![b]);
    }

    #[test]
    fn cells_returns_all_cell_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let ids: Vec<_> = sheet.cells().collect();
        assert!(ids.contains(&a));
        assert!(ids.contains(&b));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn cells_returns_empty_for_empty_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.cells().count(), 0);
    }

    #[test]
    fn relationships_returns_all_relationship_ids() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let ids: Vec<_> = sheet.relationships().collect();
        assert_eq!(ids, vec![r]);
    }

    #[test]
    fn relationships_returns_empty_for_empty_sheet() {
        let sheet = Sheet::new();
        assert_eq!(sheet.relationships().count(), 0);
    }

    #[test]
    fn cell_adj_returns_adjacent_relationships() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.cell_adj(a).unwrap().contains(&r));
        assert!(sheet.cell_adj(b).unwrap().contains(&r));
    }

    #[test]
    fn cell_adj_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(sheet.cell_adj(CellId::default()).is_none());
    }

    #[test]
    fn relationship_adj_returns_adjacent_cells() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let r = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        let adj = sheet.relationship_adj(r).unwrap();
        assert!(adj.contains(&a));
        assert!(adj.contains(&b));
    }

    #[test]
    fn relationship_adj_returns_none_for_invalid_id() {
        let sheet = Sheet::new();
        assert!(sheet.relationship_adj(RelationshipId::default()).is_none());
    }

    #[test]
    fn selected_method_returns_none_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.selected_method(rel).is_none());
    }

    #[test]
    fn selected_method_returns_index_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source,
        // making the a → b method eligible.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert_eq!(sheet.selected_method(rel), Some(0));
    }

    #[test]
    fn method_inputs_returns_inputs_for_valid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert_eq!(sheet.method_inputs(rel, 0), Some([a].as_slice()));
    }

    #[test]
    fn method_outputs_returns_outputs_for_valid_method() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert_eq!(sheet.method_outputs(rel, 0), Some([b].as_slice()));
    }

    #[test]
    fn method_inputs_returns_none_for_out_of_bounds_idx() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.method_inputs(rel, 99).is_none());
    }

    #[test]
    fn method_outputs_returns_none_for_out_of_bounds_idx() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(sheet.method_outputs(rel, 99).is_none());
    }

    #[test]
    fn is_source_returns_false_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(!sheet.is_source(a));
    }

    #[test]
    fn is_source_returns_true_for_input_cell_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(sheet.is_source(a));
    }

    #[test]
    fn is_source_returns_false_for_output_cell_after_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        assert!(!sheet.is_source(b));
    }

    #[test]
    fn propagate_without_replan_returns_conflict_before_propagate() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x))])
            .unwrap();
        assert!(matches!(
            sheet.propagate_without_replan(),
            Err(Error::Conflict)
        ));
    }

    #[test]
    fn propagate_without_replan_executes_cached_plan() {
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |x: &i32| Ok(*x * 2))])
            .unwrap();
        // Write to `a` so it has the highest strength and becomes the source.
        sheet.write(a, 0_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate_without_replan().unwrap();
        assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
    }
}
