//! Flat, source-independent occurrence and authored-method mappings.
//!
//! Occurrences follow depth-first inclusion order. Their parent links retain
//! invocation paths without copying full paths or retaining recipe DAG edges.
//!
//! ```
//! use adam_rs::{Sheet, TemplateBuilder};
//! let template = TemplateBuilder::new().finish()?;
//! let instance = template.instantiate(&mut Sheet::new(), &[])?;
//! let root = &instance.occurrences()[0];
//! assert_eq!(root.template(), &template.id());
//! assert!(root.parent().is_none());
//! assert_eq!(instance.occurrence(root.id()).unwrap().id(), root.id());
//! # Ok::<(), adam_rs::TemplateError>(())
//! ```

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use super::{CellSlot, Instance, RelationshipSlot};
use crate::{CellId, RelationshipId};

/// Identifies an immutable recipe without retaining its factories or inclusion edges.
///
/// Clones share identity. Equality and hashing compare ownership, not structure.
///
/// # Examples
/// See the [occurrence example](InstanceOccurrence).
#[derive(Clone, Debug)]
pub struct TemplateId(pub(super) Rc<()>);

impl PartialEq for TemplateId {
    /// Compares immutable recipe ownership.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for TemplateId {}

impl Hash for TemplateId {
    /// Hashes the ownership identity used by equality.
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.0).hash(state);
    }
}

/// Identifies one inclusion occurrence within exactly one live expansion.
///
/// The retained token keeps identity distinct after the instance is dropped.
///
/// # Examples
/// See the [occurrence example](InstanceOccurrence).
#[derive(Clone, Debug)]
pub struct OccurrenceId {
    pub(super) owner: Rc<()>,
    pub(super) index: usize,
}

impl PartialEq for OccurrenceId {
    /// Compares expansion ownership and occurrence index.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.owner, &other.owner) && self.index == other.index
    }
}

impl Eq for OccurrenceId {}

impl Hash for OccurrenceId {
    /// Hashes the expansion owner and local index used by equality.
    fn hash<H: Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.owner).hash(state);
        self.index.hash(state);
    }
}

/// Retains one occurrence's identity, immediate parent, and local mapping ranges.
///
/// Parent/site pairs distinguish repeated uses of shared children. Root occurrences
/// have neither a parent nor an inclusion site. No record owns another record.
///
/// ```
/// use adam_rs::{Sheet, TemplateBuilder};
/// let template = TemplateBuilder::new().finish()?;
/// let instance = template.instantiate(&mut Sheet::new(), &[])?;
/// let root = &instance.occurrences()[0];
/// assert_eq!(root.template(), &template.id());
/// assert!(root.parent().is_none());
/// assert!(root.inclusion_site().is_none());
/// assert_eq!(instance.occurrence(root.id()).unwrap().id(), root.id());
/// # Ok::<(), adam_rs::TemplateError>(())
/// ```
#[derive(Debug)]
pub struct InstanceOccurrence {
    pub(super) id: OccurrenceId,
    pub(super) template: TemplateId,
    pub(super) parent: Option<OccurrenceId>,
    pub(super) inclusion_site: Option<RelationshipSlot>,
    pub(super) cells: Range<usize>,
    pub(super) groups: Range<usize>,
    pub(super) conditionals: Vec<crate::ConditionalId>,
}

impl InstanceOccurrence {
    /// Borrows live conditional identities in this definition's authored order.
    ///
    /// These identities remain usable after the template is dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// let template = adam_rs::TemplateBuilder::new().finish()?;
    /// let instance = template.instantiate(&mut adam_rs::Sheet::new(), &[])?;
    /// assert!(instance.occurrences()[0].conditionals().is_empty());
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn conditionals(&self) -> &[crate::ConditionalId] {
        &self.conditionals
    }

    /// Borrows this occurrence's expansion-scoped identity.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn id(&self) -> &OccurrenceId {
        &self.id
    }

    /// Borrows the identity of the expanded recipe.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn template(&self) -> &TemplateId {
        &self.template
    }

    /// Borrows the immediate parent identity, or returns `None` for the root.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn parent(&self) -> Option<&OccurrenceId> {
        self.parent.as_ref()
    }

    /// Borrows the inclusion group in the parent's namespace, or returns `None` for the root.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn inclusion_site(&self) -> Option<&RelationshipSlot> {
        self.inclusion_site.as_ref()
    }
}

/// Maps one compact live method index back to its authored declaration.
///
/// Private output writers have authored index zero. Source specialization leaves
/// gaps in authored indices without changing compact runtime indices.
///
/// # Examples
/// See [`Instance::method_origin`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodOrigin {
    /// Identifies the occurrence containing the authored relationship.
    pub occurrence: OccurrenceId,
    /// Identifies the relationship in that occurrence's local recipe.
    pub relationship: RelationshipSlot,
    /// Identifies the original method position before source specialization.
    pub authored_index: usize,
}

