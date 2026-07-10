//! Root [`App`] component.

use dioxus::prelude::*;

use crate::bridge::to_graph_data;
use crate::demo_source::{build_sheet, load_demo_source};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::SpTheme;

/// Root component: Spectrum theme wrapper with the graph and Inspector filling the
/// viewport. The demo pm-lang source lives in `begin/assets/demo.pm` — edit it and
/// (on desktop, under `dx serve`) it hot-reloads into this running app.
#[component]
pub fn App() -> Element {
    let initial_source = load_demo_source();
    let initial = build_sheet(&initial_source);
    if let Some(err) = &initial.error {
        eprintln!("{err}");
    }
    let (initial_sheet, initial_labels) = initial
        .sheet_labels
        .expect("demo.pm must parse successfully");
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_source);

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    rsx! {
        document::Link { rel: "icon", r#type: "image/x-icon", href: asset!("/assets/favicon.ico") }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }

        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            div {
                style: "position: fixed; inset: 0; display: flex; overflow: hidden;",
                GraphView { data: graph_data }
                Inspector { sheet, labels, active_source }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_source::DEMO_SOURCE_TEXT;

    #[test]
    fn demo_source_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn demo_source_g_forced_when_p_is_one() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (mut sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(sheet.is_forced(g_id), "g should be forced when p == 1");
    }

    #[test]
    fn demo_source_g_unforced_again_after_p_returns_to_zero() {
        let outcome = build_sheet(DEMO_SOURCE_TEXT);
        let (mut sheet, labels) = outcome.sheet_labels.expect("demo.pm must build");
        let p_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("p"))
            .unwrap();
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();

        sheet.write(p_id, 1_i32).unwrap();
        sheet.propagate().unwrap();
        sheet.write(p_id, 0_i32).unwrap();
        sheet.propagate().unwrap();

        assert!(
            !sheet.is_forced(g_id),
            "g should not be forced once p == 0 again"
        );
    }
}
