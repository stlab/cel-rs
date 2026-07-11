//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mounts D3 once via the element's `onmounted` event; pushes JSON updates
//! via `document::eval` whenever the `data` signal changes. Each update also
//! writes to `window.__beginGraphData` so that `onmounted`'s polling loop
//! always calls `init` with the latest snapshot rather than the one captured
//! at mount time.

use dioxus::prelude::*;

use crate::bridge::GraphData;
use crate::spectrum::{SpActionButton, SpActionGroup, SpIconZoomIn, SpIconZoomOut};

/// Renders the property model bipartite graph using D3.
///
/// On mount, polls until D3 is ready, then calls `window.beginGraph.init`
/// using `window.__beginGraphData`, which always holds the latest snapshot.
/// On every change to `data`, writes the latest snapshot to
/// `window.__beginGraphData` and calls `window.beginGraph.update`. The JS
/// guard in `graph.js` makes any `update` call before `init` a no-op.
#[component]
pub fn GraphView(data: ReadSignal<GraphData>) -> Element {
    use_effect(move || {
        let json = serde_json::to_string(&*data.read()).unwrap_or_default();
        spawn(async move {
            let _ = document::eval(&format!(
                "window.__beginGraphData = {}; if (typeof window.beginGraph !== 'undefined') window.beginGraph.update(window.__beginGraphData);",
                json
            ))
            .await;
        });
    });

    rsx! {
        div {
            id: "graph-container",
            style: "flex: 1; height: 100%; overflow: hidden; position: relative;",
            onmounted: move |_evt| async move {
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                // Seed __beginGraphData with the current snapshot; use_effect may
                // update it if the sheet changes before D3 finishes loading.
                // document::Script injects <script> tags asynchronously.
                let script = format!(
                    r#"if (!window.__beginGraphData) window.__beginGraphData = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init('graph-container', window.__beginGraphData);
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            },
            div {
                class: "graph-zoom-controls",
                SpActionGroup {
                    compact: true,
                    SpActionButton {
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.zoomOut();").await;
                            });
                        },
                        SpIconZoomOut {}
                    }
                    SpActionButton {
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.resetZoom();").await;
                            });
                        },
                        "Fit"
                    }
                    SpActionButton {
                        onclick: move |_| {
                            spawn(async move {
                                let _ = document::eval("window.beginGraph.zoomIn();").await;
                            });
                        },
                        SpIconZoomIn {}
                    }
                }
            }
        }
    }
}
