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

    // The reload channel is shared by two producers: `dx serve`'s hot-reload
    // notifications for the currently selected demo (below), and, on desktop,
    // a filesystem watcher on whichever file the user most recently opened
    // (installed by `OpenFileControls`). Either producer sending on `tx` wakes
    // the single consumer loop below, which reloads via `active_source`'s
    // current `SourceOrigin` — so only one reload path ever runs regardless of
    // which source is currently active.
    // `reload_tx` is wrapped in a `Signal` (rather than passed as the bare
    // `UnboundedSender`) purely so it can be a prop on `OpenFileControls`:
    // `#[component]`-generated Props require `PartialEq`, which
    // `UnboundedSender` doesn't implement but `Signal<T>` does regardless of
    // `T` (it compares by the underlying slot's identity, not `T`'s value).
    #[cfg(feature = "desktop")]
    let reload_tx: Signal<futures_channel::mpsc::UnboundedSender<()>> = {
        let mut sheet = sheet;
        let mut labels = labels;
        let mut active_source = active_source;
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
            crate::demo_source::spawn_hot_reload({
                let tx = tx.clone();
                move || {
                    let _ = tx.unbounded_send(());
                }
            });
            spawn(async move {
                use futures_util::StreamExt;
                while rx.next().await.is_some() {
                    let current = active_source.read().clone();
                    let loaded = match &current.origin {
                        SourceOrigin::Demo => {
                            eprintln!("loading begin/assets/{}.adm2", current.name);
                            load_demo_source(&current.name)
                        }
                        SourceOrigin::Opened(path) => {
                            eprintln!("loading {path}");
                            crate::open_file::read_opened_file(std::path::Path::new(path))
                        }
                    };
                    let source = match loaded {
                        Ok(source) => source,
                        Err(err) => {
                            eprintln!("{err}");
                            continue;
                        }
                    };
                    let outcome = build_sheet(&source, &current.file_name());
                    if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(ActiveSource {
                            text: source,
                            ..current
                        });
                    }
                    if let Some(msg) = outcome.error {
                        eprintln!("{msg}");
                    }
                }
            });
            Signal::new(tx)
        })
    };

    // Holds the `notify` watcher installed on the most recently opened file, if
    // any. Replacing it drops the previous watcher, which stops its OS-level
    // watch — both `OpenFileControls` (opening a different file) and
    // `DemoPicker` (switching back to a demo, via `on_demo_selected` below)
    // clear/replace this slot, so neither ever leaves a stale opened-file
    // watcher running.
    #[cfg(feature = "desktop")]
    let watcher_slot: Signal<Option<notify::RecommendedWatcher>> = use_signal(|| None);

    // `DemoPicker` itself has no notion of "desktop" or file watchers — it
    // just calls this after switching demos. On desktop this clears
    // `watcher_slot`, dropping (and so stopping) any watcher left over from a
    // previously opened file; on other platforms there's no watcher to clear.
    // Threading this through a platform-agnostic `Callback<()>` (rather than
    // giving `DemoPicker` a `#[cfg(feature = "desktop")]`-gated `watcher_slot`
    // parameter directly) sidesteps a `#[component]`-macro limitation: a
    // `#[cfg]` on one parameter correctly omits that field from the generated
    // Props struct, but the generated function body still unconditionally
    // destructures it, so cfg-gating an individual prop that way fails to
    // compile on the excluded platform.
    #[cfg(feature = "desktop")]
    let on_demo_selected: Callback<()> = {
        let mut watcher_slot = watcher_slot;
        Callback::new(move |()| {
            watcher_slot.set(None);
        })
    };

    // Web's equivalent of `watcher_slot`: holds the re-readable File System
    // Access handle id for whichever file was most recently opened, if any
    // (`None` means either nothing has been opened yet, or the browser only
    // gave us a one-shot `<input type="file">` result with nothing to
    // refresh). Lifted up to `App` — rather than living as a local
    // `use_signal` inside the web `OpenFileControls` — so `on_demo_selected`
    // below can clear it, exactly mirroring desktop's `watcher_slot` handling.
    #[cfg(not(feature = "desktop"))]
    let refresh_handle: Signal<Option<u32>> = use_signal(|| None);
    #[cfg(not(feature = "desktop"))]
    let on_demo_selected: Callback<()> = {
        let mut refresh_handle = refresh_handle;
        Callback::new(move |()| {
            refresh_handle.set(None);
        })
    };

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    // `rsx!` doesn't accept a bare `#[cfg(...)]` attribute on a child in this
    // position (it expects an element/component name next), so the
    // desktop-only `OpenFileControls` child is built as its own `Element`
    // here — under a real `#[cfg]`, so nothing on a web build references
    // `OpenFileControls`/`watcher_slot`/`notify` at all — and spliced into
    // the tree below via `{..}` interpolation.
    #[cfg(feature = "desktop")]
    let open_file_controls = rsx! {
        OpenFileControls {
            sheet,
            labels,
            active_source,
            reload_tx,
            watcher_slot,
        }
    };
    #[cfg(not(feature = "desktop"))]
    let open_file_controls = rsx! {
        OpenFileControls {
            sheet,
            labels,
            active_source,
            refresh_handle,
        }
    };

    rsx! {
        document::Link { rel: "icon", r#type: "image/x-icon", href: "/favicon.ico" }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }
        document::Script { src: asset!("/assets/open_file.js") }

        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            system: "spectrum-two".to_string(),
            div {
                style: "position: fixed; inset: 0; display: flex; flex-direction: column; overflow: hidden;",
                DemoPicker { sheet, labels, active_source, on_select: on_demo_selected }
                {open_file_controls}
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

/// Reads `path`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns an
/// empty sheet instead of failing — mirrors [`load_demo`]'s failure handling
/// (see [`App`]'s doc comment for why). The returned [`ActiveSource`] still
/// carries the opened path (and, if the read succeeded, the source text that
/// failed to parse) even on failure, so the live-reload loop keeps targeting
/// the right file.
#[cfg(feature = "desktop")]
fn load_opened(path: std::path::PathBuf) -> (Sheet, Labels, ActiveSource) {
    let file_name = path.display().to_string();
    match crate::open_file::read_opened_file(&path) {
        Ok(source) => {
            let outcome = build_sheet(&source, &file_name);
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| file_name.clone());
            let active_source = ActiveSource {
                name,
                text: source,
                origin: SourceOrigin::Opened(file_name),
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
                    name: file_name.clone(),
                    text: String::new(),
                    origin: SourceOrigin::Opened(file_name),
                },
            )
        }
    }
}

/// "Open…" button: opens the native file dialog, loads the picked file, and
/// (re)installs a filesystem watcher on it so external edits reload it live —
/// replacing any previously installed opened-file watcher (dropping the old
/// `notify::RecommendedWatcher` stops its OS-level watch, so at most one
/// opened-file watch is ever active).
///
/// - Precondition: `reload_tx` is the same channel the hot-reload consumer
///   loop in [`App`] is reading from, so a watch event here drives the same
///   origin-aware reload dispatch a demo's hot-reload does.
#[cfg(feature = "desktop")]
#[component]
fn OpenFileControls(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    reload_tx: Signal<futures_channel::mpsc::UnboundedSender<()>>,
    mut watcher_slot: Signal<Option<notify::RecommendedWatcher>>,
) -> Element {
    rsx! {
        SpActionButton {
            onclick: move |_| {
                let mut sheet = sheet;
                let mut labels = labels;
                let mut active_source = active_source;
                let reload_tx = reload_tx.read().clone();
                spawn(async move {
                    let Some(path) = crate::open_file::pick_file().await else {
                        return;
                    };
                    let (new_sheet, new_labels, new_active) = load_opened(path.clone());
                    sheet.set(new_sheet);
                    labels.set(new_labels);
                    active_source.set(new_active);
                    match crate::open_file::spawn_watch(path, move || {
                        let _ = reload_tx.unbounded_send(());
                    }) {
                        Ok(watcher) => watcher_slot.set(Some(watcher)),
                        Err(err) => eprintln!("failed to watch opened file: {err}"),
                    }
                });
            },
            "Open…"
        }
        if let SourceOrigin::Opened(_) = active_source.read().origin {
            span { style: "padding-left: 8px;", "{active_source.read().name}" }
        }
    }
}

/// Builds a sheet from a web-side [`crate::open_file::OpenedFilePayload`] and
/// returns it alongside the [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns an
/// empty sheet instead of failing — mirrors [`load_demo`]/[`load_opened`]'s
/// failure handling (see [`App`]'s doc comment for why).
#[cfg(not(feature = "desktop"))]
fn load_from_payload(
    payload: crate::open_file::OpenedFilePayload,
) -> (Sheet, Labels, ActiveSource) {
    let outcome = build_sheet(&payload.text, &payload.name);
    if let Some(err) = &outcome.error {
        eprintln!("{err}");
    }
    let active_source = ActiveSource {
        name: payload.name.clone(),
        text: payload.text,
        origin: SourceOrigin::Opened(payload.name),
    };
    match outcome.sheet_labels {
        Some((sheet, labels)) => (sheet, labels, active_source),
        None => (Sheet::new(), Labels::new(), active_source),
    }
}

/// "Open…"/"Refresh" controls for the web build: "Open…" always calls
/// `window.beginOpenFile.open()`; "Refresh" (rendered only once a
/// re-readable handle exists) re-reads that same handle. Neither watches for
/// changes automatically — browsers have no filesystem-watch API, so reload
/// here is always user-triggered.
#[cfg(not(feature = "desktop"))]
#[component]
fn OpenFileControls(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    mut refresh_handle: Signal<Option<u32>>,
) -> Element {
    rsx! {
        SpActionButton {
            onclick: move |_| {
                let mut sheet = sheet;
                let mut labels = labels;
                let mut active_source = active_source;
                let mut refresh_handle = refresh_handle;
                spawn(async move {
                    let mut eval = document::eval(crate::open_file::OPEN_SCRIPT);
                    let Ok(payload) = eval.recv::<Option<crate::open_file::OpenedFilePayload>>().await else {
                        return;
                    };
                    let Some(payload) = payload else { return };
                    refresh_handle.set(payload.id);
                    let (new_sheet, new_labels, new_active) = load_from_payload(payload);
                    sheet.set(new_sheet);
                    labels.set(new_labels);
                    active_source.set(new_active);
                });
            },
            "Open…"
        }
        if let Some(id) = *refresh_handle.read() {
            SpActionButton {
                onclick: move |_| {
                    let mut sheet = sheet;
                    let mut labels = labels;
                    let mut active_source = active_source;
                    spawn(async move {
                        let script = crate::open_file::refresh_script(id);
                        let mut eval = document::eval(&script);
                        let Ok(payload) = eval.recv::<Option<crate::open_file::OpenedFilePayload>>().await else {
                            return;
                        };
                        let Some(payload) = payload else { return };
                        let (new_sheet, new_labels, new_active) = load_from_payload(payload);
                        sheet.set(new_sheet);
                        labels.set(new_labels);
                        active_source.set(new_active);
                    });
                },
                "Refresh"
            }
        }
        if let SourceOrigin::Opened(_) = active_source.read().origin {
            span { style: "padding-left: 8px;", "{active_source.read().name}" }
        }
    }
}

/// Picker row listing every demo from [`available_demos`]; clicking one
/// loads it into `sheet`/`labels`/`active_source`, highlighting whichever
/// name matches `active_source`'s current value, then calls `on_select` —
/// on desktop, `App` uses this to clear any watcher left over from a
/// previously opened file (see `App`'s `on_demo_selected`).
#[component]
fn DemoPicker(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    on_select: Callback<()>,
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
                                on_select.call(());
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
