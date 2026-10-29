//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

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
        assert!(good.contains("sp-checkbox"));
        let bad = render("sheet s { cell x }");
        assert!(bad.contains("adam-live-error"));
        assert!(!bad.contains("sp-checkbox"));
        assert!(!bad.contains('\u{1b}'));
    }

    /// Propagation failure retains the inspector alongside its diagnostic.
    #[test]
    fn root_retains_controls_on_propagation_failure() {
        let html = render("sheet s { cell x: i64 = 0i64; out result := 1i64 / x; }");
        assert!(html.contains("sp-number-field"));
        assert!(html.contains("adam-live-error"));
        assert!(html.contains("division"));
        assert!(!html.contains('\u{1b}'));
    }
}

#[derive(Clone, PartialEq, Props)]
/// Supplies one independent inspector and its existing external graph containers.
struct RootProps {
    source: String,
    name: String,
    graph_ids: Vec<String>,
}

/// Parses `props.source`, then renders either a live [`SheetInspector`] (on success) or the
/// formatted diagnostic (on parse failure), matching how a propagate failure alongside a
/// successfully built sheet renders both. Wrapped in its own [`SpTheme`], since Spectrum Web
/// Components render unstyled (correct custom elements, but none of Spectrum's CSS
/// custom-property design tokens applied) without one — each independently-mounted `Root`
/// gets its own `<sp-theme>`, exactly as `begin`'s single, app-wide one wraps its whole tree.
/// Diagnostics render via [`Renderer::plain`], not [`Renderer::styled`]: the rendered `<pre>`
/// is a browser element, not a terminal, so ANSI escape codes would show as literal garbage
/// text rather than color.
///
/// Each mount has immutable props. Initialization stores its non-copyable sheet
/// inside scope-owned signals exactly once, including failed builds.
///
/// - Complexity: initialization includes parsing/propagation; rendering costs O(c + g)
///   components for c cells and g graph hosts, excluding graph serialization.
#[component]
fn Root(props: RootProps) -> Element {
    let (sheet_labels, error) = use_hook(|| {
        let outcome = build_sheet(&props.source, &props.name, &Renderer::plain());
        (
            outcome
                .sheet_labels
                .map(|(parsed, labels)| (Signal::new(parsed), Signal::new(labels))),
            outcome.error,
        )
    });
    let source_text = use_memo({
        let source = props.source.clone();
        move || source.clone()
    });
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    let inner = match sheet_labels {
        Some((parsed, labels)) => {
            let data = use_memo(move || to_graph_data(&parsed.read(), &labels.read()));
            rsx! {
                SheetInspector { parsed, labels, source_text, source_name }
                for id in &props.graph_ids {
                    GraphView {
                        key: "{id}",
                        graph_id: id.clone(),
                        data,
                        source_id: source_name,
                        external_container: true,
                    }
                }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
        None => {
            let error = error.unwrap_or_default();
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