/// Maps one live relationship to its occurrence, local slot and surviving method indices.
#[derive(Debug)]
pub(super) struct RelationshipOrigin {
    pub(super) occurrence: usize,
    pub(super) slot: usize,
    pub(super) methods: Range<usize>,
}

impl Instance {
    /// Enumerates live methods and their authored origins in expansion order.
    ///
    /// Tuple fields are relationship identity, compact method index, and authored origin.
    /// This avoids repeated linear searches when retaining all provenance.
    /// - Complexity: O(r + m) time for r relationships and m surviving methods;
    ///   O(1) additional space, excluding collected results.
    ///
    /// # Examples
    ///
    /// ```
    /// let template = adam_rs::TemplateBuilder::new().finish()?;
    /// let instance = template.instantiate(&mut adam_rs::Sheet::new(), &[])?;
    /// assert_eq!(instance.method_origins().count(), 0);
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn method_origins(
        &self,
    ) -> impl Iterator<Item = (RelationshipId, usize, MethodOrigin)> + '_ {
        self.relationships
            .iter()
            .zip(&self.relationship_origins)
            .flat_map(move |(&relationship, origin)| {
                let record = &self.occurrences[origin.occurrence];
                self.authored_methods[origin.methods.clone()]
                    .iter()
                    .enumerate()
                    .map(move |(live, &authored_index)| {
                        (
                            relationship,
                            live,
                            MethodOrigin {
                                occurrence: record.id.clone(),
                                relationship: RelationshipSlot {
                                    owner: record.template.0.clone(),
                                    index: origin.slot,
                                },
                                authored_index,
                            },
                        )
                    })
            })
    }
}

