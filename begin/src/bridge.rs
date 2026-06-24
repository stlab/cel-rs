//! Serialization bridge from [`property_model::Sheet`] to D3-ready JSON.
// Suppressed until bridge types are wired into App (Task 7).
#![allow(dead_code)]
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] and [`RelationshipId`] keys. [`to_graph_data`] serializes a
//! [`Sheet`] and its [`Labels`] into a [`GraphData`] value ready for JSON encoding.

use std::collections::HashMap;

use indexmap::IndexMap;
use property_model::{CellId, Error, RelationshipId, Sheet};
use serde::Serialize;
use slotmap::Key;

/// Type-erased write closure: parses a string and writes it to a cell.
pub type WriteStrFn = Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>;

/// Display and write metadata for a single cell.
pub struct CellMeta {
    /// Human-readable cell name shown in the graph and inspector.
    pub label: String,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type mismatch.
    pub write_str: WriteStrFn,
}

/// Associates human-readable labels and type-erased closures with stable sheet IDs.
pub struct Labels {
    /// Cells in insertion order (preserves sidebar ordering).
    pub cells: IndexMap<CellId, CellMeta>,
    /// Relationship labels (reserved for future tooltip display).
    pub relationships: HashMap<RelationshipId, String>,
}

impl Labels {
    /// Creates an empty label set.
    pub fn new() -> Self {
        Self {
            cells: IndexMap::new(),
            relationships: HashMap::new(),
        }
    }

    /// Registers display metadata for a cell of type `T`.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with.
    /// - Precondition: `T` matches the type registered with `Sheet::add_cell` for `id`.
    pub fn add_cell<T>(&mut self, id: CellId, label: &str)
    where
        T: std::any::Any + std::fmt::Display + std::str::FromStr + 'static,
        T::Err: std::fmt::Display,
    {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                display: Box::new(move |sheet| {
                    sheet
                        .read::<T>(id)
                        .map(|v| format!("{}", v))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(move |sheet, s| {
                    let value = s
                        .parse::<T>()
                        .map_err(|e| Error::MethodFailed(anyhow::anyhow!("parse error: {}", e)))?;
                    sheet.write(id, value)
                }),
            },
        );
    }

    /// Registers a label for a relationship.
    ///
    /// - Precondition: `id` is a live relationship in the sheet this `Labels` will be used with.
    pub fn add_relationship(&mut self, id: RelationshipId, label: &str) {
        self.relationships.insert(id, label.to_owned());
    }
}

impl Default for Labels {
    /// Returns `Labels::new()`.
    fn default() -> Self {
        Self::new()
    }
}

/// Node kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A value cell — rendered as a `<rect>`.
    Cell,
    /// A multi-way constraint — rendered as a `<circle>`.
    Relationship,
}

/// A single node in the D3 graph.
#[derive(Serialize, Clone, PartialEq)]
pub struct NodeData {
    /// Stable string ID: `"c{ffi}"` for cells, `"r{ffi}"` for relationships.
    pub id: String,
    pub kind: NodeKind,
    /// Cell label; empty string for relationships.
    pub label: String,
}

/// A single edge in the D3 graph (undirected; connects a cell to a relationship).
#[derive(Serialize, Clone, PartialEq)]
pub struct LinkData {
    pub source: String,
    pub target: String,
}

/// Placeholder for future conditional relationship groups.
#[derive(Serialize, Clone, PartialEq)]
pub struct GroupData {
    pub id: String,
    pub member_ids: Vec<String>,
    pub condition_id: String,
}

/// Complete graph snapshot ready for JSON serialization and delivery to D3.
#[derive(Serialize, Clone, PartialEq)]
pub struct GraphData {
    pub nodes: Vec<NodeData>,
    pub links: Vec<LinkData>,
    /// Stable IDs of cells that changed during the last `propagate()` call.
    pub changed: Vec<String>,
    /// Always empty; reserved for future `when`/`otherwise` conditional relationships.
    pub groups: Vec<GroupData>,
}

fn cell_node_id(id: CellId) -> String {
    format!("c{}", id.data().as_ffi())
}

fn rel_node_id(id: RelationshipId) -> String {
    format!("r{}", id.data().as_ffi())
}

/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// - Complexity: O(c + r + e) where c is the number of cells, r the number of
///   relationships, and e the number of cell–relationship adjacency pairs.
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
    let mut nodes = Vec::new();
    let mut links = Vec::new();

    for id in sheet.cells() {
        let label = labels
            .cells
            .get(&id)
            .map(|m| m.label.clone())
            .unwrap_or_default();
        nodes.push(NodeData {
            id: cell_node_id(id),
            kind: NodeKind::Cell,
            label,
        });
    }

    for id in sheet.relationships() {
        nodes.push(NodeData {
            id: rel_node_id(id),
            kind: NodeKind::Relationship,
            label: String::new(),
        });
        if let Some(adj) = sheet.relationship_adj(id) {
            for &cell_id in adj {
                links.push(LinkData {
                    source: cell_node_id(cell_id),
                    target: rel_node_id(id),
                });
            }
        }
    }

    let changed = sheet.changed().map(cell_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        groups: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use property_model::{Method, Sheet};

    fn demo_sheet() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(3.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");

        let rel = sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();
        labels.add_relationship(rel, "×");

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_produces_correct_node_counts() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert_eq!(
            data.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Cell)
                .count(),
            3
        );
        assert_eq!(
            data.nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Relationship)
                .count(),
            1
        );
    }

    #[test]
    fn to_graph_data_produces_correct_link_count() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert_eq!(data.links.len(), 3);
    }

    #[test]
    fn to_graph_data_cell_nodes_have_labels() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        let cell_labels: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Cell)
            .map(|n| n.label.as_str())
            .collect();
        assert!(cell_labels.contains(&"a"));
        assert!(cell_labels.contains(&"b"));
        assert!(cell_labels.contains(&"c"));
    }

    #[test]
    fn to_graph_data_relationship_nodes_have_empty_labels() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        for node in data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
        {
            assert!(node.label.is_empty());
        }
    }

    #[test]
    fn to_graph_data_changed_contains_changed_cell_ids() {
        let (mut sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();
        sheet.write(a_id, 2.0_f64).unwrap();
        sheet.write(b_id, 3.0_f64).unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.changed.is_empty());
    }

    #[test]
    fn to_graph_data_groups_is_empty() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert!(data.groups.is_empty());
    }

    #[test]
    fn display_closure_returns_value_string() {
        let (sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "2");
    }

    #[test]
    fn write_str_closure_parses_and_writes() {
        let (mut sheet, labels) = demo_sheet();
        let a_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("a"))
            .unwrap();
        assert!((&labels.cells[&a_id].write_str)(&mut sheet, "5.0").is_ok());
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "5");
    }
}
