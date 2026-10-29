//! Exercises rendered editor components and real Dioxus event dispatch.

use super::{canvas::*, side_panel::SidePanel, toolbar::*};
use crate::model::{cell::CellType, document::Document, geometry::Point};
use crate::ops::{cells::*, conditionals::*, relationships::*};
use dioxus::core::{ElementId, Mutation};
use dioxus::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

/// Exposes the mounted component's actual document, selection, and mode.
type State = (Signal<Document>, Signal<HashSet<NodeId>>, Signal<Tool>);
/// Shares scope-owned signals with the test without constructing a second model.
type Observer = Rc<RefCell<Option<State>>>;
/// Pairs a mounted DOM and its registered events with observed state and render count.
type Mounted = (
    VirtualDom,
    Vec<(String, ElementId)>,
    Observer,
    Rc<Cell<usize>>,
);

/// Builds an editable graph with a numeric cell, malformed formula, and Boolean conditional.
fn fixture() -> Document {
    let mut doc = Document::new("component_fixture");
    let a = add_cell(&mut doc, "amount", CellType::i64());
    let b = add_cell(&mut doc, "enabled", CellType::Bool);
    let na = add_cell_node(&mut doc, a, Point::new(40.0, 60.0));
    let nb = add_cell_node(&mut doc, b, Point::new(240.0, 60.0));
    let group = create_relationship(&mut doc, na, nb, Point::new(140.0, 160.0));
    set_member_formula(&mut doc, group, na, "amount +");
    let _ = add_conditional_from_bool_cells(&mut doc, vec![b], group, Point::new(340.0, 160.0));
    doc
}

/// Mounts one editor adapter over a shared real document.
fn host(mode: u8) -> Element {
    let document = use_signal(move || {
        let mut document = fixture();
        if mode == 6 {
            let id = document.cell_order[0];
            document.cells[id].ty = CellType::f64();
        }
        document
    });
    let selection = use_signal(|| {
        let doc = document.peek();
        match mode {
            0 | 4 | 6 => HashSet::from([NodeId::CellNode(doc.cell_nodes.keys().next().unwrap())]),
            1 | 3 => HashSet::from([NodeId::RelationshipGroup(doc.relationship_group_order[0])]),
            2 => HashSet::from([NodeId::ConditionalGroup(doc.conditional_group_order[0])]),
            _ => HashSet::new(),
        }
    });
    let active_tool = use_signal(|| Tool::AddRelationship);
    let transform = use_signal(|| ViewTransform {
        x: 10.0,
        y: 20.0,
        k: 2.0,
    });
    let (observed, renders): (Observer, Rc<Cell<usize>>) = consume_context();
    renders.set(renders.get() + 1);
    *observed.borrow_mut() = Some((document, selection, active_tool));
    let _ = *active_tool.read();
    match mode {
        3 => rsx! { Toolbar { active_tool, document, selection } },
        4 => rsx! { Canvas { document, selection, active_tool, view_transform: transform } },
        _ => rsx! { SidePanel { document, selection } },
    }
}

/// Mounts a standard VirtualDom and returns its real event registrations and observable state.
fn mount(mode: u8) -> Mounted {
    let observed: Observer = Rc::new(RefCell::new(None));
    let renders = Rc::new(Cell::new(0));
    let mut dom = VirtualDom::new_with_props(host, mode);
    dom.provide_root_context((observed.clone(), renders.clone()));
    let events = dom
        .rebuild_to_vec()
        .edits
        .into_iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } => Some((name, id)),
            _ => None,
        })
        .collect();
    (dom, events, observed, renders)
}

/// Dispatches a real click through Dioxus's standard serialized event converter.
fn click(dom: &VirtualDom, id: ElementId) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    dom.runtime().handle_event(
        "click",
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(
                dioxus_html::SerializedMouseData::default(),
            ))),
            true,
        )
        .into_any(),
        id,
    );
}

/// Dispatches a standard mouse event with explicit viewport coordinates.
fn mouse_at(dom: &VirtualDom, name: &str, id: ElementId, point: Point) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let coordinates = dioxus::html::geometry::Coordinates::new(
        (point.x, point.y).into(),
        (point.x, point.y).into(),
        (point.x, point.y).into(),
        (point.x, point.y).into(),
    );
    let data = dioxus_html::SerializedMouseData::new(
        Some(dioxus::html::input_data::MouseButton::Primary),
        Default::default(),
        coordinates,
        Default::default(),
    );
    dom.runtime().handle_event(
        name,
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
            true,
        )
        .into_any(),
        id,
    );
}

