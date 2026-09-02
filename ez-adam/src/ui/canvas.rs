//! Canvas rendering, coordinates, and gesture handling. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §4.

use crate::model::cell_node::CellNodeId;
use crate::model::conditional_group::ConditionalGroupId;
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;
use crate::ops::relationships::{add_member, create_relationship};
use dioxus::prelude::*;
use std::collections::HashSet;

/// Identifies a canvas node of any of the three kinds `ez-adam` places on
/// the canvas, for selection and drag-target tracking. UI-only — not part
/// of the Phase 1 `Document` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    CellNode(CellNodeId),
    RelationshipGroup(RelationshipGroupId),
    ConditionalGroup(ConditionalGroupId),
}

/// The canvas's current pan offset (`x`, `y`) and uniform zoom scale (`k`).
/// UI-only state — `Document`'s stored positions are always canvas/world
/// space, unaffected by this transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub x: f64,
    pub y: f64,
    pub k: f64,
}

impl ViewTransform {
    /// Returns the identity transform: no pan, 1:1 zoom.
    #[must_use]
    pub fn identity() -> Self {
        ViewTransform {
            x: 0.0,
            y: 0.0,
            k: 1.0,
        }
    }
}

/// Converts a screen-space point (e.g. a mouse event's client coordinates)
/// into canvas/world space, by inverting `transform`.
///
/// - Postcondition: `canvas_to_screen(transform, screen_to_canvas(transform, p)) == p`
///   (up to floating-point rounding).
#[must_use]
pub fn screen_to_canvas(transform: &ViewTransform, screen: Point) -> Point {
    Point::new(
        (screen.x - transform.x) / transform.k,
        (screen.y - transform.y) / transform.k,
    )
}

/// Converts a canvas/world-space point into screen space, applying
/// `transform`.
#[must_use]
pub fn canvas_to_screen(transform: &ViewTransform, canvas: Point) -> Point {
    Point::new(
        canvas.x * transform.k + transform.x,
        canvas.y * transform.k + transform.y,
    )
}

/// Returns `transform` shifted by `(dx, dy)` in screen-space pixels — a
/// plain pan, unaffected by the current zoom level.
#[must_use]
pub fn pan_by(transform: &ViewTransform, dx: f64, dy: f64) -> ViewTransform {
    ViewTransform {
        x: transform.x + dx,
        y: transform.y + dy,
        ..*transform
    }
}

/// Returns `transform` scaled by `delta` (a multiplier, e.g. `1.1` to zoom
/// in 10%), keeping the canvas point currently under `cursor_screen` fixed
/// on screen — the standard "zoom toward cursor" behavior.
///
/// - Precondition: `delta > 0.0`.
#[must_use]
pub fn zoom_at(transform: &ViewTransform, cursor_screen: Point, delta: f64) -> ViewTransform {
    debug_assert!(transform.k != 0.0, "transform scale must not be zero");
    debug_assert!(delta > 0.0, "zoom delta must be positive");
    let new_k = transform.k * delta;
    // Solve for (new_x, new_y) such that screen_to_canvas is unchanged at cursor_screen:
    // (cursor.x - new_x) / new_k == (cursor.x - transform.x) / transform.k
    let canvas_point = screen_to_canvas(transform, cursor_screen);
    ViewTransform {
        x: cursor_screen.x - canvas_point.x * new_k,
        y: cursor_screen.y - canvas_point.y * new_k,
        k: new_k,
    }
}

/// Returns `node`'s current canvas/world-space position in `doc`.
///
/// - Precondition: `node` is a valid id in `doc` (a `CellNode`,
///   `RelationshipGroup`, or `ConditionalGroup` that actually exists).
#[must_use]
pub fn node_position(doc: &Document, node: NodeId) -> Point {
    match node {
        NodeId::CellNode(id) => doc.cell_nodes[id].position,
        NodeId::RelationshipGroup(id) => doc.relationship_groups[id].position,
        NodeId::ConditionalGroup(id) => doc.conditional_groups[id].position,
    }
}

