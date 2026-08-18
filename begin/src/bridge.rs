//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON.
//!
//! [`Labels`] associates display metadata (names, type-erased display and write closures)
//! with stable [`CellId`] and [`RelationshipId`] keys. [`to_graph_data`] serializes a
//! [`Sheet`] and its [`Labels`] into a [`GraphData`] value ready for JSON encoding.

use adam_lang::type_registry::TypeShape;
use adam_rs::{CellId, ConditionalId, Error, RelationshipId, Sheet};
use annotate_snippets::Renderer;
use cel_parser::FormatRustcStyle;
use indexmap::IndexMap;
use serde::Serialize;
use slotmap::Key;
use std::any::TypeId;

/// Type-erased write closure: parses a string and writes it to a cell.
pub type WriteStrFn = Box<dyn Fn(&mut Sheet, &str) -> Result<(), Error>>;

/// Display and write metadata for a single cell.
pub struct CellMeta {
    /// Human-readable cell name shown in the graph and inspector.
    pub label: String,
    /// `true` if the cell holds a `bool`, so the Inspector can render it as a checkbox
    /// instead of a text field.
    pub is_bool: bool,
    /// Returns the current cell value as a display string.
    pub display: Box<dyn Fn(&Sheet) -> String>,
    /// Parses `s` and writes the result to the cell; returns `Err` on parse failure or type
    /// mismatch. May also always return `Err` for a cell type with no write support yet (e.g.
    /// tuples — see [`Labels::add_tuple_cell`]).
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
                is_bool: TypeId::of::<T>() == TypeId::of::<bool>(),
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

