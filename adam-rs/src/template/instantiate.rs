//! Checked binding and expansion through existing native graph mutators.

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use super::provenance::RelationshipOrigin;
use super::{
    CellSlot, Instance, InstanceOccurrence, Item, MethodRecipe, OccurrenceId, RelationshipRecipe,
    RelationshipSlot, SheetTemplate, TemplateError,
};
use crate::{CellId, CellKind, Method, RelationshipId, Sheet};

/// Allocates initialized traversal storage without infallible reservation.
///
/// - Complexity: O(n) time and space for n elements.
fn storage<T: Clone>(count: usize, value: T) -> Result<Vec<T>, TemplateError> {
    let mut result = Vec::new();
    result
        .try_reserve(count)
        .map_err(|_| TemplateError::Capacity)?;
    result.resize(count, value);
    Ok(result)
}

/// Maps declared positional edges to live IDs.
///
/// - Complexity: O(e) time and space for e edges.
fn edges(slots: &[CellSlot], cells: &[CellId]) -> Result<Vec<CellId>, TemplateError> {
    let mut result = Vec::new();
    result
        .try_reserve(slots.len())
        .map_err(|_| TemplateError::Capacity)?;
    result.extend(slots.iter().map(|s| cells[s.index]));
    Ok(result)
}

/// Checks a complete positional signature against registered concrete types.
///
/// - Complexity: O(e) time for e edges.
pub(super) fn signature_matches(sheet: &Sheet, ids: &[CellId], types: &[TypeId]) -> bool {
    ids.len() == types.len()
        && ids
            .iter()
            .zip(types)
            .all(|(id, ty)| sheet.cell_type_id(*id) == Some(*ty))
}

/// Expands group ranges into distinct live relationship IDs in authored group order.
///
/// - Complexity: O(g + r) time and O(r) space for g groups and r members.
pub(super) fn group_members(
    slots: &[RelationshipSlot],
    groups: &[Range<usize>],
    all: &[RelationshipId],
) -> Result<Vec<RelationshipId>, TemplateError> {
    let count = slots
        .iter()
        .try_fold(0usize, |n, slot| n.checked_add(groups[slot.index].len()))
        .ok_or(TemplateError::Capacity)?;
    let mut result = Vec::new();
    result
        .try_reserve(count)
        .map_err(|_| TemplateError::Capacity)?;
    for slot in slots {
        result.extend_from_slice(&all[groups[slot.index].clone()]);
    }
    Ok(result)
}

/// Holds one active occurrence during non-recursive source preflight.
struct SourceFrame<'a> {
    recipe: &'a SheetTemplate,
    sources: Vec<bool>,
    next: usize,
}

/// Holds local live mappings while replaying one occurrence in declaration order.
struct LiveFrame<'a> {
    recipe: &'a SheetTemplate,
    cells: Vec<CellId>,
    groups: Vec<Range<usize>>,
    next: usize,
    incoming: Option<usize>,
    occurrence: usize,
}

impl<'a> LiveFrame<'a> {
    /// Allocates one occurrence's local mappings using fallible reservation.
    ///
    /// - Complexity: O(c + r) time and space for local cells c and groups r.
    fn new(
        recipe: &'a SheetTemplate,
        incoming: Option<usize>,
        occurrence: usize,
    ) -> Result<Self, TemplateError> {
        Ok(Self {
            recipe,
            cells: storage(recipe.cells.len(), CellId::default())?,
            groups: storage(recipe.relationships.len(), 0..0)?,
            next: 0,
            incoming,
            occurrence,
        })
    }
}

