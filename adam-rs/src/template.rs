//! Owning, source-free recipes for composition in one native [`Sheet`].
//!
//! Parameters alias caller-owned cells; private declarations allocate fresh cells
//! in declaration order. Factories run at instantiation, never propagation.
//! Each factory must create fresh execution state and satisfy [`Method`]'s contract.
//!
//! ```
//! use adam_rs::{Method, MethodRecipe, Sheet, TemplateBuilder};
//!
//! let mut builder = TemplateBuilder::new();
//! let input = builder.parameter::<i32>("input")?;
//! let output = builder.parameter::<i32>("output")?;
//! let method = MethodRecipe::new(&[input], &[output], |ins, outs| {
//!     Ok(Method::from_fn_1_1(ins[0], outs[0], |x: &i32| Ok(*x)))
//! })?;
//! builder.relationship(vec![method])?;
//! let template = builder.finish()?;
//! let mut sheet = Sheet::new();
//! let input = sheet.add_source(7i32);
//! let output = sheet.add_cell(0i32);
//! let instance = template.instantiate(&mut sheet, &[("output", output), ("input", input)])?;
//! sheet.propagate()?;
//! assert_eq!(*sheet.read::<i32>(output)?, 7);
//! assert_eq!(instance.cells(), &[input, output]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! This path constructs native methods directly: it needs no CEL text, lexer,
//! parser or operation lookup. For checked multiplication with a source-bound
//! factor, run `cargo run -p adam-lang --example templates` in the workspace.
//! That example's native path uses only this crate's recipe and sheet APIs.
//!
//! Named actuals must cover each parameter once, use distinct live cells from
//! the destination sheet, and match the declared concrete types. Fixed sources
//! specialize away every method that would write them. An empty specialized
//! relationship fails before factories run. `out` cells cannot be actuals.
//! Parameters keep caller-owned filters, requirements, strength and aspirations;
//! component-owned policies may govern only private cells.
//!
//! Finished templates own their factories; each factory creates fresh execution
//! state. Immutable captures may be shared. Live methods survive dropping both
//! the template and its returned instance metadata. Instantiation runs factories
//! but never propagates. On error, the destination remains safely discardable;
//! instantiation does not promise rollback. [`TemplateError::Construction`]
//! retains the actual native site and original cause without invented source spans.
//! Conditional inclusion requires a transitively relationship-only component.
//! Private state remains available to unconditional inclusions.

mod builder;
mod error;
mod instantiate;
mod provenance;

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::{CellId, CellKind, Method, RelationshipId, Sheet};

pub use builder::TemplateBuilder;
pub use error::{ConstructionSite, TemplateError};
pub use provenance::{InstanceOccurrence, MethodOrigin, OccurrenceId, TemplateId};

/// Identifies a cell declaration within exactly one owning builder/template.
///
/// Clones retain identity, not ownership of a live cell.
///
/// # Examples
/// See the [module example](self).
#[derive(Clone, Debug)]
pub struct CellSlot {
    owner: Rc<()>,
    index: usize,
}

impl PartialEq for CellSlot {
    /// Compares the owning recipe and local declaration index.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.owner, &other.owner) && self.index == other.index
    }
}

impl Eq for CellSlot {}

/// Identifies a relationship declaration or inclusion group within one owning recipe.
///
/// A group expands to zero or more distinct live relationships, never a merged
/// runtime relationship. Inclusion returns one group in the including namespace.
///
/// # Examples
/// See [`TemplateBuilder::include`].
#[derive(Clone, Debug)]
pub struct RelationshipSlot {
    owner: Rc<()>,
    index: usize,
}

impl PartialEq for RelationshipSlot {
    /// Compares the owning recipe and local relationship index.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.owner, &other.owner) && self.index == other.index
    }
}

impl Eq for RelationshipSlot {}

/// Creates a fresh native method for the supplied positional edge handles.
type MethodFactory = dyn Fn(&[CellId], &[CellId]) -> anyhow::Result<Method>;

/// Stores declared local edges and a repeatable, owning native method factory.
///
/// Factories must create fresh execution state satisfying [`Method`]'s contract.
/// Their returned method's positional edges and types must match the declarations.
/// Immutable captured configuration may be shared.
///
/// # Examples
/// See the [module example](self).
pub struct MethodRecipe {
    inputs: Vec<CellSlot>,
    outputs: Vec<CellSlot>,
    make: Box<MethodFactory>,
}

