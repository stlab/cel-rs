//! Ordered recipe construction and local inclusion remapping.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::{
    CellFactory, CellRecipe, CellSlot, ConditionalRecipe, Item, MethodRecipe, PolicyRecipe,
    RelationshipRecipe, RelationshipSlot, SheetTemplate, TemplateError, TemplateInclusion,
};
use crate::conditional::MatchSource;
use crate::{CellId, CellKind, Filter, MatchExpr, Requirement, Sheet};

/// Owns declarations and validates all local references before accepting them.
///
/// Private included names do not enter this builder's declaration namespace.
///
/// # Examples
/// See the [module example](super).
pub struct TemplateBuilder {
    recipe: SheetTemplate,
    names: HashSet<String>,
    /// Tracks local groups already owned by a conditional.
    conditional_groups: HashSet<usize>,
    /// Tracks private cells already assigned a filter.
    filtered: HashSet<usize>,
}

impl Default for TemplateBuilder {
    /// Creates an empty builder with a fresh local owner.
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateBuilder {
    /// Creates an empty recipe with its own local handle namespace.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::TemplateBuilder;
    /// let template = TemplateBuilder::new().finish()?;
    /// assert!(template.is_relationship_only());
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn new() -> Self {
        Self {
            recipe: SheetTemplate {
                owner: Rc::new(()),
                cells: Vec::new(),
                relationships: Vec::new(),
                items: Vec::new(),
                written_cells: HashSet::new(),
                relationship_only: true,
                expanded_size: Some(0),
                depth: Some(1),
            },
            names: HashSet::new(),
            conditional_groups: HashSet::new(),
            filtered: HashSet::new(),
        }
    }

    /// Declares a typed formal that allocates and writes no destination cell.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::DuplicateParameter`] for a repeated declaration
    /// name, or [`TemplateError::Capacity`] when storage cannot be reserved.
    ///
    /// - Complexity: Amortized O(n) time and space for a name of length n.
    ///
    /// # Examples
    ///
    /// See the [module example](super).
    pub fn parameter<T: 'static>(&mut self, name: &str) -> Result<CellSlot, TemplateError> {
        self.add_cell_recipe(name, TypeId::of::<T>(), CellKind::Cell, None)
    }

    /// Declares a private cell initialized afresh for every live instance.
    ///
    /// The initializer owns its captures and runs only during instantiation.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::DuplicateParameter`] for a repeated declaration
    /// name, or [`TemplateError::Capacity`] when storage cannot be reserved.
    ///
    /// - Complexity: Amortized O(n) time and space for a name of length n.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{Sheet, TemplateBuilder};
    /// let mut builder = TemplateBuilder::new();
    /// builder.cell("count", || Ok(3i32))?;
    /// let mut sheet = Sheet::new();
    /// let instance = builder.finish()?.instantiate(&mut sheet, &[])?;
    /// assert_eq!(*sheet.read::<i32>(instance.cells()[0])?, 3);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn cell<T, F>(&mut self, name: &str, initial: F) -> Result<CellSlot, TemplateError>
    where
        T: Any + PartialEq + 'static,
        F: Fn() -> anyhow::Result<T> + 'static,
    {
        self.add_cell_recipe(
            name,
            TypeId::of::<T>(),
            CellKind::Cell,
            Some(Box::new(move |sheet: &mut Sheet| {
                Ok(sheet.add_cell(initial()?))
            })),
        )
    }

    /// Declares a private source initialized afresh for every live instance.
    ///
    /// Specialization removes every whole method that writes a source.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::DuplicateParameter`] for a repeated declaration
    /// name, or [`TemplateError::Capacity`] when storage cannot be reserved.
    ///
    /// - Complexity: Amortized O(n) time and space for a name of length n.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{CellKind, Sheet, TemplateBuilder};
    /// let mut builder = TemplateBuilder::new();
    /// builder.source("constant", || Ok(7i32))?;
    /// let mut sheet = Sheet::new();
    /// let instance = builder.finish()?.instantiate(&mut sheet, &[])?;
    /// assert_eq!(sheet.cell_kind(instance.cells()[0]), Some(CellKind::Source));
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn source<T, F>(&mut self, name: &str, initial: F) -> Result<CellSlot, TemplateError>
    where
        T: Any + PartialEq + 'static,
        F: Fn() -> anyhow::Result<T> + 'static,
    {
        self.add_cell_recipe(
            name,
            TypeId::of::<T>(),
            CellKind::Source,
            Some(Box::new(move |sheet: &mut Sheet| {
                Ok(sheet.add_source(initial()?))
            })),
        )
    }

