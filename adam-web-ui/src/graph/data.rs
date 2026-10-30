//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON, for `GraphView`.

use crate::labels::Labels;
use adam_rs::{CellId, ConditionalId, RelationshipId, Sheet};
use serde::Serialize;
use slotmap::Key;

/// Node kind tag used in the D3 graph.
#[derive(Serialize, Clone, PartialEq, Eq)]
///
/// # Examples
///
/// A cell snapshot carries `NodeKind::Cell`; a multi-relationship branch junction
/// carries `NodeKind::Branch`.
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
///
/// # Examples
///
/// Inspect a cell entry in `to_graph_data(&sheet, &labels).nodes` to display its
/// registered label and formatted value alongside its stable graph ID.
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
///
/// # Examples
///
/// Conditional activation links use `LinkKind::Control`; data dependencies use
/// `LinkKind::Constraint`.
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
/// `directed` is `true` when this specific link's direction is known and should render
/// with an arrowhead from `source` to `target`; `false` renders it as a plain undirected
/// line. This is a per-link fact, not a sheet-wide one: a cell ↔ relationship link is
/// directed only once *that relationship* has a selected method, so one relationship
/// having a cached plan says nothing about another relationship (or a currently-inactive
/// conditional branch's relationship) that doesn't. A match cell → conditional link is
/// always directed — evaluating a conditional's match expression doesn't depend on any
/// relationship ever being planned. Control edges are always directed too — from a
/// conditional node to a relationship, or, when a branch has more than one relationship, from
/// the conditional to an intermediate `Branch` node and from that node to each relationship —
/// and styled by `branch_index` and `branch_active`.
#[derive(Serialize, Clone, PartialEq)]
///
/// # Examples
///
/// A control link whose `branch_active` is `None` represents an unknown conditional
/// evaluation, not a successfully selected default branch.
pub struct LinkData {
    /// Stable string ID of the source node.
    pub source: String,
    /// Stable string ID of the target node.
    pub target: String,
    /// The kind of link, determining its visual rendering.
    pub kind: LinkKind,
    /// `true` if this link's direction is known and should render with an arrowhead.
    pub directed: bool,
    /// Branch index for `Control` links; `None` for `Constraint` links and default-branch control links.
    pub branch_index: Option<usize>,
    /// `true` if active, `false` if inactive; `None` for constraints or failed evaluation.
    pub branch_active: Option<bool>,
}

/// Complete graph snapshot ready for JSON serialization and delivery to D3.
#[derive(Serialize, Clone, PartialEq)]
///
/// # Examples
///
/// `to_graph_data(&sheet, &labels)` returns a serializable snapshot whose `changed`
/// IDs identify the runtime's changed cells.
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
}

/// Returns the cell's stable, kind-prefixed graph ID.
fn cell_node_id(id: CellId) -> String {
    format!("c{}", id.data().as_ffi())
}

/// Returns the relationship's stable, kind-prefixed graph ID.
fn rel_node_id(id: RelationshipId) -> String {
    format!("r{}", id.data().as_ffi())
}

/// Returns the conditional's stable, kind-prefixed graph ID.
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
    branch_active: Option<bool>,
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
            directed: true,
            branch_index,
            branch_active,
        });
        for &rel_id in rels {
            links.push(LinkData {
                source: bnode_id.clone(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                directed: true,
                branch_index,
                branch_active,
            });
        }
    } else {
        for &rel_id in rels {
            links.push(LinkData {
                source: cond_id_str.to_string(),
                target: rel_node_id(rel_id),
                kind: LinkKind::Control,
                directed: true,
                branch_index,
                branch_active,
            });
        }
    }
}