impl MethodRecipe {
    /// Creates a recipe whose local edges all belong to one owner.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::ForeignHandle`] for mixed owners, or
    /// [`TemplateError::InvalidDefinition`] for empty or repeated outputs.
    ///
    /// - Complexity: O(i + o²) time and O(i + o) space for edge counts i and o.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn new<F>(inputs: &[CellSlot], outputs: &[CellSlot], make: F) -> Result<Self, TemplateError>
    where
        F: Fn(&[CellId], &[CellId]) -> anyhow::Result<Method> + 'static,
    {
        let Some(first) = outputs.first() else {
            return Err(TemplateError::InvalidDefinition {
                detail: "a method requires an output",
                slot: None,
            });
        };
        for slot in inputs.iter().chain(outputs) {
            if !Rc::ptr_eq(&first.owner, &slot.owner) {
                return Err(TemplateError::ForeignHandle { slot: slot.clone() });
            }
        }
        if outputs
            .iter()
            .enumerate()
            .any(|(i, slot)| outputs[..i].contains(slot))
        {
            return Err(TemplateError::InvalidDefinition {
                detail: "a method repeats an output",
                slot: Some(first.clone()),
            });
        }
        Ok(Self {
            inputs: inputs.to_vec(),
            outputs: outputs.to_vec(),
            make: Box::new(make),
        })
    }
}

/// Adds a freshly initialized private cell through the ordinary sheet mutators.
type CellFactory = dyn Fn(&mut Sheet) -> anyhow::Result<CellId>;

/// Describes one typed local cell; parameters have no allocating factory.
struct CellRecipe {
    name: String,
    type_id: TypeId,
    kind: CellKind,
    make: Option<Box<CellFactory>>,
}

/// Preserves inline declaration order independently of local handle numbering.
enum Item {
    Cell(usize),
    Relationship(usize),
    Policy(PolicyRecipe),
    Conditional(ConditionalRecipe),
}

/// Registers one checked private-cell policy with fresh execution state.
type PolicyFactory = dyn Fn(&mut Sheet, CellId, &[CellId]) -> Result<(), TemplateError>;

/// Retains the private target and declared inputs of an ordered filter or requirement.
struct PolicyRecipe {
    cell: CellSlot,
    inputs: Vec<CellSlot>,
    make: Box<PolicyFactory>,
}

/// Registers a conditional using the expanded live members of its local groups.
type ConditionalFactory = dyn Fn(
    &mut Sheet,
    &[CellId],
    &[std::ops::Range<usize>],
    &[RelationshipId],
) -> Result<crate::ConditionalId, TemplateError>;

/// Retains declared match inputs and a type-erased native conditional constructor.
struct ConditionalRecipe {
    inputs: Vec<CellSlot>,
    make: Box<ConditionalFactory>,
}

/// Owns one relationship or a compact child-DAG edge with local formal bindings.
enum RelationshipRecipe {
    Methods(Vec<MethodRecipe>),
    Out(MethodRecipe),
    Include {
        child: Rc<SheetTemplate>,
        bindings: HashMap<usize, CellSlot>,
    },
}

/// Owns an immutable ordered graph recipe, independent of any live sheet.
///
/// Parameters alias bindings; private cells and methods are fresh per instance.
/// Destruction releases last-owned inclusion edges iteratively; shared children
/// and their factories remain alive until their last strong owner releases them.
///
/// # Examples
/// See the [module example](self).
pub struct SheetTemplate {
    owner: Rc<()>,
    cells: Vec<CellRecipe>,
    relationships: Vec<RelationshipRecipe>,
    items: Vec<Item>,
    /// Summarizes local outputs without enumerating transitive descendants.
    written_cells: HashSet<usize>,
    /// Caches the transitive relationship-only classification.
    relationship_only: bool,
    /// Memoizes checked expanded storage/work, with `None` denoting overflow.
    expanded_size: Option<usize>,
    /// Bounds the explicit traversal stack without traversing descendants.
    depth: Option<usize>,
}

