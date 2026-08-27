//! wasm-bindgen entry point that mounts one live [`adam_web_ui::SheetInspector`] per call, for
//! `adam-lang-book`'s live examples. Each `.adm2` example on a book page gets its own
//! independent mount (confirmed to coexist safely — see
//! `docs/superpowers/plans/2026-08-27-live-adam-book-examples.md`'s Task 8 spike); there is no
//! shared state between them.

use adam_web_ui::{SheetInspector, build_sheet};
use dioxus::prelude::*;
use wasm_bindgen::prelude::*;

#[derive(Clone, PartialEq, Props)]
struct RootProps {
    source: String,
    name: String,
}

/// Parses `props.source`, then renders either a live [`SheetInspector`] (on success) or the
/// formatted diagnostic (on parse failure), matching how a propagate failure alongside a
/// successfully built sheet renders both.
///
/// `outcome` is a plain call rather than a `use_hook` because `BuildOutcome` holds a `Sheet`,
/// which cannot be `Clone` (it owns type-erased cell values) and `use_hook` requires its state
/// to be. `sheet` and `labels` still end up seeded exactly once: `use_signal`'s initializer
/// closure only ever fires on this component's first render, and props never change after
/// [`mount`] constructs them, so no later render (should one ever occur; this component holds
/// no subscriptions of its own that would trigger one) can replace already-mounted state.
#[component]
fn Root(props: RootProps) -> Element {
    let outcome = build_sheet(&props.source, &props.name);
    let source_text = use_memo({
        let source = props.source.clone();
        move || source.clone()
    });
    let source_name = use_memo({
        let name = props.name.clone();
        move || name.clone()
    });

    match outcome.sheet_labels {
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
    }
}

/// Mounts a live [`SheetInspector`] for `source` into the DOM element with id `element_id`.
///
/// - Precondition: an element with id `element_id` already exists in the document — the
///   mdBook `live-examples` preprocessor is what creates it (see
///   `adam-lang-book-preprocessor`).
#[wasm_bindgen]
pub fn mount(element_id: &str, source: &str) {
    let props = RootProps {
        source: source.to_string(),
        name: format!("{element_id}.adm2"),
    };
    let vdom = VirtualDom::new_with_props(Root, props);
    let config = dioxus::web::Config::new().rootname(element_id);
    dioxus::web::launch::launch_virtual_dom(vdom, config);
}
