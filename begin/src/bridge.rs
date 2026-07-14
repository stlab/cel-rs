//! Serialization bridge from [`property_model::Sheet`] to D3-ready JSON.
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] and [`RelationshipId`] keys. [`to_graph_data`] serializes a
//! [`Sheet`] and its [`Labels`] into a [`GraphData`] value ready for JSON encoding.

use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;
use indexmap::IndexMap;
use property_model::{CellId, ConditionalId, Error, RelationshipId, Sheet};
use serde::Serialize;
use slotmap::Key;
use std::any::TypeId;

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
}

impl Labels {
    /// Creates an empty label set.
    pub fn new() -> Self {
        Self {
            cells: IndexMap::new(),
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
}

impl Default for Labels {
    /// Returns `Labels::new()`.
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a [`Labels`] from a pm-lang-style declaration-ordered cell name map.
///
/// Matches each `TypeId` against the built-in primitive types
/// `pm_lang::TypeRegistry::new()` registers. Cells whose `TypeId` is not one
/// of these are silently skipped — they simply won't appear in the sidebar.
///
/// - Complexity: O(n) in the number of cells.
pub fn labels_from_cell_names(cell_names: &IndexMap<String, (CellId, TypeId)>) -> Labels {
    let mut labels = Labels::new();
    for (name, &(id, type_id)) in cell_names {
        macro_rules! try_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    continue;
                }
            };
        }
        try_ty!(i8);
        try_ty!(i16);
        try_ty!(i32);
        try_ty!(i64);
        try_ty!(i128);
        try_ty!(isize);
        try_ty!(u8);
        try_ty!(u16);
        try_ty!(u32);
        try_ty!(u64);
        try_ty!(u128);
        try_ty!(usize);
        try_ty!(f32);
        try_ty!(f64);
        try_ty!(bool);
        try_ty!(String);
    }
    labels
}

/// Display name for the pm-lang source file, shown in diagnostic headers
/// (e.g. `--> begin/assets/demo.pm:8:11`).
pub const SOURCE_FILE_NAME: &str = "begin/assets/demo.pm";

