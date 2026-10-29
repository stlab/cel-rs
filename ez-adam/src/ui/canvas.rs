//! Canvas rendering, coordinates, and gesture handling. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §4.

use crate::model::cell::CellType;
use crate::model::cell_node::CellNodeId;
use crate::model::conditional_group::{CellValueLiteral, ConditionalGroupId};
use crate::model::document::Document;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;
use crate::ops::cells::delete_cell_node;
use crate::ops::conditionals;
use crate::ops::conditionals::delete_conditional_group;
use crate::ops::relationships::{
    add_member, create_relationship, delete_relationship_group, duplicate_relationship_group,
};
use dioxus::prelude::*;
use std::collections::HashSet;

#[cfg(test)]
mod ownership_geometry_tests {
    use super::*;
    use crate::ops::cells::{add_cell, add_cell_node};

    /// Reconnecting a live member is a no-op, not a precondition violation.
    #[test]
    fn repeated_connection_is_noop() {
        let mut doc = Document::new("s");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let an = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let bn = add_cell_node(&mut doc, b, Point::new(100.0, 0.0));
        let group = create_relationship(&mut doc, an, bn, Point::new(50.0, 0.0));
        let before = doc.clone();
        assert_eq!(
            add_relationship_click(
                &mut doc,
                Some(NodeId::CellNode(an)),
                NodeId::RelationshipGroup(group)
            ),
            ClickSequenceOutcome::NoOp
        );
        assert_eq!(doc, before);
    }

    /// Hit regions follow fixed screen dimensions, painter order, and diamond tips.
    #[test]
    fn hit_geometry_matches_screen_shapes() {
        let mut doc = Document::new("s");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let an = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        for k in [0.5, 2.0] {
            let transform = ViewTransform { x: 0.0, y: 0.0, k };
            assert_eq!(
                hit_test(&doc, &transform, Point::new(39.0, 0.0)),
                Some(NodeId::CellNode(an))
            );
            assert_eq!(hit_test(&doc, &transform, Point::new(41.0, 0.0)), None);
        }
        let b = add_cell(&mut doc, "b", CellType::i64());
        let bn = add_cell_node(&mut doc, b, Point::new(100.0, 0.0));
        let group = create_relationship(&mut doc, an, bn, Point::new(0.0, 0.0));
        let transform = ViewTransform::identity();
        assert_eq!(
            hit_test(&doc, &transform, Point::new(0.0, 0.0)),
            Some(NodeId::RelationshipGroup(group))
        );
        let conditional = conditionals::add_conditional_with_formula(
            &mut doc,
            vec![],
            "true",
            Point::new(0.0, 0.0),
        );
        assert_eq!(
            hit_test(&doc, &transform, Point::new(0.0, 0.0)),
            Some(NodeId::ConditionalGroup(conditional))
        );
        assert_eq!(
            hit_test(&doc, &transform, Point::new(16.0, 0.0)),
            Some(NodeId::ConditionalGroup(conditional))
        );
        assert_eq!(
            hit_test(&doc, &transform, Point::new(10.0, 10.0)),
            Some(NodeId::CellNode(an))
        );
    }

    /// A target error never completes an Add Conditional gesture.
    #[test]
    fn failed_conditional_click_is_not_completed() {
        let mut doc = Document::new("s");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let an = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let bn = add_cell_node(&mut doc, b, Point::new(100.0, 0.0));
        let group = create_relationship(&mut doc, an, bn, Point::new(50.0, 0.0));
        let conditional = conditionals::add_conditional_with_formula(
            &mut doc,
            vec![],
            "true",
            Point::new(0.0, 0.0),
        );
        let outcome = add_conditional_click(
            &mut doc,
            Some(group),
            NodeId::ConditionalGroup(conditional),
            Point::new(0.0, 0.0),
        );
        assert_ne!(outcome, ClickSequenceOutcome::Completed);
    }
}

/// Identifies a canvas node of any of the three kinds `ez-adam` places on
/// the canvas, for selection and drag-target tracking. UI-only — not part
/// of the Phase 1 `Document` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
///
/// # Examples
///
/// `NodeId::CellNode(placement)` selects one placement rather than every placement
/// sharing its underlying cell.
pub enum NodeId {
    CellNode(CellNodeId),
    RelationshipGroup(RelationshipGroupId),
    ConditionalGroup(ConditionalGroupId),
}

/// The canvas's current pan offset (`x`, `y`) and uniform zoom scale (`k`).
/// UI-only state — `Document`'s stored positions are always canvas/world
/// space, unaffected by this transform.
#[derive(Debug, Clone, Copy, PartialEq)]
///
/// # Examples
///
/// `ViewTransform { x: 10.0, y: 20.0, k: 2.0 }` maps the canvas origin to
/// screen point `(10, 20)` at double scale.
pub struct ViewTransform {
    pub x: f64,
    pub y: f64,
    pub k: f64,
}

