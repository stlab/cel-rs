//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mounts D3 once via the element's `onmounted` event; pushes JSON updates
//! via `document::eval` whenever the `data` signal changes.

use dioxus::prelude::*;

use crate::bridge::GraphData;

/// Renders the property model bipartite graph using D3.
///
/// On mount, calls `window.beginGraph.init`. On every change to `data`,
/// calls `window.beginGraph.update` with the serialized [`GraphData`] JSON.
/// The JS guard in `graph.js` makes the initial `update` call before `init`
/// a no-op, so ordering between `onmounted` and the effect is not critical.
#[component]
pub fn GraphView(data: ReadOnlySignal<GraphData>) -> Element {
    use_effect(move || {
        let json = serde_json::to_string(&*data.read()).unwrap_or_default();
        spawn(async move {
            let _ = document::eval(&format!("window.beginGraph.update({})", json)).await;
        });
    });

    rsx! {
        div {
            id: "graph-container",
            style: "flex: 1; height: 100%; overflow: hidden;",
            onmounted: move |_evt| async move {
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                let _ = document::eval(
                    &format!("window.beginGraph.init('graph-container', {})", json),
                )
                .await;
            }
        }
    }
}
