//! Serialization bridge from [`adam_rs::Sheet`] to D3-ready JSON, for `GraphView`.

use crate::labels::Labels;
use adam_rs::{CellId, ConditionalId, RelationshipId, Sheet};
use serde::Serialize;
use slotmap::Key;

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
    use adam_rs::{MatchExpr, Method};

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