impl Drop for SheetTemplate {
    /// Releases uniquely owned inclusion descendants without recursive destruction.
    ///
    /// Shared children retain their recipes unchanged. Detaching a uniquely owned
    /// child's edges before dropping it bounds destructor call depth independently
    /// of inclusion nesting.
    ///
    /// - Complexity: O(n) time for released recipe storage n, excluding captured-value
    ///   destructors; O(w) additional space for the maximum pending relationship edges w.
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.relationships);
        while let Some(relationship) = pending.pop() {
            if let RelationshipRecipe::Include { child, .. } = relationship
                && let Ok(mut child) = Rc::try_unwrap(child)
            {
                pending.append(&mut child.relationships);
            }
        }
    }
}

impl SheetTemplate {
    /// Borrows formal names in declaration order, excluding private cells.
    ///
    /// - Complexity: O(c) traversal for c local declarations.
    ///
    /// # Examples
    /// ```
    /// let mut builder = adam_rs::TemplateBuilder::new();
    /// builder.parameter::<i32>("value")?;
    /// assert_eq!(builder.finish()?.parameter_names().collect::<Vec<_>>(), ["value"]);
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.cells
            .iter()
            .filter(|cell| cell.make.is_none())
            .map(|cell| cell.name.as_str())
    }

    /// Returns this recipe's immutable identity without retaining its factories or children.
    ///
    /// # Examples
    ///
    /// ```
    /// let template = adam_rs::TemplateBuilder::new().finish()?;
    /// assert_eq!(template.id(), template.id());
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn id(&self) -> TemplateId {
        TemplateId(self.owner.clone())
    }
}

/// Checks ordinary relationship edge structure without constructing methods or a live sheet.
///
/// Every method must have distinct, nonempty outputs and reference the same union of
/// input/output cells. Output sets of different methods cannot be equal or nested.
/// Inputs may overlap outputs. This check neither specializes sources nor plans a graph.
///
/// # Errors
/// Returns the violated structural rule.
/// - Complexity: O(m²e) time and O(me) space for m methods of at most e edges.
///
/// # Examples
/// ```
/// use adam_rs::template::check_relationship_edges;
/// assert!(check_relationship_edges(&[(&[0usize][..], &[1usize][..]),
///     (&[1usize][..], &[0usize][..])]).is_ok());
/// ```
pub fn check_relationship_edges<C: Eq + std::hash::Hash>(
    methods: &[(&[C], &[C])],
) -> Result<(), &'static str> {
    if methods.is_empty() {
        return Err("empty relationship");
    }
    let mut cells = Vec::with_capacity(methods.len());
    let mut outputs = Vec::with_capacity(methods.len());
    for (input, output) in methods {
        let output_set: HashSet<_> = output.iter().collect();
        if output.is_empty() || output_set.len() != output.len() {
            return Err("a method requires distinct nonempty outputs");
        }
        let cell_set: HashSet<_> = input.iter().chain(output.iter()).collect();
        if cells.first().is_some_and(|first| first != &cell_set) {
            return Err("relationship methods reference different cell sets");
        }
        if outputs.iter().any(|previous: &HashSet<&C>| {
            previous.is_subset(&output_set) || output_set.is_subset(previous)
        }) {
            return Err("relationship output sets are equal or nested");
        }
        cells.push(cell_set);
        outputs.push(output_set);
    }
    Ok(())
}

impl SheetTemplate {
    /// Returns whether expansion contains only relationships and parameter declarations.
    ///
    /// Private cells, policies, outputs and conditionals disqualify a recipe transitively.
    /// An empty recipe qualifies, independently of source specialization.
    ///
    /// # Examples
    ///
    /// ```
    /// # use adam_rs::TemplateBuilder;
    /// assert!(TemplateBuilder::new().finish()?.is_relationship_only());
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn is_relationship_only(&self) -> bool {
        self.relationship_only
    }
}

/// Returns one owner-checked relationship-group handle in the including namespace.
///
/// The group represents the entire compact include, including transitive children.
/// It contains no live handles and does not enumerate its members before expansion.
///
/// # Examples
/// See [`TemplateBuilder::include`].
pub struct TemplateInclusion {
    relationships: Vec<RelationshipSlot>,
}

