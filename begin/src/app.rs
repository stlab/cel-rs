//! Root [`App`] component.

use dioxus::prelude::*;
use property_model::Sheet;

use crate::bridge::{Labels, to_graph_data};
use crate::demo_source::{build_sheet, load_demo_source};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::SpTheme;

/// Root component: Spectrum theme wrapper with the graph and Inspector filling the
/// viewport. The demo pm-lang source lives in `begin/assets/demo.pm` — on desktop,
/// editing it while running under `dx serve` hot-reloads the sheet into this running
/// app via [`crate::demo_source::spawn_hot_reload`], exactly as if the old Apply
/// button had been pressed.
///
/// A read or parse failure at startup does not prevent the app from launching: it
/// prints the diagnostic to stderr and starts with an empty sheet instead, so a
/// syntax error in `demo.pm` can be fixed and hot-reloaded in without restarting.
#[component]
pub fn App() -> Element {
    let (initial_sheet, initial_labels, initial_active_source) = match load_demo_source() {
        Ok(source) => {
            let outcome = build_sheet(&source);
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            match outcome.sheet_labels {
                Some((sheet, labels)) => (sheet, labels, source),
                None => (Sheet::new(), Labels::new(), String::new()),
            }
        }
        Err(err) => {
            eprintln!("{err}");
            (Sheet::new(), Labels::new(), String::new())
        }
    };
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_active_source);

    #[cfg(feature = "desktop")]
    {
        let mut sheet = sheet;
        let mut labels = labels;
        let mut active_source = active_source;
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
            crate::demo_source::spawn_hot_reload(move || {
                let _ = tx.unbounded_send(());
            });
            spawn(async move {
                use futures_util::StreamExt;
                while rx.next().await.is_some() {
                    let source = match crate::demo_source::load_demo_source() {
                        Ok(source) => source,
                        Err(err) => {
                            eprintln!("{err}");
                            continue;
                        }
                    };
                    let outcome = crate::demo_source::build_sheet(&source);
                    if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(source);
                    }
                    if let Some(msg) = outcome.error {
                        eprintln!("{msg}");
                    }
                }
            });
        });
    }

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
