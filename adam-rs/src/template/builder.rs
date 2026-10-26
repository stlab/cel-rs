//! Ordered recipe construction and local inclusion remapping.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::{
    CellFactory, CellRecipe, CellSlot, Item, MethodRecipe, RelationshipRecipe, RelationshipSlot,
    SheetTemplate, TemplateError, TemplateInclusion,
};
use crate::{CellKind, Sheet};

/// Owns declarations and validates all local references before accepting them.
///
/// Private included names do not enter this builder's declaration namespace.
/// See the [module example](super).
pub struct TemplateBuilder {
    recipe: SheetTemplate,
    names: HashSet<String>,
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
            },
            names: HashSet::new(),
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
    /// A source cannot be a declared method output.
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
    /// output sets. Private sources cannot be outputs.
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
            .find(|s| self.recipe.cells[s.index].kind == CellKind::Source)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "a private source cannot be a method output",
                slot: Some(slot.clone()),
            });
        }
        let cell_sets: Vec<HashSet<usize>> = methods
            .iter()
            .map(|m| m.inputs.iter().chain(&m.outputs).map(|s| s.index).collect())
            .collect();
        if cell_sets.iter().any(|set| set != &cell_sets[0]) {
            return Err(TemplateError::InvalidDefinition {
                detail: "relationship methods reference different cell sets",
                slot: Some(methods[0].outputs[0].clone()),
            });
        }
        let outputs: Vec<HashSet<usize>> = methods
            .iter()
            .map(|m| m.outputs.iter().map(|s| s.index).collect())
            .collect();
        if outputs.iter().enumerate().any(|(i, set)| {
            outputs[..i]
                .iter()
                .any(|other| set.is_subset(other) || other.is_subset(set))
        }) {
            return Err(TemplateError::InvalidDefinition {
                detail: "relationship output sets are equal or nested",
                slot: Some(methods[0].outputs[0].clone()),
            });
        }
        Ok(())
    }

    /// Rejects a local reference not owned by this recipe.
    fn check_slot(&self, slot: &CellSlot) -> Result<(), TemplateError> {
        if !Rc::ptr_eq(&self.recipe.owner, &slot.owner) || slot.index >= self.recipe.cells.len() {
            return Err(TemplateError::ForeignHandle { slot: slot.clone() });
        }
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
    /// for a source output, or [`TemplateError::Capacity`] on expansion overflow.
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
        if let Some(slot) = child
            .written_cells
            .iter()
            .filter_map(|i| bound.get(i))
            .find(|s| self.recipe.cells[s.index].kind == CellKind::Source)
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "an included method writes a private source",
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
    /// All current structural errors are reported by declaration mutators.
    /// The fallible boundary also supports later native recipe kinds.
    ///
    /// # Examples
    ///
    /// See the [module example](super).
    pub fn finish(self) -> Result<SheetTemplate, TemplateError> {
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