/// The selected integer cell renders exact bound controls and edits its underlying output flag.
#[test]
fn cell_panel_dispatches_output_toggle() {
    let (mut dom, events, observed, _) = mount(0);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("cell-panel") && html.contains("amount"));
    assert_eq!(html.matches("<sp-number-field").count(), 0);
    assert_eq!(html.matches("<sp-textfield").count(), 4);
    let (_, id) = events.iter().find(|(name, _)| name == "click").unwrap();
    click(&dom, *id);
    dom.render_immediate_to_vec();
    let (document, _, _) = observed.borrow().unwrap();
    let doc = document.peek();
    assert!(doc.cells[doc.cell_order[0]].output);
}

/// A relationship selection renders member names, exact drafts, and an actual parse diagnostic.
#[test]
fn relationship_panel_renders_member_error() {
    let (dom, _, _, _) = mount(1);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("relationship-panel") && html.contains("amount +"));
    assert!(html.contains("diagnostic") && html.contains("invalid=\"true\""));
    assert!(html.contains("enabled"));
}

/// The first render includes current conditional columns and unchecked columns survive a click.
#[test]
fn conditional_panel_renders_and_retains_unchecked_live_group() {
    let (mut dom, events, observed, _) = mount(2);
    assert!(dioxus_ssr::render(&dom).contains("enable-table"));
    assert_eq!(dioxus_ssr::render(&dom).matches("<sp-checkbox").count(), 2);
    let id = events.iter().rfind(|(name, _)| name == "click").unwrap().1;
    click(&dom, id);
    dom.render_immediate_to_vec();
    let (document, _, _) = observed.borrow().unwrap();
    let doc = document.peek();
    assert!(
        doc.conditional_groups[doc.conditional_group_order[0]].branches[1]
            .enabled_groups
            .is_empty()
    );
    assert_eq!(dioxus_ssr::render(&dom).matches("<sp-checkbox").count(), 2);
}

/// Toolbar clicks preserve same-mode gestures and dispatch one-shot duplication without changing mode.
#[test]
fn toolbar_dispatches_modes_and_duplicate() {
    let (mut dom, events, observed, renders) = mount(3);
    let clicks: Vec<_> = events
        .iter()
        .filter(|(name, _)| name == "click")
        .map(|(_, id)| *id)
        .collect();
    assert_eq!(clicks.len(), 4);
    let before = renders.get();
    click(&dom, clicks[1]);
    dom.render_immediate_to_vec();
    assert_eq!(renders.get(), before);
    click(&dom, clicks[3]);
    dom.render_immediate_to_vec();
    let (document, selection, active_tool) = observed.borrow().unwrap();
    assert_eq!(*active_tool.peek(), Tool::AddRelationship);
    assert_eq!(document.peek().relationship_groups.len(), 2);
    assert_eq!(selection.peek().len(), 1);
    let duplicate = *selection.peek().iter().next().unwrap();
    assert_ne!(
        duplicate,
        NodeId::RelationshipGroup(document.peek().relationship_group_order[0])
    );
    click(&dom, clicks[0]);
    assert_eq!(*active_tool.peek(), Tool::Select);
}

/// Canvas rendering applies the view transform but keeps screen-space node extents fixed.
#[test]
fn canvas_renders_transformed_geometry_and_selection() {
    let (dom, events, _, _) = mount(4);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("rotate(45 690 340)"));
    assert!(html.contains("cx=\"290\"") && html.contains("cy=\"340\""));
    assert!(html.contains("fill=\"lightblue\" stroke=\"red\""));
    for event in ["mousedown", "mousemove", "mouseup", "wheel", "keydown"] {
        assert!(
            events.iter().any(|(name, _)| name == event),
            "missing {event}"
        );
    }
}