impl ViewTransform {
    /// Returns the identity transform: no pan, 1:1 zoom.
    #[must_use]
    ///
    /// # Examples
    ///
    /// `ViewTransform::identity()` leaves canvas and screen coordinates equal.
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
///
/// # Examples
///
/// `screen_to_canvas(&ViewTransform::identity(), Point::new(4.0, 5.0))`
/// returns the point unchanged.
pub fn screen_to_canvas(transform: &ViewTransform, screen: Point) -> Point {
    Point::new(
        (screen.x - transform.x) / transform.k,
        (screen.y - transform.y) / transform.k,
    )
}

/// Converts a canvas/world-space point into screen space, applying
/// `transform`.
#[must_use]
///
/// # Examples
///
/// Convert each stored node position with `canvas_to_screen(&transform, position)`
/// before placing its fixed-screen-size shape.
pub fn canvas_to_screen(transform: &ViewTransform, canvas: Point) -> Point {
    Point::new(
        canvas.x * transform.k + transform.x,
        canvas.y * transform.k + transform.y,
    )
}

/// Returns `transform` shifted by `(dx, dy)` in screen-space pixels — a
/// plain pan, unaffected by the current zoom level.
#[must_use]
///
/// # Examples
///
/// `pan_by(&ViewTransform::identity(), 10.0, 20.0)` translates the view without zooming.
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
///
/// # Examples
///
/// `zoom_at(&transform, cursor, 2.0)` doubles scale while keeping the cursor's canvas point fixed.
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
///
/// # Examples
///
/// `node_position(&document, NodeId::CellNode(id))` reads a live placement's stored position.
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
///
/// # Examples
///
/// Pass the two canvas-space drag endpoints to `nodes_in_rect(&document, start, end)`
/// to select enclosed node centers in either drag direction.
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

/// The visual line style of an [`Edge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
///
/// # Examples
///
/// Inspect each `compute_edges(&document)` edge's style to distinguish membership
/// lines from conditional-control dependencies.
pub enum EdgeStyle {
    /// A relationship-group ↔ cell edge, or a conditional ↔ condition-cell
    /// edge.
    Solid,
    /// A conditional ↔ relationship-group edge — dashed so the "this
    /// conditional gates that relationship" link reads visually distinct
    /// from a relationship's own solid cell-membership edges.
    Dashed,
}

/// One edge to render: a line from `from` to `to` in canvas/world space,
/// drawn per `style`.
#[derive(Debug, Clone, Copy, PartialEq)]
///
/// # Examples
///
/// `compute_edges(&document)` supplies the endpoint positions and styles consumed
/// by the canvas's line layer.
pub struct Edge {
    pub from: Point,
    pub to: Point,
    pub style: EdgeStyle,
}

/// Returns every edge to render for `doc`: one solid edge per
/// relationship-group member (group ↔ cell), one dashed edge per
/// conditional-group's wrapped relationship groups (conditional ↔ each of
/// its `default`/branch `enabled_groups`, deduplicated), and one solid edge
/// per conditional-group's condition cell(s).
///
/// - Complexity: expected O(m + g + d·p) time and O(m + g + d) space for
///   relationship members m, enabled-group references g, condition dependencies d,
///   and cell placements p.
#[must_use]
///
/// # Examples
///
/// Render `compute_edges(&document)` before the node layer so membership and
/// conditional dependency lines remain behind their endpoints.
pub fn compute_edges(doc: &Document) -> Vec<Edge> {
    let mut edges = Vec::new();
    for (_, group) in &doc.relationship_groups {
        for (node, _formula) in &group.members {
            edges.push(Edge {
                from: group.position,
                to: doc.cell_nodes[*node].position,
                style: EdgeStyle::Solid,
            });
        }
    }
    // `SlotMap`'s `IntoIterator` already yields `(K, &V)` pairs, so
    // `cond_id` is available directly — no need to recover it separately.
    let mut seen = HashSet::new();
    for (cond_id, cond) in &doc.conditional_groups {
        for &group_id in cond.default.iter().chain(
            cond.branches
                .iter()
                .flat_map(|branch| &branch.enabled_groups),
        ) {
            if seen.insert((cond_id, group_id)) {
                edges.push(Edge {
                    from: cond.position,
                    to: doc.relationship_groups[group_id].position,
                    style: EdgeStyle::Dashed,
                });
            }
        }

        // An edge from the conditional group to each cell its condition
        // depends on (the cells it auto-enumerates a truth table over, or
        // the cells a `Formula`-mode expression references) — without
        // this, nothing visually shows which cell(s) actually drive the
        // conditional. A cell can have more than one `CellNode` (see
        // `CellNode`'s own doc comment); an edge is drawn to the first one
        // found, since any of a cell's placements represents the same
        // underlying value.
        for &cell_id in cond.condition.referenced_cells() {
            if let Some((_, node)) = doc.cell_nodes.iter().find(|(_, n)| n.cell == cell_id) {
                edges.push(Edge {
                    from: cond.position,
                    to: node.position,
                    style: EdgeStyle::Solid,
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

/// Returns the SVG `stroke-dasharray` value for an edge's line style.
///
/// Returns `"4"` for `EdgeStyle::Dashed`, `"0"` for `EdgeStyle::Solid` (a
/// dash length of `0` renders as an unbroken line).
fn edge_dasharray(style: EdgeStyle) -> &'static str {
    match style {
        EdgeStyle::Solid => "0",
        EdgeStyle::Dashed => "4",
    }
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
/// When `active_tool` is `Tool::AddConditional`, a mousedown on a hit node
/// instead advances the Add-Conditional click sequence via
/// `add_conditional_click`, mirroring Add-Relationship. Completing either
/// gesture (`ClickSequenceOutcome::Completed`) switches `active_tool` back
/// to `Tool::Select` so the user isn't left in the connect-mode after
/// finishing a connection. Zoom (mouse wheel, via `zoom_at`) works
/// regardless of `active_tool`.
///
/// All mouse/wheel handlers read `client_coordinates()` (viewport-relative),
/// while rendering computes shape positions via `canvas_to_screen(...)` into
/// SVG-internal coordinates — these two spaces coincide only if the `<svg>`
/// sits at the viewport origin, which it won't once a menu bar/toolbar is
/// laid out above it. Dioxus's `MouseData`/`WheelData` do expose an
/// `element_coordinates()` accessor, but per `dioxus-html`'s own
/// `InteractionElementOffset` doc comment ("coordinates of the event
/// relative to the target element") and its backing DOM `offsetX`/`offsetY`
/// semantics, that's relative to `event.target` — whichever child shape
/// (`<rect>`/`<circle>`/`<line>`) the pointer is actually over — not to this
/// `<svg>` (the `currentTarget` the listener is attached to, and the origin
/// `canvas_to_screen` renders into). So it is not a safe drop-in
/// replacement here: switching would make hit-testing correct only when the
/// cursor happens to be over the same-origin `<svg>` background and wrong
/// over any shape. Left as `client_coordinates()` pending a real
/// display-based check (see #177) rather than guessing further.
///
/// - Complexity: expected O(p + t) node construction per render for p placement slots
///   and t label bytes, plus [`compute_edges`] and child rendering.
#[component]
///
/// # Examples
///
/// Inside a Dioxus scope, `rsx! { Canvas { document, selection, active_tool, view_transform } }`
/// binds the canvas to editor state and its independent view transform.
pub fn Canvas(
    document: Signal<Document>,
    view_transform: Signal<ViewTransform>,
    selection: Signal<HashSet<NodeId>>,
    active_tool: Signal<crate::ui::toolbar::Tool>,
) -> Element {
    let mut drag_mode = use_signal(|| None::<DragMode>);
    let mut pending_first_click = use_signal(|| None::<NodeId>);
    let mut pending_conditional_source = use_signal(|| None::<RelationshipGroupId>);
    let mut gesture_error = use_signal(|| None::<&'static str>);

    // Both Add-Relationship's and Add-Conditional's click-sequence state
    // span two separate mousedown events (click A, then later click B).
    // Each is read back within the SAME `onmousedown` handler that writes
    // it (to decide whether the current click is the first or second of
    // the gesture), so an unconditional reset at the top of `onmousedown`
    // would make every click look like a fresh first click, breaking both
    // tools entirely. Instead, clear both whenever `active_tool` changes
    // (via this effect) so switching away and back between click A and
    // click B can't resume a stale first click, while a same-tool
    // two-click sequence (no tool change in between) is left untouched and
    // completes normally.
    use_effect(move || {
        active_tool.read();
        pending_first_click.set(None);
        pending_conditional_source.set(None);
    });

    let doc = document.read();
    let transform = *view_transform.read();
    let sel = selection.read();

    let edges = compute_edges(&doc);
    let band = rubber_band_rect(*drag_mode.read(), &transform);

    rsx! {
        if let Some(error) = *gesture_error.read() {
            div { role: "alert", style: "position:absolute;bottom:12px;left:12px;z-index:2;", "{error}" }
        }
        svg {
            class: "canvas",
            // Fills its containing block exactly, with no `viewBox` — so 1
            // SVG user unit equals 1 CSS pixel and mouse-event
            // `client_coordinates()` (viewport-relative) line up exactly
            // with the coordinates `canvas_to_screen` renders shapes at
            // (SVG-local, i.e. same-origin as the viewport since this
            // element and its ancestors are positioned at the viewport's
            // top-left — see `App`'s layout). `display: block` avoids the
            // few-pixel inline-element baseline gap `<svg>` gets by
            // default. Positioning the `<svg>` anywhere other than the
            // viewport's top-left would reintroduce the coordinate offset
            // this comment describes fixing (see issue #177's original
            // report).
            width: "100%",
            height: "100%",
            style: "position: absolute; top: 0; left: 0; display: block;",
            onmousedown: move |evt: Event<MouseData>| {
                gesture_error.set(None);
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let screen_point = Point::new(client_pt.x, client_pt.y);
                let shift_held = data.modifiers().shift();
                let transform = *view_transform.read();

                let current_tool = *active_tool.read();
                match current_tool {
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
                            let pending = *pending_first_click.read();
                            let outcome = add_relationship_click(
                                &mut document.write(),
                                pending,
                                clicked,
                            );
                            match outcome {
                                ClickSequenceOutcome::Pending(node) => {
                                    pending_first_click.set(Some(node));
                                }
                                ClickSequenceOutcome::Completed => {
                                    pending_first_click.set(None);
                                    active_tool.set(crate::ui::toolbar::Tool::Select);
                                }
                                ClickSequenceOutcome::NoOp => {
                                    pending_first_click.set(None);
                                }
                                ClickSequenceOutcome::Failed(error) => {
                                    gesture_error.set(Some(error));
                                    pending_first_click.set(None);
                                }
                            }
                        }
                    }
                    crate::ui::toolbar::Tool::AddConditional => {
                        let hit = hit_test(&document.read(), &transform, screen_point);
                        if let Some(clicked) = hit {
                            let canvas_point = screen_to_canvas(&transform, screen_point);
                            let pending = *pending_conditional_source.read();
                            let outcome = add_conditional_click(
                                &mut document.write(),
                                pending,
                                clicked,
                                canvas_point,
                            );
                            match outcome {
                                ClickSequenceOutcome::Pending(group) => {
                                    pending_conditional_source.set(Some(group));
                                }
                                ClickSequenceOutcome::Completed => {
                                    pending_conditional_source.set(None);
                                    active_tool.set(crate::ui::toolbar::Tool::Select);
                                }
                                ClickSequenceOutcome::NoOp => {
                                    pending_conditional_source.set(None);
                                }
                                ClickSequenceOutcome::Failed(error) => {
                                    gesture_error.set(Some(error));
                                    pending_conditional_source.set(None);
                                }
                            }
                        }
                    }
                    crate::ui::toolbar::Tool::Duplicate => {}
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
            onmouseup: move |_evt: Event<MouseData>| {
                if let Some(DragMode::RubberBand { start_canvas, current_canvas }) = *drag_mode.read() {
                    let found = nodes_in_rect(&document.read(), start_canvas, current_canvas);
                    *selection.write() = found.into_iter().collect();
                }
                drag_mode.set(None);
            },
            onwheel: move |evt: Event<WheelData>| {
                // Without this, the wheel event also triggers the browser's
                // native page-scroll behavior, which visibly moves the
                // whole app (including the pinned menu/toolbar) instead of
                // just zooming the canvas.
                evt.prevent_default();
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let cursor_point = Point::new(client_pt.x, client_pt.y);
                let delta_y = data.delta().strip_units().y;
                let zoom_delta = wheel_zoom_delta(delta_y);
                let transform = *view_transform.read();
                view_transform.set(zoom_at(&transform, cursor_point, zoom_delta));
            },
            // `tabindex` makes the (otherwise non-focusable) `<svg>` a
            // valid keyboard-event target — clicking it (already handled
            // by `onmousedown` above) also gives it focus, which is what
            // lets `onkeydown` below actually fire.
            tabindex: "0",
            onkeydown: move |evt: Event<KeyboardData>| {
                // macOS's own "Delete" key (to the left of the Return
                // key) is reported as `Key::Backspace`, per
                // `keyboard_types::Key::Backspace`'s own doc comment;
                // `Key::Delete` is the separate forward-delete key
                // (Fn+Delete on a Mac keyboard). Both should delete the
                // current selection.
                let key = evt.data().key();
                if key == dioxus::html::Key::Backspace || key == dioxus::html::Key::Delete {
                    delete_selection(&mut document.write(), &selection.read());
                    selection.write().clear();
                }
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
                    stroke_dasharray: "{edge_dasharray(edge.style)}",
                }
            }
            for (id, cell_node) in &doc.cell_nodes {
                {
                    let p = canvas_to_screen(&transform, node_position(&doc, NodeId::CellNode(id)));
                    let selected = sel.contains(&NodeId::CellNode(id));
                    let name = &doc.cells[cell_node.cell].name;
                    let half_width = cell_label_half_width(name);
                    rsx! {
                        rect {
                            x: "{p.x - half_width}",
                            y: "{p.y - CELL_HALF_HEIGHT}",
                            width: "{half_width * 2.0}",
                            height: "{CELL_HALF_HEIGHT * 2.0}",
                            rx: "6",
                            fill: "lightblue",
                            stroke: node_stroke(selected),
                        }
                        text {
                            x: "{p.x}",
                            y: "{p.y}",
                            text_anchor: "middle",
                            dominant_baseline: "middle",
                            font_size: "12",
                            "{name}"
                        }
                    }
                }
            }
            for (id, group) in &doc.relationship_groups {
                {
                    let p = canvas_to_screen(
                        &transform,
                        node_position(&doc, NodeId::RelationshipGroup(id)),
                    );
                    let selected = sel.contains(&NodeId::RelationshipGroup(id));
                    rsx! {
                        circle {
                            cx: "{p.x}",
                            cy: "{p.y}",
                            r: "12",
                            fill: "lightgreen",
                            stroke: node_stroke(selected),
                        }
                        text {
                            x: "{p.x}",
                            y: "{p.y - 18.0}",
                            text_anchor: "middle",
                            dominant_baseline: "middle",
                            font_size: "12",
                            "{group.display_name}"
                        }
                    }
                }
            }
            for (id, cond) in &doc.conditional_groups {
                {
                    let p = canvas_to_screen(
                        &transform,
                        node_position(&doc, NodeId::ConditionalGroup(id)),
                    );
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
                        text {
                            x: "{p.x}",
                            y: "{p.y - 20.0}",
                            text_anchor: "middle",
                            dominant_baseline: "middle",
                            font_size: "12",
                            "{cond.display_name}"
                        }
                    }
                }
            }
        }
    }
}

/// The half-height (in screen pixels) treated as "on" a cell node for hit
/// testing — matches [`Canvas`]'s rendered cell box height. The half-width
/// varies per cell (see [`cell_label_half_width`]) since the box widens to
/// fit longer names. Relationship/conditional groups use a fixed
/// [`GROUP_HIT_RADIUS`] half-extent instead, since their shapes don't
/// resize for a label.
const CELL_HALF_HEIGHT: f64 = 15.0;
const GROUP_HIT_RADIUS: f64 = 12.0;

/// Returns a cell rect's half-width, in canvas units, wide enough to fit
/// `name`'s label — the minimum half-width matches the original fixed box
/// size (for short names); longer names widen it via a rough monospace
/// character-width estimate, since no real text-measurement API is used.
/// Shared by [`Canvas`]'s rendering and [`hit_test`] so the clickable area
/// always matches the drawn box exactly.
///
/// - Complexity: O(n) in `name.len()`.
#[must_use]
fn cell_label_half_width(name: &str) -> f64 {
    const CHAR_WIDTH: f64 = 7.0;
    const MIN_HALF_WIDTH: f64 = 40.0;
    const PADDING: f64 = 10.0;
    let estimated = (name.chars().count() as f64) * CHAR_WIDTH / 2.0 + PADDING;
    estimated.max(MIN_HALF_WIDTH)
}

/// Returns the last-painted shape under a screen-space point, or `None`.
///
/// Shape sizes stay fixed in screen pixels at every zoom, matching [`Canvas`].
///
/// - Complexity: O(n + t) time for nodes n and cell-label characters t; O(1) space.
#[must_use]
///
/// # Examples
///
/// `hit_test(&document, &transform, cursor)` returns the topmost visible node
/// whose screen-space shape contains the cursor.
pub fn hit_test(doc: &Document, transform: &ViewTransform, screen_point: Point) -> Option<NodeId> {
    let conditional = doc
        .conditional_groups
        .iter()
        .filter(|(_, group)| {
            let p = canvas_to_screen(transform, group.position);
            (screen_point.x - p.x).abs() + (screen_point.y - p.y).abs()
                <= GROUP_HIT_RADIUS * std::f64::consts::SQRT_2
        })
        .map(|(id, _)| NodeId::ConditionalGroup(id))
        .last();
    conditional
        .or_else(|| {
            doc.relationship_groups
                .iter()
                .filter(|(_, group)| {
                    distance(screen_point, canvas_to_screen(transform, group.position))
                        <= GROUP_HIT_RADIUS
                })
                .map(|(id, _)| NodeId::RelationshipGroup(id))
                .last()
        })
        .or_else(|| {
            doc.cell_nodes
                .iter()
                .filter(|(_, node)| {
                    let p = canvas_to_screen(transform, node.position);
                    (screen_point.x - p.x).abs()
                        <= cell_label_half_width(&doc.cells[node.cell].name)
                        && (screen_point.y - p.y).abs() <= CELL_HALF_HEIGHT
                })
                .map(|(id, _)| NodeId::CellNode(id))
                .last()
        })
}

/// Returns the Euclidean distance between `a` and `b`.
fn distance(a: Point, b: Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Converts a wheel event's `delta_y` into a `zoom_at` multiplier, clamped so a
/// single large scroll tick can't invert or zero the scale.
fn wheel_zoom_delta(delta_y: f64) -> f64 {
    1.0 + (-delta_y * 0.001).clamp(-0.5, 0.5)
}

/// The outcome of one click in a two-click "connect two things" gesture
/// (Add Relationship's [`add_relationship_click`], Add Conditional's
/// [`add_conditional_click`]) — distinct from a plain "new pending state"
/// return value so a caller can tell a genuinely *completed* gesture (the
/// document changed) apart from a first click or a mismatched click that
/// merely happens to also leave nothing pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
///
/// # Examples
///
/// Keep the pending node from `Pending(node)`, return to selection on `Completed`,
/// and display `Failed(message)` without claiming that the connection succeeded.
pub enum ClickSequenceOutcome<T> {
    /// This was the first click of the sequence; `T` is the pending state
    /// to remember for the next click.
    Pending(T),
    /// This was a matching second click: the gesture completed and
    /// mutated the document.
    Completed,
    /// This click didn't advance the gesture (a mismatched second click,
    /// or a first click on something that can't start one) — any prior
    /// pending state should be discarded.
    NoOp,
    /// The target rejected the gesture; the tool remains active and displays the error.
    Failed(&'static str),
}

/// Advances the Add-Relationship tool's click sequence: given whatever was
/// clicked previously (`pending_first_click`, `None` if this is a fresh
/// sequence) and what was just clicked (`clicked`), either creates a new
/// relationship group, extends an existing one, or does nothing (a bare
/// first click on something other than a cell).
///
/// - `(None, cell)` → `Pending(cell)`.
/// - `(Some(cell_a), cell_b)` → creates a relationship binding both, placed
///   at the midpoint between the two cells' own node positions (not either
///   click position), returns `Completed`.
/// - `(Some(cell), group)` or `(Some(group), cell)` → adds `cell` as a
///   member of `group`, returns `Completed`.
/// - Any other combination (e.g. a bare first click on a group, or two
///   groups) → `NoOp` with no mutation — not a meaningful gesture.
///
/// # Examples
///
/// Start with `pending = None`; pass the first clicked cell to obtain a pending
/// connection, then pass the second cell or existing group to complete it.
pub fn add_relationship_click(
    doc: &mut Document,
    pending_first_click: Option<NodeId>,
    clicked: NodeId,
) -> ClickSequenceOutcome<NodeId> {
    match (pending_first_click, clicked) {
        (None, NodeId::CellNode(_)) => ClickSequenceOutcome::Pending(clicked),
        (None, _) => ClickSequenceOutcome::NoOp,
        (Some(NodeId::CellNode(a)), NodeId::CellNode(b)) => {
            if a == b || doc.cell_nodes[a].cell == doc.cell_nodes[b].cell {
                return ClickSequenceOutcome::NoOp;
            }
            let pa = doc.cell_nodes[a].position;
            let pb = doc.cell_nodes[b].position;
            let midpoint = Point::new((pa.x + pb.x) / 2.0, (pa.y + pb.y) / 2.0);
            let _ = create_relationship(doc, a, b, midpoint);
            ClickSequenceOutcome::Completed
        }
        (Some(NodeId::CellNode(cell)), NodeId::RelationshipGroup(group))
        | (Some(NodeId::RelationshipGroup(group)), NodeId::CellNode(cell)) => {
            if doc.relationship_groups[group]
                .members
                .iter()
                .any(|(node, _)| *node == cell)
            {
                return ClickSequenceOutcome::NoOp;
            }
            add_member(doc, group, cell);
            ClickSequenceOutcome::Completed
        }
        _ => ClickSequenceOutcome::NoOp,
    }
}

/// Advances the Add-Conditional tool's click sequence: given whatever
/// relationship group was clicked previously (`pending_source`, `None` if
/// this is a fresh sequence) and what was just clicked (`clicked`), either
/// records a fresh pending source or completes the gesture against the
/// newly clicked target via [`add_conditional_target`].
///
/// - `(None, RelationshipGroup(g))` → `Pending(g)`.
/// - `(Some(g), CellNode(_))` or `(Some(g), ConditionalGroup(_))` →
///   returns `Completed` on success or `Failed` with the target error.
/// - Any other combination (e.g. a bare first click on a cell, or two
///   relationship groups) → `NoOp` with no mutation — not a meaningful
///   gesture.
///
/// # Examples
///
/// First click a relationship group, then click a Boolean cell or existing
/// conditional; only a `Completed` outcome finishes the connection tool.
pub fn add_conditional_click(
    doc: &mut Document,
    pending_source: Option<RelationshipGroupId>,
    clicked: NodeId,
    position: Point,
) -> ClickSequenceOutcome<RelationshipGroupId> {
    match (pending_source, clicked) {
        (None, NodeId::RelationshipGroup(g)) => ClickSequenceOutcome::Pending(g),
        (None, _) => ClickSequenceOutcome::NoOp,
        (Some(g), NodeId::CellNode(_)) | (Some(g), NodeId::ConditionalGroup(_)) => {
            match add_conditional_target(doc, g, clicked, position) {
                Ok(()) => ClickSequenceOutcome::Completed,
                Err(error) => ClickSequenceOutcome::Failed(error),
            }
        }
        _ => ClickSequenceOutcome::NoOp,
    }
}

/// Completes an Add-Conditional connection from `group` to `target`: wraps
/// `group` in a new conditional group at `position`, `Cells`-mode if
/// `target` is a `Bool` cell, `Formula`-mode otherwise — or, if `target` is
/// an existing conditional group, attaches `group` to it instead (see
/// [`attach_group_to_conditional`]).
///
/// In `Formula`-mode, the formula expression itself starts empty (filled in
/// later via the side panel), but a single placeholder branch with value
/// `CellValueLiteral::Bool(true)` is added immediately and `group` is
/// enabled on it. This is not a guess at the referenced cell's own type —
/// branch values always match the *expression's* evaluated type (a boolean
/// comparison the user will write, e.g. `"threshold > 2.0"`), which is
/// `Bool` regardless of what type of cell the formula references, so
/// `Bool(true)` is the correct placeholder shape (every existing
/// Formula-mode branch in this codebase's Phase-1 test suite uses it for
/// exactly this reason). Doing this immediately — rather than waiting for
/// the side panel — also keeps `group` from silently becoming orphaned:
/// `compute_edges` only draws a conditional→group edge for groups that
/// appear in `cond.default` or some branch's `enabled_groups`, so without
/// this, `group` would render disconnected until the user finishes editing
/// the side panel.
///
/// # Errors
///
/// Returns `Err` if `target` is not a cell node or an existing conditional
/// group (e.g. targeting another relationship group is not a meaningful
/// gesture for this tool).
///
/// # Examples
///
/// Connect a pending relationship to a Boolean placement to create a two-row
/// truth table; a non-Boolean placement instead starts a scalar formula draft.
pub fn add_conditional_target(
    doc: &mut Document,
    group: RelationshipGroupId,
    target: NodeId,
    position: Point,
) -> Result<(), &'static str> {
    match target {
        NodeId::CellNode(node) => {
            let cell_id = doc.cell_nodes[node].cell;
            if matches!(doc.cells[cell_id].ty, CellType::Bool) {
                let _ = conditionals::add_conditional_from_bool_cells(
                    doc,
                    vec![cell_id],
                    group,
                    position,
                );
            } else {
                let cond_id = conditionals::add_conditional_with_formula(
                    doc,
                    vec![cell_id],
                    String::new(),
                    position,
                );
                let branch_index =
                    conditionals::add_branch(doc, cond_id, vec![CellValueLiteral::Bool(true)]);
                conditionals::toggle_enabled_group(doc, cond_id, branch_index, group);
            }
            Ok(())
        }
        NodeId::ConditionalGroup(existing) => attach_group_to_conditional(doc, existing, group),
        _ => Err("Add Conditional target must be a cell or an existing conditional group"),
    }
}

/// Attaches `group` to `conditional`'s enable-table, toggling it on the
/// last branch — the branch enumeration order [`conditionals::add_conditional_from_bool_cells`]
/// and the single-branch case in [`add_conditional_target`] both put their
/// "most relevant" branch last (an all-cells-true combination, or the one
/// placeholder branch a freshly created conditional starts with), so this
/// is the most useful default target for a group newly connected to an
/// already-existing conditional. The side panel's enable-table lets the
/// user move it to a different branch afterward.
///
/// # Errors
///
/// Returns `Err` if `conditional` has no branches yet to attach to.
fn attach_group_to_conditional(
    doc: &mut Document,
    conditional: ConditionalGroupId,
    group: RelationshipGroupId,
) -> Result<(), &'static str> {
    let branch_count = doc.conditional_groups[conditional].branches.len();
    let Some(last_branch) = branch_count.checked_sub(1) else {
        return Err("Add Conditional target has no branches to attach a group to yet");
    };
    conditionals::toggle_enabled_group(doc, conditional, last_branch, group);
    Ok(())
}

/// Moves `node` by `delta` (canvas-space), mutating `doc` directly (this
/// is the one canvas gesture that bypasses `ops::*`, since dragging is a
/// pure position update with no other invariant to maintain — unlike
/// `ops::relationships::create_relationship` etc., there's no
/// `ops::canvas::move_node` in Phase 1 to call).
///
/// - Precondition: `node` is a valid id in `doc`.
///
/// # Examples
///
/// After converting two cursor positions to canvas space, call
/// `apply_drag_delta(&mut document, selected, delta)` with their displacement.
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

/// Duplicates every relationship group in `selection` (ignoring any
/// selected cell/conditional nodes, which this tool doesn't act on),
/// offsetting each duplicate by `offset`. Returns the new groups' ids.
///
/// - Precondition: selected relationship IDs are live in `doc`.
/// - Complexity: O(s + m + t) time and space for selected IDs, copied member
///   placements, and copied display-name/formula text.
///
/// # Examples
///
/// Select several relationship groups and invoke this one-shot action to copy
/// each of them; selected cell and conditional nodes remain unchanged.
pub fn duplicate_selection(
    doc: &mut Document,
    selection: &HashSet<NodeId>,
    offset: Point,
) -> Vec<RelationshipGroupId> {
    selection
        .iter()
        .filter_map(|node| match node {
            NodeId::RelationshipGroup(id) => Some(duplicate_relationship_group(doc, *id, offset)),
            _ => None,
        })
        .collect()
}

/// Deletes every node in `selection` from `doc`, dispatching by kind to
/// [`delete_cell_node`] / [`delete_relationship_group`] /
/// [`delete_conditional_group`]. Each of those already keeps `doc`
/// internally consistent as part of its own cascade (see their doc
/// comments), so the only thing this function needs to guard against is a
/// node whose deletion was already triggered as a side effect of deleting
/// an earlier entry in the same `selection` (e.g. selecting both a
/// relationship group and the last cell node bound to it — deleting the
/// cell node cascades to delete the now-empty group first) — skipping any
/// entry that's no longer a valid key makes the overall result independent
/// of `selection`'s (unspecified) iteration order.
///
/// - Complexity: O(n·m) — O(n) selected nodes, each triggering an O(m)
///   cascade through the rest of `doc`.
///
/// # Examples
///
/// `delete_selection(&mut document, &selection)` removes the selected objects
/// while tolerating cascading removal of other selected IDs.
pub fn delete_selection(doc: &mut Document, selection: &HashSet<NodeId>) {
    for node in selection {
        match node {
            NodeId::CellNode(id) => {
                if doc.cell_nodes.contains_key(*id) {
                    delete_cell_node(doc, *id);
                }
            }
            NodeId::RelationshipGroup(id) => {
                if doc.relationship_groups.contains_key(*id) {
                    delete_relationship_group(doc, *id);
                }
            }
            NodeId::ConditionalGroup(id) => {
                if doc.conditional_groups.contains_key(*id) {
                    delete_conditional_group(doc, *id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::model::cell::CellType as CT;
    use crate::model::document::Document;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    #[test]
    /// Dropping a conditional connection onto a Boolean creates a truth-table selector.
    fn add_conditional_target_onto_a_bool_cell_creates_a_cells_mode_conditional() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        let result = add_conditional_target(
            &mut doc,
            group,
            NodeId::CellNode(flag_node),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_ok());
        assert_eq!(doc.conditional_groups_in_order().count(), 1);
    }

    #[test]
    /// Non-Boolean conditional targets start an editable scalar formula.
    fn add_conditional_target_onto_a_non_bool_cell_starts_formula_mode() {
        let mut doc = Document::new("demo");
        let threshold = add_cell(&mut doc, "threshold", CT::f64());
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let threshold_node = add_cell_node(&mut doc, threshold, Point::new(0.0, 20.0));

        let result = add_conditional_target(
            &mut doc,
            group,
            NodeId::CellNode(threshold_node),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_ok());
        let (_, cond) = doc.conditional_groups_in_order().next().unwrap();
        assert!(matches!(
            cond.condition,
            crate::model::conditional_group::ConditionExpr::Formula { .. }
        ));
        assert_eq!(cond.branches.len(), 1);
        assert!(cond.branches[0].enabled_groups.contains(&group));
    }

    #[test]
    /// Unsupported conditional targets return an error.
    fn add_conditional_target_onto_a_non_cell_target_errors() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let result = add_conditional_target(
            &mut doc,
            group,
            NodeId::RelationshipGroup(group),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_err());
    }

    #[test]
    /// Existing conditional targets gain the connected relationship.
    fn add_conditional_target_onto_an_existing_conditional_attaches_the_group() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let first_group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        add_conditional_target(
            &mut doc,
            first_group,
            NodeId::CellNode(flag_node),
            Point::new(0.0, 40.0),
        )
        .unwrap();
        let (cond_id, _) = doc.conditional_groups_in_order().next().unwrap();

        let c = add_cell(&mut doc, "c", CT::i64());
        let d = add_cell(&mut doc, "d", CT::i64());
        let c_node = add_cell_node(&mut doc, c, Point::new(0.0, 100.0));
        let d_node = add_cell_node(&mut doc, d, Point::new(10.0, 100.0));
        let second_group = create_relationship(&mut doc, c_node, d_node, Point::new(5.0, 105.0));

        let result = add_conditional_target(
            &mut doc,
            second_group,
            NodeId::ConditionalGroup(cond_id),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_ok());
        assert_eq!(doc.conditional_groups_in_order().count(), 1);
        let last_branch = doc.conditional_groups[cond_id].branches.len() - 1;
        assert!(
            doc.conditional_groups[cond_id].branches[last_branch]
                .enabled_groups
                .contains(&second_group)
        );
    }

    #[test]
    /// Empty conditional tables fail attachment explicitly.
    fn add_conditional_target_onto_a_conditional_with_no_branches_errors() {
        use crate::ops::conditionals::add_conditional_with_formula;

        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "x", CT::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "x > 1.0", Point::new(0.0, 0.0));
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let result = add_conditional_target(
            &mut doc,
            group,
            NodeId::ConditionalGroup(cond),
            Point::new(0.0, 0.0),
        );

        assert!(result.is_err());
    }

    #[test]
    /// The first relationship click begins a conditional connection.
    fn add_conditional_click_first_click_on_a_relationship_group_sets_pending() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let pending = add_conditional_click(
            &mut doc,
            None,
            NodeId::RelationshipGroup(group),
            Point::new(5.0, 5.0),
        );

        assert_eq!(pending, ClickSequenceOutcome::Pending(group));
        assert!(doc.conditional_groups_in_order().next().is_none());
    }

    #[test]
    /// Conditional connections ignore an unsupported first click.
    fn add_conditional_click_first_click_on_a_non_group_is_a_no_op() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));

        let pending = add_conditional_click(
            &mut doc,
            None,
            NodeId::CellNode(a_node),
            Point::new(0.0, 0.0),
        );

        assert_eq!(pending, ClickSequenceOutcome::NoOp);
    }

    #[test]
    /// Completing a connection onto a Boolean creates its conditional group.
    fn add_conditional_click_second_click_on_a_bool_cell_creates_a_conditional() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let pending = add_conditional_click(
            &mut doc,
            Some(group),
            NodeId::CellNode(flag_node),
            Point::new(0.0, 40.0),
        );

        assert_eq!(pending, ClickSequenceOutcome::Completed);
        assert_eq!(doc.conditional_groups_in_order().count(), 1);
    }

    #[test]
    /// Completing a connection onto a conditional attaches the pending relationship.
    fn add_conditional_click_second_click_on_an_existing_conditional_attaches_the_group() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let first_group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        add_conditional_click(
            &mut doc,
            Some(first_group),
            NodeId::CellNode(flag_node),
            Point::new(0.0, 40.0),
        );
        let (cond_id, _) = doc.conditional_groups_in_order().next().unwrap();

        let c = add_cell(&mut doc, "c", CT::i64());
        let d = add_cell(&mut doc, "d", CT::i64());
        let c_node = add_cell_node(&mut doc, c, Point::new(0.0, 100.0));
        let d_node = add_cell_node(&mut doc, d, Point::new(10.0, 100.0));
        let second_group = create_relationship(&mut doc, c_node, d_node, Point::new(5.0, 105.0));

        let pending = add_conditional_click(
            &mut doc,
            Some(second_group),
            NodeId::ConditionalGroup(cond_id),
            Point::new(0.0, 40.0),
        );

        assert_eq!(pending, ClickSequenceOutcome::Completed);
        let last_branch = doc.conditional_groups[cond_id].branches.len() - 1;
        assert!(
            doc.conditional_groups[cond_id].branches[last_branch]
                .enabled_groups
                .contains(&second_group)
        );
    }

    #[test]
    /// An invalid second target cancels the gesture without document edits.
    fn add_conditional_click_mismatched_second_click_clears_pending_with_no_mutation() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group_a = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let c = add_cell(&mut doc, "c", CT::i64());
        let d = add_cell(&mut doc, "d", CT::i64());
        let c_node = add_cell_node(&mut doc, c, Point::new(0.0, 20.0));
        let d_node = add_cell_node(&mut doc, d, Point::new(10.0, 20.0));
        let group_b = create_relationship(&mut doc, c_node, d_node, Point::new(5.0, 25.0));

        let pending = add_conditional_click(
            &mut doc,
            Some(group_a),
            NodeId::RelationshipGroup(group_b),
            Point::new(5.0, 25.0),
        );

        assert_eq!(pending, ClickSequenceOutcome::NoOp);
        assert!(doc.conditional_groups_in_order().next().is_none());
    }

