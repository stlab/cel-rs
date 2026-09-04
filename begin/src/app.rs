//! Root [`App`] component.

use adam_rs::Sheet;
use dioxus::prelude::*;

use crate::example_source::{ActiveSource, SourceOrigin, available_examples, load_example_source};
use adam_web_ui::GraphView;
use adam_web_ui::Labels;
use adam_web_ui::SheetInspector;
use adam_web_ui::spectrum::{
    SpActionButton, SpActionGroup, SpDivider, SpHeading, SpIconZoomIn, SpIconZoomOut, SpSideNav,
    SpSideNavItem, SpSwitch, SpTheme,
};
use adam_web_ui::to_graph_data;
use adam_web_ui::{Renderer, build_sheet};

/// Root component: Spectrum theme wrapper with an examples picker, the graph, and
/// the SheetInspector filling the viewport. `begin` ships with several example
/// property models (`begin/examples/*.adm2` — see
/// [`crate::example_source::available_examples`]); [`ExamplesPicker`] switches
/// which one is loaded. On desktop, editing the *currently selected* example's
/// file, or adding/removing a file under `begin/examples/`, live-updates this
/// running app via [`crate::example_source::spawn_examples_watch`], exactly as
/// if the old Apply button had been pressed.
///
/// A read or parse failure loading an example does not prevent the app from
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

    let initial_example_name = available_examples().first().cloned().unwrap_or_default();
    let (initial_sheet, initial_labels, initial_active_source) =
        load_example(&initial_example_name);
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);
    let active_source = use_signal(|| initial_active_source);
    let example_names = use_signal(available_examples);

    // Holds the `notify` watcher on `begin/examples/` established in the
    // `reload_tx` hook below. Unlike `watcher_slot` (the opened-file watcher,
    // which changes targets as the user opens different files), this watch
    // target never changes for the life of the app — the slot exists purely
    // to keep the `RecommendedWatcher` alive for as long as `App` is mounted;
    // dropping it (e.g. if it were a temporary instead) would immediately
    // stop the OS-level watch, exactly as `spawn_examples_watch`'s own doc
    // comment warns.
    #[cfg(feature = "desktop")]
    let examples_watcher_slot: Signal<Option<notify::RecommendedWatcher>> = use_signal(|| None);

    // The reload channel is shared by two producers: a filesystem watch on
    // `begin/examples/` for the currently selected example (below), and, on desktop,
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
        let mut example_names = example_names;
        let mut examples_watcher_slot = examples_watcher_slot;
        use_hook(move || {
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<()>();
            match crate::example_source::spawn_examples_watch({
                let tx = tx.clone();
                move || {
                    let _ = tx.unbounded_send(());
                }
            }) {
                Ok(watcher) => examples_watcher_slot.set(Some(watcher)),
                Err(err) => eprintln!("failed to watch begin/examples/: {err}"),
            }
            spawn(async move {
                use futures_util::StreamExt;
                while rx.next().await.is_some() {
                    example_names.set(crate::example_source::available_examples());
                    let current = active_source.read().clone();
                    let loaded = match &current.origin {
                        SourceOrigin::Example => {
                            eprintln!("loading begin/examples/{}.adm2", current.name);
                            load_example_source(&current.name)
                        }
                        SourceOrigin::Opened(path) => {
                            eprintln!("loading {}", path.to_string_lossy());
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
                    let outcome = build_sheet(&source, &current.file_name(), &Renderer::styled());
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
    // `ExamplesPicker` (switching back to an example, via `on_example_selected` below)
    // clear/replace this slot, so neither ever leaves a stale opened-file
    // watcher running.
    #[cfg(feature = "desktop")]
    let watcher_slot: Signal<Option<notify::RecommendedWatcher>> = use_signal(|| None);

    // `ExamplesPicker` itself has no notion of "desktop" or file watchers — it
    // just calls this after switching examples. On desktop this clears
    // `watcher_slot`, dropping (and so stopping) any watcher left over from a
    // previously opened file; on other platforms there's no watcher to clear.
    // Threading this through a platform-agnostic `Callback<()>` (rather than
    // giving `ExamplesPicker` a `#[cfg(feature = "desktop")]`-gated `watcher_slot`
    // parameter directly) sidesteps a `#[component]`-macro limitation: a
    // `#[cfg]` on one parameter correctly omits that field from the generated
    // Props struct, but the generated function body still unconditionally
    // destructures it, so cfg-gating an individual prop that way fails to
    // compile on the excluded platform.
    #[cfg(feature = "desktop")]
    let on_example_selected: Callback<()> = {
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
    // `use_signal` inside the web `OpenFileControls` — so `on_example_selected`
    // below can clear it, exactly mirroring desktop's `watcher_slot` handling.
    #[cfg(not(feature = "desktop"))]
    let refresh_handle: Signal<Option<u32>> = use_signal(|| None);
    #[cfg(not(feature = "desktop"))]
    let on_example_selected: Callback<()> = {
        let mut refresh_handle = refresh_handle;
        Callback::new(move |()| {
            refresh_handle.set(None);
        })
    };

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));
    // Identifies which source the current graph_data snapshot belongs to —
    // stable across a hot-reload of the *same* file (so an in-place edit
    // keeps the graph's live layout), but distinct whenever a different example
    // or opened file becomes active. See `GraphView`'s doc comment for why
    // graph.js needs this: cell/relationship node ids are only unique within
    // one Sheet, so without an explicit "did the source change" signal, an
    // example switch can silently recycle an old id for an unrelated node and
    // carry over its stale layout position (and previously, its stale box
    // width — see the graph.js fix this follows).
    let source_id = use_memo(move || active_source.read().file_name());
    let source_text = use_memo(move || active_source.read().text.clone());
    let source_name = use_memo(move || active_source.read().file_name());
    let graph_id = use_signal(|| "graph-container".to_string());

    // Drives graph.js's show/hide-inactive-branches mode via
    // `window.beginGraph.setShowInactive`; lives here (rather than in
    // `GraphView`) because its control now sits in the top bar alongside
    // `OpenFileControls`, not inside the graph canvas. Defaults to `true`
    // (dim, not hide) to match graph.js's own initial `showInactive` value,
    // and persists across example/file switches since `App` is never
    // remounted by them.
    //
    // Also writes `window.__beginShowInactive`, mirroring the existing
    // `window.__beginGraphData` seam: `GraphView`'s effect calls `init` (not
    // `update`) on a source switch, which builds a brand-new `GraphInstance` in
    // graph.js — that instance seeds its own `showInactive` from this global
    // rather than defaulting to `true`, so this toggle's current value survives
    // a source switch (which this `use_effect` alone would not re-run for,
    // since `graph_id`/`source_id` aren't read here — only `show_inactive`
    // itself re-fires this effect).
    let mut show_inactive = use_signal(|| true);
    use_effect(move || {
        let show = *show_inactive.read();
        let id = graph_id.peek().clone();
        spawn(async move {
            let _ = document::eval(&format!(
                "window.__beginShowInactive = {show}; if (typeof window.beginGraph !== 'undefined') window.beginGraph.setShowInactive('{id}', {show});"
            ))
            .await;
        });
    });

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
        document::Link { rel: "stylesheet", href: asset!("/assets/app-shell.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Link { rel: "stylesheet", href: asset!("/assets/inspector.css") }
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
                div {
                    style: "padding: 8px 12px; border-bottom: 1px solid #ccc; flex: none; display: flex; align-items: center; gap: 16px;",
                    {open_file_controls}
                    SpSwitch {
                        checked: *show_inactive.read(),
                        onclick: move |_| {
                            let next = !*show_inactive.read();
                            show_inactive.set(next);
                        },
                        "Show inactive"
                    }
                    SpActionGroup {
                        compact: true,
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.zoomOut('{id}');")).await;
                                });
                            },
                            SpIconZoomOut {}
                        }
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.resetZoom('{id}');")).await;
                                });
                            },
                            "Fit"
                        }
                        SpActionButton {
                            onclick: move |_| {
                                let id = graph_id.peek().clone();
                                spawn(async move {
                                    let _ = document::eval(&format!("window.beginGraph.zoomIn('{id}');")).await;
                                });
                            },
                            SpIconZoomIn {}
                        }
                    }
                }
                div {
                    style: "flex: 1; display: flex; overflow: hidden; min-height: 0;",
                    ExamplesPicker { sheet, labels, active_source, example_names, on_select: on_example_selected }
                    GraphView { graph_id, data: graph_data, source_id }
                    SheetInspector { sheet, labels, source_text, source_name }
                }
            }
        }
    }
}

