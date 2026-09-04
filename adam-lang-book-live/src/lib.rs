//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

use adam_web_ui::spectrum::SpTheme;
use adam_web_ui::{GraphView, to_graph_data};
use adam_web_ui::{Renderer, SheetInspector, build_sheet};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[derive(Clone, PartialEq, Props)]
struct RootProps {
    source: String,
    name: String,
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
/// `outcome` is a plain call rather than a `use_hook` because `BuildOutcome` holds a `Sheet`,
/// which cannot be `Clone` (it owns type-erased cell values) and `use_hook` requires its state
/// to be. `sheet` and `labels` still end up seeded exactly once: `use_signal`'s initializer
/// closure only ever fires on this component's first render, and props never change after
/// [`mount`] constructs them, so no later render (should one ever occur; this component holds
/// no subscriptions of its own that would trigger one) can replace already-mounted state.
///
/// Caution: the `use_signal` calls below live inside a `match` arm on `outcome.sheet_labels`,
/// which is only sound because `build_sheet` is deterministic over an unchanging `props` — the
/// arm taken can never differ across re-renders of the same component instance. If `Root` ever
/// gains a reason to re-render with different props, or `build_sheet` ever becomes
/// non-deterministic, this conditional-hook-call pattern would need reworking (e.g. calling
/// `use_signal` unconditionally with an `Option`-shaped initial value instead).
#[component]
fn Root(props: RootProps) -> Element {
    let outcome = build_sheet(&props.source, &props.name, &Renderer::plain());
    let source_text = use_memo({
        let source = props.source.clone();
        move || source.clone()
    });
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    let inner = match outcome.sheet_labels {
        Some((sheet, labels)) => {
            let sheet = use_signal(|| sheet);
            let labels = use_signal(|| labels);
            let error = outcome.error.clone();
            rsx! {
                SheetInspector { sheet, labels, source_text, source_name }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
        None => {
            let error = outcome.error.unwrap_or_default();
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
/// as the diagnostic file name shown in any parse/propagate error.
///
/// - Precondition: an element with id `element_id` already exists in the document — the
///   mdBook `live-examples` preprocessor is what creates it (see
///   `adam-lang-book-preprocessor`).
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str, name: &str) {
    let props = RootProps {
        source: source.to_string(),
        name: format!("{name}.adm2"),
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}

#[derive(Clone, PartialEq, Props)]
struct GraphRootProps {
    source: String,
    name: String,
    graph_id: String,
}

/// Parses `props.source`, then renders either a live [`GraphView`] (on success) or the
/// formatted diagnostic (on parse failure) — mirroring [`Root`]'s same two-outcome shape.
/// `graph_id`/`source_id` are both set to `props.graph_id`: within one independent mount the
/// source never changes, so there's nothing for `GraphView`'s destroy-vs-update logic to
/// distinguish (see [`mount_graph`]'s doc comment for why `graph_id` isn't just `element_id`).
#[component]
fn GraphRoot(props: GraphRootProps) -> Element {
    let outcome = build_sheet(&props.source, &props.name, &Renderer::plain());
    let graph_id = use_memo({
        let id = props.graph_id.clone();
        move || id.clone()
    });
    let source_id = use_memo({
        let id = props.graph_id.clone();
        move || id.clone()
    });

    match outcome.sheet_labels {
        Some((sheet, labels)) => {
            let sheet = use_signal(|| sheet);
            let labels = use_signal(|| labels);
            let data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));
            let error = outcome.error.clone();
            rsx! {
                GraphView { graph_id, data, source_id }
                if let Some(err) = error {
                    pre { class: "adam-live-error", "{err}" }
                }
            }
        }
        None => {
            let error = outcome.error.unwrap_or_default();
            rsx! {
                pre { class: "adam-live-error", "{error}" }
            }
        }
    }
}

/// Mounts a live [`GraphView`] for `source` into the DOM element with id `element_id`, using
/// `name` (the example's `data-example` attribute, e.g. `"tutorial/first_sheet"`) as the
/// diagnostic file name shown in any parse/propagate error.
///
/// `GraphView`'s own rendered `<div>` (what `graph.js` actually attaches D3 to) gets a derived
/// id, `"{element_id}-container"`, distinct from `element_id` itself: `element_id` names the
/// wrapper `<div>` the mdBook preprocessor already created (see `adam-lang-book-preprocessor`)
/// and that Dioxus mounts *into* as a child, so giving `GraphView`'s own `<div>` the same id
/// would create a duplicate-id DOM, and `document.getElementById` would resolve to the outer
/// wrapper instead of the div `graph.js` needs.
///
/// - Precondition: an element with id `element_id` already exists in the document.
#[wasm_bindgen]
pub fn mount_graph(element_id: &str, source: &str, name: &str) {
    let props = GraphRootProps {
        source: source.to_string(),
        name: format!("{name}.adm2"),
        graph_id: format!("{element_id}-container"),
    };
    let vdom = VirtualDom::new_with_props(GraphRoot, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
