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
        let built = match build_sheet(source, name) {
            Err(adam_web_ui::BuildError::Runtime(failure)) if *name == "cycle" => {
                assert!(matches!(failure.error, adam_rs::Error::Cycle { .. }));
                continue;
            }
            Ok(built) => built,
            Err(error) => panic!("{name} failed unexpectedly: {error:?}"),
        };
        let mut parsed = built.parsed;
        let labels = built.labels;
        match *name {
            "diamond" => {
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(), 2.0);
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["d"].0).unwrap(), 3.0);
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["b"].0).unwrap(), 1.5);
            }
            "diamond-wing" => {
                assert_eq!(*parsed.read::<f64>(parsed.cell_names["b"].0).unwrap(), 3.0);
                assert_eq!(
                    *parsed.read::<f64>(parsed.cell_names["c"].0).unwrap(),
                    2.0 / 3.0
                );
            }
            "inequality" => {
                let a = *parsed.read::<i32>(parsed.cell_names["a"].0).unwrap();
                let b = *parsed.read::<i32>(parsed.cell_names["b"].0).unwrap();
                let c = *parsed.read::<i32>(parsed.cell_names["c"].0).unwrap();
                assert!(a <= b && b <= c && c <= 100);
            }
            "out-cell" => {
                assert_eq!(
                    (labels.cells[&parsed.output_names["result"]].display)(&parsed),
                    "(0.0, 0.0)"
                );
            }
            "image_resize" => {
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
    let (model, initial_error, active_source, displayed_source) = use_hook(|| {
        let (sheet, mut source, _) = load_example("diamond");
        let owner = sheet.expect("diamond builds successfully");
        source.origin = SourceOrigin::Opened("diamond".into());
        (
            Signal::new(Some(LiveSheet {
                sheet: SheetHandle::from_owner(owner, EventHandler::new(sheet_outcome)),
            })),
            Signal::new(None),
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
            model, initial_error, active_source, displayed_source, example_names: names,
            on_select: move |_| callbacks.set(callbacks.get() + 1),
        } }
    } else {
        #[cfg(not(feature = "desktop"))]
        {
            rsx! { OpenFileControls { model, initial_error, active_source, displayed_source, refresh_handle } }
        }
        #[cfg(feature = "desktop")]
        {
            rsx! { OpenFileControls { model, initial_error, active_source, displayed_source, reload_tx, watcher_slot } }
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
    assert!(dioxus_ssr::render(&dom).contains("aria-current=\"page\""));
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
    for expected in ["adam-theme", "Examples", "Cells", "graph-legend", "Open…"] {
        assert!(html.contains(expected), "missing {expected}");
    }
}

/// Recovers a mounted application from an initial construction failure.
#[test]
fn mounted_app_recovers_after_initial_build_failure() {
    let observer: AppLifecycleObserver = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new(recovery_host);
    dom.provide_root_context(observer.clone());
    let edits = dom.rebuild_to_vec().edits;

    let initial_html = dioxus_ssr::render(&dom);
    assert!(initial_html.contains("adam-live-error"));
    assert!(!initial_html.contains("graph-container"));
    #[cfg(feature = "desktop")]
    assert!(
        observer
            .borrow()
            .as_ref()
            .is_some_and(|state| state.4.is_some()),
        "the example reload channel remains available after initial failure"
    );

    let click = edits
        .into_iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(id),
            _ => None,
        })
        .expect("test recovery action is mounted");
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
        click,
    );
    dom.render_immediate_to_vec();

    let recovered_html = dioxus_ssr::render(&dom);
    assert!(!recovered_html.contains("adam-live-error"));
    assert!(recovered_html.contains("graph-container"));
}

/// Mounts the initial failure and a test action that applies a successful reload result.
fn recovery_host() -> Element {
    let observer = consume_context::<AppLifecycleObserver>();
    rsx! {
        AppWithExample { initial_name: "__missing_recovery_example".to_string() }
        button {
            onclick: move |_| {
                let (model, error, active, displayed, _reload_tx) = observer
                    .borrow()
                    .as_ref()
                    .expect("the app publishes state before the click")
                    .clone();
                let source = "sheet recovered { cell x: i64 = 10i64; }";
                let owner = build_owner(source, "recovered.adm2")
                    .expect("replacement source builds");
                install_loaded_source(
                    model,
                    error,
                    active,
                    displayed,
                    Some(owner),
                    ActiveSource {
                        name: "cycle".to_string(),
                        text: source.to_string(),
                        origin: SourceOrigin::Example,
                    },
                );
            },
            "Recover"
        }
    }
}