/// Serializes `sheet` and `labels` into a [`GraphData`] snapshot for D3.
///
/// Constraint links: when `sheet.selected_method` supplies a relationship method,
/// its links are directed (inputs → relationship → outputs, [`LinkData::directed`]
/// `true`). Otherwise all cells adjacent to that relationship are emitted as undirected
/// source→relationship edges (`directed` `false`) — this is decided per relationship, so one
/// relationship having a plan never marks another relationship's links as directed.
///
/// Conditional nodes: for each conditional, emits one `Conditional` node, one always-directed
/// `Constraint` link from the match cell to the conditional node, and one `Control` link per
/// relationship in each branch/default. When a branch (or the default) holds more than one
/// relationship, its control links route through an intermediate `Branch` junction node
/// (`conditional → branch →
/// relationship`) instead of a direct edge, so the branch's relationships visually group
/// together; branches with 0 or 1 relationships keep a direct edge. Control links carry
/// `branch_index` and `branch_active` for rendering, shared identically across both hops of a
/// junction-routed branch.
///
/// - Complexity: O(c + r + e + t) traversal/copy work for cells, relationships,
///   emitted links, and copied text; cell formatting, conditional matching/evaluation,
///   and sheet classification queries add their own costs.
///
/// # Examples
///
/// ```
/// let sheet = adam_rs::Sheet::new();
/// let data = adam_web_ui::to_graph_data(&sheet, &adam_web_ui::Labels::new());
/// assert!(data.nodes.is_empty());
/// ```
pub fn to_graph_data(sheet: &Sheet, labels: &Labels) -> GraphData {
    let mut nodes = Vec::new();
    let mut links = Vec::new();

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
            if let Some(inputs) = sheet.method_inputs(id, method_idx) {
                for &cell_id in inputs {
                    links.push(LinkData {
                        source: cell_node_id(cell_id),
                        target: rel_node_id(id),
                        kind: LinkKind::Constraint,
                        directed: true,
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
                        directed: true,
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
                    directed: false,
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

        // Constraint links: every match cell → conditional node. Always directed --
        // evaluating a conditional's match expression doesn't depend on any relationship
        // (in this branch or any other) ever having a selected method, so this must not
        // be gated on relationship planning the way cell ↔ relationship links are.
        if let Some(match_cells) = sheet.conditional_match_cells(cond_id) {
            for &match_cell in match_cells {
                links.push(LinkData {
                    source: cell_node_id(match_cell),
                    target: node_id.clone(),
                    kind: LinkKind::Constraint,
                    directed: true,
                    branch_index: None,
                    branch_active: None,
                });
            }
        }

        let active_branch = sheet.conditional_active_branch(cond_id).ok();

        // Control links for named branches
        let branch_count = sheet.conditional_branch_count(cond_id).unwrap_or(0);
        for branch in 0..branch_count {
            let is_active = active_branch.map(|active| active == Some(branch));
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
        let default_active = active_branch.map(|active| active.is_none());
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluation failure leaves every branch's activity explicitly unknown.
    #[test]
    fn conditional_evaluation_error_does_not_activate_default() {
        let source = r#"sheet s {
            source divisor: i64 = 1i64;
            cell a: i64;
            cell b: i64;
            conditional 1i64 / divisor {
                1i64 => { relationship { a := b; } }
                _ => { relationship { b := a; } }
            }
        }"#;
        let outcome = crate::build_sheet(source, "s.adm2", &crate::Renderer::plain());
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let (mut sheet, labels) = outcome.sheet_labels.unwrap();
        let divisor = sheet.cell_names["divisor"].0;
        sheet.write(divisor, 0i64).unwrap();
        assert!(sheet.propagate().is_err());
        let data = to_graph_data(&sheet, &labels);
        let controls = data
            .links
            .iter()
            .filter(|link| link.kind == LinkKind::Control)
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 2);
        assert!(controls.iter().all(|link| link.branch_active.is_none()));
    }
    use adam_rs::{MatchExpr, Method};

    /// Builds an unpropagated three-cell product relationship and scalar labels.
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

    /// Builds an unpropagated product sheet with its output cell inserted first.
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

    /// Builds one reversible relationship enabled by an integer selector's zero branch.
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

    /// Builds one one-way relationship enabled by an integer selector's zero branch.
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

    /// Builds two one-way relationships sharing a named conditional branch.
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

    /// Builds two one-way relationships sharing an unconditional default branch.
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
    /// Single-relationship branches connect directly without a junction node.
    fn to_graph_data_omits_branch_node_for_single_relationship_branch() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            !data.nodes.iter().any(|n| n.kind == NodeKind::Branch),
            "expected no Branch node when every branch has at most one relationship"
        );
    }

    #[test]
    /// Multi-relationship named branches route both control hops through a junction.
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
    /// Multi-relationship defaults route both control hops through a junction.
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
    /// Serialization emits one node per cell and relationship in the fixture.
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
    /// Serialization emits every relationship membership link.
    fn to_graph_data_produces_correct_link_count() {
        let (sheet, labels) = demo_sheet();
        let data = to_graph_data(&sheet, &labels);
        assert_eq!(data.links.len(), 3);
    }

    #[test]
    /// Cell graph nodes preserve registered labels.
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
    /// Anonymous relationship graph nodes have empty labels.
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
    /// Changed graph IDs exactly represent the runtime's changed-cell set without duplicates.
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
        let expected: std::collections::HashSet<_> = sheet.changed().map(cell_node_id).collect();
        assert_eq!(
            data.changed
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>(),
            expected
        );
        assert_eq!(data.changed.len(), expected.len());
    }

    #[test]
    /// Relationships without a selected plan render undirected membership links.
    fn to_graph_data_constraint_links_undirected_before_propagate() {
        let (sheet, labels) = demo_sheet_with_plan();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.links
                .iter()
                .filter(|l| matches!(l.kind, LinkKind::Constraint))
                .all(|l| !l.directed),
            "no relationship has a cached plan yet, so every constraint link must be undirected"
        );
    }

    #[test]
    /// Selected relationship methods produce directed dependency links.
    fn to_graph_data_constraint_links_directed_after_propagate() {
        let (mut sheet, labels) = demo_sheet_with_plan();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.links
                .iter()
                .filter(|l| matches!(l.kind, LinkKind::Constraint))
                .any(|l| l.directed),
            "the relationship's cached plan should make its constraint links directed"
        );
    }

    #[test]
    /// Selector dependencies remain directed when their conditional branch is inactive.
    fn to_graph_data_match_cell_link_is_directed_even_when_conditional_branch_is_inactive() {
        // Reproduces adam-lang-book's constrain.adm2: a sheet whose only relationship
        // lives inside a conditional, currently not matching any branch. No relationship
        // anywhere in the sheet has a cached plan, but the match cell -> conditional link
        // must still render directed -- evaluating the conditional's match expression
        // doesn't depend on any relationship ever being planned.
        let (mut sheet, labels) = sheet_with_conditional();
        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let cond_id = data
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Conditional)
            .map(|n| n.id.clone())
            .unwrap();
        let match_link = data
            .links
            .iter()
            .find(|l| matches!(l.kind, LinkKind::Constraint) && l.target == cond_id)
            .expect("expected a Constraint link targeting the conditional node");
        assert!(
            match_link.directed,
            "the match cell -> conditional link must be directed regardless of whether \
             the conditional's own relationship currently has a plan"
        );
    }

    #[test]
    /// One relationship's selected plan does not direct another unplanned relationship's links.
    fn to_graph_data_undirected_relationship_link_is_independent_of_other_relationships() {
        // R1 (a -> b) is unconditional and always planned; R2 (x <-> y, two methods) lives
        // in a conditional that currently doesn't match any branch, so R2 has no plan. R1
        // having a plan must not make R2's fallback adjacency links directed too -- each
        // relationship's `directed` flag is its own fact, not a sheet-wide one.
        let mut sheet = Sheet::new();
        let mut labels = Labels::new();
        let a = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(a, "a");
        let b = sheet.add_cell(0.0_f64);
        labels.add_cell::<f64>(b, "b");
        sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();

        let x = sheet.add_cell(1.0_f64);
        labels.add_cell::<f64>(x, "x");
        let y = sheet.add_cell(2.0_f64);
        labels.add_cell::<f64>(y, "y");
        let p = sheet.add_cell(0_i32);
        labels.add_cell::<i32>(p, "p");
        let rel_xy = sheet
            .add_relationship(vec![
                Method::from_fn_1_1(x, y, |v: &f64| Ok(*v)),
                Method::from_fn_1_1(y, x, |v: &f64| Ok(*v)),
            ])
            .unwrap();
        sheet
            .add_conditional(
                MatchExpr::cell(p),
                vec![(vec![1_i32], vec![rel_xy])],
                vec![],
            )
            .unwrap();

        sheet.propagate().unwrap();
        let data = to_graph_data(&sheet, &labels);

        let rel_xy_id = rel_node_id(rel_xy);
        assert!(
            data.links
                .iter()
                .filter(|l| matches!(l.kind, LinkKind::Constraint)
                    && (l.source == rel_xy_id || l.target == rel_xy_id))
                .all(|l| !l.directed),
            "R2's links must stay undirected even though R1 elsewhere has a cached plan"
        );
    }

    #[test]
    /// Selected input dependencies point toward the relationship.
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
    /// Selected output dependencies point away from the relationship.
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
    /// Conditional selectors produce conditional graph nodes.
    fn to_graph_data_emits_conditional_node() {
        let (sheet, labels) = sheet_with_conditional();
        let data = to_graph_data(&sheet, &labels);
        assert!(
            data.nodes.iter().any(|n| n.kind == NodeKind::Conditional),
            "expected a Conditional node"
        );
    }

    #[test]
    /// Conditional selectors retain their match-cell dependency links.
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
    /// Active branch relationships retain their branch index and active control state.
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
    /// Serialized graph snapshots omit the obsolete groups field.
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
    /// Forced active cells appear in the graph's forced-cell IDs.
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
    /// Inactive conditional relationships do not force their former output cells.
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
    /// Forced active relationships appear in the graph's forced-relationship IDs.
    fn to_graph_data_forced_relationships_field_contains_forced_relationship() {
        let (mut sheet, labels) = sheet_with_forced_conditional();
        let rel_id = sheet.relationships().next().unwrap();
        sheet.propagate().unwrap();

        let data = to_graph_data(&sheet, &labels);
        assert!(data.forced_relationships.contains(&rel_node_id(rel_id)));
    }

    #[test]
    /// Inactive relationships do not appear in forced-relationship IDs.
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
