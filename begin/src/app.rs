//! Root [`App`] component and demo pm-lang source.

use dioxus::prelude::*;

use crate::bridge::to_graph_data;
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::source_panel::{SourcePanel, build_sheet};
use crate::spectrum::SpTheme;

/// Default pm-lang source: two independent bidirectional constraint systems
/// (`a × b = c` and `d × e = f`) linked by a conditional on `p`.
///
/// - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
/// - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active, and a
///   single-method relationship `g = c × 10` also becomes active — `g` is *forced*
///   while this branch is active (see [`property_model::Sheet::is_forced`]), so its
///   Inspector field is disabled and it is highlighted in the graph.
/// - Any other `p`: the two systems are independent and `g` is not forced.
pub const DEMO_SOURCE: &str = r#"sheet demo {
    cell a: f64 = 2.0;
    cell b: f64 = 3.0;
    cell c: f64;
    cell d: f64 = 4.0;
    cell e: f64 = 5.0;
    cell f: f64;
    cell g: f64;
    cell p: i32 = 0;

    relationship {
        method [a, b] -> [c] { a * b }
        method [b, c] -> [a] { c / b }
        method [a, c] -> [b] { c / a }
    }

    relationship {
        method [d, e] -> [f] { d * e }
        method [e, f] -> [d] { f / e }
        method [d, f] -> [e] { f / d }
    }

    conditional p {
        0i32 => {
            method [f] -> [c] { f }
            method [c] -> [f] { c }
        }
        1i32 => {
            method [f] -> [c] { f * 2.0 }
            method [c] -> [f] { c / 2.0 }
        }
    }

    conditional p {
        1i32 => {
            method [c] -> [g] { c * 10.0 }
        }
    }
}
"#;

/// Root component: Spectrum theme wrapper, graph+inspector row on top and a
/// collapsible pm-lang source panel docked at the bottom.
#[component]
pub fn App() -> Element {
    let editor_source = use_signal(|| DEMO_SOURCE.to_string());
    let applied_source = use_signal(|| DEMO_SOURCE.to_string());
    let source_panel_open = use_signal(|| true);

    let initial = build_sheet(DEMO_SOURCE);
    let (initial_sheet, initial_labels) = initial
        .sheet_labels
        .expect("DEMO_SOURCE must parse successfully");
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let error = use_signal(|| initial.error);
    // DEMO_SOURCE always parses, so any startup error is a runtime (propagate)
    // error, never a parse error.
    let error_is_parse = use_signal(|| false);

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
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    GraphView { data: graph_data }
                    Inspector { sheet, labels, error, error_is_parse, applied_source }
                }
                SourcePanel {
                    editor_source,
                    applied_source,
                    sheet,
                    labels,
                    error,
                    error_is_parse,
                    open: source_panel_open,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_source_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(DEMO_SOURCE);
        let (sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn demo_source_g_forced_when_p_is_one() {
        let outcome = build_sheet(DEMO_SOURCE);
        let (mut sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
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
        let outcome = build_sheet(DEMO_SOURCE);
        let (mut sheet, labels) = outcome.sheet_labels.expect("DEMO_SOURCE must build");
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
