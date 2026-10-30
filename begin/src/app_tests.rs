//! Exercises Begin's real component composition and source-selection callbacks.

use super::*;
use dioxus::core::{ElementId, Mutation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Exposes the mounted component's recovery source and displayed provenance.
type Observer = Rc<RefCell<Option<(Signal<ActiveSource>, Signal<ActiveSource>)>>>;

/// Every authored example exercises its documented values or deliberate algebraic-cycle failure.
#[test]
fn authored_examples_preserve_observable_contracts() {
    for (name, source) in crate::example_source::EXAMPLES_WITH_SOURCE {
        let outcome = build_sheet(source, name, &Renderer::plain());
        let (mut parsed, labels) = outcome
            .sheet_labels
            .unwrap_or_else(|| panic!("{name} did not parse"));
        match *name {
            "cycle" => {
                assert!(outcome.error.is_some());
                assert!(matches!(
                    parsed.propagate(),
                    Err(adam_rs::Error::Cycle { .. })
                ));
            }
            "diamond" => {
                assert!(outcome.error.is_none());
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(), 2.0);
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["d"].0).unwrap(), 3.0);
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["b"].0).unwrap(), 1.5);
            }
            "diamond-wing" => {
                assert!(outcome.error.is_none());
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["b"].0).unwrap(), 3.0);
                assert_eq!(
                    *parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(),
                    2.0 / 3.0
                );
            }
            "inequality" => {
                assert!(outcome.error.is_none());
                let a = *parsed.read::<i32>(parsed.cell_names["a"].0).unwrap();
                let b = *parsed.read::<i32>(parsed.cell_names["b"].0).unwrap();
                let c = *parsed.read::<i32>(parsed.cell_names["c"].0).unwrap();
                assert!(a <= b && b <= c && c <= 100);
            }
            "out-cell" => {
                assert!(outcome.error.is_none());
                assert_eq!(
                    (labels.cells[&parsed.output_names["result"]].display)(&parsed),
                    "(0.0, 0.0)"
                );
            }
            "image_resize" => {
                assert!(outcome.error.is_none());
                assert_eq!(
                    *parsed
                        .read::<i64>(parsed.output_names["byte_count"])
                        .unwrap(),
                    61_440_000
                );
                let width = parsed.cell_names["width_pixels"].0;
                parsed.write(width, 800i32).unwrap();
                parsed.propagate().unwrap();
                assert_eq!(
                    *parsed
                        .read::<i32>(parsed.cell_names["height_pixels"].0)
                        .unwrap(),
                    600
                );
                assert_eq!(
                    *parsed
                        .read::<i64>(parsed.output_names["byte_count"])
                        .unwrap(),
                    15_360_000
                );
            }
            "toy_example" => {
                assert!(outcome.error.is_none());
                let value = |name: &str| *parsed.read::<f64>(parsed.cell_names[name].0).unwrap();
                assert_eq!(value("c"), value("a") * value("b"));
                assert_eq!(value("f"), value("d") * value("e"));
                assert_eq!(value("c"), value("f"));
                assert_eq!(*parsed.read::<i32>(parsed.cell_names["p"].0).unwrap(), 0);
            }
            _ => panic!("add an observable contract for new example {name}"),
        }
    }
}

/// Responses cannot outlive a source owner or supersede a newer file request.
#[cfg(not(feature = "desktop"))]
#[test]
fn file_replies_require_current_source_and_request() {
    let first = FileSourceOwner(Rc::new(()));
    let same_diagnostic_name_new_source = FileSourceOwner(Rc::new(()));
    assert!(file_reply_is_current(&first, &first, 2, 2));
    assert!(!file_reply_is_current(&first, &first, 1, 2));
    assert!(!file_reply_is_current(
        &first,
        &same_diagnostic_name_new_source,
        2,
        2
    ));
}