/// Loads example `name`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns an
/// empty sheet instead of failing — see [`App`]'s doc comment for why. The
/// returned [`ActiveSource`] still carries `name` (and, if the read
/// succeeded, the source text that failed to parse) even on failure, so the
/// desktop hot-reload loop keeps reloading the right file and can recover
/// once the on-disk error is fixed, instead of losing track of which
/// example was selected.
///
/// - Complexity: O(n) in the length of the example's source, plus the cost
///   of one `build_sheet` parse/propagate.
fn load_example(name: &str) -> (Sheet, Labels, ActiveSource) {
    match load_example_source(name) {
        Ok(source) => {
            let outcome = build_sheet(
                &source,
                &format!("begin/examples/{name}.adm2"),
                &Renderer::styled(),
            );
            if let Some(err) = &outcome.error {
                adam_web_ui::diagnostics::report_error(err);
            }
            let active_source = ActiveSource {
                name: name.to_string(),
                text: source,
                origin: SourceOrigin::Example,
            };
            match outcome.sheet_labels {
                Some((sheet, labels)) => (sheet, labels, active_source),
                None => (Sheet::new(), Labels::new(), active_source),
            }
        }
        Err(err) => {
            adam_web_ui::diagnostics::report_error(&err);
            (
                Sheet::new(),
                Labels::new(),
                ActiveSource {
                    name: name.to_string(),
                    text: String::new(),
                    origin: SourceOrigin::Example,
                },
            )
        }
    }
}