/// Sends one explicit event to the app's production reload consumer.
#[cfg(feature = "desktop")]
#[component]
fn ReloadHost(initial_name: String) -> Element {
    let observer = consume_context::<AppLifecycleObserver>();
    rsx! {
        AppWithExample { initial_name }
        button {
            onclick: move |_| {
                let sender = observer
                    .borrow()
                    .as_ref()
                    .and_then(|state| state.4.clone())
                    .expect("desktop app creates the reload channel");
                sender.unbounded_send(()).expect("reload receiver remains mounted");
            },
            "Reload"
        }
    }
}

/// Removes a temporary example file created by a desktop reload test.
#[cfg(feature = "desktop")]
struct ExampleFileGuard(std::path::PathBuf);

#[cfg(feature = "desktop")]
impl Drop for ExampleFileGuard {
    /// Removes the test fixture when its mounted app test exits.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Reloads a repaired on-disk example after its initial build failed.
#[cfg(feature = "desktop")]
#[test]
fn mounted_app_reloads_repaired_example_after_initial_build_failure() {
    let name = format!("__test_recovery_{}", std::process::id());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(format!("{name}.adm2"));
    let _fixture = ExampleFileGuard(path.clone());
    std::fs::write(&path, "sheet broken { cell x }").unwrap();

    let observer: AppLifecycleObserver = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new_with_props(
        ReloadHost,
        ReloadHostProps {
            initial_name: name.clone(),
        },
    );
    dom.provide_root_context(observer);
    let edits = dom.rebuild_to_vec().edits;
    let initial_html = dioxus_ssr::render(&dom);
    assert!(initial_html.contains("adam-live-error"));
    assert!(initial_html.contains("expected"), "{initial_html}");
    assert!(!initial_html.contains("<input>"), "{initial_html}");
    assert!(!initial_html.contains("graph-container"));

    std::fs::write(&path, "sheet repaired { cell x: i64 = 10i64; }").unwrap();
    let click = edits
        .into_iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(id),
            _ => None,
        })
        .expect("reload action is mounted");
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
        click,
    );
    dom.render_immediate_to_vec();

    let recovered_html = dioxus_ssr::render(&dom);
    assert!(!recovered_html.contains("adam-live-error"));
    assert!(recovered_html.contains("graph-container"));
}

/// Rejected reloads retain the accepted edits, journal, and displayed source text.
#[test]
fn rejected_reload_retains_accepted_edits_and_provenance() {
    /// Exercises the production installation path after an accepted user edit.
    fn host() -> Element {
        use_hook(|| {
            let source = ActiveSource {
                name: "accepted".into(),
                text: "sheet s { cell x: i32 = 1; }".into(),
                origin: SourceOrigin::Example,
            };
            let sheet = SheetHandle::from_owner(
                build_owner(&source.text, &source.file_name()).unwrap(),
                EventHandler::new(sheet_outcome),
            );
            sheet.cell("x").submit("7".into());
            let model = Signal::new(Some(LiveSheet { sheet }));
            let active = Signal::new(source.clone());
            let displayed = Signal::new(source.clone());
            install_loaded_source(
                model,
                Signal::new(None),
                active,
                displayed,
                None,
                ActiveSource {
                    text: "invalid".into(),
                    ..source.clone()
                },
            );
            assert_eq!(displayed.peek().text, source.text);
            assert_eq!(sheet.cell("x").view().unwrap().text, "7");
            assert_eq!(
                sheet
                    .with_accepted_state(|state| state.transactions.len())
                    .unwrap(),
                1
            );
        });
        rsx! {}
    }
    VirtualDom::new(host).rebuild_in_place();
}