/// Returns every node in `doc` whose position falls within the axis-aligned
/// rectangle spanning `corner_a`/`corner_b` (in either corner order) —
/// the rubber-band selection test.
///
/// - Complexity: O(n) in the total number of nodes in `doc`.
#[must_use]
pub fn nodes_in_rect(doc: &Document, corner_a: Point, corner_b: Point) -> Vec<NodeId> {
    let min_x = corner_a.x.min(corner_b.x);
    let max_x = corner_a.x.max(corner_b.x);
    let min_y = corner_a.y.min(corner_b.y);
    let max_y = corner_a.y.max(corner_b.y);
    let contains = |p: Point| p.x >= min_x && p.x <= max_x && p.y >= min_y && p.y <= max_y;

    let mut found = Vec::new();
    for (id, node) in &doc.cell_nodes {
        if contains(node.position) {
            found.push(NodeId::CellNode(id));
        }
    }
    for (id, group) in &doc.relationship_groups {
        if contains(group.position) {
            found.push(NodeId::RelationshipGroup(id));
        }
    }
    for (id, group) in &doc.conditional_groups {
        if contains(group.position) {
            found.push(NodeId::ConditionalGroup(id));
        }
    }
    found
}

/// One edge to render: a line from `from` to `to` in canvas/world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub from: Point,
    pub to: Point,
}

/// Returns every edge to render for `doc`: one per relationship-group
/// member (group ↔ cell) and one per conditional-group's wrapped
/// relationship groups (conditional ↔ each of its `default`/branch
/// `enabled_groups`, deduplicated).
///
/// - Complexity: O(n) in the total number of relationship-group members
///   plus conditional-group branch/default entries.
#[must_use]
pub fn compute_edges(doc: &Document) -> Vec<Edge> {
    let mut edges = Vec::new();
    for (_, group) in &doc.relationship_groups {
        for (node, _formula) in &group.members {
            edges.push(Edge {
                from: group.position,
                to: doc.cell_nodes[*node].position,
            });
        }
    }
    // `SlotMap`'s `IntoIterator` already yields `(K, &V)` pairs, so
    // `cond_id` is available directly — no need to recover it separately.
    let mut seen = HashSet::new();
    for (cond_id, cond) in &doc.conditional_groups {
        let mut linked_groups: Vec<_> = cond.default.iter().copied().collect();
        for branch in &cond.branches {
            linked_groups.extend(branch.enabled_groups.iter().copied());
        }
        for group_id in linked_groups {
            if seen.insert((cond_id, group_id)) {
                edges.push(Edge {
                    from: cond.position,
                    to: doc.relationship_groups[group_id].position,
                });
            }
        }
    }
    edges
}

/// Returns the stroke color for a canvas node based on selection state.
///
/// Returns `"red"` if `selected` is true, `"black"` otherwise.
fn node_stroke(selected: bool) -> &'static str {
    if selected { "red" } else { "black" }
}

/// Which gesture [`Canvas`]'s current mouse drag is performing.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    /// Dragging a hit node to a new position; `last_screen` is the previous
    /// mouse position.
    Node { node: NodeId, last_screen: Point },
    /// Panning the view; `last_screen` is the previous mouse position.
    Pan { last_screen: Point },
    /// Drawing a rubber-band selection rectangle in canvas space.
    RubberBand {
        start_canvas: Point,
        current_canvas: Point,
    },
}

/// Decides which drag gesture a mousedown at `screen_point` starts: dragging
/// a hit node, panning (empty canvas, no shift), or rubber-band selecting
/// (empty canvas, shift held).
#[must_use]
fn start_drag(
    doc: &Document,
    transform: &ViewTransform,
    screen_point: Point,
    shift_held: bool,
) -> DragMode {
    match hit_test(doc, transform, screen_point) {
        Some(node) => DragMode::Node {
            node,
            last_screen: screen_point,
        },
        None if shift_held => DragMode::RubberBand {
            start_canvas: screen_to_canvas(transform, screen_point),
            current_canvas: screen_to_canvas(transform, screen_point),
        },
        None => DragMode::Pan {
            last_screen: screen_point,
        },
    }
}