impl TemplateInclusion {
    /// Borrows the single inclusion-group handle without enumerating descendants.
    ///
    /// # Examples
    ///
    /// See [`TemplateBuilder::include`].
    pub fn relationships(&self) -> &[RelationshipSlot] {
        &self.relationships
    }
}

/// Owns an opaque instance identity and its mapped live handles.
///
/// Dropping this object does not remove graph structure from the destination.
/// Cells begin with root parameters/private cells in local declaration order, then
/// included private cells in inline expansion order. Bound child formals introduce
/// no duplicate cell entries. Relationships follow inline expansion order and
/// remain distinct even when their recipe belongs to a group.
/// Occurrence records retain local mappings and immutable identity tokens, not
/// template factories or recursively owned inclusion edges.
///
/// # Examples
/// See the [module example](self).
#[derive(Debug)]
pub struct Instance {
    _identity: Rc<()>,
    cells: Vec<CellId>,
    relationships: Vec<RelationshipId>,
    occurrences: Vec<InstanceOccurrence>,
    local_cells: Vec<CellId>,
    local_groups: Vec<std::ops::Range<usize>>,
    relationship_origins: Vec<provenance::RelationshipOrigin>,
    authored_methods: Vec<usize>,
}

impl Instance {
    /// Borrows live relationship handles in inline declaration order.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn relationships(&self) -> &[RelationshipId] {
        &self.relationships
    }

    /// Borrows all mapped cells, including unchanged parameter identities.
    ///
    /// # Examples
    ///
    /// See the [module example](self).
    pub fn cells(&self) -> &[CellId] {
        &self.cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Preserves slot identity through cloning and distinguishes owners and indices.
    #[test]
    fn local_handle_identity_laws() {
        let mut a = TemplateBuilder::new();
        let x = a.parameter::<i32>("x").unwrap();
        let y = a.parameter::<i32>("y").unwrap();
        let foreign = TemplateBuilder::new().parameter::<i32>("x").unwrap();
        assert_eq!(x, x.clone());
        assert_eq!(x.clone(), x);
        assert_ne!(x, y);
        assert_ne!(x, foreign);
        let group = RelationshipSlot {
            owner: x.owner.clone(),
            index: 0,
        };
        assert_eq!(group, group.clone());
        assert_ne!(
            group,
            RelationshipSlot {
                owner: y.owner.clone(),
                index: 1
            }
        );
        assert_ne!(
            group,
            RelationshipSlot {
                owner: foreign.owner.clone(),
                index: 0
            }
        );
    }

    /// Rejects each malformed relationship boundary and accepts overlapping non-nested outputs.
    #[test]
    fn relationship_edges_obey_structural_contract() {
        assert!(check_relationship_edges::<usize>(&[]).is_err());
        assert!(check_relationship_edges(&[(&[0][..], &[][..])]).is_err());
        assert!(check_relationship_edges(&[(&[0][..], &[1, 1][..])]).is_err());
        assert!(check_relationship_edges(&[(&[0][..], &[1][..]), (&[1][..], &[2][..])]).is_err());
        assert!(check_relationship_edges(&[(&[0][..], &[1][..]), (&[0][..], &[1][..])]).is_err());
        assert!(
            check_relationship_edges(&[(&[0, 2][..], &[1][..]), (&[0][..], &[1, 2][..])]).is_err()
        );
        assert!(
            check_relationship_edges(&[(&[2][..], &[0, 1][..]), (&[0][..], &[1, 2][..])]).is_ok()
        );
    }

    /// Retains empty-template summaries and the inclusion's single borrowed group.
    #[test]
    fn empty_template_and_inclusion_accessors() {
        let child = Rc::new(TemplateBuilder::new().finish().unwrap());
        assert_eq!(child.parameter_names().count(), 0);
        assert!(child.is_relationship_only());
        let mut builder = TemplateBuilder::new();
        let inclusion = builder.include(child, &[]).unwrap();
        assert_eq!(inclusion.relationships().len(), 1);
        let instance = builder
            .finish()
            .unwrap()
            .instantiate(&mut Sheet::new(), &[])
            .unwrap();
        assert!(instance.cells().is_empty());
        assert!(instance.relationships().is_empty());
        assert_eq!(
            instance.relationship_group(
                instance.occurrences()[0].id(),
                &inclusion.relationships()[0]
            ),
            Some(&[][..])
        );
    }
}