/// False requirements restore the accepted journal exactly once and retire old bindings.
#[test]
fn false_requirement_restores_accepted_history_once() {
    /// Counts actual factory calls while the production callback handles rejection.
    fn host() -> Element {
        use_hook(|| {
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let owner = SheetOwner::new(Rc::new(move || {
                observed.set(observed.get() + 1);
                build_sheet("sheet s { source x: i32 = 1; source valid: bool = true require { @limit x < 10; }; }", "require.adm2")
            }), "require/v1".into()).unwrap();
            let sheet = SheetHandle::from_owner(owner, EventHandler::new(sheet_outcome));
            sheet.cell("x").submit("3".into());
            let old = sheet.cell("x");
            old.submit("20".into());
            assert_eq!(calls.get(), 2);
            assert_eq!(sheet.cell("x").view().unwrap().text, "3");
            assert_eq!(
                sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
                1
            );
            old.submit("5".into());
            assert_eq!(sheet.cell("x").view().unwrap().text, "3");
            assert_eq!(calls.get(), 2);
        });
        rsx! {}
    }

    VirtualDom::new(host).rebuild_in_place();
}

/// A recording failure keeps the accepted publication and journal without rebuilding.
#[test]
fn recording_failure_keeps_accepted_transaction() {
    /// Makes the recording-only capture fail after publication succeeds.
    fn host() -> Element {
        use_hook(|| {
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let captures = Rc::new(Cell::new(0));
            let owner = SheetOwner::new(
                Rc::new(move || {
                    observed.set(observed.get() + 1);
                    let mut built = build_sheet("sheet s { cell x: i32 = 1; }", "recording.adm2")?;
                    let id = built.keys["x"];
                    let captures = captures.clone();
                    built.labels.cells.get_mut(&id).unwrap().capture = Box::new(move |parsed| {
                        captures.set(captures.get() + 1);
                        if captures.get() == 4 {
                            Err(adam_web_ui::input::InputError::UnsupportedEditor(
                                "recording fixture".into(),
                            ))
                        } else {
                            Ok(parsed.read::<i32>(id).unwrap().to_string())
                        }
                    });
                    Ok(built)
                }),
                "recording/v1".into(),
            )
            .unwrap();
            let sheet = SheetHandle::from_owner(owner, EventHandler::new(sheet_outcome));
            sheet.start_recording(&["x"]).unwrap();
            sheet.cell("x").submit("7".into());
            assert!(sheet.inspect(|_, _| ()).is_ok());
            assert_eq!(sheet.cell("x").view().unwrap().text, "7");
            assert_eq!(
                sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
                1
            );
            assert_eq!(calls.get(), 1);
            assert!(sheet.last_recording().is_none());
        });
        rsx! {}
    }
    VirtualDom::new(host).rebuild_in_place();
}

/// Malformed graph input and transport failures never trigger application shutdown.
#[test]
fn graph_transport_and_invalid_input_are_not_fatal() {
    /// Exercises the application callback with each non-runtime classification.
    fn host() -> Element {
        let boundary = use_context_provider(HostBoundary::new);
        use_hook(move || {
            for error in [
                adam_web_ui::graph::GraphFailure::InvalidInput("bad bounds".into()),
                adam_web_ui::graph::GraphFailure::Transport("bad wire".into()),
                adam_web_ui::graph::GraphFailure::Unavailable,
            ] {
                graph_outcome(error);
                assert!(!boundary.is_fatal());
            }
        });
        rsx! {}
    }
    VirtualDom::new(host).rebuild_in_place();
}

/// A requested restoration failure reports the error without recursively retrying it.
#[test]
fn restoration_failure_does_not_retry_factory() {
    /// Makes only the initial factory invocation succeed.
    fn host() -> Element {
        use_hook(|| {
            let calls = Rc::new(Cell::new(0));
            let observed = calls.clone();
            let owner = SheetOwner::new(Rc::new(move || {
                    observed.set(observed.get() + 1);
                    let source = if observed.get() == 1 {
                        "sheet s { source x: i32 = 1; source valid: bool = true require { @limit x < 10; }; }"
                    } else {
                        "sheet invalid { cell x }"
                    };
                    build_sheet(source, "restoration.adm2")
                }), "restore/v1".into()).unwrap();
            let sheet = SheetHandle::from_owner(owner, EventHandler::new(sheet_outcome));
            sheet.cell("x").submit("20".into());
            assert_eq!(calls.get(), 2);
            assert!(matches!(
                sheet.inspect(|_, _| ()),
                Err(BindingError::Unavailable)
            ));
            assert_eq!(sheet.cell("x").view().unwrap().text, "1");
            assert_eq!(
                sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
                0
            );
        });
        rsx! {}
    }
    VirtualDom::new(host).rebuild_in_place();
}