    /// Registers display-only metadata for a tuple-typed cell of any shape.
    ///
    /// `write_str` always returns `Err` — no tuple-literal parser exists yet (tracked as a
    /// follow-up: see the "Support editing tuple-typed cells in `begin`" GitHub issue). The
    /// field still participates fully in the Inspector's existing invalid/warning/disabled
    /// machinery, since that's entirely keyed on `CellId`, not on any per-type behavior.
    ///
    /// - Precondition: `id` is a live cell in the sheet this `Labels` will be used with, holding
    ///   a `cel_runtime::DynamicSequence`.
    pub fn add_tuple_cell(&mut self, id: CellId, label: &str) {
        self.cells.insert(
            id,
            CellMeta {
                label: label.to_owned(),
                is_bool: false,
                display: Box::new(move |sheet| {
                    sheet
                        .read::<cel_runtime::DynamicSequence>(id)
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|_| "?".to_owned())
                }),
                write_str: Box::new(|_sheet, _s| {
                    Err(Error::MethodFailed(anyhow::anyhow!(
                        "editing tuple-typed cells is not yet supported"
                    )))
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

/// Formats a floating-point value for display, rounded to 2 decimal places with
/// trailing zeros (and a bare trailing decimal point) trimmed.
///
/// Used in place of plain `Display` for `f32`/`f64` cells so graph labels and
/// Inspector fields show `86.67` and `300` rather than `86.666666666667` and
/// `300.0`. Not applied to other cell types: precision has no meaningful effect
/// on integers, and it would truncate `String` values outright.
///
/// # Examples
///
/// ```text
/// format_rounded(86.666666666667) == "86.67"
/// format_rounded(300.0)            == "300"
/// format_rounded(-0.001)           == "0"
/// ```
pub fn format_rounded(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Builds a [`Labels`] from an adam-lang-style declaration-ordered cell name map.
///
/// Matches each scalar cell's `TypeId` against the built-in primitive types
/// `adam_lang::TypeRegistry::new()` registers. A tuple-typed cell
/// (`TypeShape::Tuple`) appears with a Debug-formatted, display-only entry via
/// [`Labels::add_tuple_cell`]. Cells whose `TypeId` is none of the built-in
/// primitives are silently skipped, so they simply won't appear in the sidebar.
///
/// - Complexity: O(n) in the number of cells.
pub fn labels_from_cell_names(cell_names: &IndexMap<String, (CellId, TypeShape)>) -> Labels {
    let mut labels = Labels::new();
    for (name, (id, shape)) in cell_names {
        let id = *id;
        let type_id = match shape {
            TypeShape::Named(type_id) => *type_id,
            TypeShape::Tuple(_) => {
                labels.add_tuple_cell(id, name);
                continue;
            }
        };
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

        macro_rules! try_float_ty {
            ($T:ty) => {
                if type_id == TypeId::of::<$T>() {
                    labels.add_cell::<$T>(id, name);
                    if let Some(meta) = labels.cells.get_mut(&id) {
                        meta.display = Box::new(move |sheet| {
                            sheet
                                .read::<$T>(id)
                                .map(|v| format_rounded(*v as f64))
                                .unwrap_or_else(|_| "?".to_owned())
                        });
                    }
                    continue;
                }
            };
        }
        try_float_ty!(f32);
        try_float_ty!(f64);

        try_ty!(bool);
        try_ty!(String);
    }
    labels
}

/// Formats an [`Error`] as a rustc-style diagnostic when possible.
///
/// `Error::MethodFailed` wraps an `anyhow::Error` raised by a compiled method
/// body; when that error carries a `SpanContext` (attached automatically by
/// cel-parser's `span-diagnostics` feature for built-in arithmetic ops) this
/// renders a full caret diagnostic against `source`, ANSI-colored for a
/// terminal, with `file_name` (e.g. `"begin/assets/toy_example.adm2"`) shown
/// in the diagnostic header. All other variants have no source span and fall
/// back to their `Display` message, ignoring `file_name`.
pub fn format_adam_error(e: &Error, source: &str, file_name: &str) -> String {
    match e {
        Error::MethodFailed(inner) => {
            inner.format_rustc_style(source, file_name, 1, &Renderer::styled())
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
    /// An invisible junction node grouping a branch's relationships when a
    /// branch (or the default) holds more than one; rendered as a zero-size point.
    Branch,
}

/// A single node in the D3 graph.
#[derive(Serialize, Clone, PartialEq)]
pub struct NodeData {
    /// Stable string ID: `"c{ffi}"` for cells, `"r{ffi}"` for relationships, `"cond{ffi}"` for
    /// conditionals, `"br{ffi}_{branch}"` for a named branch's junction node, `"br{ffi}_def"`
    /// for the default's junction node.
    pub id: String,
    /// The kind of node, determining its visual rendering.
    pub kind: NodeKind,
    /// Cell label (e.g. `"a"`); empty string for relationships, conditionals, and branch junction nodes.
    pub label: String,
    /// Current cell value as a display string; empty string for relationships, conditionals, and
    /// branch junction nodes.
    pub value: String,
}

/// Link kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// A regular constraint edge (cell ↔ relationship, or match cell → conditional node).
    Constraint,
    /// A control edge from a conditional node toward a branch's relationship(s): a direct edge
    /// to the relationship when the branch has at most one, or, when it has more than one, a
    /// two-hop path through an intermediate `Branch` junction node (see [`LinkData`]'s doc for
    /// the full two-hop description).
    Control,
}

/// A single edge in the D3 graph.
///
/// When [`GraphData::arrows`] is `false` constraint edges are undirected; when `true`
/// they are directed from `source` to `target`. Control edges are always directed — from a
/// conditional node to a relationship, or, when a branch has more than one relationship, from
/// the conditional to an intermediate `Branch` node and from that node to each relationship —
/// and styled by `branch_index` and `branch_active`.
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
    /// [`adam_rs::Sheet::is_forced`]); consumers should disable input for these
    /// cells and may render them distinctly.
    pub forced: Vec<String>,
    /// Stable IDs of relationships forced by the planner (see
    /// [`adam_rs::Sheet::is_relationship_forced`]); consumers may render them
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

/// Returns the stable node ID for the junction node of one branch (or the default) of a
/// conditional: `"br{ffi}_{branch}"` for a named branch, `"br{ffi}_def"` for the default.
fn branch_node_id(id: ConditionalId, branch: Option<usize>) -> String {
    match branch {
        Some(b) => format!("br{}_{}", id.data().as_ffi(), b),
        None => format!("br{}_def", id.data().as_ffi()),
    }
}

/// Pushes control links (and, when needed, a junction node) for one branch — named or
/// default — of a conditional.
///
/// - Postcondition: when `rels.len() >= 2`, pushes one `Branch` node, one
///   `conditional → branch` control link, and one `branch → relationship` control link per
///   entry in `rels`, all sharing `branch_index`/`branch_active`. When `rels.len() <= 1`,
///   pushes at most one direct `conditional → relationship` control link (none if `rels` is
///   empty), matching the pre-junction-node behavior.
/// - Complexity: O(k) where k = `rels.len()` (the number of relationships in this branch or default).
fn push_branch_links(
    nodes: &mut Vec<NodeData>,
    links: &mut Vec<LinkData>,
    cond_id_str: &str,
    cond_id: ConditionalId,
    branch_index: Option<usize>,
    branch_active: bool,
    rels: &[RelationshipId],
) {
    if rels.len() >= 2 {
        let bnode_id = branch_node_id(cond_id, branch_index);
        nodes.push(NodeData {
            id: bnode_id.clone(),
            kind: NodeKind::Branch,
            label: String::new(),
            value: String::new(),
        });
        links.push(LinkData {
            source: cond_id_str.to_string(),
            target: bnode_id.clone(),
            kind: LinkKind::Control,
            branch_index,
            branch_active: Some(branch_active),
        });
        for &rel_id in rels {
            links.push(LinkData {
                source: bnode_id.clone(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                branch_index,
                branch_active: Some(branch_active),
            });
        }
    } else {
        for &rel_id in rels {
            links.push(LinkData {
                source: cond_id_str.to_string(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                branch_index,
                branch_active: Some(branch_active),
            });
        }
    }
}

/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// Constraint links: when a plan is cached (`sheet.selected_method` returns `Some`) links are
/// directed (inputs → relationship → outputs) and [`GraphData::arrows`] is `true`. Otherwise
/// all cells adjacent to the relationship are emitted as undirected source→relationship edges.
///
/// Conditional nodes: for each conditional, emits one `Conditional` node, one `Constraint` link
/// from the match cell to the conditional node, and one `Control` link per relationship in each
/// branch/default. When a branch (or the default) holds more than one relationship, its control
/// links route through an intermediate `Branch` junction node (`conditional → branch →
/// relationship`) instead of a direct edge, so the branch's relationships visually group
/// together; branches with 0 or 1 relationships keep a direct edge. Control links carry
/// `branch_index` and `branch_active` for rendering, shared identically across both hops of a
/// junction-routed branch.
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

        // Constraint links: every match cell → conditional node
        if let Some(match_cells) = sheet.conditional_match_cells(cond_id) {
            for &match_cell in match_cells {
                links.push(LinkData {
                    source: cell_node_id(match_cell),
                    target: node_id.clone(),
                    kind: LinkKind::Constraint,
                    branch_index: None,
                    branch_active: None,
                });
            }
        }

        // `to_graph_data` is read-only display code, not the `propagate()` path: by the
        // time it runs, `propagate()` has already evaluated this same expression
        // successfully, so a fresh failure here would itself be a precondition violation.
        // Treat it as "no active branch" for rendering rather than threading Result through
        // graph construction.
        let active_branch = sheet.conditional_active_branch(cond_id).ok().flatten();

        // Control links for named branches
        let branch_count = sheet.conditional_branch_count(cond_id).unwrap_or(0);
        for branch in 0..branch_count {
            let is_active = active_branch == Some(branch);
            if let Some(rels) = sheet.conditional_branch_relationships(cond_id, branch) {
                push_branch_links(
                    &mut nodes,
                    &mut links,
                    &node_id,
                    cond_id,
                    Some(branch),
                    is_active,
                    rels,
                );
            }
        }

        // Control links for default relationships
        let default_active = active_branch.is_none();
        if let Some(default_rels) = sheet.conditional_default_relationships(cond_id) {
            push_branch_links(
                &mut nodes,
                &mut links,
                &node_id,
                cond_id,
                None,
                default_active,
                default_rels,
            );
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
    use adam_rs::{MatchExpr, Method, Sheet};

    #[test]
    fn format_adam_error_invalid_id_falls_back_to_display() {
        let msg = format_adam_error(&Error::InvalidId, "source text", "test.adm2");
        assert_eq!(msg, "invalid cell or relationship id");
    }

    #[test]
    fn format_adam_error_method_failed_renders_caret_diagnostic() {
        use cel_parser::{SourceSpan, SpanContext};

        let source = "1i32 / 0i32";
        let span = SourceSpan::new(1, 0, 1, 11);
        let inner = anyhow::anyhow!("division by zero").context(SpanContext::new(span));
        let err = Error::MethodFailed(inner);

        let msg = format_adam_error(&err, source, "test.adm2");

        assert!(msg.contains("division by zero"), "{msg}");
        assert!(msg.contains(source), "{msg}");
    }

    #[test]
    fn format_rounded_trims_trailing_zeros_and_point() {
        assert_eq!(format_rounded(86.666666666667), "86.67");
        assert_eq!(format_rounded(300.0), "300");
        assert_eq!(format_rounded(2.5), "2.5");
        assert_eq!(format_rounded(0.0), "0");
    }

    #[test]
    fn format_rounded_negative_zero_has_no_minus_sign() {
        assert_eq!(format_rounded(-0.0), "0");
        assert_eq!(format_rounded(-0.001), "0");
    }

    #[test]
    fn labels_from_cell_names_rounds_float_display_to_two_decimals() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(86.666666666667_f64);

        let mut cell_names = IndexMap::new();
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));

        let labels = labels_from_cell_names(&cell_names);

        assert_eq!((labels.cells[&a].display)(&sheet), "86.67");
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
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<f64>())));
        cell_names.insert("b".to_string(), (b, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("c".to_string(), (c, TypeShape::Named(TypeId::of::<bool>())));
        cell_names.insert(
            "d".to_string(),
            (d, TypeShape::Named(TypeId::of::<String>())),
        );

        let labels = labels_from_cell_names(&cell_names);

        assert_eq!(labels.cells.len(), 4);
        assert_eq!((labels.cells[&a].display)(&sheet), "2");
        assert_eq!((labels.cells[&b].display)(&sheet), "3");
        assert_eq!((labels.cells[&c].display)(&sheet), "true");
        assert_eq!((labels.cells[&d].display)(&sheet), "hi");
    }

    #[test]
    fn labels_from_cell_names_includes_tuple_typed_cells() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let pair = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));

        let mut cell_names = IndexMap::new();
        cell_names.insert(
            "pair".to_string(),
            (
                pair,
                TypeShape::Tuple(vec![
                    TypeShape::Named(TypeId::of::<i32>()),
                    TypeShape::Named(TypeId::of::<f64>()),
                ]),
            ),
        );

        let labels = labels_from_cell_names(&cell_names);

        assert_eq!(labels.cells.len(), 1);
        assert_eq!((labels.cells[&pair].display)(&sheet), "(3, 4.5)");
    }

    #[test]
    fn labels_from_cell_names_preserves_declaration_order() {
        use std::any::TypeId;

        let mut sheet = Sheet::new();
        let z = sheet.add_cell(1_i32);
        let a = sheet.add_cell(2_i32);

        let mut cell_names = IndexMap::new();
        cell_names.insert("z".to_string(), (z, TypeShape::Named(TypeId::of::<i32>())));
        cell_names.insert("a".to_string(), (a, TypeShape::Named(TypeId::of::<i32>())));

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
            .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![])
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
            .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        (sheet, labels)
    }

    fn sheet_with_multi_relationship_branch() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, c, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional(
                MatchExpr::cell(p),
                vec![(vec![0_i32], vec![rel1, rel2])],
                vec![],
            )
            .unwrap();

        (sheet, labels)
    }