/// Real canvas events complete connections and move a selected node in canvas coordinates.
#[test]
fn canvas_dispatches_connection_and_drag() {
    let (mut dom, events, observed, _) = mount(4);
    dom.render_immediate_to_vec();
    let event_id = |name| events.iter().find(|(event, _)| event == name).unwrap().1;
    let (document, _, active_tool) = observed.borrow().unwrap();
    for (index, point) in [Point::new(90.0, 140.0), Point::new(490.0, 140.0)]
        .into_iter()
        .enumerate()
    {
        mouse_at(&dom, "mousedown", event_id("mousedown"), point);
        dom.render_immediate_to_vec();
        assert_eq!(document.peek().relationship_groups.len(), index + 1);
    }
    assert_eq!(*active_tool.peek(), Tool::Select);
    mouse_at(
        &dom,
        "mousedown",
        event_id("mousedown"),
        Point::new(90.0, 140.0),
    );
    dom.render_immediate_to_vec();
    mouse_at(
        &dom,
        "mousemove",
        event_id("mousemove"),
        Point::new(110.0, 160.0),
    );
    mouse_at(
        &dom,
        "mouseup",
        event_id("mouseup"),
        Point::new(110.0, 160.0),
    );
    let doc = document.peek();
    assert_eq!(
        doc.cell_nodes.values().next().unwrap().position,
        Point::new(50.0, 70.0)
    );
}

/// Replacement retires actual Canvas gestures even when the new document reuses its slot keys.
#[test]
fn workspace_replacement_retires_pending_drag_and_connection() {
    /// Observes production replacement inputs without constructing a second editor composition.
    type ReplacementObserver = Rc<RefCell<Option<(State, Signal<u64>)>>>;
    /// Mounts the same workspace function as App with externally observable owner signals.
    fn replacement_host((connecting, observed): (bool, ReplacementObserver)) -> Element {
        let document = use_signal(fixture);
        let selection = use_signal(HashSet::new);
        let active_tool = use_signal(move || {
            if connecting {
                Tool::AddRelationship
            } else {
                Tool::Select
            }
        });
        let transform = use_signal(|| ViewTransform {
            x: 10.0,
            y: 20.0,
            k: 2.0,
        });
        let epoch = use_signal(|| 0u64);
        *observed.borrow_mut() = Some(((document, selection, active_tool), epoch));
        super::app::editor_workspace(
            document,
            transform,
            selection,
            active_tool,
            *epoch.read(),
            rsx! {},
        )
    }
    for connecting in [false, true] {
        let observed: ReplacementObserver = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new_with_props(replacement_host, (connecting, observed.clone()));
        let initial = dom.rebuild_to_vec();
        let event_id = |name| {
            initial
                .edits
                .iter()
                .find_map(|edit| match edit {
                    Mutation::NewEventListener { name: event, id } if event == name => Some(*id),
                    _ => None,
                })
                .unwrap()
        };
        dom.render_immediate_to_vec();
        mouse_at(
            &dom,
            "mousedown",
            event_id("mousedown"),
            Point::new(90.0, 140.0),
        );
        dom.render_immediate_to_vec();
        let ((document, selection, mut active_tool), epoch) = observed.borrow().unwrap();
        let replacement = fixture();
        assert_eq!(document.peek().cell_order[0], replacement.cell_order[0]);
        super::app::replace_editor_document(
            document,
            selection,
            active_tool,
            epoch,
            replacement.clone(),
        );
        let changes = dom.render_immediate_to_vec();
        let remounted = |name| {
            changes.edits.iter().find_map(|edit| match edit {
                Mutation::NewEventListener { name: event, id } if event == name => Some(*id),
                _ => None,
            })
        };
        if connecting {
            active_tool.set(Tool::AddRelationship);
            dom.render_immediate_to_vec();
            mouse_at(
                &dom,
                "mousedown",
                remounted("mousedown").unwrap_or(event_id("mousedown")),
                Point::new(490.0, 140.0),
            );
        } else {
            mouse_at(
                &dom,
                "mousemove",
                remounted("mousemove").unwrap_or(event_id("mousemove")),
                Point::new(110.0, 160.0),
            );
        }
        assert_eq!(
            *document.peek(),
            replacement,
            "old gesture mutated the replacement: connecting={connecting}"
        );
        assert!(
            remounted("mousemove").is_some(),
            "Canvas listeners must belong to a new scope"
        );
    }
}

#[path = "numeric_event_tests.rs"]
mod numeric_events;

/// An empty editor composes its toolbar and canvas without an input-blocking side panel.
#[cfg(not(feature = "desktop"))]
#[test]
fn app_renders_editor_adapters() {
    let mut dom = VirtualDom::new(super::App);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("<sp-theme") && html.contains("<svg"));
    assert!(html.contains("Add Relationship"));
    assert!(!html.contains("side-panel"));
}