/// Reads `path`, builds its sheet, and returns it alongside the
/// [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns `None`
/// in place of a sheet/labels pair, leaving the caller's last-good sheet and
/// labels in place instead of replacing them with an empty one — unlike
/// [`load_example`] (used only for the initial pick of an *example*, which has no
/// "last-good" state to preserve across a switch), this is what the design's
/// Global Constraints require for opening/refreshing a file specifically. The
/// returned [`ActiveSource`] still carries the opened path (and, if the read
/// succeeded, the source text that failed to parse) even on failure, so the
/// live-reload loop keeps targeting the right file and can recover once it's
/// fixed.
///
/// - Complexity: O(n) in the size of the file at `path`, plus the cost of
///   one `build_sheet` parse/propagate.
#[cfg(feature = "desktop")]
fn load_opened(path: std::path::PathBuf) -> (Option<(Sheet, Labels)>, ActiveSource) {
    let file_name = path.display().to_string();
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.clone());
    match crate::open_file::read_opened_file(&path) {
        Ok(source) => {
            let outcome = build_sheet(&source, &file_name, &Renderer::styled());
            if let Some(err) = &outcome.error {
                eprintln!("{err}");
            }
            let active_source = ActiveSource {
                name,
                text: source,
                origin: SourceOrigin::Opened(path.into_os_string()),
            };
            (outcome.sheet_labels, active_source)
        }
        Err(err) => {
            eprintln!("{err}");
            (
                None,
                ActiveSource {
                    name,
                    text: String::new(),
                    origin: SourceOrigin::Opened(path.into_os_string()),
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
///   origin-aware reload dispatch an example's hot-reload does.
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
        SpActionGroup {
            compact: true,
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
                        let (new_sheet_labels, new_active) = load_opened(path.clone());
                        if let Some((new_sheet, new_labels)) = new_sheet_labels {
                            sheet.set(new_sheet);
                            labels.set(new_labels);
                        }
                        active_source.set(new_active);
                        match crate::open_file::spawn_watch(path, move || {
                            let _ = reload_tx.unbounded_send(());
                        }) {
                            Ok(watcher) => watcher_slot.set(Some(watcher)),
                            Err(err) => {
                                eprintln!("failed to watch opened file: {err}");
                                // Otherwise a failed watch on the new file would leave
                                // the *previous* file's watcher running, violating "at
                                // most one opened-file watch is ever active" — active_source
                                // has already switched to the new file above, so a stale
                                // watch on the old one is never correct to keep.
                                watcher_slot.set(None);
                            }
                        }
                    });
                },
                "Open…"
            }
        }
        if let SourceOrigin::Opened(_) = active_source.read().origin {
            span { style: "padding-left: 8px;", "{active_source.read().name}" }
        }
    }
}