    fn sheet_with_multi_relationship_default() -> (Sheet, Labels) {
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();

        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        let c = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(c, "c");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");

        let rel1 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        let rel2 = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, c, |v: &f64| Ok(*v))])
            .unwrap();

        sheet
            .add_conditional::<i32>(MatchExpr::cell(p), vec![], vec![rel1, rel2])
            .unwrap();

        (sheet, labels)
    }

    #[test]
    fn to_graph_data_omits_branch_node_for_single_relationship_branch() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            !data.nodes.iter().any(|n| n.kind == NodeKind::Branch),
            "expected no Branch node when every branch has at most one relationship"
        );
    }

    #[test]
    fn to_graph_data_routes_multi_relationship_branch_through_branch_node() {
        let (sheet, labels) = sheet_with_multi_relationship_branch();
        let data = to_graph_data(&sheet, &labels);

        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        let branch_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Branch)
            .map(|n| n.id.clone())
            .expect("expected a Branch node");

        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Control)
                    && l.source == cond_id
                    && l.target == branch_id
                    && l.branch_index == Some(0)
                    && l.branch_active == Some(true)),
            "expected a Control link from the conditional to the branch node"
        );

        let rel_ids: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .collect();
        assert_eq!(rel_ids.len(), 2);
        for rel_id in rel_ids {
            assert!(
                data.links
                    .iter()
                    .any(|l| matches!(l.kind, LinkKind::Control)
                        && l.source == branch_id
                        && l.target == rel_id
                        && l.branch_index == Some(0)
                        && l.branch_active == Some(true)),
                "expected a Control link from the branch node to relationship {rel_id}"
            );
        }
    }

    #[test]
    fn to_graph_data_routes_multi_relationship_default_through_branch_node() {
        let (sheet, labels) = sheet_with_multi_relationship_default();
        let data = to_graph_data(&sheet, &labels);

        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        let branch_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Branch)
            .map(|n| n.id.clone())
            .expect("expected a Branch node for the default relationships");

        assert!(
            data.links
                .iter()
                .any(|l| matches!(l.kind, LinkKind::Control)
                    && l.source == cond_id
                    && l.target == branch_id
                    && l.branch_index.is_none()
                    && l.branch_active == Some(true)),
            "expected a Control link from the conditional to the default branch node"
        );

        let rel_ids: Vec<_> = data
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Relationship)
            .map(|n| n.id.clone())
            .collect();
        assert_eq!(rel_ids.len(), 2);
        for rel_id in rel_ids {
            assert!(
                data.links
                    .iter()
                    .any(|l| matches!(l.kind, LinkKind::Control)
                        && l.source == branch_id
                        && l.target == rel_id
                        && l.branch_index.is_none()
                        && l.branch_active == Some(true)),
                "expected a Control link from the default branch node to relationship {rel_id}"
            );
        }
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
    fn add_tuple_cell_display_returns_rust_debug_formatted_string() {
        let mut sheet = Sheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        assert_eq!((meta.display)(&sheet), "(3, 4.5)");
    }

    #[test]
    fn add_tuple_cell_write_str_always_errs_without_mutating_the_sheet() {
        let mut sheet = Sheet::new();
        let cell_id = sheet.add_cell(cel_runtime::DynamicSequence::from_tuple((3i32, 4.5f64)));
        let mut labels = Labels::new();
        labels.add_tuple_cell(cell_id, "pair");
        let meta = labels.cells.get(&cell_id).unwrap();
        let before = sheet
            .read::<cel_runtime::DynamicSequence>(cell_id)
            .unwrap()
            .clone();
        let result = (meta.write_str)(&mut sheet, "(1, 2.0)");
        assert!(result.is_err());
        let after = sheet.read::<cel_runtime::DynamicSequence>(cell_id).unwrap();
        assert_eq!(&before, after);
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
