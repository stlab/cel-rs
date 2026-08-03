//! Root [`App`] component.

use adam_rs::Sheet;
use dioxus::prelude::*;

use crate::bridge::{Labels, to_graph_data};
use crate::demo_source::{
    ActiveSource, SourceOrigin, available_demos, build_sheet, load_demo_source,
};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::{SpActionButton, SpActionGroup, SpTheme};

/// Root component: Spectrum theme wrapper with a demo picker, the graph, and
/// the Inspector filling the viewport. `begin` ships with several example
/// property models (`begin/assets/*.adm2` — see
/// [`crate::demo_source::available_demos`]); [`DemoPicker`] switches which
/// one is loaded. On desktop, editing the *currently selected* demo's file
/// while running under `dx serve` hot-reloads the sheet into this running
/// app via [`crate::demo_source::spawn_hot_reload`], exactly as if the old
/// Apply button had been pressed.
///
/// A read or parse failure loading a demo does not prevent the app from
/// launching or switching: it prints the diagnostic to stderr and falls back
/// to an empty sheet instead, so a syntax error can be fixed and
/// hot-reloaded in without restarting.
#[component]
pub fn App() -> Element {
    // The webview always probes `/favicon.ico` at the origin root regardless of the
    // `document::Link` below (standard Chromium behavior). That literal path can't be
    // reached through the `asset!` pipeline, which always serves under `/assets/`, so
    // intercept it directly and answer with the same file served to the web build from
    // `begin/public/favicon.ico`.
    #[cfg(feature = "desktop")]
    dioxus::desktop::use_asset_handler("favicon.ico", |_request, responder| {
        let response = dioxus::desktop::wry::http::Response::builder()
            .header("Content-Type", "image/x-icon")
            .header("Access-Control-Allow-Origin", "*")
            .body(include_bytes!("../public/favicon.ico").to_vec())
            .expect("valid favicon response");
        responder.respond(response);
    });

    let initial_demo_name = available_demos().first().copied().unwrap_or_default();
    let (initial_sheet, initial_labels, initial_active_source) = load_demo(initial_demo_name);
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
                    let name = active_source.read().name.clone();
                    eprintln!("loading begin/assets/{name}.adm2");
                    let source = match crate::demo_source::load_demo_source(&name) {
                        Ok(source) => source,
                        Err(err) => {
                            eprintln!("{err}");
                            continue;
                        }
                    };
                    let outcome = build_sheet(&source, &format!("begin/assets/{name}.adm2"));
                    if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(ActiveSource {
                            name: name.clone(),
                            text: source,
                            origin: SourceOrigin::Demo,
                        });
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
        document::Link { rel: "icon", r#type: "image/x-icon", href: "/favicon.ico" }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }

        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            system: "spectrum-two".to_string(),
            div {
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                DemoPicker { sheet, labels, active_source }
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    GraphView { data: graph_data }
                    Inspector { sheet, labels, active_source }
                }
            }
        }
    }
}

/// Loads demo `name`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns an
/// empty sheet instead of failing — see [`App`]'s doc comment for why. The
/// returned [`ActiveSource`] still carries `name` (and, if the read
/// succeeded, the source text that failed to parse) even on failure, so the
/// desktop hot-reload loop keeps reloading the right file and can recover
/// once the on-disk error is fixed, instead of losing track of which demo
/// was selected.
fn load_demo(name: &str) -> (Sheet, Labels, ActiveSource) {
    match load_demo_source(name) {
        Ok(source) => {
            let outcome = build_sheet(&source, &format!("begin/assets/{name}.adm2"));
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            let active_source = ActiveSource {
                name: name.to_string(),
                text: source,
                origin: SourceOrigin::Demo,
            };
            match outcome.sheet_labels {
                Some((sheet, labels)) => (sheet, labels, active_source),
                None => (Sheet::new(), Labels::new(), active_source),
            }
        }
        Err(err) => {
            eprintln!("{err}");
            (
                Sheet::new(),
                Labels::new(),
                ActiveSource {
                    name: name.to_string(),
                    text: String::new(),
                    origin: SourceOrigin::Demo,
                },
            )
        }
    }
}

/// Picker row listing every demo from [`available_demos`]; clicking one
/// loads it into `sheet`/`labels`/`active_source`, highlighting whichever
/// name matches `active_source`'s current value.
#[component]
fn DemoPicker(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
) -> Element {
    let current = active_source.read().name.clone();

    rsx! {
        div {
            style: "padding: 8px 12px; border-bottom: 1px solid #ccc; flex: none;",
            SpActionGroup {
                compact: true,
                for &name in available_demos() {
                    SpActionButton {
                        key: "{name}",
                        selected: name == current,
                        onclick: {
                            let mut sheet = sheet;
                            let mut labels = labels;
                            let mut active_source = active_source;
                            move |_| {
                                let (new_sheet, new_labels, new_active_source) = load_demo(name);
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                                active_source.set(new_active_source);
                            }
                        },
                        "{name}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toy_example_source() -> &'static str {
        crate::demo_source::DEMOS_WITH_SOURCE
            .iter()
            .find(|&&(name, _)| name == "toy_example")
            .map(|&(_, source)| source)
            .expect("toy_example.adm2 must be bundled")
    }

    #[test]
    fn demo_source_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(toy_example_source(), "toy_example.adm2");
        let (sheet, labels) = outcome.sheet_labels.expect("toy_example.adm2 must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn demo_source_g_forced_when_p_is_one() {
        let outcome = build_sheet(toy_example_source(), "toy_example.adm2");
        let (mut sheet, labels) = outcome.sheet_labels.expect("toy_example.adm2 must build");
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
        let outcome = build_sheet(toy_example_source(), "toy_example.adm2");
        let (mut sheet, labels) = outcome.sheet_labels.expect("toy_example.adm2 must build");
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

    #[test]
    fn load_demo_unknown_name_falls_back_to_empty_sheet() {
        let (sheet, labels, active) = load_demo("does_not_exist");
        assert_eq!(sheet.cells().count(), 0);
        assert_eq!(labels.cells.len(), 0);
        assert_eq!(
            active.name, "does_not_exist",
            "name must be preserved on failure so hot-reload keeps targeting the right file"
        );
    }
}