/// Mounts a picker or platform-specific open controls over a real parsed sheet.
fn host(picker: bool) -> Element {
    let (parsed, labels, active_source, displayed_source) = use_hook(|| {
        let (parsed, labels, mut source) = load_example("diamond");
        source.origin = SourceOrigin::Opened("diamond".into());
        (
            Signal::new(parsed),
            Signal::new(labels),
            Signal::new(source.clone()),
            Signal::new(source),
        )
    });
    let (observer, callbacks): (Observer, Rc<Cell<usize>>) = consume_context();
    *observer.borrow_mut() = Some((active_source, displayed_source));
    let names = use_signal(|| vec!["diamond".to_string(), "image_resize".to_string()]);
    #[cfg(not(feature = "desktop"))]
    let refresh_handle = use_signal(|| Some(7));
    #[cfg(feature = "desktop")]
    let reload_tx = use_signal(|| futures_channel::mpsc::unbounded::<()>().0);
    #[cfg(feature = "desktop")]
    let watcher_slot = use_signal(|| None);
    if picker {
        rsx! { ExamplesPicker {
            parsed, labels, active_source, displayed_source, example_names: names,
            on_select: move |_| callbacks.set(callbacks.get() + 1),
        } }
    } else {
        #[cfg(not(feature = "desktop"))]
        {
            rsx! { OpenFileControls { parsed, labels, active_source, displayed_source, refresh_handle } }
        }
        #[cfg(feature = "desktop")]
        {
            rsx! { OpenFileControls { parsed, labels, active_source, displayed_source, reload_tx, watcher_slot } }
        }
    }
}

/// Creates the normal VirtualDom while observing production-owned signals.
fn mount(picker: bool) -> (VirtualDom, Vec<ElementId>, Observer, Rc<Cell<usize>>) {
    let observer: Observer = Rc::new(RefCell::new(None));
    let callbacks = Rc::new(Cell::new(0));
    let mut dom = VirtualDom::new_with_props(host, picker);
    dom.provide_root_context((observer.clone(), callbacks.clone()));
    let clicks = dom
        .rebuild_to_vec()
        .edits
        .into_iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(id),
            _ => None,
        })
        .collect();
    (dom, clicks, observer, callbacks)
}

/// Selecting an example replaces both source identities and notifies the owning application once.
#[test]
fn examples_picker_dispatches_real_selection() {
    let (mut dom, clicks, observer, callbacks) = mount(true);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("image_resize") && html.contains("diamond"));
    assert!(
        !html.contains("selected=\"true\""),
        "an opened same-name file is not an example selection"
    );
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
        clicks[1],
    );
    dom.render_immediate_to_vec();
    let (active, displayed) = observer.borrow().unwrap();
    assert_eq!(active.peek().name, "image_resize");
    assert_eq!(displayed.peek().text, active.peek().text);
    assert!(matches!(active.peek().origin, SourceOrigin::Example));
    assert_eq!(callbacks.get(), 1);
    assert!(dioxus_ssr::render(&dom).contains("selected=\"true\""));
}

/// Open controls expose the opened source and only web builds offer explicit refresh.
#[test]
fn open_controls_render_platform_actions_and_source() {
    let (dom, clicks, _, _) = mount(false);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Open…") && html.contains("diamond"));
    #[cfg(feature = "desktop")]
    {
        assert_eq!(clicks.len(), 1);
        assert!(!html.contains("Refresh"));
    }
    #[cfg(not(feature = "desktop"))]
    {
        assert_eq!(clicks.len(), 2);
        assert!(html.contains("Refresh"));
    }
}

/// The application composes the actual graph, Inspector, picker, and legend.
#[cfg(not(feature = "desktop"))]
#[test]
fn app_renders_all_main_adapters() {
    let mut dom = VirtualDom::new(App);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    for expected in ["<sp-theme", "Examples", "Cells", "graph-legend", "Open…"] {
        assert!(html.contains(expected), "missing {expected}");
    }
}
