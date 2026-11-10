//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

use adam_web_ui::binding::{
    BindingError, HostOutcome, ModelFactory, RecoveryState, SheetEvent, SheetHandle, use_sheet,
};
use adam_web_ui::components::Theme;
use adam_web_ui::{
    GraphView, InspectorPresentation, Renderer, SheetInspector, build_sheet, to_graph_data,
};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the real book root without starting a browser renderer.
    ///
    /// - Complexity: O(source/output bytes) storage plus Root build, propagation, and rendering work.
    fn render(source: &str) -> String {
        let mut dom = VirtualDom::new_with_props(
            Root,
            RootProps {
                source: source.to_string(),
                name: "same.adm2".to_string(),
                graph_ids: vec![],
                root_id: "test-root".into(),
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Successful sheets render controls; parse failure renders only a plain diagnostic.
    #[test]
    fn root_renders_success_and_parse_failure() {
        let good = render("sheet s { cell x: bool; }");
        assert!(good.contains("adam-theme"));
        assert!(!good.contains("sp-theme"));
        assert!(good.contains("role=\"checkbox\""));
        let bad = render("sheet s { cell x }");
        assert!(bad.contains("adam-live-error"));
        assert!(!bad.contains("role=\"checkbox\""));
        assert!(!bad.contains('\u{1b}'));
    }

    /// Propagation failure renders only its diagnostic, without a failed model.
    #[test]
    fn root_renders_runtime_failure_without_controls() {
        let html = render("sheet s { cell x: i64 = 0i64; out result := 1i64 / x; }");
        assert!(!html.contains("<sp-textfield"));
        assert!(!html.contains("sp-number-field"));
        assert!(html.contains("adam-live-error"));
        assert!(html.contains("division"));
        assert!(!html.contains('\u{1b}'));
    }

    /// Graph glue renders only accepted snapshots and exposes unavailable ownership.
    #[test]
    fn graph_reads_only_accepted_live_handle() {
        /// Mounts an external graph over either a live or deliberately discarded owner.
        fn host(unavailable: bool) -> Element {
            let sheet = use_sheet(
                std::rc::Rc::new(|| build_sheet("sheet s { cell x: i32 = 1; }", "s.adm2")),
                "s/v1".into(),
                EventHandler::new(|_| {}),
            )
            .unwrap();
            use_hook(move || {
                if unavailable {
                    sheet.cell("x").submit("bad".into());
                }
            });
            let source_id = use_memo(|| "s.adm2".to_string());
            rsx! { BookGraph { graph_id: "book-graph", sheet, source_id, on_error: |_| {} } }
        }
        for unavailable in [false, true] {
            let mut dom = VirtualDom::new_with_props(host, unavailable);
            dom.rebuild_in_place();
            let html = dioxus_ssr::render(&dom);
            assert_eq!(html.contains("Graph unavailable:"), unavailable, "{html}");
        }
    }

    /// Rejection restores aspirations and a fatal root cannot retire its equal-named peer.
    #[test]
    fn independent_boundaries_restore_then_retire_only_the_failing_root() {
        use std::{cell::RefCell, rc::Rc};
        type Observed = Rc<
            RefCell<
                Option<(
                    adam_web_ui::binding::SheetHandle,
                    Rc<RefCell<Boundary>>,
                    Rc<RefCell<Vec<String>>>,
                )>,
            >,
        >;
        /// Mounts the actual boundary policy over one independently owned handle.
        fn host(observed: Observed) -> Element {
            let boundary = Rc::new(RefCell::new(Boundary::default()));
            let saved = Rc::new(RefCell::new(Vec::new()));
            let state = boundary.clone();
            let records = saved.clone();
            let sheet = use_sheet(
                Rc::new(|| {
                    build_sheet(
                        "sheet s { cell x: i32 = 2; out result: i32 := 10 / x; }",
                        "same.adm2",
                    )
                }),
                "same/v1".into(),
                EventHandler::new(move |event| {
                    handle_outcome(
                        &mut state.borrow_mut(),
                        event,
                        "root",
                        "source",
                        "same.adm2",
                        |json| {
                            records.borrow_mut().push(json.to_owned());
                            Ok(())
                        },
                    );
                }),
            )
            .unwrap();
            use_hook(move || {
                *observed.borrow_mut() = Some((sheet, boundary, saved));
            });
            rsx! { SheetInspector { sheet } }
        }
        let first: Observed = Rc::default();
        let second: Observed = Rc::default();
        let mut one = VirtualDom::new_with_props(host, first.clone());
        let mut two = VirtualDom::new_with_props(host, second.clone());
        one.rebuild_in_place();
        two.rebuild_in_place();
        let (sheet, boundary, saved) = first.borrow().as_ref().unwrap().clone();
        let peer = second.borrow().as_ref().unwrap().0;
        sheet.cell("x").submit("5".into());
        handle_outcome(
            &mut boundary.borrow_mut(),
            SheetEvent {
                sheet,
                outcome: HostOutcome::RecordingFailed(
                    adam_web_ui::binding::CaptureError::UnnamedSource,
                ),
            },
            "root",
            "source",
            "same.adm2",
            |_| panic!("recording failure is accepted"),
        );
        assert_eq!(sheet.cell("x").view().unwrap().text, "5");
        assert!(!boundary.borrow().closed);
        sheet.cell("x").submit("invalid".into());
        assert_eq!(sheet.cell("x").view().unwrap().text, "5");
        sheet.cell("x").submit("0".into());
        assert!(boundary.borrow().closed);
        assert!(sheet.inspect(|_, _| ()).is_err());
        assert_eq!(saved.borrow().len(), 1);
        assert!(saved.borrow()[0].contains("same/v1"));
        peer.cell("x").submit("4".into());
        assert_eq!(peer.cell("x").view().unwrap().text, "4");
        assert!(!second.borrow().as_ref().unwrap().1.borrow().closed);
    }

    /// Storage failure remains visible alongside the original fatal diagnostic.
    #[test]
    fn persistence_failures_never_report_success() {
        let mut state = Boundary::default();
        finish_shutdown(
            &mut state,
            "runtime failure".into(),
            Err("quota denied".into()),
        );
        assert!(state.closed);
        assert!(state.message.as_deref().unwrap().contains("quota denied"));
        assert!(
            state
                .message
                .as_deref()
                .unwrap()
                .contains("runtime failure")
        );
    }

    /// Graph input/transport errors stay recoverable; discarded graph candidates close once.
    #[test]
    fn graph_failure_classification_controls_only_its_boundary() {
        use adam_web_ui::graph::GraphFailure;
        for error in [
            GraphFailure::InvalidInput("bounds".into()),
            GraphFailure::Transport("wire".into()),
        ] {
            let mut state = Boundary::default();
            handle_graph_failure(&mut state, error, None, "root", "source", "name", |_| {
                panic!("recoverable graph failure must not persist")
            });
            assert!(!state.closed);
            assert!(state.message.is_some());
        }
        let error = build_sheet(
            "sheet s { cell x: i32 = 0; out y: i32 := 10 / x; }",
            "graph.adm2",
        )
        .err()
        .unwrap();
        let adam_web_ui::BuildError::Runtime(error) = error else {
            panic!("runtime fixture")
        };
        for error in [GraphFailure::Runtime(error), GraphFailure::Unavailable] {
            let mut state = Boundary::default();
            handle_graph_failure(
                &mut state,
                error,
                None,
                "root",
                "source",
                "name",
                |_| Ok(()),
            );
            assert!(state.closed);
            assert!(
                state
                    .message
                    .as_deref()
                    .unwrap()
                    .contains("Accepted journal unavailable")
            );
            let prior = state.message.clone();
            handle_graph_failure(
                &mut state,
                GraphFailure::Transport("late".into()),
                None,
                "root",
                "source",
                "name",
                |_| panic!("closed root cannot retry"),
            );
            assert_eq!(state.message, prior);
        }
    }

    /// A failed restoration reports its typed diagnostic, saves once, and never retries.
    #[test]
    fn restoration_failure_closes_the_boundary_without_recursive_retry() {
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };
        /// Exposes only the tested boundary and its production-owned handle.
        type Observer = Rc<RefCell<Option<(SheetHandle, Rc<RefCell<Boundary>>)>>>;
        /// Counts factory calls and persistence requests independently of the model.
        type Counts = (Rc<Cell<u32>>, Rc<Cell<u32>>, Observer);
        /// Installs a factory that rejects its first reconstruction attempt.
        fn host((builds, saves, observed): Counts) -> Element {
            let state = use_hook(|| Rc::new(RefCell::new(Boundary::default())));
            let factory: ModelFactory = Rc::new(move || {
                builds.set(builds.get().checked_add(1).unwrap());
                build_sheet(
                    if builds.get() == 1 {
                        "sheet s { cell x: i32 = 1; }"
                    } else {
                        "sheet s { cell x }"
                    },
                    "same.adm2",
                )
            });
            let boundary = state.clone();
            let sheet = use_sheet(
                factory,
                "same/v1".into(),
                EventHandler::new(move |event| {
                    handle_outcome(
                        &mut boundary.borrow_mut(),
                        event,
                        "root",
                        "accepted source",
                        "same.adm2",
                        |json| {
                            assert!(json.contains("accepted source"));
                            saves.set(saves.get().checked_add(1).unwrap());
                            Ok(())
                        },
                    );
                }),
            )
            .unwrap();
            use_hook(move || {
                *observed.borrow_mut() = Some((sheet, state));
            });
            rsx! {}
        }
        let builds = Rc::new(Cell::new(0));
        let saves = Rc::new(Cell::new(0));
        let observed: Observer = Rc::default();
        let mut dom =
            VirtualDom::new_with_props(host, (builds.clone(), saves.clone(), observed.clone()));
        dom.rebuild_in_place();
        let (sheet, boundary) = observed.borrow().as_ref().unwrap().clone();
        sheet.cell("x").submit("invalid".into());
        assert!(boundary.borrow().closed);
        assert!(
            boundary
                .borrow()
                .message
                .as_deref()
                .unwrap()
                .contains("expected `:` or `=`")
        );
        assert_eq!((builds.get(), saves.get()), (2, 1));
        handle_outcome(
            &mut boundary.borrow_mut(),
            SheetEvent {
                sheet,
                outcome: HostOutcome::Rejected(adam_web_ui::binding::HostRejection::Binding(
                    BindingError::Unavailable,
                )),
            },
            "root",
            "source",
            "same.adm2",
            |_| panic!("no persistence retry"),
        );
        assert_eq!((builds.get(), saves.get()), (2, 1));
    }

    /// Equal DOM names in different publication documents retain distinct recovery records.
    #[test]
    fn recovery_storage_identity_distinguishes_equal_named_roots() {
        assert_ne!(
            recovery_storage_key("inspector", &[0; 16]),
            recovery_storage_key("inspector", &[1; 16]),
        );
        assert_ne!(
            recovery_storage_key("first", &[0; 16]),
            recovery_storage_key("second", &[0; 16]),
        );
    }
}

/// Reports graph transport/input errors without retiring an otherwise accepted root.
///
/// Runtime and unavailable graph candidates close only their owning root and
/// persist its accepted inputs; a closed root never retries.
/// - Complexity: O(d + j) for diagnostic and serialized journal bytes.
fn handle_graph_failure(
    state: &mut Boundary,
    error: adam_web_ui::graph::GraphFailure,
    sheet: Option<SheetHandle>,
    root: &str,
    source: &str,
    name: &str,
    persist: impl FnOnce(&str) -> Result<(), String>,
) {
    if state.closed {
        return;
    }
    let message = error.to_string();
    match error {
        adam_web_ui::graph::GraphFailure::Runtime(_)
        | adam_web_ui::graph::GraphFailure::Unavailable => {
            let saved = sheet
                .ok_or_else(|| "Accepted journal unavailable".to_string())
                .and_then(|sheet| recovery_json(sheet, root, source, name))
                .and_then(|json| persist(&json));
            finish_shutdown(state, message, saved);
        }
        adam_web_ui::graph::GraphFailure::InvalidInput(_)
        | adam_web_ui::graph::GraphFailure::Transport(_) => {
            state.message = Some(message);
        }
    }
}

#[derive(Clone, PartialEq, Props)]
/// Supplies one independent inspector and its existing external graph containers.
struct RootProps {
    /// Retains the immutable accepted source text.
    source: String,
    /// Supplies diagnostics, not event routing identity.
    name: String,
    /// Names graph containers owned exclusively by this root.
    graph_ids: Vec<String>,
    /// Names this exact persistent host boundary.
    root_id: String,
}

/// Builds one owner and renders its Inspector or a plain construction diagnostic.
///
/// Each root retains accepted provenance and history after its children unmount.
/// Fatal failure closes only this root; graphs read the same accepted owner.
///
/// - Complexity: initialization includes parsing/propagation; rendering costs O(c + g)
///   components for c cells and g graph hosts, excluding graph serialization.
#[component]
fn Root(props: RootProps) -> Element {
    let props = use_hook(move || std::rc::Rc::new(props));
    let mut boundary = use_signal(Boundary::default);
    let factory: ModelFactory = use_hook({
        let props = props.clone();
        move || std::rc::Rc::new(move || build_sheet(&props.source, &props.name)) as ModelFactory
    });
    let on_outcome = use_callback({
        let props = props.clone();
        move |event| {
            handle_outcome(
                &mut boundary.write(),
                event,
                &props.root_id,
                &props.source,
                &props.name,
                |json| persist_browser(&props.root_id, json),
            )
        }
    });
    let initialized = use_sheet(factory, props.name.clone(), on_outcome);
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    let graph_error = use_callback({
        let initialized = initialized.clone();
        let props = props.clone();
        move |error: adam_web_ui::graph::GraphFailure| {
            handle_graph_failure(
                &mut boundary.write(),
                error,
                initialized.as_ref().ok().copied(),
                &props.root_id,
                &props.source,
                &props.name,
                |json| persist_browser(&props.root_id, json),
            );
        }
    });
    let inner = if boundary.read().closed {
        rsx! {}
    } else {
        match initialized {
            Ok(sheet) => {
                rsx! {
                    SheetInspector { sheet, presentation: InspectorPresentation::Card }
                    for id in &props.graph_ids {
                        BookGraph {
                            key: "{id}",
                            graph_id: id.clone(),
                            sheet,
                            source_id: source_name,
                            on_error: graph_error,
                        }
                    }
                }
            }
            Err(error) => {
                let error = error.render(&Renderer::plain());
                rsx! {
                    pre { class: "adam-live-error", role: "alert", "{error}" }
                }
            }
        }
    };

    rsx! {
        Theme {
            if let Some(message) = &boundary.read().message {
                pre { class: "adam-live-error", role: "alert", "{message}" }
            }
            {inner}
        }
    }
}

/// Owns one root's irreversible shutdown state and visible diagnostics.
#[derive(Default)]
struct Boundary {
    /// Forbids subsequent recovery, mutation handling, and graph installation.
    closed: bool,
    /// Survives editor and graph unmount independently of their model.
    message: Option<String>,
}

/// Borrows accepted source/configuration and ordered aspirations for persistence.
#[derive(serde::Serialize)]
struct RecoveryDocument<'a> {
    /// Versions the host provenance and scalar codec contract.
    version: u32,
    /// Identifies this exact DOM root rather than its possibly shared diagnostic name.
    root: &'a str,
    /// Retains accepted source bytes.
    source: &'a str,
    /// Retains diagnostic provenance.
    name: &'a str,
    /// Retains schema identity and accepted aspiration groups, not effective recordings.
    recovery: &'a RecoveryState,
}

