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

mod builder;
mod error;
mod instantiate;

use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::{CellId, CellKind, Method, RelationshipId, Sheet};

pub use builder::TemplateBuilder;
pub use error::TemplateError;

/// Identifies a cell declaration within exactly one owning builder/template.
///
/// Clones retain identity, not ownership of a live cell.
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
/// Immutable captured configuration may be shared. See the [module example](self).
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
}

/// Owns one relationship or a compact child-DAG edge with local formal bindings.
enum RelationshipRecipe {
    Methods(Vec<MethodRecipe>),
    Include {
        child: Rc<SheetTemplate>,
        bindings: HashMap<usize, CellSlot>,
    },
}

/// Owns an immutable ordered graph recipe, independent of any live sheet.
///
/// Parameters alias bindings; private cells and methods are fresh per instance.
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
}

impl SheetTemplate {
    /// Returns whether expansion allocates no private cells.
    ///
    /// Parameters do not count as private cells. An empty recipe qualifies.
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
/// See the [module example](self).
#[derive(Debug)]
pub struct Instance {
    _identity: Rc<()>,
    cells: Vec<CellId>,
    relationships: Vec<RelationshipId>,
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
