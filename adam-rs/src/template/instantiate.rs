//! Checked binding and expansion through existing native graph mutators.

use std::collections::HashMap;
use std::rc::Rc;

use super::{
    CellSlot, Instance, Item, MethodRecipe, RelationshipRecipe, SheetTemplate, TemplateError,
};
use crate::{CellId, CellKind, Error, ErrorSite, Method, Sheet};

impl SheetTemplate {
    /// Expands this recipe into `destination`, preserving bound-cell identities and values.
    ///
    /// All binding and authored structure checks precede mutation and factory calls.
    /// Out-cell arguments are unsupported. Source-output pruning is not implemented
    /// yet: a source actual used as an output is rejected before mutation.
    /// Later factory/native failures may leave partial structure; the destination
    /// remains safely discardable. Factories run only here, never at propagation.
    ///
    /// # Errors
    ///
    /// Returns binding errors for unknown, missing, repeated, aliased, invalid,
    /// out-kind, or wrongly typed actuals. Returns [`TemplateError::Factory`] for
    /// failed initializers/method factories, [`TemplateError::InvalidDefinition`]
    /// for factory edge/type mismatches, and [`TemplateError::Runtime`] for native
    /// graph construction failures.
    ///
    /// - Complexity: Expected O(d + c + b + Σ(m² × e)) time and O(c + b + e + r + h)
    ///   additional space for expanded declarations d, cells c, bindings b,
    ///   relationships r, depth h, and each relationship's m methods of at most e
    ///   edges, excluding factory costs.
    ///
    /// # Examples
    ///
    /// See the [module example](super).
    pub fn instantiate(
        &self,
        destination: &mut Sheet,
        bindings: &[(&str, CellId)],
    ) -> Result<Instance, TemplateError> {
        let bound = self.bind(bindings, |name, actual| {
            let type_id =
                destination
                    .cell_type_id(*actual)
                    .ok_or_else(|| TemplateError::InvalidActual {
                        name: name.into(),
                        actual: *actual,
                    })?;
            if destination.cell_kind(*actual) == Some(CellKind::Out) {
                return Err(TemplateError::OutArgument {
                    name: name.into(),
                    actual: *actual,
                });
            }
            Ok(type_id)
        })?;
        // Reject unsupported bound source outputs before any private factory runs.
        for index in &self.written_cells {
            if let Some(actual) = bound.get(index)
                && destination.cell_kind(*actual) == Some(CellKind::Source)
            {
                return Err(TemplateError::Runtime(Error::InvalidCellKind {
                    sites: vec![ErrorSite::Cell(*actual)],
                }));
            }
        }
        let mut instance = Instance {
            _identity: Rc::new(()),
            cells: Vec::new(),
            relationships: Vec::new(),
        };
        let mut cells = self.expand(destination, &bound, &mut instance, false)?;
        cells.append(&mut instance.cells);
        instance.cells = cells;
        Ok(instance)
    }

    /// Expands one DAG occurrence using already checked local bindings.
    ///
    /// Included private cells join the instance's extra-cell list; root locals
    /// remain in the returned mapping. Group members retain distinct live IDs.
    ///
    /// - Precondition: Bindings cover exactly this recipe's formals with matching types.
    /// - Complexity: O(N + C + E + Σ(m² × e)) time and O(C + E + R + D) space
    ///   for expanded declarations N, cells C, edges E, relationships R, depth D,
    ///   and native relationship validations.
    fn expand(
        &self,
        destination: &mut Sheet,
        bound: &HashMap<usize, CellId>,
        instance: &mut Instance,
        included: bool,
    ) -> Result<Vec<CellId>, TemplateError> {
        debug_assert!(
            self.cells
                .iter()
                .enumerate()
                .all(|(i, cell)| { cell.make.is_some() || bound.contains_key(&i) })
        );
        let mut cells = Vec::new();
        cells
            .try_reserve(self.cells.len())
            .map_err(|_| TemplateError::Capacity)?;
        cells.resize(self.cells.len(), CellId::default());
        for (&index, &actual) in bound {
            cells[index] = actual;
        }
        instance
            .relationships
            .try_reserve(self.relationships.len())
            .map_err(|_| TemplateError::Capacity)?;
        // Replay authored order without allocating or writing parameter cells.
        for item in &self.items {
            match item {
                Item::Cell(index) => {
                    if let Some(make) = &self.cells[*index].make {
                        cells[*index] =
                            make(destination).map_err(|source| TemplateError::Factory {
                                source,
                                slot: CellSlot {
                                    owner: self.owner.clone(),
                                    index: *index,
                                },
                            })?;
                        if included {
                            instance.cells.push(cells[*index]);
                        }
                    }
                }
                Item::Relationship(index) => match &self.relationships[*index] {
                    RelationshipRecipe::Methods(recipes) => {
                        let methods = recipes
                            .iter()
                            .map(|method| method.instantiate(destination, &cells))
                            .collect::<Result<Vec<_>, _>>()?;
                        instance.relationships.push(
                            destination
                                .add_relationship(methods)
                                .map_err(TemplateError::Runtime)?,
                        );
                    }
                    RelationshipRecipe::Include { child, bindings } => {
                        let actuals = bindings
                            .iter()
                            .map(|(i, slot)| (*i, cells[slot.index]))
                            .collect();
                        child.expand(destination, &actuals, instance, true)?;
                    }
                },
            }
        }
        Ok(cells)
    }
}

impl MethodRecipe {
    /// Creates and verifies one fresh method against the recipe's positional edges and types.
    ///
    /// - Precondition: `cells` maps every local edge to a registered live cell.
    /// - Complexity: O(i + o) time and space for input and output counts i and o.
    fn instantiate(&self, sheet: &Sheet, cells: &[CellId]) -> Result<Method, TemplateError> {
        debug_assert!(
            self.inputs
                .iter()
                .chain(&self.outputs)
                .all(|s| s.index < cells.len())
        );
        let inputs: Vec<_> = self.inputs.iter().map(|s| cells[s.index]).collect();
        let outputs: Vec<_> = self.outputs.iter().map(|s| cells[s.index]).collect();
        let method = (self.make)(&inputs, &outputs).map_err(|source| TemplateError::Factory {
            source,
            slot: self.outputs[0].clone(),
        })?;
        if method.inputs != inputs
            || method.outputs != outputs
            || method.input_types.len() != inputs.len()
            || method.output_types.len() != outputs.len()
            || inputs
                .iter()
                .zip(&method.input_types)
                .chain(outputs.iter().zip(&method.output_types))
                .any(|(id, ty)| sheet.cell_type_id(*id) != Some(*ty))
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "method factory edges or concrete types differ from its recipe",
                slot: Some(self.outputs[0].clone()),
            });
        }
        Ok(method)
    }
}