    /// Appends a checked local cell declaration and its ordered body item.
    ///
    /// - Complexity: Amortized O(n) time and space for a name of length n.
    fn add_cell_recipe(
        &mut self,
        name: &str,
        type_id: TypeId,
        kind: CellKind,
        make: Option<Box<CellFactory>>,
    ) -> Result<CellSlot, TemplateError> {
        if self.names.contains(name) {
            return Err(TemplateError::DuplicateParameter { name: name.into() });
        }
        self.recipe
            .cells
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        let index = self.recipe.cells.len();
        self.names.insert(name.into());
        self.recipe.relationship_only &= make.is_none();
        self.recipe.cells.push(CellRecipe {
            name: name.into(),
            type_id,
            kind,
            make,
        });
        self.recipe.items.push(Item::Cell(index));
        Ok(CellSlot {
            owner: self.recipe.owner.clone(),
            index,
        })
    }

    /// Appends a relationship after validating owners and native edge-set rules.
    ///
    /// Methods must reference the same cell set and have distinct, non-nested
    /// output sets. Source-writing methods remain authored until specialization.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::EmptyRelationship`], [`TemplateError::ForeignHandle`],
    /// [`TemplateError::InvalidDefinition`], or [`TemplateError::Capacity`].
    ///
    /// - Complexity: O(m² × e) time and O(m × e) space for m methods of at most e edges.
    ///
    /// # Examples
    ///
    /// See the [module example](super).
    pub fn relationship(
        &mut self,
        methods: Vec<MethodRecipe>,
    ) -> Result<RelationshipSlot, TemplateError> {
        self.validate_relationship(&methods)?;
        self.recipe
            .relationships
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        let index = self.recipe.relationships.len();
        self.recipe
            .written_cells
            .extend(methods.iter().flat_map(|m| &m.outputs).map(|s| s.index));
        self.recipe
            .relationships
            .push(RelationshipRecipe::Methods(methods));
        self.recipe.items.push(Item::Relationship(index));
        Ok(RelationshipSlot {
            owner: self.recipe.owner.clone(),
            index,
        })
    }

