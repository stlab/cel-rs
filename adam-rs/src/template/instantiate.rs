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

#[cfg(test)]
thread_local! {
    /// Counts entered source-check frames to verify the specialization complexity bound.
    static SOURCE_PREFLIGHT_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
/// - Precondition: each slot belongs to the mapped recipe and indexes `cells`.
/// - Complexity: O(e) time and space for e edges.
fn edges(slots: &[CellSlot], cells: &[CellId]) -> Result<Vec<CellId>, TemplateError> {
    debug_assert!(slots.iter().all(|slot| slot.index < cells.len()));
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
/// - Precondition: each slot belongs to the mapped recipe and indexes `groups`;
///   every selected group range indexes `all`.
/// - Complexity: O(g + r) time and O(r) space for g groups and r members.
pub(super) fn group_members(
    slots: &[RelationshipSlot],
    groups: &[Range<usize>],
    all: &[RelationshipId],
) -> Result<Vec<RelationshipId>, TemplateError> {
    debug_assert!(slots.iter().all(|slot| {
        groups
            .get(slot.index)
            .is_some_and(|group| group.start <= group.end && group.end <= all.len())
    }));
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
    incoming: Option<RelationshipSlot>,
}

/// Holds local live mappings while replaying one occurrence in declaration order.
///
/// `next` separates entered items from pending items. Every entered cell has a live
/// handle; unentered private cells retain the sentinel until construction. Completed
/// group ranges refer to the instance's flattened relationship storage, including
/// each inclusion's contiguous transitive members.
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

    /// Retains the current conditional's local identity and active inclusion chain.
    ///
    /// - Precondition: The most recently entered item is a conditional.
    /// - Complexity: O(n + h) time and O(h) space for entered local items n and depth h.
    fn conditional_error(&self, source: TemplateError, instance: &Instance) -> TemplateError {
        debug_assert!(matches!(
            self.recipe.items.get(self.next - 1),
            Some(Item::Conditional(_))
        ));
        let conditional = self.recipe.items[..self.next - 1]
            .iter()
            .filter(|item| matches!(item, Item::Conditional(_)))
            .count();
        TemplateError::ConditionalConstruction {
            template: self.recipe.id(),
            conditional,
            inclusions: self.inclusions(instance),
            source: Box::new(source),
        }
    }

    /// Retains only the current occurrence's active root-to-child inclusion identities.
    ///
    /// - Complexity: O(h) time and space for ancestor depth h.
    fn inclusions(&self, instance: &Instance) -> Vec<RelationshipSlot> {
        let mut inclusions = std::iter::successors(Some(self.occurrence), |&index| {
            instance.occurrences[index]
                .parent
                .as_ref()
                .map(|parent| parent.index)
        })
        .filter_map(|index| instance.occurrences[index].inclusion_site.clone())
        .collect::<Vec<_>>();
        inclusions.reverse();
        inclusions
    }

    /// Wraps an original construction failure at its exact authored boundary.
    ///
    /// - Complexity: O(h) time and space for ancestor depth h.
    fn construction_error(
        &self,
        source: TemplateError,
        site: super::ConstructionSite,
        instance: &Instance,
    ) -> TemplateError {
        TemplateError::Construction {
            template: self.recipe.id(),
            site,
            inclusions: self.inclusions(instance),
            source: Box::new(source),
        }
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
    /// out-kind, or wrongly typed actuals. Returns [`TemplateError::Construction`]
    /// for initializer, method, policy, relationship and empty-specialization failures,
    /// retaining the exact definition, authored site and actual inclusion chain.
    /// Its source preserves the original [`TemplateError::Factory`],
    /// [`TemplateError::InvalidDefinition`], [`TemplateError::Runtime`] or
    /// [`TemplateError::EmptyRelationship`]. Returns [`TemplateError::Capacity`]
    /// without a declaration site on checked-size overflow or failed reservation.
    /// A conditional construction failure returns [`TemplateError::ConditionalConstruction`],
    /// retaining its definition, registration ordinal and actual root-to-child inclusion
    /// sites. Its source is the original [`TemplateError::ConditionalFactory`],
    /// [`TemplateError::InvalidDefinition`], [`TemplateError::Capacity`] or native
    /// construction error.
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
    /// - Complexity: expected O(w) time and O(k + h) space, where w counts local
    ///   cells, relationships, method outputs and child masks across distinct
    ///   (recipe, complete source mask) specializations; k counts specializations
    ///   and their mask cells, and h is inclusion depth. Distinct masks can still
    ///   grow exponentially.
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
    /// - Complexity: expected O(bc + w) time and O(k + h) space for b bindings,
    ///   c root cells, and local checking work w across distinct (recipe, complete
    ///   source mask) specializations. k counts specializations and their mask
    ///   cells, and h is inclusion depth. Distinct masks can still grow exponentially.
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

    /// Checks each distinct recipe and complete source mask once by iterative DFS.
    ///
    /// - Precondition: The mask covers this recipe's local cells.
    /// # Errors
    /// Returns capacity failure or an emptied constituent relationship.
    /// Successful completions alone enter the cache; failures retain their active
    /// inclusion ancestry. Live expansion remains occurrence-specific.
    ///
    /// - Complexity: expected O(w) time and O(k + h) space, where w counts local
    ///   cells, relationships, method outputs and child masks across distinct
    ///   specializations, k counts specializations and their mask cells, and h is
    ///   inclusion depth.
    ///   Distinct complete masks can still grow exponentially.
    fn preflight_source_mask(&self, sources: Vec<bool>) -> Result<(), TemplateError> {
        debug_assert_eq!(sources.len(), self.cells.len());
        let mut complete = HashSet::new();
        let mut stack = Vec::new();
        stack
            .try_reserve(self.depth.ok_or(TemplateError::Capacity)?)
            .map_err(|_| TemplateError::Capacity)?;
        stack.push(SourceFrame {
            recipe: self,
            sources,
            next: 0,
            incoming: None,
        });
        while let Some(frame) = stack.last_mut() {
            #[cfg(test)]
            if frame.next == 0 {
                SOURCE_PREFLIGHT_VISITS.with(|visits| visits.set(visits.get() + 1));
            }
            let Some(relationship) = frame.recipe.relationships.get(frame.next) else {
                let frame = stack.pop().expect("active source frame");
                complete
                    .try_reserve(1)
                    .map_err(|_| TemplateError::Capacity)?;
                complete.insert((frame.recipe.id(), frame.sources));
                continue;
            };
            frame.next += 1;
            match relationship {
                RelationshipRecipe::Methods(methods) => {
                    if !methods
                        .iter()
                        .any(|m| m.outputs.iter().all(|s| !frame.sources[s.index]))
                    {
                        let template = frame.recipe.id();
                        let site = super::ConstructionSite::Relationship(RelationshipSlot {
                            owner: frame.recipe.owner.clone(),
                            index: frame.next - 1,
                        });
                        return Err(TemplateError::Construction {
                            template,
                            site,
                            inclusions: stack
                                .iter()
                                .filter_map(|frame| frame.incoming.clone())
                                .collect(),
                            source: Box::new(TemplateError::EmptyRelationship),
                        });
                    }
                }
                RelationshipRecipe::Out(_) => {}
                RelationshipRecipe::Include { child, bindings } => {
                    let mut sources = storage(child.cells.len(), false)?;
                    for (i, cell) in child.cells.iter().enumerate() {
                        sources[i] = cell.kind == CellKind::Source
                            || bindings.get(&i).is_some_and(|s| frame.sources[s.index]);
                    }
                    let key = (child.id(), sources);
                    if complete.contains(&key) {
                        continue;
                    }
                    let incoming = Some(RelationshipSlot {
                        owner: frame.recipe.owner.clone(),
                        index: frame.next - 1,
                    });
                    stack.push(SourceFrame {
                        recipe: child,
                        sources: key.1,
                        next: 0,
                        incoming,
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
            conditionals: Vec::new(),
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
                        frame.cells[*index] = make(destination)
                            .map_err(|source| TemplateError::Factory {
                                source,
                                slot: CellSlot {
                                    owner: frame.recipe.owner.clone(),
                                    index: *index,
                                },
                            })
                            .map_err(|error| {
                                frame.construction_error(
                                    error,
                                    super::ConstructionSite::Cell(CellSlot {
                                        owner: frame.recipe.owner.clone(),
                                        index: *index,
                                    }),
                                    instance,
                                )
                            })?;
                        if frame.incoming.is_some() {
                            instance.cells.push(frame.cells[*index]);
                        }
                    }
                }
                Item::Policy(policy) => {
                    let ids = edges(&policy.inputs, &frame.cells)?;
                    (policy.make)(destination, frame.cells[policy.cell.index], &ids).map_err(
                        |error| {
                            frame.construction_error(
                                error,
                                super::ConstructionSite::Policy {
                                    cell: policy.cell.clone(),
                                    index: frame.recipe.items[..frame.next - 1]
                                        .iter()
                                        .filter(|item| {
                                            matches!(item, Item::Policy(previous)
                                        if previous.cell == policy.cell)
                                        })
                                        .count(),
                                },
                                instance,
                            )
                        },
                    )?;
                }
                Item::Conditional(conditional) => {
                    let ids = edges(&conditional.inputs, &frame.cells)?;
                    instance.occurrences[frame.occurrence]
                        .conditionals
                        .try_reserve(1)
                        .map_err(|_| TemplateError::Capacity)?;
                    let id = (conditional.make)(
                        destination,
                        &ids,
                        &frame.groups,
                        &instance.relationships,
                    )
                    .map_err(|error| frame.conditional_error(error, instance))?;
                    instance.occurrences[frame.occurrence].conditionals.push(id);
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
                            for (method_index, recipe) in recipes.iter().enumerate() {
                                methods.push(
                                    recipe.instantiate(destination, &frame.cells).map_err(
                                        |error| {
                                            frame.construction_error(
                                                error,
                                                super::ConstructionSite::Method {
                                                    relationship: RelationshipSlot {
                                                        owner: frame.recipe.owner.clone(),
                                                        index: *index,
                                                    },
                                                    index: method_index,
                                                },
                                                instance,
                                            )
                                        },
                                    )?,
                                );
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
                                destination.add_relationship(methods).map_err(|error| {
                                    frame.construction_error(
                                        TemplateError::Runtime(error),
                                        super::ConstructionSite::Relationship(RelationshipSlot {
                                            owner: frame.recipe.owner.clone(),
                                            index: *index,
                                        }),
                                        instance,
                                    )
                                })?,
                            );
                            instance.relationship_origins.push(RelationshipOrigin {
                                occurrence: frame.occurrence,
                                slot: *index,
                                methods: methods_start..instance.authored_methods.len(),
                            });
                            frame.groups[*index].end = instance.relationships.len();
                        }
                        RelationshipRecipe::Out(writer) => {
                            let method =
                                writer
                                    .instantiate(destination, &frame.cells)
                                    .map_err(|error| {
                                        frame.construction_error(
                                            error,
                                            super::ConstructionSite::Method {
                                                relationship: RelationshipSlot {
                                                    owner: frame.recipe.owner.clone(),
                                                    index: *index,
                                                },
                                                index: 0,
                                            },
                                            instance,
                                        )
                                    })?;
                            let relationship =
                                destination.add_out_writer(method).map_err(|error| {
                                    frame.construction_error(
                                        TemplateError::Runtime(error),
                                        super::ConstructionSite::Relationship(RelationshipSlot {
                                            owner: frame.recipe.owner.clone(),
                                            index: *index,
                                        }),
                                        instance,
                                    )
                                })?;
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
                                conditionals: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::SOURCE_PREFLIGHT_VISITS;
    use crate::TemplateBuilder;
    use std::rc::Rc;

    /// Validates complete positional signatures, including empty, foreign and mismatched inputs.
    #[test]
    fn signatures_check_arity_liveness_and_concrete_types() {
        use super::{CellId, Sheet, TypeId, signature_matches};
        let mut sheet = Sheet::new();
        let number = sheet.add_cell(3_i32);
        let flag = sheet.add_cell(true);
        assert!(signature_matches(&sheet, &[], &[]));
        assert!(signature_matches(
            &sheet,
            &[number, flag],
            &[TypeId::of::<i32>(), TypeId::of::<bool>()]
        ));
        assert!(!signature_matches(&sheet, &[number], &[]));
        assert!(!signature_matches(&sheet, &[], &[TypeId::of::<i32>()]));
        assert!(!signature_matches(
            &sheet,
            &[number],
            &[TypeId::of::<bool>()]
        ));
        assert!(!signature_matches(
            &sheet,
            &[CellId::default()],
            &[TypeId::of::<i32>()]
        ));
    }

    /// Maps live edges and complete inclusion ranges in the caller's requested order.
    #[test]
    fn local_storage_and_positional_mapping_preserve_order() {
        use super::{
            CellId, LiveFrame, Method, MethodRecipe, Sheet, TemplateError, edges, group_members,
            storage,
        };
        assert_eq!(storage(0, 7_u8).unwrap(), []);
        assert_eq!(storage(2, 7_u8).unwrap(), [7, 7]);
        assert!(matches!(
            storage(usize::MAX, 0_u8),
            Err(TemplateError::Capacity)
        ));
        let mut builder = TemplateBuilder::new();
        let a = builder.parameter::<i32>("a").unwrap();
        let b = builder.parameter::<i32>("b").unwrap();
        let group = builder
            .relationship(vec![
                MethodRecipe::new(
                    std::slice::from_ref(&a),
                    std::slice::from_ref(&b),
                    |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |v: &i32| Ok(*v))),
                )
                .unwrap(),
            ])
            .unwrap();
        let template = builder.finish().unwrap();
        let frame = LiveFrame::new(&template, None, 0).unwrap();
        assert_eq!(frame.cells, [CellId::default(); 2]);
        assert_eq!(frame.groups.as_slice(), std::slice::from_ref(&(0..0)));
        assert_eq!(frame.next, 0);
        let mut sheet = Sheet::new();
        let live_a = sheet.add_cell(3_i32);
        let live_b = sheet.add_cell(0_i32);
        let instance = template
            .instantiate(&mut sheet, &[("a", live_a), ("b", live_b)])
            .unwrap();
        assert_eq!(edges(&[b, a], &[live_a, live_b]).unwrap(), [live_b, live_a]);
        assert_eq!(
            group_members(
                &[group],
                std::slice::from_ref(&(0..1)),
                instance.relationships()
            )
            .unwrap(),
            instance.relationships()
        );
        assert!(group_members(&[], &[], &[]).unwrap().is_empty());
    }

    /// Checks work by distinct immutable specializations rather than expanded occurrences.
    #[test]
    fn source_preflight_checks_shared_dag_specializations_once() {
        let depth = 12;
        let mut template = Rc::new(TemplateBuilder::new().finish().unwrap());
        for _ in 0..depth {
            let mut builder = TemplateBuilder::new();
            builder.include(template.clone(), &[]).unwrap();
            builder.include(template, &[]).unwrap();
            template = Rc::new(builder.finish().unwrap());
        }
        SOURCE_PREFLIGHT_VISITS.with(|visits| visits.set(0));
        template.check_source_bindings(&[]).unwrap();
        SOURCE_PREFLIGHT_VISITS.with(|visits| {
            assert!(
                visits.get() <= 2 * (depth + 1),
                "visited {} frames",
                visits.get()
            );
        });
    }
}