/// Returns the screen-space `(x, y, width, height)` rectangle to render for
/// `drag_mode`, if it is currently drawing a rubber-band selection —
/// `None` for `Node`, `Pan`, or no active drag.
#[must_use]
fn rubber_band_rect(
    drag_mode: Option<DragMode>,
    transform: &ViewTransform,
) -> Option<(f64, f64, f64, f64)> {
    let Some(DragMode::RubberBand {
        start_canvas,
        current_canvas,
    }) = drag_mode
    else {
        return None;
    };
    let a = canvas_to_screen(transform, start_canvas);
    let b = canvas_to_screen(transform, current_canvas);
    Some((
        a.x.min(b.x),
        a.y.min(b.y),
        (a.x - b.x).abs(),
        (a.y - b.y).abs(),
    ))
}

/// Renders `document`'s cells, relationship groups, and conditional
/// groups as SVG shapes, transformed by `view_transform`, with anything in
/// `selection` visually highlighted. When `active_tool` is `Tool::Select`,
/// supports click-to-select and drag-to-move (via
/// `hit_test`/`apply_drag_delta`), pan (empty-canvas drag), and shift-drag
/// rubber-band selection (via `nodes_in_rect`) — see `start_drag` for how a
/// mousedown picks among those drag gestures. When `active_tool` is
/// `Tool::AddRelationship`, a mousedown on a hit node instead advances the
/// Add-Relationship click sequence via `add_relationship_click`; a
/// mousedown on empty canvas is a no-op (no pan/rubber-band in this tool).
/// Zoom (mouse wheel, via `zoom_at`) works regardless of `active_tool`.
#[component]
pub fn Canvas(
    document: Signal<Document>,
    view_transform: Signal<ViewTransform>,
    selection: Signal<HashSet<NodeId>>,
    active_tool: Signal<crate::ui::toolbar::Tool>,
) -> Element {
    let mut drag_mode = use_signal(|| None::<DragMode>);
    let mut pending_first_click = use_signal(|| None::<NodeId>);

    let doc = document.read();
    let transform = *view_transform.read();
    let sel = selection.read();

    let edges = compute_edges(&doc);
    let band = rubber_band_rect(*drag_mode.read(), &transform);

    rsx! {
        svg {
            class: "canvas",
            onmousedown: move |evt: Event<MouseData>| {
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let screen_point = Point::new(client_pt.x, client_pt.y);
                let shift_held = data.modifiers().shift();
                let transform = *view_transform.read();

                match *active_tool.read() {
                    crate::ui::toolbar::Tool::Select => {
                        let doc = document.read();
                        let mode = start_drag(&doc, &transform, screen_point, shift_held);
                        drop(doc);
                        if let DragMode::Node { node, .. } = mode {
                            *selection.write() = std::iter::once(node).collect();
                        }
                        drag_mode.set(Some(mode));
                    }
                    crate::ui::toolbar::Tool::AddRelationship => {
                        let hit = hit_test(&document.read(), &transform, screen_point);
                        if let Some(clicked) = hit {
                            let canvas_point = screen_to_canvas(&transform, screen_point);
                            let new_pending = add_relationship_click(
                                &mut document.write(),
                                *pending_first_click.read(),
                                clicked,
                                canvas_point,
                            );
                            pending_first_click.set(new_pending);
                        }
                    }
                    crate::ui::toolbar::Tool::AddConditional | crate::ui::toolbar::Tool::Duplicate => {}
                }
            },
            onmousemove: move |evt: Event<MouseData>| {
                let Some(mode) = *drag_mode.read() else {
                    return;
                };
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let current_screen_point = Point::new(client_pt.x, client_pt.y);

                match mode {
                    DragMode::Node { node, last_screen } => {
                        let transform = *view_transform.read();
                        let last_canvas = screen_to_canvas(&transform, last_screen);
                        let current_canvas = screen_to_canvas(&transform, current_screen_point);
                        let delta = Point::new(
                            current_canvas.x - last_canvas.x,
                            current_canvas.y - last_canvas.y,
                        );
                        apply_drag_delta(&mut document.write(), node, delta);
                        drag_mode.set(Some(DragMode::Node {
                            node,
                            last_screen: current_screen_point,
                        }));
                    }
                    DragMode::Pan { last_screen } => {
                        let dx = current_screen_point.x - last_screen.x;
                        let dy = current_screen_point.y - last_screen.y;
                        let transform = *view_transform.read();
                        view_transform.set(pan_by(&transform, dx, dy));
                        drag_mode.set(Some(DragMode::Pan {
                            last_screen: current_screen_point,
                        }));
                    }
                    DragMode::RubberBand { start_canvas, .. } => {
                        let transform = *view_transform.read();
                        let current_canvas = screen_to_canvas(&transform, current_screen_point);
                        drag_mode.set(Some(DragMode::RubberBand {
                            start_canvas,
                            current_canvas,
                        }));
                    }
                }
            },
            onmouseup: move |_evt| {
                if let Some(DragMode::RubberBand { start_canvas, current_canvas }) = *drag_mode.read() {
                    let found = nodes_in_rect(&document.read(), start_canvas, current_canvas);
                    *selection.write() = found.into_iter().collect();
                }
                drag_mode.set(None);
            },
            onwheel: move |evt: Event<WheelData>| {
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let cursor_point = Point::new(client_pt.x, client_pt.y);
                let delta_y = data.delta().strip_units().y;
                let zoom_delta = 1.0 + (-delta_y * 0.001).clamp(-0.5, 0.5);
                let transform = *view_transform.read();
                view_transform.set(zoom_at(&transform, cursor_point, zoom_delta));
            },
            if let Some((x, y, width, height)) = band {
                rect {
                    x: "{x}",
                    y: "{y}",
                    width: "{width}",
                    height: "{height}",
                    fill: "none",
                    stroke: "black",
                    stroke_dasharray: "4",
                }
            }
            for edge in &edges {
                line {
                    x1: "{canvas_to_screen(&transform, edge.from).x}",
                    y1: "{canvas_to_screen(&transform, edge.from).y}",
                    x2: "{canvas_to_screen(&transform, edge.to).x}",
                    y2: "{canvas_to_screen(&transform, edge.to).y}",
                    stroke: "black",
                }
            }
            for (id, cell_node) in &doc.cell_nodes {
                {
                    let p = canvas_to_screen(&transform, cell_node.position);
                    let selected = sel.contains(&NodeId::CellNode(id));
                    rsx! {
                        rect {
                            x: "{p.x - 40.0}",
                            y: "{p.y - 15.0}",
                            width: "80",
                            height: "30",
                            rx: "6",
                            fill: "lightblue",
                            stroke: node_stroke(selected),
                        }
                    }
                }
            }
            for (id, group) in &doc.relationship_groups {
                {
                    let p = canvas_to_screen(&transform, group.position);
                    let selected = sel.contains(&NodeId::RelationshipGroup(id));
                    rsx! {
                        circle {
                            cx: "{p.x}",
                            cy: "{p.y}",
                            r: "12",
                            fill: "lightgreen",
                            stroke: node_stroke(selected),
                        }
                    }
                }
            }
            for (id, cond) in &doc.conditional_groups {
                {
                    let p = canvas_to_screen(&transform, cond.position);
                    let selected = sel.contains(&NodeId::ConditionalGroup(id));
                    rsx! {
                        rect {
                            x: "{p.x - 12.0}",
                            y: "{p.y - 12.0}",
                            width: "24",
                            height: "24",
                            transform: "rotate(45 {p.x} {p.y})",
                            fill: "orange",
                            stroke: node_stroke(selected),
                        }
                    }
                }
            }
        }
    }
}

