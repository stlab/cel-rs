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
        let mut linked_groups: Vec<_> = cond.default.to_vec();
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
/// When `active_tool` is `Tool::AddConditional`, a mousedown on a
/// relationship-group node starts a drag (tracked internally, not via
/// `DragMode`); a mouseup over any node then completes the gesture via
/// `add_conditional_drag`, wrapping the source group in a new conditional
/// group. Zoom (mouse wheel, via `zoom_at`) works regardless of
/// `active_tool`.
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
#[component]
pub fn Canvas(
    document: Signal<Document>,
    view_transform: Signal<ViewTransform>,
    selection: Signal<HashSet<NodeId>>,
    active_tool: Signal<crate::ui::toolbar::Tool>,
) -> Element {
    let mut drag_mode = use_signal(|| None::<DragMode>);
    let mut pending_first_click = use_signal(|| None::<NodeId>);
    let mut pending_conditional_source = use_signal(|| None::<RelationshipGroupId>);

    // Add-Relationship's click-sequence state spans two separate mousedown
    // events (click A, then later click B), unlike `pending_conditional_source`
    // (a single mousedown-to-mouseup drag). So it can't be reset at the top
    // of `onmousedown` the same way — that handler's own `AddRelationship`
    // arm reads `pending_first_click` back a few lines later to decide
    // whether the current click is the first or second of the gesture, and
    // an unconditional reset there would make every click look like a fresh
    // first click, breaking the tool entirely. Instead, clear it whenever
    // `active_tool` changes (via this effect) so switching away and back
    // between click A and click B can't resume a stale first click, while a
    // same-tool two-click sequence (no tool change in between) is left
    // untouched and completes normally.
    use_effect(move || {
        active_tool.read();
        pending_first_click.set(None);
    });

    let doc = document.read();
    let transform = *view_transform.read();
    let sel = selection.read();

    let edges = compute_edges(&doc);
    let band = rubber_band_rect(*drag_mode.read(), &transform);

    rsx! {
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
                let data = evt.data();
                let client_pt = data.client_coordinates();
                let screen_point = Point::new(client_pt.x, client_pt.y);
                let shift_held = data.modifiers().shift();
                let transform = *view_transform.read();

                // Any new mousedown invalidates a previous, uncompleted
                // Add-Conditional drag's pending source (e.g. the user
                // switched tools, or released the mouse outside the SVG,
                // so the AddConditional arm of `onmouseup` never ran to
                // clear it) — reset unconditionally before dispatching so
                // a later, unrelated AddConditional gesture can never fire
                // against a stale group. Unlike `pending_first_click` (see
                // the `use_effect` above this `rsx!` block), nothing in this
                // handler reads `pending_conditional_source` back, so an
                // unconditional reset here is safe.
                pending_conditional_source.set(None);

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
                            let new_pending = add_relationship_click(
                                &mut document.write(),
                                *pending_first_click.read(),
                                clicked,
                            );
                            pending_first_click.set(new_pending);
                        }
                    }
                    crate::ui::toolbar::Tool::AddConditional => {
                        let hit = hit_test(&document.read(), &transform, screen_point);
                        if let Some(NodeId::RelationshipGroup(group)) = hit {
                            pending_conditional_source.set(Some(group));
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
            onmouseup: move |evt: Event<MouseData>| {
                if let Some(DragMode::RubberBand { start_canvas, current_canvas }) = *drag_mode.read() {
                    let found = nodes_in_rect(&document.read(), start_canvas, current_canvas);
                    *selection.write() = found.into_iter().collect();
                }
                drag_mode.set(None);

                if *active_tool.read() == crate::ui::toolbar::Tool::AddConditional {
                    let source = *pending_conditional_source.read();
                    if let Some(group) = source {
                        let data = evt.data();
                        let client_pt = data.client_coordinates();
                        let mouseup_screen_point = Point::new(client_pt.x, client_pt.y);
                        let transform = *view_transform.read();
                        let hit = hit_test(&document.read(), &transform, mouseup_screen_point);
                        if let Some(target) = hit {
                            let _ = add_conditional_drag(
                                &mut document.write(),
                                group,
                                target,
                                screen_to_canvas(&transform, mouseup_screen_point),
                            );
                        }
                        pending_conditional_source.set(None);
                    }
                }
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

/// The half-height (in canvas units) treated as "on" a cell node for hit
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
        let half_width = cell_label_half_width(&doc.cells[node.cell].name);
        if dx <= half_width && dy <= CELL_HALF_HEIGHT {
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

/// Converts a wheel event's `delta_y` into a `zoom_at` multiplier, clamped so a
/// single large scroll tick can't invert or zero the scale.
fn wheel_zoom_delta(delta_y: f64) -> f64 {
    1.0 + (-delta_y * 0.001).clamp(-0.5, 0.5)
}

/// Advances the Add-Relationship tool's click sequence: given whatever was
/// clicked previously (`pending_first_click`, `None` if this is a fresh
/// sequence) and what was just clicked (`clicked`), either creates a new
/// relationship group, extends an existing one, or does nothing (a bare
/// first click on something other than a cell), returning the new pending
/// state.
///
/// - `(None, cell)` → pending becomes `Some(cell)`.
/// - `(Some(cell_a), cell_b)` → creates a relationship binding both, placed
///   at the midpoint between the two cells' own node positions (not either
///   click position), returns `None`.
/// - `(Some(cell), group)` or `(Some(group), cell)` → adds `cell` as a
///   member of `group`, returns `None`.
/// - Any other combination (e.g. a bare first click on a group, or two
///   groups) → returns `None` with no mutation — not a meaningful gesture.
pub fn add_relationship_click(
    doc: &mut Document,
    pending_first_click: Option<NodeId>,
    clicked: NodeId,
) -> Option<NodeId> {
    match (pending_first_click, clicked) {
        (None, NodeId::CellNode(_)) => Some(clicked),
        (None, _) => None,
        (Some(NodeId::CellNode(a)), NodeId::CellNode(b)) => {
            let pa = doc.cell_nodes[a].position;
            let pb = doc.cell_nodes[b].position;
            let midpoint = Point::new((pa.x + pb.x) / 2.0, (pa.y + pb.y) / 2.0);
            let _ = create_relationship(doc, a, b, midpoint);
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

/// Completes an Add-Conditional drag from `group` onto `target`: wraps
/// `group` in a new conditional group at `position`, `Cells`-mode if
/// `target` is a `Bool` cell, `Formula`-mode otherwise.
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
/// Returns `Err` if `target` is not a cell node at all (e.g. dropping onto
/// another relationship or conditional group is not a meaningful gesture
/// for this tool).
pub fn add_conditional_drag(
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
/// and the drag-created single-branch case in [`add_conditional_drag`]
/// both put their "most relevant" branch last (an all-cells-true
/// combination, or the one placeholder branch a fresh drag-created
/// conditional starts with), so this is the most useful default target
/// for a group dragged onto an already-existing conditional. The side
/// panel's enable-table lets the user move it to a different branch
/// afterward.
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
/// - Complexity: O(n) in `selection.len()`.
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
    fn add_conditional_drag_onto_a_bool_cell_creates_a_cells_mode_conditional() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        let result = add_conditional_drag(
            &mut doc,
            group,
            NodeId::CellNode(flag_node),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_ok());
        assert_eq!(doc.conditional_groups_in_order().count(), 1);
    }

    #[test]
    fn add_conditional_drag_onto_a_non_bool_cell_starts_formula_mode() {
        let mut doc = Document::new("demo");
        let threshold = add_cell(&mut doc, "threshold", CT::f64());
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let threshold_node = add_cell_node(&mut doc, threshold, Point::new(0.0, 20.0));

        let result = add_conditional_drag(
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
    fn add_conditional_drag_onto_a_non_cell_target_errors() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let result = add_conditional_drag(
            &mut doc,
            group,
            NodeId::RelationshipGroup(group),
            Point::new(0.0, 40.0),
        );

        assert!(result.is_err());
    }

    #[test]
    fn add_conditional_drag_onto_an_existing_conditional_attaches_the_group() {
        let mut doc = Document::new("demo");
        let flag = add_cell(&mut doc, "flag", CT::Bool);
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let first_group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));
        let flag_node = add_cell_node(&mut doc, flag, Point::new(0.0, 20.0));
        add_conditional_drag(
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

        let result = add_conditional_drag(
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
    fn add_conditional_drag_onto_a_conditional_with_no_branches_errors() {
        use crate::ops::conditionals::add_conditional_with_formula;

        let mut doc = Document::new("demo");
        let x = add_cell(&mut doc, "x", CT::f64());
        let cond = add_conditional_with_formula(&mut doc, vec![x], "x > 1.0", Point::new(0.0, 0.0));
        let a = add_cell(&mut doc, "a", CT::i64());
        let b = add_cell(&mut doc, "b", CT::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let result = add_conditional_drag(
            &mut doc,
            group,
            NodeId::ConditionalGroup(cond),
            Point::new(0.0, 0.0),
        );

        assert!(result.is_err());
    }

    #[test]
    fn add_relationship_click_first_click_on_a_cell_sets_pending() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));

        let pending = add_relationship_click(&mut doc, None, NodeId::CellNode(a_node));

        assert_eq!(pending, Some(NodeId::CellNode(a_node)));
        assert!(doc.relationship_groups_in_order().next().is_none());
    }

    #[test]
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

        assert_eq!(pending, None);
        let (_, group) = doc.relationship_groups_in_order().next().unwrap();
        assert_eq!(group.position, Point::new(5.0, 10.0));
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

        let pending = add_relationship_click(&mut doc, None, NodeId::RelationshipGroup(group));

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
    fn wheel_zoom_delta_of_zero_is_a_no_op_multiplier() {
        assert!((wheel_zoom_delta(0.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn wheel_zoom_delta_clamps_a_large_positive_delta_y_to_the_lower_bound() {
        // A large positive delta_y (scroll down, zoom out) clamps to 1.0 - 0.5.
        assert!((wheel_zoom_delta(100_000.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn wheel_zoom_delta_clamps_a_large_negative_delta_y_to_the_upper_bound() {
        // A large negative delta_y (scroll up, zoom in) clamps to 1.0 + 0.5.
        assert!((wheel_zoom_delta(-100_000.0) - 1.5).abs() < 1e-9);
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
    fn cell_label_half_width_has_a_minimum_for_short_names() {
        assert_eq!(cell_label_half_width("a"), 40.0);
    }

    #[test]
    fn cell_label_half_width_widens_for_long_names() {
        let long_name = "a_very_long_cell_name_indeed";
        assert!(cell_label_half_width(long_name) > 40.0);
    }

    #[test]
    fn hit_test_finds_a_cell_with_a_long_name_beyond_the_minimum_box_width() {
        let mut doc = Document::new("demo");
        let long_name = "a_very_long_cell_name_indeed";
        let a = add_cell(&mut doc, long_name, CellType::i64());
        let node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let t = ViewTransform::identity();
        let half_width = cell_label_half_width(long_name);
        assert!(half_width > 40.0, "test assumes this name widens the box");

        // 39 units out is inside the old fixed 40.0 half-width, but well
        // within this longer name's actual (wider) box.
        let hit = hit_test(&doc, &t, Point::new(39.0, 0.0));
        assert_eq!(hit, Some(NodeId::CellNode(node)));

        // Just past the actual (widened) box edge should miss.
        let miss = hit_test(&doc, &t, Point::new(half_width + 1.0, 0.0));
        assert_eq!(miss, None);
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

    #[test]
    fn duplicate_selection_duplicates_every_selected_relationship_group() {
        let mut doc = Document::new("demo");
        let a = add_cell(&mut doc, "a", CellType::i64());
        let b = add_cell(&mut doc, "b", CellType::i64());
        let a_node = add_cell_node(&mut doc, a, Point::new(0.0, 0.0));
        let b_node = add_cell_node(&mut doc, b, Point::new(10.0, 0.0));
        let group = create_relationship(&mut doc, a_node, b_node, Point::new(5.0, 5.0));

        let mut selection = std::collections::HashSet::new();
        selection.insert(NodeId::RelationshipGroup(group));

        let duplicated = duplicate_selection(&mut doc, &selection, Point::new(0.0, 50.0));

        assert_eq!(duplicated.len(), 1);
        assert_eq!(doc.relationship_groups_in_order().count(), 2);
    }

    #[test]
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

    #[test]
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
        assert!(
            edges
                .iter()
                .any(|e| e.from == cond_position && e.to == flag_position)
        );
    }
}