/// Builds a sheet from a web-side [`crate::open_file::OpenedFilePayload`] and
/// returns it alongside the [`ActiveSource`] describing what just loaded.
///
/// A read or parse failure prints the diagnostic to stderr and returns `None`
/// in place of a sheet/labels pair, leaving the caller's last-good sheet and
/// labels in place instead of replacing them with an empty one (see
/// [`load_opened`]'s doc comment for why this differs from [`load_example`]).
///
/// - Complexity: O(n) in the length of `payload.text`, plus the cost of one
///   `build_sheet` parse/propagate.
#[cfg(not(feature = "desktop"))]
fn load_from_payload(
    payload: crate::open_file::OpenedFilePayload,
) -> (Option<(Sheet, Labels)>, ActiveSource) {
    let outcome = build_sheet(&payload.text, &payload.name, &Renderer::styled());
    if let Some(err) = &outcome.error {
        adam_web_ui::diagnostics::report_error(err);
    }
    let active_source = ActiveSource {
        name: payload.name.clone(),
        text: payload.text,
        origin: SourceOrigin::Opened(payload.name.into()),
    };
    (outcome.sheet_labels, active_source)
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
        SpActionGroup {
            compact: true,
            SpActionButton {
                onclick: move |_| {
                    let mut sheet = sheet;
                    let mut labels = labels;
                    let mut active_source = active_source;
                    let mut refresh_handle = refresh_handle;
                    spawn(async move {
                        let mut eval = document::eval(crate::open_file::OPEN_SCRIPT);
                        let result = eval.recv::<Option<crate::open_file::OpenResult>>().await;
                        let Ok(result) = result else {
                            adam_web_ui::diagnostics::report_error(&format!(
                                "failed to open file: eval channel error: {result:?}"
                            ));
                            return;
                        };
                        let Some(result) = result else { return }; // cancelled — silent no-op
                        let payload = match result {
                            crate::open_file::OpenResult::Payload(payload) => payload,
                            crate::open_file::OpenResult::Failed { error } => {
                                adam_web_ui::diagnostics::report_error(&format!(
                                    "failed to open file: {error}"
                                ));
                                return;
                            }
                        };
                        refresh_handle.set(payload.id);
                        let (new_sheet_labels, new_active) = load_from_payload(payload);
                        if let Some((new_sheet, new_labels)) = new_sheet_labels {
                            sheet.set(new_sheet);
                            labels.set(new_labels);
                        }
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
                            let result = eval.recv::<Option<crate::open_file::OpenResult>>().await;
                            let Ok(result) = result else {
                                adam_web_ui::diagnostics::report_error(&format!(
                                    "failed to refresh file: eval channel error: {result:?}"
                                ));
                                return;
                            };
                            let Some(result) = result else { return }; // stale/unknown id — silent no-op
                            let payload = match result {
                                crate::open_file::OpenResult::Payload(payload) => payload,
                                crate::open_file::OpenResult::Failed { error } => {
                                    adam_web_ui::diagnostics::report_error(&format!(
                                        "failed to refresh file: {error}"
                                    ));
                                    return;
                                }
                            };
                            let (new_sheet_labels, new_active) = load_from_payload(payload);
                            if let Some((new_sheet, new_labels)) = new_sheet_labels {
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                            }
                            active_source.set(new_active);
                        });
                    },
                    "Refresh"
                }
            }
        }
        if let SourceOrigin::Opened(_) = active_source.read().origin {
            span { style: "padding-left: 8px;", "{active_source.read().name}" }
        }
    }
}