/// The half-width/height (in canvas units) treated as "on" a node for hit
/// testing — matches [`Canvas`]'s rendered shape sizes (cells: 40×15
/// half-extents; relationship/conditional groups: 12 half-extent).
const CELL_HIT_HALF_EXTENT: (f64, f64) = (40.0, 15.0);
const GROUP_HIT_RADIUS: f64 = 12.0;

/// Returns the topmost node under `screen_point` (converted to canvas
/// space via `transform`), or `None` if no node is there. Checks cell
/// nodes, then relationship groups, then conditional groups, matching
/// [`Canvas`]'s draw order (later-drawn shapes are checked first only in
/// that sense — ties within a kind are broken by `SlotMap` iteration
/// order, which is unspecified but acceptable since nodes don't overlap
/// in practice).
///
/// - Complexity: O(n) in the total number of nodes in `doc`.
#[must_use]
pub fn hit_test(doc: &Document, transform: &ViewTransform, screen_point: Point) -> Option<NodeId> {
    let canvas_point = screen_to_canvas(transform, screen_point);
    for (id, node) in &doc.cell_nodes {
        let dx = (canvas_point.x - node.position.x).abs();
        let dy = (canvas_point.y - node.position.y).abs();
        if dx <= CELL_HIT_HALF_EXTENT.0 && dy <= CELL_HIT_HALF_EXTENT.1 {
            return Some(NodeId::CellNode(id));
        }
    }
    for (id, group) in &doc.relationship_groups {
        if distance(canvas_point, group.position) <= GROUP_HIT_RADIUS {
            return Some(NodeId::RelationshipGroup(id));
        }
    }
    for (id, group) in &doc.conditional_groups {
        if distance(canvas_point, group.position) <= GROUP_HIT_RADIUS {
            return Some(NodeId::ConditionalGroup(id));
        }
    }
    None
}