impl SheetTemplate {
    /// Expands this recipe into `destination`, preserving bound-cell identities and values.
    ///
    /// Binding, declared structure, expanded sizes, result reservation and empty
    /// specialized relationships are checked before mutation or factory calls, including
    /// relationships in inactive branches. A source output removes its entire method.
    /// Every authored method factory is checked against its declared edges/types
    /// before pruning or registration, even when that method writes a source.
    /// Out-cell arguments are unsupported.
    /// Later factory, allocation or native failures may leave partial structure; the destination
    /// remains safely discardable. Factories run only here, never at propagation.
    ///
    /// # Errors
    ///
    /// Returns binding errors for unknown, missing, repeated, aliased, invalid,
    /// out-kind, or wrongly typed actuals. Returns [`TemplateError::Factory`] for
    /// failed initializers/method factories, [`TemplateError::InvalidDefinition`]
    /// for factory edge/type mismatches, and [`TemplateError::Runtime`] for native
    /// graph construction failures. Returns [`TemplateError::EmptyRelationship`]
    /// if specialization removes all directions, or [`TemplateError::Capacity`]
    /// on checked-size overflow or failed expansion reservation.
    /// A failed match-expression factory returns [`TemplateError::ConditionalFactory`].
    ///
    /// - Complexity: Expected O(d + c + b + Σ(m² × e)) time and O(d + c + b + e + r + h)
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
        let size = self.expanded_size.ok_or(TemplateError::Capacity)?;
        let mut instance = Instance {
            _identity: Rc::new(()),
            cells: Vec::new(),
            relationships: Vec::new(),
            occurrences: Vec::new(),
            local_cells: Vec::new(),
            local_groups: Vec::new(),
            relationship_origins: Vec::new(),
            authored_methods: Vec::new(),
        };
        instance
            .cells
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .relationships
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .occurrences
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .local_cells
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .local_groups
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .relationship_origins
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        instance
            .authored_methods
            .try_reserve(size)
            .map_err(|_| TemplateError::Capacity)?;
        self.preflight_sources(destination, &bound)?;
        self.expand(destination, &bound, &mut instance)?;
        Ok(instance)
    }

    /// Rejects every source-specialized empty constituent without invoking factories.
    ///
    /// Source status flows through checked child bindings; private sources remain fixed.
    /// Cached sizes reject oversized DAGs before this traversal.
    ///
    /// - Complexity: O(c + e + r) time and O(c + h) space for expanded cells c,
    ///   edges e, relationships r and nesting depth h.
    fn preflight_sources(
        &self,
        destination: &Sheet,
        bound: &HashMap<usize, CellId>,
    ) -> Result<(), TemplateError> {
        let mut sources = storage(self.cells.len(), false)?;
        for (i, cell) in self.cells.iter().enumerate() {
            sources[i] = cell.kind == CellKind::Source
                || bound
                    .get(&i)
                    .is_some_and(|id| destination.cell_kind(*id) == Some(CellKind::Source));
        }
        self.preflight_source_mask(sources)
    }

    /// Checks complete named formal source statuses without constructing a live sheet.
    ///
    /// Uses the same whole-method pruning preflight as [`Self::instantiate`].
    /// No initializer, method, policy, or conditional factory executes.
    ///
    /// # Errors
    /// Rejects missing, unknown or repeated names, capacity overflow, or a constituent
    /// relationship left without a method, including inactive branches.
    /// - Complexity: O(bc + e) time and O(c + h) space for b bindings, c expanded
    ///   cells, e expanded edges, and inclusion depth h.
    ///
    /// # Examples
    /// ```
    /// let mut builder = adam_rs::TemplateBuilder::new();
    /// builder.parameter::<i32>("x")?;
    /// builder.finish()?.check_source_bindings(&[("x", true)])?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn check_source_bindings(&self, bindings: &[(&str, bool)]) -> Result<(), TemplateError> {
        self.expanded_size.ok_or(TemplateError::Capacity)?;
        let mut sources = storage(self.cells.len(), false)?;
        let mut seen = HashSet::new();
        for &(name, source) in bindings {
            let Some(index) = self
                .cells
                .iter()
                .position(|cell| cell.make.is_none() && cell.name == name)
            else {
                return Err(TemplateError::InvalidDefinition {
                    detail: "unknown source-binding parameter",
                    slot: None,
                });
            };
            if !seen.insert(index) {
                return Err(TemplateError::InvalidDefinition {
                    detail: "repeated source-binding parameter",
                    slot: None,
                });
            }
            sources[index] = source;
        }
        if seen.len() != self.parameter_names().count() {
            return Err(TemplateError::InvalidDefinition {
                detail: "missing source-binding parameter",
                slot: None,
            });
        }
        for (index, cell) in self.cells.iter().enumerate() {
            sources[index] |= cell.kind == CellKind::Source;
        }
        self.preflight_source_mask(sources)
    }

    /// Traverses whole-method source specialization from a checked local mask.
    ///
    /// - Precondition: The mask covers this recipe's local cells.
    /// # Errors
    /// Returns capacity failure or an emptied constituent relationship.
    /// - Complexity: O(c + e + r) time and O(c + h) space in expanded structure.
    fn preflight_source_mask(&self, sources: Vec<bool>) -> Result<(), TemplateError> {
        debug_assert_eq!(sources.len(), self.cells.len());
        let mut stack = Vec::new();
        stack
            .try_reserve(self.depth.ok_or(TemplateError::Capacity)?)
            .map_err(|_| TemplateError::Capacity)?;
        stack.push(SourceFrame {
            recipe: self,
            sources,
            next: 0,
        });
        while let Some(frame) = stack.last_mut() {
            let Some(relationship) = frame.recipe.relationships.get(frame.next) else {
                stack.pop();
                continue;
            };
            frame.next += 1;
            match relationship {
                RelationshipRecipe::Methods(methods) => {
                    if !methods
                        .iter()
                        .any(|m| m.outputs.iter().all(|s| !frame.sources[s.index]))
                    {
                        return Err(TemplateError::EmptyRelationship);
                    }
                }
                RelationshipRecipe::Out(_) => {}
                RelationshipRecipe::Include { child, bindings } => {
                    let mut sources = storage(child.cells.len(), false)?;
                    for (i, cell) in child.cells.iter().enumerate() {
                        sources[i] = cell.kind == CellKind::Source
                            || bindings.get(&i).is_some_and(|s| frame.sources[s.index]);
                    }
                    stack.push(SourceFrame {
                        recipe: child,
                        sources,
                        next: 0,
                    });
                }
            }
        }
        Ok(())
    }

    /// Replays ordered registration using an explicit stack and contiguous group ranges.
    ///
    /// - Precondition: Source and binding preflight succeeded and instance storage is reserved.
    /// - Complexity: O(d + c + r + e + Σ(m² × e)) time and O(d + c + r + h + e) space
    ///   for expanded declarations d, cells c, relationships r, edges e and depth h,
    ///   excluding factories.
    fn expand(
        &self,
        destination: &mut Sheet,
        bound: &HashMap<usize, CellId>,
        instance: &mut Instance,
    ) -> Result<(), TemplateError> {
        let mut root = LiveFrame::new(self, None, 0)?;
        for (&index, &actual) in bound {
            root.cells[index] = actual;
        }
        instance.cells.resize(self.cells.len(), CellId::default());
        let mut stack = Vec::new();
        stack
            .try_reserve(self.depth.ok_or(TemplateError::Capacity)?)
            .map_err(|_| TemplateError::Capacity)?;
        instance.occurrences.push(InstanceOccurrence {
            id: OccurrenceId {
                owner: instance._identity.clone(),
                index: 0,
            },
            template: self.id(),
            parent: None,
            inclusion_site: None,
            cells: 0..0,
            groups: 0..0,
        });
        stack.push(root);
        while let Some(frame) = stack.last_mut() {
            let Some(item) = frame.recipe.items.get(frame.next) else {
                let finished = stack.pop().unwrap();
                let cells_start = instance.local_cells.len();
                instance.local_cells.extend_from_slice(&finished.cells);
                let groups_start = instance.local_groups.len();
                instance.local_groups.extend_from_slice(&finished.groups);
                let record = &mut instance.occurrences[finished.occurrence];
                record.cells = cells_start..instance.local_cells.len();
                record.groups = groups_start..instance.local_groups.len();
                if let Some(index) = finished.incoming {
                    stack.last_mut().unwrap().groups[index].end = instance.relationships.len();
                } else {
                    instance.cells[..finished.cells.len()].copy_from_slice(&finished.cells);
                }
                continue;
            };
            frame.next += 1;
            match item {
                Item::Cell(index) => {
                    if let Some(make) = &frame.recipe.cells[*index].make {
                        frame.cells[*index] =
                            make(destination).map_err(|source| TemplateError::Factory {
                                source,
                                slot: CellSlot {
                                    owner: frame.recipe.owner.clone(),
                                    index: *index,
                                },
                            })?;
                        if frame.incoming.is_some() {
                            instance.cells.push(frame.cells[*index]);
                        }
                    }
                }
                Item::Policy(policy) => {
                    let ids = edges(&policy.inputs, &frame.cells)?;
                    (policy.make)(destination, frame.cells[policy.cell.index], &ids)?;
                }
                Item::Conditional(conditional) => {
                    let ids = edges(&conditional.inputs, &frame.cells)?;
                    (conditional.make)(destination, &ids, &frame.groups, &instance.relationships)?;
                }
                Item::Relationship(index) => {
                    let start = instance.relationships.len();
                    frame.groups[*index] = start..start;
                    match &frame.recipe.relationships[*index] {
                        RelationshipRecipe::Methods(recipes) => {
                            let methods_start = instance.authored_methods.len();
                            let mut methods = Vec::new();
                            methods
                                .try_reserve(recipes.len())
                                .map_err(|_| TemplateError::Capacity)?;
                            // Validate all authored factories before removing entire source-writing methods.
                            for recipe in recipes {
                                methods.push(recipe.instantiate(destination, &frame.cells)?);
                            }
                            let mut authored_index = 0;
                            methods.retain(|method| {
                                let survives = method
                                    .outputs
                                    .iter()
                                    .all(|id| destination.cell_kind(*id) != Some(CellKind::Source));
                                if survives {
                                    instance.authored_methods.push(authored_index);
                                }
                                authored_index += 1;
                                survives
                            });
                            debug_assert!(!methods.is_empty());
                            instance.relationships.push(
                                destination
                                    .add_relationship(methods)
                                    .map_err(TemplateError::Runtime)?,
                            );
                            instance.relationship_origins.push(RelationshipOrigin {
                                occurrence: frame.occurrence,
                                slot: *index,
                                methods: methods_start..instance.authored_methods.len(),
                            });
                            frame.groups[*index].end = instance.relationships.len();
                        }
                        RelationshipRecipe::Out(writer) => {
                            let method = writer.instantiate(destination, &frame.cells)?;
                            let relationship = destination
                                .add_out_writer(method)
                                .map_err(TemplateError::Runtime)?;
                            instance.relationships.push(relationship);
                            let methods_start = instance.authored_methods.len();
                            instance.authored_methods.push(0);
                            instance.relationship_origins.push(RelationshipOrigin {
                                occurrence: frame.occurrence,
                                slot: *index,
                                methods: methods_start..instance.authored_methods.len(),
                            });
                            frame.groups[*index].end = instance.relationships.len();
                        }
                        RelationshipRecipe::Include { child, bindings } => {
                            let occurrence = instance.occurrences.len();
                            let mut next = LiveFrame::new(child, Some(*index), occurrence)?;
                            for (i, slot) in bindings {
                                next.cells[*i] = frame.cells[slot.index];
                            }
                            instance.occurrences.push(InstanceOccurrence {
                                id: OccurrenceId {
                                    owner: instance._identity.clone(),
                                    index: occurrence,
                                },
                                template: child.id(),
                                parent: Some(instance.occurrences[frame.occurrence].id.clone()),
                                inclusion_site: Some(RelationshipSlot {
                                    owner: frame.recipe.owner.clone(),
                                    index: *index,
                                }),
                                cells: 0..0,
                                groups: 0..0,
                            });
                            stack.push(next);
                        }
                    }
                }
            }
        }
        Ok(())
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
        let inputs = edges(&self.inputs, cells)?;
        let outputs = edges(&self.outputs, cells)?;
        let method = (self.make)(&inputs, &outputs).map_err(|source| TemplateError::Factory {
            source,
            slot: self.outputs[0].clone(),
        })?;
        if method.inputs != inputs
            || method.outputs != outputs
            || !signature_matches(sheet, &inputs, &method.input_types)
            || !signature_matches(sheet, &outputs, &method.output_types)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "method factory edges or concrete types differ from its recipe",
                slot: Some(self.outputs[0].clone()),
            });
        }
        Ok(method)
    }
}