    #[test]
    /// The first cell click begins a relationship connection.
    fn add_relationship_click_first_click_on_a_cell_sets_pending() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));

        let pending = add_relationship_click(&mut doc, None, NodeId::CellNode(a_node));

        assert_eq!(
            pending,
            ClickSequenceOutcome::Pending(NodeId::CellNode(a_node))
        );
        assert!(doc.relationship_groups_in_order().next().is_none());
    }

    #[test]
    /// Connecting two cells creates a relationship at their midpoint.
    fn add_relationship_click_second_click_on_a_cell_creates_a_group_at_the_midpoint() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 20.0));

        let pending = add_relationship_click(
            &mut doc,
            Some(NodeId::CellNode(a_node)),
            NodeId::CellNode(b_node),
        );

        assert_eq!(pending, ClickSequenceOutcome::Completed);
        let (_, group) = doc.relationship_groups_in_order().next().unwrap();
        assert_eq!(group.position, Point::new(5.0, 10.0));
    }

    #[test]
    /// Connecting a cell to a relationship adds its membership.
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
        );

        assert_eq!(pending, ClickSequenceOutcome::Completed);
        assert_eq!(doc.relationship_groups[group].members.len(), 3);
    }

    #[test]
    /// Relationship connections ignore noncell starting targets.
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

        let pending = add_relationship_click(&mut doc, None, NodeId::RelationshipGroup(group));

        assert_eq!(pending, ClickSequenceOutcome::NoOp);
        assert_eq!(doc.relationship_groups[group].members.len(), 2);
    }

    #[test]
    /// Position queries return a cell placement's stored canvas coordinates.
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
    /// Rectangular selection includes only enclosed node centers.
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
    /// Rectangular selection is independent of drag direction.
    fn nodes_in_rect_handles_corners_given_in_either_order() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let inside = add_cell_node(&mut doc, a, Point::new(5.0, 5.0));

        // corner_a is bottom-right, corner_b is top-left — should still work.
        let found = nodes_in_rect(&doc, Point::new(10.0, 10.0), Point::new(0.0, 0.0));

        assert!(found.contains(&NodeId::CellNode(inside)));
    }

    #[test]
    /// Identity transforms preserve both screen and canvas coordinates.
    fn identity_transform_is_a_no_op_both_ways() {
        let t = ViewTransform::identity();
        let p = Point::new(3.0, 4.0);
        assert_eq!(screen_to_canvas(&t, p), p);
        assert_eq!(canvas_to_screen(&t, p), p);
    }

    #[test]
    /// Valid view transforms invert their coordinate conversion.
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
    /// Coordinate conversion accounts for both pan offsets and zoom.
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
    /// Panning updates offsets without changing scale.
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
    /// Cursor-centered zoom preserves the canvas point under the cursor.
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
    /// Zoom applies the requested multiplicative scale change.
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
    /// A zero wheel delta leaves zoom unchanged.
    fn wheel_zoom_delta_of_zero_is_a_no_op_multiplier() {
        assert!((wheel_zoom_delta(0.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    /// Large positive wheel deltas respect the lower zoom multiplier.
    fn wheel_zoom_delta_clamps_a_large_positive_delta_y_to_the_lower_bound() {
        // A large positive delta_y (scroll down, zoom out) clamps to 1.0 - 0.5.
        assert!((wheel_zoom_delta(100_000.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    /// Large negative wheel deltas respect the upper zoom multiplier.
    fn wheel_zoom_delta_clamps_a_large_negative_delta_y_to_the_upper_bound() {
        // A large negative delta_y (scroll up, zoom in) clamps to 1.0 + 0.5.
        assert!((wheel_zoom_delta(-100_000.0) - 1.5).abs() < 1e-9);
    }

    #[test]
    /// Selected nodes use the selection stroke color.
    fn node_stroke_selected_node_returns_red() {
        assert_eq!(node_stroke(true), "red");
    }

    #[test]
    /// Unselected nodes use the ordinary stroke color.
    fn node_stroke_unselected_node_returns_black() {
        assert_eq!(node_stroke(false), "black");
    }

    #[test]
    /// Hit testing finds points inside a cell's visible bounds.
    fn hit_test_finds_a_cell_node_near_its_position() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(100.0, 100.0));
        let t = ViewTransform::identity();

        let hit = hit_test(&doc, &t, Point::new(105.0, 102.0));

        assert_eq!(hit, Some(NodeId::CellNode(node)));
    }

    #[test]
    /// Empty canvas points produce no hit target.
    fn hit_test_returns_none_for_empty_canvas_area() {
        let doc = Document::new("demo");
        let t = ViewTransform::identity();
        assert_eq!(hit_test(&doc, &t, Point::new(500.0, 500.0)), None);
    }

    #[test]
    /// Short labels retain the minimum visible cell width.
    fn cell_label_half_width_has_a_minimum_for_short_names() {
        assert_eq!(cell_label_half_width("a"), 40.0);
    }

    #[test]
    /// Long labels expand their visible cell bounds.
    fn cell_label_half_width_widens_for_long_names() {
        let long_name = "a_very_long_cell_name_indeed";
        assert!(cell_label_half_width(long_name) > 40.0);
    }

    #[test]
    /// Long-label hit bounds extend beyond the minimum box width.
    fn hit_test_finds_a_cell_with_a_long_name_beyond_the_minimum_box_width() {
        let mut doc = Document::new("demo");
        let long_name = "a_very_long_cell_name_indeed";
        let a = add_cell(&mut doc, long_name, CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let t = ViewTransform::identity();
        let half_width = cell_label_half_width(long_name);
        assert!(half_width > 40.0, "test assumes this name widens the box");

        // This lies beyond the old fixed box and inside the widened label box.
        let hit = hit_test(&doc, &t, Point::new(60.0, 0.0));
        assert_eq!(hit, Some(NodeId::CellNode(node)));

        // Just past the actual (widened) box edge should miss.
        let miss = hit_test(&doc, &t, Point::new(half_width + 1.0, 0.0));
        assert_eq!(miss, None);
    }

    #[test]
    /// Node dragging applies its canvas-space displacement.
    fn apply_drag_delta_moves_a_cell_node() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(10.0, 10.0));

        apply_drag_delta(&mut doc, NodeId::CellNode(node), Point::new(5.0, -3.0));

        assert_eq!(doc.cell_nodes[node].position, Point::new(15.0, 7.0));
    }

    #[test]
    /// Dragging a node starts node movement.
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
    /// Node hits take precedence over shift-based rubber-band selection.
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
    /// Ordinary empty-canvas drags pan the view.
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
    /// Shift-dragging empty canvas starts rectangular selection.
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
    /// Node movement does not display a rubber-band rectangle.
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
    /// View panning does not display a rubber-band rectangle.
    fn rubber_band_rect_returns_none_for_pan_mode() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::Pan {
            last_screen: Point::new(0.0, 0.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), None);
    }

    #[test]
    /// Idle canvases have no rubber-band rectangle.
    fn rubber_band_rect_returns_none_when_no_drag_is_active() {
        let t = ViewTransform::identity();
        assert_eq!(rubber_band_rect(None, &t), None);
    }

    #[test]
    /// Rubber-band rectangles span both gesture endpoints.
    fn rubber_band_rect_computes_bounds_from_corners() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::RubberBand {
            start_canvas: Point::new(10.0, 20.0),
            current_canvas: Point::new(30.0, 50.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), Some((10.0, 20.0, 20.0, 30.0)));
    }

    #[test]
    /// Rubber-band bounds are independent of drag direction.
    fn rubber_band_rect_handles_corners_given_in_either_order() {
        let t = ViewTransform::identity();
        let mode = Some(DragMode::RubberBand {
            start_canvas: Point::new(30.0, 50.0),
            current_canvas: Point::new(10.0, 20.0),
        });
        assert_eq!(rubber_band_rect(mode, &t), Some((10.0, 20.0, 20.0, 30.0)));
    }

    #[test]
    /// Rubber-band rendering transforms canvas corners into screen coordinates.
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

    #[test]
    /// Duplication copies every selected relationship and ignores selected cells.
    fn duplicate_selection_duplicates_every_selected_relationship_group() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::RelationshipGroup(group));
        let other = create_relationship(&mut doc, a_node, b_node, Point::new(50.0, 50.0));
        selection.insert(NodeId::RelationshipGroup(other));
        selection.insert(NodeId::CellNode(a_node));

        let duplicated = duplicate_selection(&mut doc, &selection, Point::new(0.0, 50.0));

        assert_eq!(duplicated.len(), 2);
        assert_eq!(doc.relationship_groups_in_order().count(), 4);
    }

    #[test]
    /// Selecting only unsupported node kinds leaves duplication unchanged.
    fn duplicate_selection_ignores_non_relationship_group_selections() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));

        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::CellNode(a_node));

        let duplicated = duplicate_selection(&mut doc, &selection, Point::new(0.0, 50.0));

        assert!(duplicated.is_empty());
    }

    #[test]
    /// Deleting a cell placement removes the selected placement.
    fn delete_selection_deletes_a_selected_cell_node() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::CellNode(a_node));

        delete_selection(&mut doc, &selection);

        assert!(!doc.cell_nodes.contains_key(a_node));
        assert!(!doc.cells.contains_key(a));
    }

    #[test]
    /// Deleting a relationship removes that group.
    fn delete_selection_deletes_a_selected_relationship_group() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::RelationshipGroup(group));

        delete_selection(&mut doc, &selection);

        assert!(!doc.relationship_groups.contains_key(group));
        // The member cell nodes are untouched.
        assert!(doc.cell_nodes.contains_key(a_node));
        assert!(doc.cell_nodes.contains_key(b_node));
    }

    #[test]
    /// Deleting a conditional removes that group.
    fn delete_selection_deletes_a_selected_conditional_group() {
        use crate::ops::conditionals::add_conditional_with_formula;

        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "x", CT::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "x > 1.0", Point::new(0.0, 0.0));
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::ConditionalGroup(cond));

        delete_selection(&mut doc, &selection);

        assert!(!doc.conditional_groups.contains_key(cond));
    }

    #[test]
    /// Cascading deletions tolerate other selections becoming stale.
    fn delete_selection_handles_cross_invalidation_without_panicking() {
        // Selecting both a relationship group AND its only two cell nodes:
        // deleting the second cell node cascades to delete the
        // now-emptied group before `delete_selection` ever processes the
        // group's own entry in `selection` — this must not panic
        // regardless of `HashSet`'s iteration order.
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::CellNode(a_node));
        selection.insert(NodeId::CellNode(b_node));
        selection.insert(NodeId::RelationshipGroup(group));

        delete_selection(&mut doc, &selection);

        assert!(!doc.relationship_groups.contains_key(group));
        assert!(!doc.cell_nodes.contains_key(a_node));
        assert!(!doc.cell_nodes.contains_key(b_node));
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::model::cell::CellType;
    use crate::ops::cells::{add_cell, add_cell_node};
    use crate::ops::relationships::create_relationship;

    #[test]
    /// Relationship rendering connects every member placement.
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
        assert!(edges.iter().all(|e| e.style == EdgeStyle::Solid));
    }

    #[test]
    /// Conditional edges deduplicate groups shared by defaults and branches.
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
        let cond_to_group: Vec<_> = edges
            .iter()
            .filter(|e| {
                e.from == doc.conditional_groups[cond].position
                    && e.to == doc.relationship_groups[group].position
            })
            .collect();
        assert_eq!(cond_to_group.len(), 1);
        assert_eq!(cond_to_group[0].style, EdgeStyle::Dashed);
    }

    #[test]
    /// Conditional rendering connects its selector's placed cell.
    fn compute_edges_connects_conditional_group_to_its_condition_cell() {
        use crate::ops::conditionals::add_conditional_from_bool_cells;

        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CellType::Bool);
        let flag_node = add_cell_node(&mut doc, flag, Point::new(50.0, 50.0));
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let cond =
            add_conditional_from_bool_cells(&mut doc, vec![flag], group, Point::new(0.0, 20.0));

        let edges = compute_edges(&doc);
        let cond_position = doc.conditional_groups[cond].position;
        let flag_position = doc.cell_nodes[flag_node].position;
        let cond_to_flag = edges
            .iter()
            .find(|e| e.from == cond_position && e.to == flag_position);
        assert_eq!(cond_to_flag.map(|e| e.style), Some(EdgeStyle::Solid));
    }
}