    /// Checks ownership and native relationship invariants without running factories.
    ///
    /// - Complexity: O(m² × e) time and O(m × e) space for m methods of at most e edges.
    fn validate_relationship(&self, methods: &[MethodRecipe]) -> Result<(), TemplateError> {
        if methods.is_empty() {
            return Err(TemplateError::EmptyRelationship);
        }
        for slot in methods
            .iter()
            .flat_map(|m| m.inputs.iter().chain(&m.outputs))
        {
            self.check_slot(slot)?;
        }
        if let Some(slot) = methods
            .iter()
            .flat_map(|m| &m.outputs)
            .find(|s| self.recipe.cells[s.index].kind == CellKind::Out)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "an out cell already has its sole writer",
                slot: Some(slot.clone()),
            });
        }
        let edges: Vec<_> = methods
            .iter()
            .map(|method| (method.inputs.as_slice(), method.outputs.as_slice()))
            .collect();
        super::check_relationship_edges(&edges).map_err(|detail| TemplateError::InvalidDefinition {
            detail,
            slot: Some(methods[0].outputs[0].clone()),
        })
    }

    /// Rejects a local reference not owned by this recipe.
    fn check_slot(&self, slot: &CellSlot) -> Result<(), TemplateError> {
        if !Rc::ptr_eq(&self.recipe.owner, &slot.owner) || slot.index >= self.recipe.cells.len() {
            return Err(TemplateError::ForeignHandle { slot: slot.clone() });
        }
        Ok(())
    }

    /// Promotes a private ordinary cell through its sole, single-output writer.
    ///
    /// Formal parameters cannot become outputs. Requirements attach separately,
    /// in declaration order, through [`Self::require`].
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::InvalidDefinition`] for a non-private, non-ordinary,
    /// multiply-written, or non-single output; also returns relationship validation errors.
    ///
    /// - Complexity: O(i + r) time and O(i) space for inputs i and prior output summaries r.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{TemplateBuilder, MethodRecipe, Method};
    /// let mut b = TemplateBuilder::new();
    /// let input = b.parameter::<i32>("input")?;
    /// let output = b.cell("output", || Ok(0i32))?;
    /// b.out(MethodRecipe::new(&[input], &[output], |i, o| {
    ///     Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x)))
    /// })?)?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn out(&mut self, writer: MethodRecipe) -> Result<RelationshipSlot, TemplateError> {
        self.validate_relationship(std::slice::from_ref(&writer))?;
        let output = &writer.outputs[0];
        self.check_private(output)?;
        if writer.outputs.len() != 1
            || self.recipe.cells[output.index].kind != CellKind::Cell
            || self.recipe.written_cells.contains(&output.index)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "out requires one private ordinary cell with no prior writer",
                slot: Some(output.clone()),
            });
        }
        self.recipe
            .relationships
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        let index = self.recipe.relationships.len();
        self.recipe.cells[output.index].kind = CellKind::Out;
        self.recipe.written_cells.insert(output.index);
        self.recipe
            .relationships
            .push(RelationshipRecipe::Out(writer));
        self.recipe.items.push(Item::Relationship(index));
        self.recipe.relationship_only = false;
        Ok(RelationshipSlot {
            owner: self.recipe.owner.clone(),
            index,
        })
    }

    /// Rejects policy targets that are not private declarations of this owner.
    fn check_private(&self, cell: &CellSlot) -> Result<(), TemplateError> {
        self.check_slot(cell)?;
        if self.recipe.cells[cell.index].make.is_none() {
            return Err(TemplateError::InvalidDefinition {
                detail: "component policies and out writers require private targets",
                slot: Some(cell.clone()),
            });
        }
        Ok(())
    }

    /// Attaches one freshly constructed filter to a private cell.
    ///
    /// The factory receives the live target and positional dynamic arguments.
    /// Its filter must declare those exact arguments and their concrete types.
    ///
    /// # Errors
    ///
    /// Returns ownership, capacity, or [`TemplateError::InvalidDefinition`] errors
    /// for a formal target, duplicate filter, or self-dependent filter.
    ///
    /// - Complexity: O(i) time and space for i inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{TemplateBuilder, Filter};
    /// let mut b = TemplateBuilder::new();
    /// let x = b.cell("x", || Ok(0i32))?;
    /// b.filter(&x, &[], |_, _| Ok(Filter::from_fn_0(|x: &i32| Ok((*x).max(0)))))?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn filter<F>(
        &mut self,
        cell: &CellSlot,
        inputs: &[CellSlot],
        make: F,
    ) -> Result<(), TemplateError>
    where
        F: Fn(CellId, &[CellId]) -> anyhow::Result<Filter> + 'static,
    {
        self.check_private(cell)?;
        if self.filtered.contains(&cell.index) || inputs.contains(cell) {
            return Err(TemplateError::InvalidDefinition {
                detail: "duplicate or self-dependent filter",
                slot: Some(cell.clone()),
            });
        }
        let slot = cell.clone();
        self.add_policy(
            cell,
            inputs,
            Box::new(move |sheet, target, ids| {
                let filter = make(target, ids).map_err(|source| TemplateError::Factory {
                    source,
                    slot: slot.clone(),
                })?;
                if filter.0.args != ids
                    || filter.0.value_type != sheet.cell_type_id(target).unwrap()
                    || !super::instantiate::signature_matches(sheet, ids, &filter.0.arg_types)
                {
                    return Err(TemplateError::InvalidDefinition {
                        detail: "filter factory signature differs from its recipe",
                        slot: Some(slot.clone()),
                    });
                }
                sheet
                    .add_filter(target, filter)
                    .map_err(TemplateError::Runtime)
            }),
        )?;
        self.filtered.insert(cell.index);
        Ok(())
    }

    /// Attaches a freshly constructed requirement to a private cell.
    ///
    /// The factory must declare exactly the supplied positional inputs and types.
    /// Native attachment evaluates ordinary/source requirements immediately;
    /// output requirements wait until propagation.
    ///
    /// # Errors
    ///
    /// Returns ownership, capacity, or [`TemplateError::InvalidDefinition`] errors
    /// for a formal target. Factory and native errors occur during instantiation.
    ///
    /// - Complexity: O(i) time and space for i inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{TemplateBuilder, Requirement};
    /// let mut b = TemplateBuilder::new();
    /// let x = b.cell("x", || Ok(1i32))?;
    /// b.require(&x, &[x.clone()], |i| Ok(Requirement::from_fn_1(i[0], |x: &i32| Ok(*x > 0))))?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn require<F>(
        &mut self,
        cell: &CellSlot,
        inputs: &[CellSlot],
        make: F,
    ) -> Result<(), TemplateError>
    where
        F: Fn(&[CellId]) -> anyhow::Result<Requirement> + 'static,
    {
        let slot = cell.clone();
        self.add_policy(
            cell,
            inputs,
            Box::new(move |sheet, target, ids| {
                let requirement = make(ids).map_err(|source| TemplateError::Factory {
                    source,
                    slot: slot.clone(),
                })?;
                if requirement.inputs != ids
                    || !super::instantiate::signature_matches(sheet, ids, &requirement.input_types)
                {
                    return Err(TemplateError::InvalidDefinition {
                        detail: "requirement factory signature differs from its recipe",
                        slot: Some(slot.clone()),
                    });
                }
                sheet
                    .add_requirement(target, requirement)
                    .map(|_| ())
                    .map_err(TemplateError::Runtime)
            }),
        )
    }

    /// Appends a private policy after validating all local input owners.
    ///
    /// - Complexity: O(i) time and space for i inputs.
    fn add_policy(
        &mut self,
        cell: &CellSlot,
        inputs: &[CellSlot],
        make: Box<super::PolicyFactory>,
    ) -> Result<(), TemplateError> {
        self.check_private(cell)?;
        for input in inputs {
            self.check_slot(input)?;
        }
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe.items.push(Item::Policy(PolicyRecipe {
            cell: cell.clone(),
            inputs: inputs.to_vec(),
            make,
        }));
        self.recipe.relationship_only = false;
        Ok(())
    }

    /// Registers native conditional branches over direct relationships or included groups.
    ///
    /// Included groups must be transitively relationship-only before specialization.
    /// Each constituent remains a distinct native relationship; no extra activation
    /// layer is introduced. Keys follow native first-match semantics.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::ConditionalComponent`] for stateful groups, or
    /// [`TemplateError::InvalidDefinition`] for foreign, reused, or duplicate groups
    /// or empty branch keys. Input ownership and capacity errors are also reported.
    ///
    /// - Complexity: O(i + g + k) time and space for inputs i, groups g, and keys k.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::{TemplateBuilder, MatchExpr};
    /// let mut b = TemplateBuilder::new();
    /// let guard = b.parameter::<bool>("guard")?;
    /// b.conditional(&[guard], |i| Ok(MatchExpr::cell(i[0])), vec![(vec![true], vec![])], vec![])?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn conditional<T, F>(
        &mut self,
        inputs: &[CellSlot],
        make: F,
        branches: Vec<(Vec<T>, Vec<RelationshipSlot>)>,
        default: Vec<RelationshipSlot>,
    ) -> Result<(), TemplateError>
    where
        T: Clone + Any + PartialEq + 'static,
        F: Fn(&[CellId]) -> anyhow::Result<MatchExpr> + 'static,
    {
        let (keys, groups): (Vec<_>, Vec<_>) = branches.into_iter().unzip();
        if keys.iter().any(Vec::is_empty) {
            return Err(TemplateError::InvalidDefinition {
                detail: "conditional branch requires keys",
                slot: None,
            });
        }
        self.conditional_factory(inputs, make, move || Ok(keys.clone()), groups, default)
    }

    /// Registers conditional groups with fresh keys constructed once per instance.
    ///
    /// Neither factory executes during preparation. Group ownership, uniqueness and
    /// relationship-only eligibility are checked before mutation. Keys follow native
    /// first-match semantics and must have one nonempty vector per declared branch.
    ///
    /// # Errors
    /// Reports invalid inputs/groups before mutation; reports failed factories, invalid
    /// key counts, empty keys or native conditional errors during construction.
    /// - Complexity: O(i + b + g) preparation for inputs i, branches b and groups g;
    ///   construction adds factory costs and O(i + b + k + g + r) for keys k and
    ///   expanded relationship members r.
    ///
    /// # Examples
    /// ```
    /// let mut b = adam_rs::TemplateBuilder::new();
    /// let flag = b.parameter::<bool>("flag")?;
    /// b.conditional_factory::<bool, _, _>(&[flag],
    ///     |ids| Ok(adam_rs::MatchExpr::cell(ids[0])),
    ///     || Ok(vec![vec![true]]), vec![vec![]], vec![])?;
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn conditional_factory<T, F, K>(
        &mut self,
        inputs: &[CellSlot],
        make: F,
        keys: K,
        branches: Vec<Vec<RelationshipSlot>>,
        default: Vec<RelationshipSlot>,
    ) -> Result<(), TemplateError>
    where
        T: Any + PartialEq + 'static,
        F: Fn(&[CellId]) -> anyhow::Result<MatchExpr> + 'static,
        K: Fn() -> anyhow::Result<Vec<Vec<T>>> + 'static,
    {
        for input in inputs {
            self.check_slot(input)?;
        }
        let mut seen = HashSet::new();
        for group in branches.iter().flatten().chain(&default) {
            if !Rc::ptr_eq(&self.recipe.owner, &group.owner)
                || group.index >= self.recipe.relationships.len()
                || self.conditional_groups.contains(&group.index)
                || !seen.insert(group.index)
            {
                return Err(TemplateError::InvalidDefinition {
                    detail: "foreign or already conditional relationship group",
                    slot: None,
                });
            }
            let eligible = match &self.recipe.relationships[group.index] {
                RelationshipRecipe::Methods(_) => true,
                RelationshipRecipe::Out(_) => false,
                RelationshipRecipe::Include { child, .. } => child.relationship_only,
            };
            if !eligible {
                return Err(TemplateError::ConditionalComponent);
            }
        }
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe.items.push(Item::Conditional(ConditionalRecipe {
            inputs: inputs.to_vec(),
            make: Box::new(move |sheet, ids, groups, relationships| {
                let source =
                    make(ids).map_err(|source| TemplateError::ConditionalFactory { source })?;
                let valid = match &source.0 {
                    MatchSource::Cell(id) => {
                        ids == [*id] && sheet.cell_type_id(*id) == Some(TypeId::of::<T>())
                    }
                    MatchSource::Expr(expr) => {
                        expr.inputs == ids
                            && expr.output_type == TypeId::of::<T>()
                            && super::instantiate::signature_matches(sheet, ids, &expr.input_types)
                    }
                };
                if !valid {
                    return Err(TemplateError::InvalidDefinition {
                        detail: "conditional factory signature differs from its recipe",
                        slot: None,
                    });
                }
                let mut live_branches = Vec::new();
                live_branches
                    .try_reserve(branches.len())
                    .map_err(|_| TemplateError::Capacity)?;
                let keys = keys().map_err(|source| TemplateError::ConditionalFactory { source })?;
                if keys.len() != branches.len() || keys.iter().any(Vec::is_empty) {
                    return Err(TemplateError::InvalidDefinition {
                        detail: "conditional key factory differs from its branches",
                        slot: None,
                    });
                }
                for (live_keys, slots) in keys.into_iter().zip(&branches) {
                    live_branches.push((
                        live_keys,
                        super::instantiate::group_members(slots, groups, relationships)?,
                    ));
                }
                let live_default =
                    super::instantiate::group_members(&default, groups, relationships)?;
                sheet
                    .add_conditional(source, live_branches, live_default)
                    .map_err(TemplateError::Runtime)
            }),
        }));
        self.conditional_groups.extend(seen);
        self.recipe.relationship_only = false;
        Ok(())
    }

    /// Appends a compact child-DAG edge with checked named bindings.
    ///
    /// Binding and structural failures leave the builder unchanged. Immutable
    /// factories are shared, but every live instantiation creates fresh native methods.
    /// The returned single group slot represents all transitive relationships;
    /// only instantiation enumerates its distinct live members in inline order.
    ///
    /// # Errors
    ///
    /// Returns binding errors for unknown, missing, duplicate, aliased, foreign,
    /// or wrongly typed arguments; returns [`TemplateError::InvalidDefinition`]
    /// for a private out argument, or [`TemplateError::Capacity`] on reservation failure.
    ///
    /// - Complexity: Expected O(c + b) time and space for the child's own local
    ///   cells c and bindings b, independent of transitive expansion size.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::rc::Rc;
    /// # use adam_rs::TemplateBuilder;
    /// let child = Rc::new(TemplateBuilder::new().finish()?);
    /// let mut parent = TemplateBuilder::new();
    /// let included = parent.include(child, &[])?;
    /// assert_eq!(included.relationships().len(), 1);
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn include(
        &mut self,
        child: Rc<SheetTemplate>,
        bindings: &[(&str, CellSlot)],
    ) -> Result<TemplateInclusion, TemplateError> {
        let bound = child.bind(bindings, |_, slot| {
            self.check_slot(slot)?;
            Ok(self.recipe.cells[slot.index].type_id)
        })?;
        if let Some(slot) = bound
            .values()
            .find(|s| self.recipe.cells[s.index].kind == CellKind::Out)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "an included parameter cannot bind an out cell",
                slot: Some(slot.clone()),
            });
        }
        self.recipe
            .relationships
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        self.recipe
            .items
            .try_reserve(1)
            .map_err(|_| TemplateError::Capacity)?;
        let index = self.recipe.relationships.len();
        let handles = vec![RelationshipSlot {
            owner: self.recipe.owner.clone(),
            index,
        }];
        self.recipe.written_cells.extend(
            child
                .written_cells
                .iter()
                .filter_map(|i| bound.get(i))
                .map(|s| s.index),
        );
        self.recipe.relationship_only &= child.relationship_only;
        self.recipe.relationships.push(RelationshipRecipe::Include {
            child,
            bindings: bound,
        });
        self.recipe.items.push(Item::Relationship(index));
        Ok(TemplateInclusion {
            relationships: handles,
        })
    }

    /// Finalizes the owning immutable recipe without invoking any factory.
    ///
    /// # Errors
    ///
    /// Rejects included parameters that became private outputs after inclusion.
    /// Expanded-size overflow is retained for diagnosis before live expansion.
    ///
    /// - Complexity: O(n + e) time for local declarations n and edges e; no DAG expansion.
    ///
    /// # Examples
    ///
    /// See the [module example](super).
    pub fn finish(mut self) -> Result<SheetTemplate, TemplateError> {
        let mut size = self
            .recipe
            .cells
            .len()
            .checked_add(self.recipe.items.len())
            .and_then(|n| n.checked_add(self.recipe.relationships.len()))
            // Every occurrence needs a record, even when its recipe is empty.
            .and_then(|n| n.checked_add(1));
        let mut depth = Some(1usize);
        for relationship in &self.recipe.relationships {
            match relationship {
                RelationshipRecipe::Include { child, bindings } => {
                    if let Some(slot) = bindings
                        .values()
                        .find(|s| self.recipe.cells[s.index].kind == CellKind::Out)
                    {
                        return Err(TemplateError::InvalidDefinition {
                            detail: "an included parameter cannot bind an out cell",
                            slot: Some(slot.clone()),
                        });
                    }
                    size = size.and_then(|n| child.expanded_size.and_then(|c| n.checked_add(c)));
                    depth = depth
                        .and_then(|d| child.depth.and_then(|c| c.checked_add(1)).map(|c| d.max(c)));
                }
                RelationshipRecipe::Methods(methods) => {
                    for method in methods {
                        size = size
                            .and_then(|n| n.checked_add(method.inputs.len()))
                            .and_then(|n| n.checked_add(method.outputs.len()))
                            .and_then(|n| n.checked_add(1));
                    }
                }
                RelationshipRecipe::Out(method) => {
                    size = size
                        .and_then(|n| n.checked_add(method.inputs.len()))
                        .and_then(|n| n.checked_add(method.outputs.len()));
                }
            }
        }
        self.recipe.expanded_size = size;
        self.recipe.depth = depth;
        Ok(self.recipe)
    }
}