/// Sidebar panel listing every example from `example_names`; clicking one
/// loads it into `sheet`/`labels`/`active_source`, highlighting whichever
/// name matches `active_source`'s current value, then calls `on_select` —
/// on desktop, `App` uses this to clear any watcher left over from a
/// previously opened file (see `App`'s `on_example_selected`). Scrolls
/// internally once the list outgrows the panel's height, so the list can
/// grow arbitrarily without crowding the rest of the window.
#[component]
fn ExamplesPicker(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<ActiveSource>,
    example_names: Signal<Vec<String>>,
    on_select: Callback<()>,
) -> Element {
    let is_example_active = matches!(active_source.read().origin, SourceOrigin::Example);
    let current = active_source.read().name.clone();
    // Empty string never matches a real example name, so when an opened file
    // (rather than an example) is active, this deliberately leaves every
    // `SpSideNavItem` unselected instead of matching one by coincidence.
    let sidenav_value = if is_example_active {
        current.clone()
    } else {
        String::new()
    };

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box; border-right: 1px solid #ccc;",
            SpHeading { "Examples" }
            SpDivider {}
            SpSideNav {
                value: sidenav_value,
                for name in example_names.read().iter().cloned() {
                    SpSideNavItem {
                        key: "{name}",
                        label: name.clone(),
                        value: name.clone(),
                        selected: is_example_active && name == current,
                        onclick: {
                            let mut sheet = sheet;
                            let mut labels = labels;
                            let mut active_source = active_source;
                            let name = name.clone();
                            move |_| {
                                let (new_sheet, new_labels, new_active_source) = load_example(&name);
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                                active_source.set(new_active_source);
                                on_select.call(());
                            }
                        },
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
        crate::example_source::EXAMPLES_WITH_SOURCE
            .iter()
            .find(|&&(name, _)| name == "toy_example")
            .map(|&(_, source)| source)
            .expect("toy_example.adm2 must be bundled")
    }

    #[test]
    fn toy_example_g_not_forced_when_p_is_zero() {
        let outcome = build_sheet(
            toy_example_source(),
            "toy_example.adm2",
            &Renderer::styled(),
        );
        let (sheet, labels) = outcome.sheet_labels.expect("toy_example.adm2 must build");
        let g_id = sheet
            .cells()
            .find(|&id| labels.cells.get(&id).map(|m| m.label.as_str()) == Some("g"))
            .unwrap();
        assert!(!sheet.is_forced(g_id), "g should not be forced when p == 0");
    }

    #[test]
    fn toy_example_g_forced_when_p_is_one() {
        let outcome = build_sheet(
            toy_example_source(),
            "toy_example.adm2",
            &Renderer::styled(),
        );
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
    fn toy_example_g_unforced_again_after_p_returns_to_zero() {
        let outcome = build_sheet(
            toy_example_source(),
            "toy_example.adm2",
            &Renderer::styled(),
        );
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
    fn load_example_unknown_name_falls_back_to_empty_sheet() {
        let (sheet, labels, active) = load_example("does_not_exist");
        assert_eq!(sheet.cells().count(), 0);
        assert_eq!(labels.cells.len(), 0);
        assert_eq!(
            active.name, "does_not_exist",
            "name must be preserved on failure so hot-reload keeps targeting the right file"
        );
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn load_opened_missing_file_returns_none_sheet_labels() {
        let path = std::path::PathBuf::from("/definitely/does/not/exist/nope.adm2");

        let (sheet_labels, active) = load_opened(path);

        assert!(
            sheet_labels.is_none(),
            "a read failure must return None, not an empty sheet, so the caller can leave its last-good sheet/labels in place"
        );
        assert_eq!(active.name, "nope.adm2");
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn load_opened_parse_error_returns_none_sheet_labels() {
        let path = std::env::temp_dir().join("begin_app_test_load_opened_parse_error.adm2");
        std::fs::write(&path, "sheet s { cell x }").unwrap();

        let (sheet_labels, active) = load_opened(path.clone());

        std::fs::remove_file(&path).unwrap();
        assert!(
            sheet_labels.is_none(),
            "a parse failure must return None, not an empty sheet, so the caller can leave its last-good sheet/labels in place"
        );
        assert_eq!(active.name, "begin_app_test_load_opened_parse_error.adm2");
    }

    #[test]
    #[cfg(not(feature = "desktop"))]
    fn load_from_payload_parse_error_returns_none_sheet_labels() {
        let payload = crate::open_file::OpenedFilePayload {
            id: None,
            name: "broken.adm2".to_string(),
            text: "sheet s { cell x }".to_string(),
        };

        let (sheet_labels, active) = load_from_payload(payload);

        assert!(
            sheet_labels.is_none(),
            "a parse failure must return None, not an empty sheet, so the caller can leave its last-good sheet/labels in place"
        );
        assert_eq!(active.name, "broken.adm2");
    }
}
