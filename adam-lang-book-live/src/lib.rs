//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

use adam_web_ui::binding::{BindingError, HostOutcome, ModelFactory, SheetEvent, use_sheet};
use adam_web_ui::spectrum::SpTheme;
use adam_web_ui::{GraphView, Renderer, SheetInspector, build_sheet, to_graph_data};
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
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Successful sheets render controls; parse failure renders only a plain diagnostic.
    #[test]
    fn root_renders_success_and_parse_failure() {
        let good = render("sheet s { cell x: bool; }");
        assert!(good.contains("sp-theme"));
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
            rsx! { BookGraph { graph_id: "book-graph", sheet, source_id } }
        }
        for unavailable in [false, true] {
            let mut dom = VirtualDom::new_with_props(host, unavailable);
            dom.rebuild_in_place();
            let html = dioxus_ssr::render(&dom);
            assert_eq!(html.contains("Graph unavailable:"), unavailable, "{html}");
        }
    }
}

#[derive(Clone, PartialEq, Props)]
/// Supplies one independent inspector and its existing external graph containers.
struct RootProps {
    source: String,
    name: String,
    graph_ids: Vec<String>,
}

/// Builds one owner and renders its Inspector or a plain construction diagnostic.
///
/// Each mount has immutable props and caches initialization, including failed
/// builds. The Inspector installs its Dioxus theme; the outer Spectrum shell
/// remains until the book shell migration. Graphs read the same accepted owner.
///
/// - Complexity: initialization includes parsing/propagation; rendering costs O(c + g)
///   components for c cells and g graph hosts, excluding graph serialization.
#[component]
fn Root(props: RootProps) -> Element {
    let factory: ModelFactory = std::rc::Rc::new({
        let source = props.source.clone();
        let name = props.name.clone();
        move || build_sheet(&source, &name)
    });
    let initialized = use_sheet(factory, props.name.clone(), EventHandler::new(book_outcome));
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    let inner = match initialized {
        Ok(sheet) => {
            rsx! {
                SheetInspector { sheet }
                for id in &props.graph_ids {
                    BookGraph {
                        key: "{id}",
                        graph_id: id.clone(),
                        sheet,
                        source_id: source_name,
                    }
                }
            }
        }
        Err(error) => {
            let error = error.render(&Renderer::plain());
            rsx! {
                pre { class: "adam-live-error", "{error}" }
            }
        }
    };

    rsx! {
        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            system: "spectrum-two".to_string(),
            {inner}
        }
    }
}

/// Reports structured host failures and explicitly restores recoverable candidates.
///
/// Fatal propagation never rebuilds; recording failure preserves accepted state.
///
/// - Complexity: Diagnostic bytes plus explicit reconstruction cost on rejection.
fn book_outcome(event: SheetEvent) {
    match event.outcome {
        HostOutcome::Accepted => {}
        HostOutcome::Rejected(error) => {
            adam_web_ui::diagnostics::report_error(&format!("{error:?}"));
            if matches!(
                event.sheet.inspect(|_, _| ()),
                Err(BindingError::Unavailable)
            ) && let Err(error) = event.sheet.restore_accepted()
            {
                adam_web_ui::diagnostics::report_error(&error.render(&Renderer::plain()));
            }
        }
        HostOutcome::Fatal(error) => adam_web_ui::diagnostics::report_error(&format!("{error:?}")),
        HostOutcome::RecordingFailed(error) => {
            adam_web_ui::diagnostics::report_error(&format!("{error:?}"))
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
) -> Element {
    let snapshot = sheet.inspect(|parsed, labels| to_graph_data(parsed, labels));
    match snapshot {
        Ok(snapshot) => rsx! { AcceptedGraph { graph_id, snapshot, source_id } },
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
) -> Element {
    let data = use_memo(use_reactive((&snapshot,), |(snapshot,)| snapshot));
    rsx! { GraphView { graph_id, data, source_id, external_container: true,
        on_error: move |error: adam_web_ui::graph::GraphFailure| {
            adam_web_ui::diagnostics::report_error(&error.to_string());
        }
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
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
