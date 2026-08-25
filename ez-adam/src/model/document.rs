//! The single source-of-truth document a `ez-adam` editor session edits.

use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

use crate::model::cell::{Cell, CellId};
use crate::model::cell_node::{CellNode, CellNodeId};
use crate::model::conditional_group::{ConditionalGroup, ConditionalGroupId};
use crate::model::relationship_group::{RelationshipGroup, RelationshipGroupId};

/// The current on-disk format version for [`Document`]'s JSON
/// serialization.
///
/// Bump this and add a migration path in [`crate::persistence`] whenever
/// `Document`'s shape changes in a way that breaks deserializing older
/// files.
pub const CURRENT_FORMAT_VERSION: u32 = 1;

/// A complete `ez-adam` editor document: one `.adm2` `sheet`'s worth of
/// cells, canvas placements, relationship groups, and conditional groups.
///
/// The `*_order` fields record declaration order explicitly, since
/// `SlotMap` iteration order is unspecified but `.adm2` generation needs
/// deterministic output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// The on-disk format version for this document.
    pub format_version: u32,
    /// The name of the sheet represented by this document.
    pub sheet_name: String,
    /// All cells in this document, indexed by `CellId`.
    pub cells: SlotMap<CellId, Cell>,
    /// Declaration order for cells.
    pub cell_order: Vec<CellId>,
    /// All cell nodes (canvas placements) in this document, indexed by
    /// `CellNodeId`.
    pub cell_nodes: SlotMap<CellNodeId, CellNode>,
    /// All relationship groups in this document, indexed by
    /// `RelationshipGroupId`.
    pub relationship_groups: SlotMap<RelationshipGroupId, RelationshipGroup>,
    /// Declaration order for relationship groups.
    pub relationship_group_order: Vec<RelationshipGroupId>,
    /// All conditional groups in this document, indexed by
    /// `ConditionalGroupId`.
    pub conditional_groups: SlotMap<ConditionalGroupId, ConditionalGroup>,
    /// Declaration order for conditional groups.
    pub conditional_group_order: Vec<ConditionalGroupId>,
}

impl Document {
    /// Creates a new, empty document for a sheet named `sheet_name`.
    #[must_use]
    pub fn new(sheet_name: impl Into<String>) -> Self {
        Document {
            format_version: CURRENT_FORMAT_VERSION,
            sheet_name: sheet_name.into(),
            cells: SlotMap::with_key(),
            cell_order: Vec::new(),
            cell_nodes: SlotMap::with_key(),
            relationship_groups: SlotMap::with_key(),
            relationship_group_order: Vec::new(),
            conditional_groups: SlotMap::with_key(),
            conditional_group_order: Vec::new(),
        }
    }

    /// Iterates over `(CellId, &Cell)` in declaration order.
    ///
    /// - Complexity: O(n) in the number of cells.
    pub fn cells_in_order(&self) -> impl Iterator<Item = (CellId, &Cell)> {
        self.cell_order
            .iter()
            .map(move |id| (*id, &self.cells[*id]))
    }

    /// Iterates over `(RelationshipGroupId, &RelationshipGroup)` in
    /// declaration order.
    ///
    /// - Complexity: O(n) in the number of relationship groups.
    pub fn relationship_groups_in_order(
        &self,
    ) -> impl Iterator<Item = (RelationshipGroupId, &RelationshipGroup)> {
        self.relationship_group_order
            .iter()
            .map(move |id| (*id, &self.relationship_groups[*id]))
    }

    /// Iterates over `(ConditionalGroupId, &ConditionalGroup)` in
    /// declaration order.
    ///
    /// - Complexity: O(n) in the number of conditional groups.
    pub fn conditional_groups_in_order(
        &self,
    ) -> impl Iterator<Item = (ConditionalGroupId, &ConditionalGroup)> {
        self.conditional_group_order
            .iter()
            .map(move |id| (*id, &self.conditional_groups[*id]))
    }
}

/// Manual `PartialEq` implementation for `Document`.
///
/// This is implemented manually rather than derived because `SlotMap<K, V>`
/// does not derive `PartialEq`. The implementation compares the explicit
/// order vectors directly and compares `SlotMap` fields by iterating their
/// (key, value) pairs, which works correctly because `slotmap`'s serde impl
/// preserves key identity and iteration order through serialization.
///
/// If new fields are added to `Document`, this impl must be updated to
/// compare them.
impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.format_version == other.format_version
            && self.sheet_name == other.sheet_name
            && self.cell_order == other.cell_order
            && self.relationship_group_order == other.relationship_group_order
            && self.conditional_group_order == other.conditional_group_order
            && self.cells.iter().eq(other.cells.iter())
            && self.cell_nodes.iter().eq(other.cell_nodes.iter())
            && self
                .relationship_groups
                .iter()
                .eq(other.relationship_groups.iter())
            && self
                .conditional_groups
                .iter()
                .eq(other.conditional_groups.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::{Cell, CellType};

    #[test]
    fn new_is_empty() {
        let doc = Document::new("demo");
        assert_eq!(doc.sheet_name, "demo");
        assert_eq!(doc.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(doc.cells_in_order().count(), 0);
        assert_eq!(doc.relationship_groups_in_order().count(), 0);
        assert_eq!(doc.conditional_groups_in_order().count(), 0);
    }

    #[test]
    fn identical_documents_are_equal() {
        let doc1 = Document::new("sheet1");
        let doc2 = Document::new("sheet1");
        assert_eq!(doc1, doc2);
    }

    #[test]
    fn different_sheet_names_compare_unequal() {
        let doc1 = Document::new("sheet1");
        let doc2 = Document::new("sheet2");
        assert_ne!(doc1, doc2);
    }

    #[test]
    fn different_format_versions_compare_unequal() {
        let doc1 = Document::new("sheet1");
        let mut doc2 = Document::new("sheet1");
        doc2.format_version = 2;
        assert_ne!(doc1, doc2);
    }

    #[test]
    fn documents_with_different_cells_are_not_equal() {
        let mut a = Document::new("demo");
        let b = Document::new("demo");

        let cell = Cell::new("test_cell", CellType::f64());
        let cell_id = a.cells.insert(cell);
        a.cell_order.push(cell_id);

        assert_ne!(a, b);
    }
}
