//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mounts D3 once via the element's `onmounted` event; pushes JSON updates
//! via `document::eval` whenever the `data` signal changes. Each update also
//! writes to `window.__beginGraphData` so that `onmounted`'s polling loop
//! always calls `init` with the latest snapshot rather than the one captured
//! at mount time.
//!
//! `source_id` (see `App`'s doc comment for how it's derived) is passed
//! alongside every `init`/`update` call so `graph.js` can tell "the same
//! source got a new snapshot" (e.g. a hot-reloaded edit — keep the live
//! layout) apart from "a different demo/file just became active" (wipe the
//! layout cache instead of risking a stale position/width bleeding in from
//! an unrelated node that happens to reuse the same id — cell/relationship
//! node ids are only unique within one `Sheet`, not across different ones).

use dioxus::prelude::*;

use crate::bridge::GraphData;

/// Renders the property model bipartite graph using D3.
///
/// On mount, polls until D3 is ready, then calls `window.beginGraph.init`
/// using `window.__beginGraphData`, which always holds the latest snapshot.
/// On every change to `data`, writes the latest snapshot to
/// `window.__beginGraphData` and calls `window.beginGraph.update`. The JS
/// guard in `graph.js` makes any `update` call before `init` a no-op.
///
/// The zoom controls and the "Show inactive" toggle live in `App`'s top bar
/// (not here) — they only ever call `window.beginGraph.*`/set a signal `App`
/// owns, so they don't need to be inside this component to work.
#[component]
pub fn GraphView(data: ReadSignal<GraphData>, source_id: ReadSignal<String>) -> Element {
    use_effect(move || {
        let json = serde_json::to_string(&*data.read()).unwrap_or_default();
        let source_id_json = serde_json::to_string(&*source_id.read()).unwrap_or_default();
        spawn(async move {
            let _ = document::eval(&format!(
                "window.__beginGraphData = {}; if (typeof window.beginGraph !== 'undefined') window.beginGraph.update(window.__beginGraphData, {});",
                json, source_id_json
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
                let source_id_json = serde_json::to_string(&source_id.peek().clone()).unwrap_or_default();
                // Seed __beginGraphData with the current snapshot; use_effect may
                // update it if the sheet changes before D3 finishes loading.
                // document::Script injects <script> tags asynchronously.
                let script = format!(
                    r#"if (!window.__beginGraphData) window.__beginGraphData = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init('graph-container', window.__beginGraphData, {source_id_json});
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            },
            GraphLegend {}
        }
    }
}

/// Explains the graph's shapes, line styles, and outline colors. A static
/// key, not tied to `data` - every symbol it documents is fixed by
/// `graph.css`/`graph.js`, not by which sheet happens to be loaded.
#[component]
fn GraphLegend() -> Element {
    rsx! {
        div {
            class: "graph-legend",
            // Every row's icon sits in a fixed-width `graph-legend-icon`
            // column, whatever shape is actually inside it (a 14px square,
            // a 10px diamond, a 22px-wide edge glyph) - so the text after
            // it always starts at the same x position instead of drifting
            // with each icon's own width.
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape cell" }
                }
                "Cell (value)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape relationship" }
                }
                "Relationship (constraint)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape conditional" }
                }
                "Conditional (branch)"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #arrowhead marker: a solid triangle,
                // tip in the direction of travel.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line { x1: "2", y1: "7", x2: "16", y2: "7", stroke: "#444", stroke_width: "1.5" }
                        path { d: "M16,3.5 L21,7 L16,10.5 Z", fill: "#444" }
                    }
                }
                "Depends on"
            }
            div {
                class: "graph-legend-row",
                // Mirrors graph.js's #dot marker: a solid circle on a
                // dashed line, matching link-control edges.
                div {
                    class: "graph-legend-icon",
                    svg {
                        view_box: "0 0 22 14",
                        width: "22",
                        height: "14",
                        line {
                            x1: "2", y1: "7", x2: "17", y2: "7",
                            stroke: "#444", stroke_width: "1.5", stroke_dasharray: "4 3",
                        }
                        circle { cx: "19", cy: "7", r: "3", fill: "#444" }
                    }
                }
                "Activates when matched"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape forced" }
                }
                "Forced (not directly editable)"
            }
            div {
                class: "graph-legend-row",
                div { class: "graph-legend-icon",
                    div { class: "graph-legend-shape inactive" }
                }
                "Inactive (branch not selected)"
            }
        }
    }
}