impl SheetTemplate {
    /// Builds a local-index binding map after checking names, aliases, and concrete types.
    ///
    /// - Complexity: Expected O(c + b) time and O(c + b) space for cells c and bindings b.
    pub(super) fn bind<A, F>(
        &self,
        bindings: &[(&str, A)],
        type_of: F,
    ) -> Result<HashMap<usize, A>, TemplateError>
    where
        A: Clone + Eq + std::hash::Hash,
        F: Fn(&str, &A) -> Result<TypeId, TemplateError>,
    {
        let parameters: HashMap<&str, usize> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.make.is_none())
            .map(|(i, c)| (c.name.as_str(), i))
            .collect();
        let mut result = HashMap::with_capacity(bindings.len());
        let mut actuals = HashMap::with_capacity(bindings.len());
        for (name, actual) in bindings {
            let &index = parameters
                .get(name)
                .ok_or_else(|| TemplateError::UnknownParameter {
                    name: (*name).into(),
                })?;
            if result.contains_key(&index) {
                return Err(TemplateError::DuplicateParameter {
                    name: (*name).into(),
                });
            }
            if let Some(first) = actuals.insert(actual.clone(), *name) {
                return Err(TemplateError::AliasedArgument {
                    first: first.into(),
                    second: (*name).into(),
                });
            }
            let found = type_of(name, actual)?;
            let expected = self.cells[index].type_id;
            if expected != found {
                return Err(TemplateError::TypeMismatch {
                    name: (*name).into(),
                    expected,
                    found,
                });
            }
            result.insert(index, actual.clone());
        }
        for (index, cell) in self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.make.is_none())
        {
            if !result.contains_key(&index) {
                return Err(TemplateError::MissingParameter {
                    name: cell.name.clone(),
                });
            }
        }
        Ok(result)
    }
}