/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`, ANSI-colored for a
/// terminal. All other variants have no source span and fall back to their
/// `Display` message.
pub fn format_property_model_error(e: &Error, source: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, SOURCE_FILE_NAME, 1, &Renderer::styled())
        }
        other => other.to_string(),
    }
}

/// Node kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// A value cell — rendered as a `<rect>`.
    Cell,
    /// A multi-way constraint — rendered as a `<circle>`.
    Relationship,
    /// A conditional switch — rendered as a diamond (rotated `<rect>`).
    Conditional,
}

/// A single node in the D3 graph.
#[derive(Serialize, Clone, PartialEq)]
pub struct NodeData {
    /// Stable string ID: `"c{ffi}"` for cells, `"r{ffi}"` for relationships, `"cond{ffi}"` for conditionals.
    pub id: String,
    /// The kind of node, determining its visual rendering.
    pub kind: NodeKind,
    /// Cell label (e.g. `"a"`); empty string for relationships and conditionals.
    pub label: String,
    /// Current cell value as a display string; empty string for relationships and conditionals.
    pub value: String,
}

/// Link kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// A regular constraint edge (cell ↔ relationship, or match cell → conditional node).
    Constraint,
    /// A control edge from a conditional node to a branch relationship.
    Control,
}

/// A single edge in the D3 graph.
///
/// When [`GraphData::arrows`] is `false` constraint edges are undirected; when `true`
/// they are directed from `source` to `target`. Control edges are always directed
/// (conditional node → relationship) and styled by `branch_index` and `branch_active`.
#[derive(Serialize, Clone, PartialEq)]
pub struct LinkData {
    /// Stable string ID of the source node.
    pub source: String,
    /// Stable string ID of the target node.
    pub target: String,
    /// The kind of link, determining its visual rendering.
    pub kind: LinkKind,
    /// Branch index for `Control` links; `None` for `Constraint` links and default-branch control links.
    pub branch_index: Option<usize>,
    /// `true` if this branch is currently active; `None` for `Constraint` links.
    pub branch_active: Option<bool>,
}

/// Complete graph snapshot ready for JSON serialization and delivery to D3.
#[derive(Serialize, Clone, PartialEq)]
pub struct GraphData {
    /// All nodes in the graph snapshot.
    pub nodes: Vec<NodeData>,
    /// All links (constraint and control) in the graph snapshot.
    pub links: Vec<LinkData>,
    /// Stable IDs of cells that changed during the last `propagate()` call.
    pub changed: Vec<String>,
    /// Stable IDs of cells forced by an active relationship (see
    /// [`property_model::Sheet::is_forced`]); consumers should disable input for these
    /// cells and may render them distinctly.
    pub forced: Vec<String>,
    /// Stable IDs of relationships forced by the planner (see
    /// [`property_model::Sheet::is_relationship_forced`]); consumers may render them
    /// distinctly, along with their constraint edges.
    pub forced_relationships: Vec<String>,
    /// `true` when at least one relationship has a cached plan and constraint links are directed
    /// where plans exist; `false` when no plan has been computed.
    pub arrows: bool,
}

fn cell_node_id(id: CellId) -> String {
    format!("c{}", id.data().as_ffi())
}

fn rel_node_id(id: RelationshipId) -> String {
    format!("r{}", id.data().as_ffi())
}

fn cond_node_id(id: ConditionalId) -> String {
    format!("cond{}", id.data().as_ffi())
}

/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// Constraint links: when a plan is cached (`sheet.selected_method` returns `Some`) links are
/// directed (inputs → relationship → outputs) and [`GraphData::arrows`] is `true`. Otherwise
/// all cells adjacent to the relationship are emitted as undirected source→relationship edges.
///
/// Conditional nodes: for each conditional, emits one `Conditional` node, one `Constraint` link
/// from the match cell to the conditional node, and one `Control` link per relationship in each
/// branch/default. Control links carry `branch_index` and `branch_active` for rendering.
///
/// - Complexity: O(c + r + e + cond·b·k) where c = cells, r = relationships, e = adjacency pairs,
///   cond = conditionals, b = branches per conditional, k = keys per branch.
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let mut arrows = false;

    // Cell nodes
    for id in sheet.cells() {
        let (label, value) = labels
            .cells
            .get(&id)
            .map(|m| (m.label.clone(), (m.display)(sheet)))
            .unwrap_or_default();
        nodes.push(NodeData {
            id: cell_node_id(id),
            kind: NodeKind::Cell,
            label,
            value,
        });
    }

    // Relationship nodes and constraint links
    for id in sheet.relationships() {
        nodes.push(NodeData {
            id: rel_node_id(id),
            kind: NodeKind::Relationship,
            label: String::new(),
            value: String::new(),
        });

        if let Some(method_idx) = sheet.selected_method(id) {
            arrows = true;
            if let Some(inputs) = sheet.method_inputs(id, method_idx) {
                for &cell_id in inputs {
                    links.push(LinkData {
                        source: cell_node_id(cell_id),
                        target: rel_node_id(id),
                        kind: LinkKind::Constraint,
                        branch_index: None,
                        branch_active: None,
                    });
                }
            }
            if let Some(outputs) = sheet.method_outputs(id, method_idx) {
                for &cell_id in outputs {
                    links.push(LinkData {
                        source: rel_node_id(id),
                        target: cell_node_id(cell_id),
                        kind: LinkKind::Constraint,
                        branch_index: None,
                        branch_active: None,
                    });
                }
            }
        } else if let Some(adj) = sheet.relationship_adj(id) {
            for &cell_id in adj {
                links.push(LinkData {
                    source: cell_node_id(cell_id),
                    target: rel_node_id(id),
                    kind: LinkKind::Constraint,
                    branch_index: None,
                    branch_active: None,
                });
            }
        }
    }

    // Conditional nodes and control links
    for cond_id in sheet.conditionals() {
        let node_id = cond_node_id(cond_id);
        nodes.push(NodeData {
            id: node_id.clone(),
            kind: NodeKind::Conditional,
            label: String::new(),
            value: String::new(),
        });

        // Constraint link: match cell → conditional node
        if let Some(match_cell) = sheet.conditional_match_cell(cond_id) {
            links.push(LinkData {
                source: cell_node_id(match_cell),
                target: node_id.clone(),
                kind: LinkKind::Constraint,
                branch_index: None,
                branch_active: None,
            });
        }

        let active_branch = sheet.conditional_active_branch(cond_id);

        // Control links for named branches
        let branch_count = sheet.conditional_branch_count(cond_id).unwrap_or(0);
        for branch in 0..branch_count {
            let is_active = active_branch == Some(branch);
            if let Some(rels) = sheet.conditional_branch_relationships(cond_id, branch) {
                for &rel_id in rels {
                    links.push(LinkData {
                        source: node_id.clone(),
                        target: rel_node_id(rel_id),
                        kind: LinkKind::Control,
                        branch_index: Some(branch),
                        branch_active: Some(is_active),
                    });
                }
            }
        }

        // Control links for default relationships
        let default_active = active_branch.is_none();
        if let Some(default_rels) = sheet.conditional_default_relationships(cond_id) {
            for &rel_id in default_rels {
                links.push(LinkData {
                    source: node_id.clone(),
                    target: rel_node_id(rel_id),
                    kind: LinkKind::Control,
                    branch_index: None,
                    branch_active: Some(default_active),
                });
            }
        }
    }

    let changed = sheet.changed().map(cell_node_id).collect();
    let forced = sheet.forced_cells().map(cell_node_id).collect();
    let forced_relationships = sheet.forced_relationships().map(rel_node_id).collect();

    GraphData {
        nodes,
        links,
        changed,
        forced,
        forced_relationships,
        arrows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use property_model::{Method, Sheet};

    #[test]
    fn format_property_model_error_invalid_id_falls_back_to_display() {
        let msg = format_property_model_error(&Error::InvalidId, "source text");
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_property_model_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed(inner);

        let msg = format_property_model_error(&err, source);

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }

    #[test]
    fn labels_from_cell_names_builds_entries_for_supported_types() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(2.0_f64);
        let b = sheet.add_cell(3_i32);
        let c = sheet.add_cell(true);
        let d = sheet.add_cell("hi".to_string());

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeId::of::<f64>()));
        cell_names.insert("b".to_string(), (b, TypeId::of::<i32>()));
        cell_names.insert("c".to_string(), (c, TypeId::of::<bool>()));
        cell_names.insert("d".to_string(), (d, TypeId::of::<String>()));

        let labels = labels_from_cell_names(&cell_names);

        assert_eq!(labels.cells.len(), 4);
        assert_eq!((labels.cells[&a].display)(&sheet), "2");
        assert_eq!((labels.cells[&b].display)(&sheet), "3");
        assert_eq!((labels.cells[&c].display)(&sheet), "true");
        assert_eq!((labels.cells[&d].display)(&sheet), "hi");
    }

    #[test]
    fn labels_from_cell_names_preserves_declaration_order() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let z = sheet.add_cell(1_i32);
        let a = sheet.add_cell(2_i32);

        let mut cell_names = IndexMap::new();
        cell_names.insert("z".to_string(), (z, TypeId::of::<i32>()));
        cell_names.insert("a".to_string(), (a, TypeId::of::<i32>()));

        let labels = labels_from_cell_names(&cell_names);
        let ids: Vec<_> = labels.cells.keys().copied().collect();
        assert_eq!(ids, vec![z, a]);
    }

    fn demo_sheet() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(3.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");

        sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();

        (sheet, labels)
    }

    // Separate helper that adds the output cell first so propagation succeeds.
    fn demo_sheet_with_plan() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        // c added first → lowest strength (output by default).
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(3.0_f64);
        labels.add_cell::<f64>(b, "b");

        sheet
            .add_relationship(vec![Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| {
                Ok(x * y)
            })])
            .unwrap();

        (sheet, labels)
    }

    fn sheet_with_conditional() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(a, b, |v: &f64| Ok(*v)),
                Method::from_fn_1_1(b, a, |v: &f64| Ok(*v)),
            ])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        (sheet, labels)
    }

    fn sheet_with_forced_conditional() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional(p, vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

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
    fn to_graph_data_arrows_false_before_propagate() {
        let (sheet, labels) = demo_sheet_with_plan();
        let data = to_graph_data(&sheet, &labels);
        assert!(!data.arrows);
    }

    #[test]
    fn to_graph_data_arrows_true_after_propagate() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);
        assert!(data.arrows);
    }

    #[test]
    fn to_graph_data_directed_input_links_target_relationship() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let rel_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .unwrap();

        let to_rel: Vec<_> = data
            .links
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::Constraint) && l.target == rel_id)
            .collect();
        assert_eq!(to_rel.len(), 2);
    }

    #[test]
    fn to_graph_data_directed_output_links_source_relationship() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let rel_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .unwrap();

        let from_rel: Vec<_> = data
            .links
            .iter()
            .filter(|l| matches!(l.kind, LinkKind::Constraint) && l.source == rel_id)
            .collect();
        assert_eq!(from_rel.len(), 1);
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
        assert!((labels.cells[&a_id].write_str)(&mut sheet, "5.0").is_ok());
        let display = &labels.cells[&a_id].display;
        assert_eq!(display(&sheet), "5");
    }

    #[test]
    fn to_graph_data_emits_conditional_node() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.nodes.iter().any(|n| n.kind == NodeKind::Conditional),
            "expected a Conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_constraint_link_from_match_cell_to_conditional() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Constraint) && l.target == cond_id),
            "expected a Constraint link targeting the conditional node"
        );
    }

    #[test]
    fn to_graph_data_emits_control_link_for_branch_relationship() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Control)),
            "expected at least one Control link"
        );
    }

    #[test]
    fn to_graph_data_active_branch_control_link_is_active() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let active_control = data
            .links
            .iter()
            .find(|l| matches!(l.kind, LinkKind::Control) && l.branch_index == Some(0));
        assert!(
            active_control.is_some(),
            "expected a Control link for branch 0"
        );
        assert_eq!(active_control.unwrap().branch_active, Some(true));
    }

    #[test]
    fn to_graph_data_no_groups_field() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        let json = serde_json::to_string(&data).unwrap();
        assert!(
            !json.contains("\"groups\""),
            "GraphData must not contain groups"
        );
    }

    #[test]
    fn to_graph_data_forced_field_contains_forced_cell() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        sheet.propagate().unwrap();

        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(data.forced.contains(&cell_node_id(b_id)));
    }

    #[test]
    fn to_graph_data_forced_field_excludes_cell_when_branch_inactive() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        let b_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("b"))
            .unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.forced.contains(&cell_node_id(b_id)));
    }

    #[test]
    fn to_graph_data_forced_relationships_field_contains_forced_relationship() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let rel_id = sheet.relationships().next().unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(data.forced_relationships.contains(&rel_node_id(rel_id)));
    }

    #[test]
    fn to_graph_data_forced_relationships_field_excludes_relationship_when_branch_inactive() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let rel_id = sheet.relationships().next().unwrap();
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(!data.forced_relationships.contains(&rel_node_id(rel_id)));
    }
}