/// Returns the Euclidean distance between `a` and `b`.
fn distance(a: Point, b: Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Advances the Add-Relationship tool's click sequence: given whatever was
/// clicked previously (`pending_first_click`, `None` if this is a fresh
/// sequence) and what was just clicked (`clicked`), either creates a new
/// relationship group, extends an existing one, or does nothing (a bare
/// first click on something other than a cell), returning the new pending
/// state.
///
/// - `(None, cell)` → pending becomes `Some(cell)`.
/// - `(Some(cell_a), cell_b)` → creates a relationship binding both,
///   returns `None`.
/// - `(Some(cell), group)` or `(Some(group), cell)` → adds `cell` as a
///   member of `group`, returns `None`.
/// - Any other combination (e.g. a bare first click on a group, or two
///   groups) → returns `None` with no mutation — not a meaningful gesture.
pub fn add_relationship_click(
    doc: &mut Document,
    pending_first_click: Option<NodeId>,
    clicked: NodeId,
    position: Point,
) -> Option<NodeId> {
    match (pending_first_click, clicked) {
        (None, NodeId::CellNode(_)) => Some(clicked),
        (None, _) => None,
        (Some(NodeId::CellNode(a)), NodeId::CellNode(b)) => {
            let _ = create_relationship(doc, a, b, position);
            None
        }
        (Some(NodeId::CellNode(cell)), NodeId::RelationshipGroup(group))
        | (Some(NodeId::RelationshipGroup(group)), NodeId::CellNode(cell)) => {
            add_member(doc, group, cell);
            None
        }
        _ => None,
    }
}

/// Moves `node` by `delta` (canvas-space), mutating `doc` directly (this
/// is the one canvas gesture that bypasses `ops::*`, since dragging is a
/// pure position update with no other invariant to maintain — unlike
/// `ops::relationships::create_relationship` etc., there's no
/// `ops::canvas::move_node` in Phase 1 to call).
///
/// - Precondition: `node` is a valid id in `doc`.
pub fn apply_drag_delta(doc: &mut Document, node: NodeId, delta: Point) {
    let apply = |p: &mut Point| {
        p.x += delta.x;
        p.y += delta.y;
    };
    match node {
        NodeId::CellNode(id) => apply(&mut doc.cell_nodes[id].position),
        NodeId::RelationshipGroup(id) => apply(&mut doc.relationship_groups[id].position),
        NodeId::ConditionalGroup(id) => apply(&mut doc.conditional_groups[id].position),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::document::Document;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    #[test]
    fn add_relationship_click_first_click_on_a_cell_sets_pending() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));

        let pending = add_relationship_click(
            &mut doc,
            None,
            NodeId::CellNode(a_node),
            Point::new(0.0, 0.0),
        );

        assert_eq!(pending, Some(NodeId::CellNode(a_node)));
        assert!(doc.relationship_groups_in_order().next().is_none());
    }

    #[test]
    fn add_relationship_click_second_click_on_a_cell_creates_a_group() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));

        let pending = add_relationship_click(
            &mut doc,
            Some(NodeId::CellNode(a_node)),
            NodeId::CellNode(b_node),
            Point::new(5.0, 5.0),
        );

        assert_eq!(pending, None);
        assert_eq!(doc.relationship_groups_in_order().count(), 1);
    }

    #[test]
    fn add_relationship_click_second_click_on_an_existing_group_adds_a_member() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let c = add_cell(&mut doc, "c", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let c_node = add_cell_node(&mut doc, c, Point::new(20.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let pending = add_relationship_click(
            &mut doc,
            Some(NodeId::RelationshipGroup(group)),
            NodeId::CellNode(c_node),
            Point::new(5.0, 5.0),
        );

        assert_eq!(pending, None);
        assert_eq!(doc.relationship_groups[group].members.len(), 3);
    }

    #[test]
    fn add_relationship_click_first_click_on_a_non_cell_stays_pending_none() {
        // Clicking a relationship group first (not a cell) doesn't start a
        // valid pending state for creating a NEW relationship — only
        // extending an existing one via a second click makes sense, and
        // that requires the group to be the SECOND click's target with a
        // cell as pending, or vice versa. A bare first click on a group
        // with nothing pending is a no-op.
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let pending = add_relationship_click(
            &mut doc,
            None,
            NodeId::RelationshipGroup(group),
            Point::new(5.0, 5.0),
        );

        assert_eq!(pending, None);
        assert_eq!(doc.relationship_groups[group].members.len(), 2);
    }

    #[test]
    fn node_position_returns_a_cell_nodes_position() {
        let mut doc = Document::new("demo");
        let cell = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, cell, Point::new(5.0, 7.0));
        assert_eq!(
            node_position(&doc, NodeId::CellNode(node)),
            Point::new(5.0, 7.0)
        );
    }

    #[test]
    fn nodes_in_rect_includes_only_nodes_within_the_bounds() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let inside = add_cell_node(&mut doc, a, Point::new(5.0, 5.0));
        let outside = add_cell_node(&mut doc, b, Point::new(50.0, 50.0));

        let found = nodes_in_rect(&doc, Point::new(0.0, 0.0), Point::new(10.0, 10.0));

        assert!(found.contains(&NodeId::CellNode(inside)));
        assert!(!found.contains(&NodeId::CellNode(outside)));
    }

    #[test]
    fn nodes_in_rect_handles_corners_given_in_either_order() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let inside = add_cell_node(&mut doc, a, Point::new(5.0, 5.0));

        // corner_a is bottom-right, corner_b is top-left — should still work.
        let found = nodes_in_rect(&doc, Point::new(10.0, 10.0), Point::new(0.0, 0.0));

        assert!(found.contains(&NodeId::CellNode(inside)));
    }

    #[test]
    fn identity_transform_is_a_no_op_both_ways() {
        let t = ViewTransform::identity();
        let p = Point::new(3.0, 4.0);
        assert_eq!(screen_to_canvas(&t, p), p);
        assert_eq!(canvas_to_screen(&t, p), p);
    }

    #[test]
    fn screen_to_canvas_and_back_round_trips() {
        let t = ViewTransform {
            x: 10.0,
            y: -5.0,
            k: 2.0,
        };
        let p = Point::new(100.0, 50.0);
        let canvas = screen_to_canvas(&t, p);
        let back = canvas_to_screen(&t, canvas);
        assert!((back.x - p.x).abs() < 1e-9);
        assert!((back.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn screen_to_canvas_applies_pan_and_zoom_correctly() {
        // A view panned by (10, 20) and zoomed 2x: screen (30, 40) should
        // map to canvas ((30-10)/2, (40-20)/2) = (10, 10).
        let t = ViewTransform {
            x: 10.0,
            y: 20.0,
            k: 2.0,
        };
        let canvas = screen_to_canvas(&t, Point::new(30.0, 40.0));
        assert_eq!(canvas, Point::new(10.0, 10.0));
    }

    #[test]
    fn pan_by_shifts_x_and_y_only() {
        let t = ViewTransform {
            x: 5.0,
            y: 5.0,
            k: 2.0,
        };
        let panned = pan_by(&t, 3.0, -2.0);
        assert_eq!(
            panned,
            ViewTransform {
                x: 8.0,
                y: 3.0,
                k: 2.0
            }
        );
    }

    #[test]
    fn zoom_at_keeps_the_cursors_canvas_point_fixed() {
        let t = ViewTransform {
            x: 0.0,
            y: 0.0,
            k: 1.0,
        };
        let cursor = Point::new(100.0, 100.0);
        let before = screen_to_canvas(&t, cursor);

        let zoomed = zoom_at(&t, cursor, 1.5);

        let after = screen_to_canvas(&zoomed, cursor);
        assert!((after.x - before.x).abs() < 1e-9);
        assert!((after.y - before.y).abs() < 1e-9);
        assert_eq!(zoomed.k, 1.5);
    }

    #[test]
    fn zoom_at_multiplies_scale_by_delta() {
        let t = ViewTransform {
            x: 0.0,
            y: 0.0,
            k: 2.0,
        };
        let zoomed = zoom_at(&t, Point::new(0.0, 0.0), 1.1);
        assert!((zoomed.k - 2.2).abs() < 1e-9);
    }

    #[test]
    fn node_stroke_selected_node_returns_red() {
        assert_eq!(node_stroke(true), "red");
    }

    #[test]
    fn node_stroke_unselected_node_returns_black() {
        assert_eq!(node_stroke(false), "black");
    }

    #[test]
    fn hit_test_finds_a_cell_node_near_its_position() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(100.0, 100.0));
        let t = ViewTransform::identity();

        let hit = hit_test(&doc, &t, Point::new(105.0, 102.0));

        assert_eq!(hit, Some(NodeId::CellNode(node)));
    }

    #[test]
    fn hit_test_returns_none_for_empty_canvas_area() {
        let doc = Document::new("demo");
        let t = ViewTransform::identity();
        assert_eq!(hit_test(&doc, &t, Point::new(500.0, 500.0)), None);
    }

    #[test]
    fn apply_drag_delta_moves_a_cell_node() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(10.0, 10.0));

        apply_drag_delta(&mut doc, NodeId::CellNode(node), Point::new(5.0, -3.0));

        assert_eq!(doc.cell_nodes[node].position, Point::new(15.0, 7.0));
    }

    #[test]
    fn start_drag_hitting_a_node_returns_node_mode() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(100.0, 100.0));
        let t = ViewTransform::identity();

        let mode = start_drag(&doc, &t, Point::new(105.0, 102.0), false);

        assert_eq!(
            mode,
            DragMode::Node {
                node: NodeId::CellNode(node),
                last_screen: Point::new(105.0, 102.0),
            }
        );
    }

    #[test]
    fn start_drag_hitting_a_node_returns_node_mode_even_with_shift_held() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(100.0, 100.0));
        let t = ViewTransform::identity();

        let mode = start_drag(&doc, &t, Point::new(105.0, 102.0), true);

        assert_eq!(
            mode,
            DragMode::Node {
                node: NodeId::CellNode(node),
                last_screen: Point::new(105.0, 102.0),
            }
        );
    }

    #[test]
    fn start_drag_on_empty_canvas_without_shift_returns_pan_mode() {
        let doc = Document::new("demo");
        let t = ViewTransform::identity();

        let mode = start_drag(&doc, &t, Point::new(500.0, 500.0), false);

        assert_eq!(
            mode,
            DragMode::Pan {
                last_screen: Point::new(500.0, 500.0),
            }
        );
    }

    #[test]
    fn start_drag_on_empty_canvas_with_shift_returns_rubber_band_mode() {
        let doc = Document::new("demo");
        let t = ViewTransform::identity();

        let mode = start_drag(&doc, &t, Point::new(500.0, 500.0), true);

        assert_eq!(
            mode,
            DragMode::RubberBand {
                start_canvas: Point::new(500.0, 500.0),
                current_canvas: Point::new(500.0, 500.0),
            }
        );
    }

    #[test]
    fn rubber_band_rect_returns_none_for_node_mode() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let t = ViewTransform::identity();
        let mode = Some(DragMode::Node {
            node: NodeId::CellNode(node),
            last_screen: Point::new(0.0, 0.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), None);
    }

    #[test]
    fn rubber_band_rect_returns_none_for_pan_mode() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::Pan {
            last_screen: Point::new(0.0, 0.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), None);
    }

    #[test]
    fn rubber_band_rect_returns_none_when_no_drag_is_active() {
        let t = ViewTransform::identity();
        assert_eq!(rubber_band_rect(None, &t), None);
    }

    #[test]
    fn rubber_band_rect_computes_bounds_from_corners() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::RubberBand {
            start_canvas: Point::new(10.0, 20.0),
            current_canvas: Point::new(30.0, 50.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), Some((10.0, 20.0, 20.0, 30.0)));
    }

    #[test]
    fn rubber_band_rect_handles_corners_given_in_either_order() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::RubberBand {
            start_canvas: Point::new(30.0, 50.0),
            current_canvas: Point::new(10.0, 20.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), Some((10.0, 20.0, 20.0, 30.0)));
    }

    #[test]
    fn rubber_band_rect_applies_the_view_transform() {
        let t = ViewTransform {
            x: 10.0,
            y: 0.0,
            k: 2.0,
        };
        let mode = Some(DragMode::RubberBand {
            start_canvas: Point::new(0.0, 0.0),
            current_canvas: Point::new(10.0, 10.0),
        });
        // canvas (0,0) -> screen (10, 0); canvas (10,10) -> screen (30, 20).
        assert_eq!(rubber_band_rect(mode, &t), Some((10.0, 0.0, 20.0, 20.0)));
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    #[test]
    fn compute_edges_connects_relationship_group_to_each_member() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let _ = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let edges = compute_edges(&doc);
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().all(|e| e.from == Point::new(5.0, 5.0)));
    }

    #[test]
    fn compute_edges_connects_conditional_group_to_default_and_branch_groups_once_each() {
        use crate::model::conditional_group::CellValueLiteral;
        use crate::ops::conditionals::{
            add_branch, add_conditional_with_formula, toggle_enabled_group,
        };

        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "x", CellType::f64());
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let cond =
            add_conditional_with_formula(&mut doc, vec![x], "x > 1.0", Point::new(0.0, 20.0));
        add_branch(&mut doc, cond, vec![CellValueLiteral::Bool(true)]);
        // Add group to both default and branch — should deduplicate to 1 edge.
        {
            let cond_mut = &mut doc.conditional_groups[cond];
            cond_mut.default.push(group);
        }
        toggle_enabled_group(&mut doc, cond, 0, group);

        let edges = compute_edges(&doc);
        let cond_to_group = edges
            .iter()
            .filter(|e| {
                e.from == doc.conditional_groups[cond].position
                    && e.to == doc.relationship_groups[group].position
            })
            .count();
        assert_eq!(cond_to_group, 1);
    }
}