impl std::hash::Hash for CellSlot {
    /// Hashes the immutable owner identity and local index consistently with equality.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&Rc::as_ptr(&self.owner), state);
        std::hash::Hash::hash(&self.index, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starts fresh namespaces and keeps the builder usable after a rejected declaration.
    #[test]
    fn defaults_and_rejected_declarations_preserve_builder_contract() {
        let mut builder = TemplateBuilder::default();
        let slot = builder.parameter::<i32>("input").unwrap();
        assert!(matches!(
            builder.parameter::<bool>("input"),
            Err(TemplateError::DuplicateParameter { .. })
        ));
        let foreign = TemplateBuilder::new().parameter::<i32>("input").unwrap();
        assert_ne!(slot, foreign);
        assert_eq!(HashSet::from([slot.clone(), slot, foreign]).len(), 2);
        builder.cell("private", || Ok(9_i32)).unwrap();
        let template = builder.finish().unwrap();
        assert_eq!(template.parameter_names().collect::<Vec<_>>(), ["input"]);
        assert!(!template.is_relationship_only());
        let mut sheet = Sheet::new();
        let actual = sheet.add_cell(3_i32);
        let instance = template
            .instantiate(&mut sheet, &[("input", actual)])
            .unwrap();
        assert_eq!(instance.cells().len(), 2);
        assert_eq!(*sheet.read::<i32>(instance.cells()[1]).unwrap(), 9);
        assert!(
            TemplateBuilder::default()
                .finish()
                .unwrap()
                .is_relationship_only()
        );
    }
}