impl Instance {
    /// Borrows occurrences in depth-first inclusion order, beginning with the root.
    ///
    /// Filtering by immediate parent yields children in authored inclusion order.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn occurrences(&self) -> &[InstanceOccurrence] {
        &self.occurrences
    }

    /// Returns an occurrence only if its identity belongs to this expansion.
    ///
    /// # Examples
    ///
    /// See the [occurrence example](InstanceOccurrence).
    pub fn occurrence(&self, id: &OccurrenceId) -> Option<&InstanceOccurrence> {
        if !Rc::ptr_eq(&self._identity, &id.owner) {
            return None;
        }
        self.occurrences.get(id.index)
    }

    /// Maps an owner-checked local cell to its live identity within an occurrence.
    ///
    /// Returns `None` for a foreign occurrence or recipe handle. Bound formals
    /// retain the caller's cell identity.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_rs::{TemplateBuilder, Sheet};
    /// let mut builder = TemplateBuilder::new();
    /// let slot = builder.cell("private", || Ok(1i32))?;
    /// let instance = builder.finish()?.instantiate(&mut Sheet::new(), &[])?;
    /// assert_eq!(instance.cell(instance.occurrences()[0].id(), &slot), Some(instance.cells()[0]));
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn cell(&self, occurrence: &OccurrenceId, slot: &CellSlot) -> Option<CellId> {
        let record = self.occurrence(occurrence)?;
        if !Rc::ptr_eq(&record.template.0, &slot.owner) {
            return None;
        }
        self.local_cells[record.cells.clone()]
            .get(slot.index)
            .copied()
    }

    /// Borrows a local relationship's live members, preserving separate runtime identities.
    ///
    /// Inclusion groups contain their full transitive expansion, possibly empty.
    /// Returns `None` for foreign occurrence or recipe handles.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_rs::{TemplateBuilder, Sheet};
    /// use std::rc::Rc;
    /// let child = Rc::new(TemplateBuilder::new().finish()?);
    /// let mut builder = TemplateBuilder::new();
    /// let included = builder.include(child, &[])?;
    /// let instance = builder.finish()?.instantiate(&mut Sheet::new(), &[])?;
    /// assert_eq!(instance.relationship_group(instance.occurrences()[0].id(),
    ///     &included.relationships()[0]), Some(&[][..]));
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn relationship_group(
        &self,
        occurrence: &OccurrenceId,
        slot: &RelationshipSlot,
    ) -> Option<&[RelationshipId]> {
        let record = self.occurrence(occurrence)?;
        if !Rc::ptr_eq(&record.template.0, &slot.owner) {
            return None;
        }
        let range = self.local_groups[record.groups.clone()].get(slot.index)?;
        Some(&self.relationships[range.clone()])
    }

    /// Returns the authored origin of a compact live method in this expansion.
    ///
    /// Returns `None` if the relationship is absent or the method index is out of range.
    /// - Complexity: O(r) time for r live relationships.
    ///
    /// # Examples
    ///
    /// ```
    /// use adam_rs::{TemplateBuilder, MethodRecipe, Method, Sheet};
    /// let mut builder = TemplateBuilder::new();
    /// let a = builder.cell("a", || Ok(1i32))?;
    /// let b = builder.cell("b", || Ok(0i32))?;
    /// builder.relationship(vec![MethodRecipe::new(&[a], &[b], |i, o| {
    ///     Ok(Method::from_fn_1_1(i[0], o[0], |x: &i32| Ok(*x)))
    /// })?])?;
    /// let instance = builder.finish()?.instantiate(&mut Sheet::new(), &[])?;
    /// let origin = instance.method_origin(instance.relationships()[0], 0).unwrap();
    /// assert_eq!(origin.authored_index, 0);
    /// # Ok::<(), adam_rs::TemplateError>(())
    /// ```
    pub fn method_origin(
        &self,
        relationship: RelationshipId,
        live_method: usize,
    ) -> Option<MethodOrigin> {
        let index = self
            .relationships
            .iter()
            .position(|id| *id == relationship)?;
        let origin = &self.relationship_origins[index];
        let authored_index = *self.authored_methods[origin.methods.clone()].get(live_method)?;
        let record = &self.occurrences[origin.occurrence];
        Some(MethodOrigin {
            occurrence: record.id.clone(),
            relationship: RelationshipSlot {
                owner: record.template.0.clone(),
                index: origin.slot,
            },
            authored_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Method, MethodRecipe, Sheet, TemplateBuilder};
    use std::collections::HashSet;

    /// Preserves clone/hash identity laws and rejects foreign owners and out-of-range mappings.
    #[test]
    fn identity_and_mapping_boundaries() {
        let mut builder = TemplateBuilder::new();
        let input = builder.cell("input", || Ok(3_i32)).unwrap();
        let output = builder.cell("output", || Ok(0_i32)).unwrap();
        let relationship = builder
            .relationship(vec![
                MethodRecipe::new(
                    std::slice::from_ref(&input),
                    std::slice::from_ref(&output),
                    |i, o| Ok(Method::from_fn_1_1(i[0], o[0], |v: &i32| Ok(*v))),
                )
                .unwrap(),
            ])
            .unwrap();
        let template = builder.finish().unwrap();
        let other = TemplateBuilder::new().finish().unwrap();
        assert_eq!(
            HashSet::from([template.id(), template.id().clone(), other.id()]).len(),
            2
        );
        let mut sheet = Sheet::new();
        let instance = template.instantiate(&mut sheet, &[]).unwrap();
        let foreign = other.instantiate(&mut sheet, &[]).unwrap();
        let root = &instance.occurrences()[0];
        assert_eq!(root.template(), &template.id());
        assert!(root.parent().is_none());
        assert!(root.inclusion_site().is_none());
        assert!(root.conditionals().is_empty());
        assert_eq!(
            HashSet::from([
                root.id().clone(),
                root.id().clone(),
                foreign.occurrences()[0].id().clone()
            ])
            .len(),
            2
        );
        assert!(instance.occurrence(foreign.occurrences()[0].id()).is_none());
        let invalid = OccurrenceId {
            owner: root.id.owner.clone(),
            index: 1,
        };
        assert!(instance.occurrence(&invalid).is_none());
        assert_eq!(instance.cell(root.id(), &input), Some(instance.cells()[0]));
        assert_eq!(instance.cell(root.id(), &output), Some(instance.cells()[1]));
        let invalid_slot = CellSlot {
            owner: template.owner.clone(),
            index: 2,
        };
        assert!(instance.cell(root.id(), &invalid_slot).is_none());
        let foreign_slot = CellSlot {
            owner: other.owner.clone(),
            index: 0,
        };
        assert!(instance.cell(root.id(), &foreign_slot).is_none());
        assert_eq!(
            instance.relationship_group(root.id(), &relationship),
            Some(instance.relationships())
        );
        let invalid_group = RelationshipSlot {
            owner: template.owner.clone(),
            index: 1,
        };
        assert!(
            instance
                .relationship_group(root.id(), &invalid_group)
                .is_none()
        );
        let foreign_group = RelationshipSlot {
            owner: other.owner.clone(),
            index: 0,
        };
        assert!(
            instance
                .relationship_group(root.id(), &foreign_group)
                .is_none()
        );
        for (rel, live, origin) in instance.method_origins() {
            assert_eq!(instance.method_origin(rel, live), Some(origin));
            assert!(instance.method_origin(rel, live + 1).is_none());
        }
        assert!(
            instance
                .method_origin(RelationshipId::default(), 0)
                .is_none()
        );
        assert!(foreign.cells().is_empty());
        assert!(foreign.relationships().is_empty());
        assert_eq!(foreign.method_origins().count(), 0);
    }
}