/// Serializes the accepted journal without reading a failed candidate or cloning history.
///
/// # Errors
/// Returns unavailable journal or serialization diagnostics.
/// - Complexity: O(source and accepted journal bytes) time and storage.
fn recovery_json(
    sheet: SheetHandle,
    root: &str,
    source: &str,
    name: &str,
) -> Result<String, String> {
    sheet
        .with_accepted_state(|recovery| {
            serde_json::to_string(&RecoveryDocument {
                version: 1,
                root,
                source,
                name,
                recovery,
            })
        })
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// Writes accepted recovery state synchronously before child scopes unmount.
///
/// A browser-generated identity prevents equal DOM names in other documents
/// from overwriting previously saved recovery records.
/// # Errors
/// Returns unavailable browser storage/identity, denied access, or quota diagnostics.
/// - Complexity: O(key and JSON bytes) browser storage work.
fn persist_browser(root: &str, json: &str) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().ok_or("Browser window unavailable")?;
        let mut identity = [0_u8; 16];
        window
            .crypto()
            .map_err(|error| format!("{error:?}"))?
            .get_random_values_with_u8_array(&mut identity)
            .map_err(|error| format!("{error:?}"))?;
        let storage = window
            .local_storage()
            .map_err(|error| format!("{error:?}"))?
            .ok_or("Browser storage unavailable")?;
        storage
            .set_item(&recovery_storage_key(root, &identity), json)
            .map_err(|error| format!("{error:?}"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (root, json);
        Err("Browser storage unavailable".into())
    }
}

/// Names one recovery document independently of equal DOM IDs in other documents.
///
/// Distinct browser-generated identities retain earlier failures without overwriting.
/// - Complexity: O(root bytes) time and space.
#[cfg(any(target_arch = "wasm32", test))]
fn recovery_storage_key(root: &str, identity: &[u8; 16]) -> String {
    format!("stlab.adam.live.recovery.v1:{root}:{identity:02x?}")
}

/// Permanently closes one root and retains fatal and persistence diagnostics.
///
/// - Complexity: O(diagnostic bytes).
fn finish_shutdown(state: &mut Boundary, message: String, saved: Result<(), String>) {
    state.closed = true;
    state.message = Some(match saved {
        Ok(()) => message,
        Err(error) => format!("{message}\nRecovery persistence failed: {error}"),
    });
    adam_web_ui::diagnostics::report_error(state.message.as_deref().expect("shutdown diagnostic"));
}

/// Handles one root's outcomes with explicit recovery and no recursive retry.
///
/// Fatal propagation persists only accepted provenance and aspirations. Recording
/// failure is accepted and never rolls back. Restoration failure closes the root.
/// - Complexity: Diagnostic/serialization bytes plus restoration cost on rejection.
fn handle_outcome(
    state: &mut Boundary,
    event: SheetEvent,
    root: &str,
    source: &str,
    name: &str,
    persist: impl FnOnce(&str) -> Result<(), String>,
) {
    if state.closed {
        return;
    }
    match event.outcome {
        HostOutcome::Accepted => state.message = None,
        HostOutcome::Rejected(error) => {
            let restore = matches!(
                event.sheet.inspect(|_, _| ()),
                Err(BindingError::Unavailable)
            );
            state.message =
                Some(adam_web_ui::BuildError::Recovery(Box::new(error)).render(&Renderer::plain()));
            if restore && let Err(error) = event.sheet.restore_accepted() {
                let saved =
                    recovery_json(event.sheet, root, source, name).and_then(|json| persist(&json));
                finish_shutdown(state, error.render(&Renderer::plain()), saved);
            }
        }
        HostOutcome::Fatal(error) => {
            let message =
                adam_web_ui::BuildError::Runtime(Box::new(error)).render(&Renderer::plain());
            let saved =
                recovery_json(event.sheet, root, source, name).and_then(|json| persist(&json));
            finish_shutdown(state, message, saved);
        }
        HostOutcome::RecordingFailed(error) => {
            state.message = Some(format!("Accepted edit; recording failed: {error:?}"));
        }
    }
}

/// Projects the graph only from the accepted live handle.
///
/// - Complexity: Graph projection and serialization.
#[component]
fn BookGraph(
    graph_id: String,
    sheet: adam_web_ui::binding::SheetHandle,
    source_id: Memo<String>,
    on_error: EventHandler<adam_web_ui::graph::GraphFailure>,
) -> Element {
    let snapshot = sheet.inspect(|parsed, labels| to_graph_data(parsed, labels));
    match snapshot {
        Ok(snapshot) => rsx! { AcceptedGraph { graph_id, snapshot, source_id, on_error } },
        Err(error) => rsx! { p { role: "alert", "Graph unavailable: {error}" } },
    }
}

/// Supplies a checked accepted snapshot to the existing external graph renderer.
///
/// - Complexity: O(graph size) snapshot transfer.
#[component]
fn AcceptedGraph(
    graph_id: String,
    snapshot: adam_web_ui::GraphData,
    source_id: Memo<String>,
    on_error: EventHandler<adam_web_ui::graph::GraphFailure>,
) -> Element {
    let data = use_memo(use_reactive((&snapshot,), |(snapshot,)| snapshot));
    rsx! { GraphView { graph_id, data, source_id, external_container: true,
        on_error
    } }
}

/// Mounts a live [`SheetInspector`] for `source` into the DOM element with id `element_id`,
/// using `name` (the example's `data-example` attribute, e.g. `"relationships/conflict_error"`)
/// as the diagnostic file name shown in any parse/propagate error. Also drives each id in
/// `graph_ids` (the graph containers bound to this example's sheet) on every propagate,
/// through an independent shared [`GraphView`] channel owner per container.
///
/// Distinct host elements receive independent controls even when `name` is identical.
/// A graph container has one current owner: mounting another view for the same graph
/// ID retires its preceding channel rather than creating independent graph contents.
///
/// - Precondition: an element with id `element_id` already exists in the document — the
///   mdBook `live-examples` preprocessor is what creates it (see
///   `adam-lang-book-preprocessor`).
///
/// - Complexity: setup copies source/diagnostic-name bytes; mounting additionally
///   parses, propagates, and renders the sheet and requested graph hosts.
///
/// # Examples
///
/// ```javascript
/// // After initializing the WASM module and creating both DOM hosts:
/// mount("inspector", source, "tutorial/first_sheet", ["graph"]);
/// ```
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str, name: &str, graph_ids: Vec<String>) {
    let props = RootProps {
        source: source.to_string(),
        name: format!("{name}.adm2"),
        graph_ids,
        root_id: element_id.to_owned(),
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
