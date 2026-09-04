//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mounts D3 once via the element's `onmounted` event; pushes JSON updates
//! via `document::eval` whenever the `data` signal changes. Each update also
//! writes to its own entry in `window.__beginGraphData` (a map keyed by
//! container id) so that `onmounted`'s polling loop always calls `init` with
//! the latest snapshot for this container rather than the one captured at
//! mount time. Keying by container id also keeps multiple `GraphView`s on one
//! page from clobbering each other's snapshots.
//!
//! `graph_id` names the `<div>` this instance mounts into, and is passed to the
//! `window.beginGraph.init` call — see `begin/assets/graph.js` — so the graph
//! attaches to this component's own container rather than a hardcoded id.
//!
//! `source_id` (see `App`'s doc comment for how it's derived) is compared against the last
//! source this component initialized for: unchanged means "the same source got a new
//! snapshot" (e.g. a hot-reloaded edit — call `update`, keeping the live layout); changed means
//! "a different demo/file just became active" — call `init`, which replaces any existing
//! `graph.js` instance for this id with a brand new one, so a stale position/width can never
//! bleed in from an unrelated node that happens to reuse the same id (cell/relationship node
//! ids are only unique within one `Sheet`, not across different ones).

use dioxus::prelude::*;

use super::data::GraphData;

/// Returns `true` when `current_source` differs from `initialized_source` — i.e. [`GraphView`]
/// should call `window.beginGraph.init` (a different sheet, needing a fresh D3 instance with no
/// carried-over layout) rather than `update` (the same sheet, new data — preserve layout).
fn source_changed(current_source: &str, initialized_source: &str) -> bool {
    current_source != initialized_source
}

/// Builds the JavaScript that stores `data` (serialized) as container `container_id`'s entry in
/// `window.__beginGraphData` and invokes `window.beginGraph.<call>(container_id, data)` when the
/// driver script (`begin/assets/graph.js`) is loaded.
///
/// `container_id` is embedded as a JSON-encoded JS string literal, so any characters it contains
/// (quotes, backslashes, newlines) are escaped rather than breaking or injecting into the emitted
/// JS — `graph_id` is a public [`GraphView`] prop, so the id is not assumed to be a bare DOM id.
///
/// - Precondition: `call` is `"init"` or `"update"`.
/// - Complexity: O(n) in the size of `data` (serialization plus string formatting).
pub fn graph_drive_script(container_id: &str, data: &GraphData, call: &str) -> String {
    let id = serde_json::to_string(container_id).unwrap_or_else(|_| "\"\"".to_string());
    let json = serde_json::to_string(data).unwrap_or_default();
    format!(
        "window.__beginGraphData = window.__beginGraphData || {{}}; \
         window.__beginGraphData[{id}] = {json}; \
         if (typeof window.beginGraph !== 'undefined') \
         window.beginGraph.{call}({id}, window.__beginGraphData[{id}]);"
    )
}

/// Renders the property model bipartite graph using D3.
///
/// On mount, polls until D3 is ready, then calls `window.beginGraph.init`
/// using this container's own entry in `window.__beginGraphData` (a map
/// keyed by container id), which always holds its latest snapshot. On every
/// change to `data`, writes the latest snapshot to that same entry and calls
/// `window.beginGraph.update`. Keying by container id keeps multiple
/// `GraphView`s on one page from clobbering each other's snapshots. The JS
/// guard in `graph.js` makes any `update` call before `init` a no-op.
///
/// The zoom controls and the "Show inactive" toggle live in `App`'s top bar
/// (not here) — they only ever call `window.beginGraph.*`/set a signal `App`
/// owns, so they don't need to be inside this component to work.
#[component]
pub fn GraphView(
    graph_id: ReadSignal<String>,
    data: ReadSignal<GraphData>,
    source_id: ReadSignal<String>,
) -> Element {
    let container_id = graph_id.read().clone();
    let mut initialized_source = use_signal(|| source_id.peek().clone());

    use_effect(move || {
        let id = graph_id.read().clone();
        let current_source = source_id.read().clone();
        let is_new_source = source_changed(&current_source, &initialized_source.peek());
        if is_new_source {
            initialized_source.set(current_source);
        }
        let call = if is_new_source { "init" } else { "update" };
        let script = graph_drive_script(&id, &data.read(), call);
        spawn(async move {
            let _ = document::eval(&script).await;
        });
    });

    rsx! {
        div {
            id: "{container_id}",
            class: "graph-view",
            onmounted: move |_evt| async move {
                let id = graph_id.peek().clone();
                // Embed the container id as a JSON-encoded JS string literal (see
                // `graph_drive_script`) so a quote/backslash/newline in the id can't break or
                // inject into the polling script.
                let id = serde_json::to_string(&id).unwrap_or_else(|_| "\"\"".to_string());
                let json = serde_json::to_string(&data.peek().clone()).unwrap_or_default();
                let script = format!(
                    r#"window.__beginGraphData = window.__beginGraphData || {{}};
                       if (!({id} in window.__beginGraphData)) window.__beginGraphData[{id}] = {json};
                       (function tryInit(n) {{
                           if (typeof d3 !== 'undefined' && typeof window.beginGraph !== 'undefined') {{
                               window.beginGraph.init({id}, window.__beginGraphData[{id}]);
                           }} else if (n > 0) {{
                               setTimeout(function() {{ tryInit(n - 1); }}, 50);
                           }}
                       }})(60);"#
                );
                let _ = document::eval(&script).await;
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_changed_is_false_for_identical_sources() {
        assert!(!source_changed(
            "tutorial/first_sheet",
            "tutorial/first_sheet"
        ));
    }

    #[test]
    fn source_changed_is_true_for_different_sources() {
        assert!(source_changed(
            "tutorial/first_sheet",
            "tutorial/area_with_requirement"
        ));
    }

    fn empty_graph_data() -> GraphData {
        GraphData {
            nodes: vec![],
            links: vec![],
            changed: vec![],
            forced: vec![],
            forced_relationships: vec![],
            arrows: false,
        }
    }

    #[test]
    fn graph_drive_script_init_calls_begin_graph_init() {
        let script = graph_drive_script("g1", &empty_graph_data(), "init");
        // The id is embedded as a JSON string literal (double-quoted).
        assert!(script.contains("window.beginGraph.init(\"g1\""));
        assert!(script.contains("window.__beginGraphData[\"g1\"]"));
    }

    #[test]
    fn graph_drive_script_update_calls_begin_graph_update() {
        let script = graph_drive_script("g1", &empty_graph_data(), "update");
        assert!(script.contains("window.beginGraph.update(\"g1\""));
    }

    #[test]
    fn graph_drive_script_guards_on_begin_graph_being_defined() {
        let script = graph_drive_script("g1", &empty_graph_data(), "init");
        assert!(script.contains("typeof window.beginGraph !== 'undefined'"));
    }

    #[test]
    fn graph_drive_script_embeds_the_serialized_data() {
        let script = graph_drive_script("g1", &empty_graph_data(), "init");
        // GraphData serializes its fields; `nodes` is always present.
        assert!(script.contains("\"nodes\""));
    }

    #[test]
    fn graph_drive_script_escapes_a_quote_in_the_container_id() {
        // A container id containing a single quote must not break out of the JS string or
        // inject: JSON-encoding turns `'` into a plain character inside a double-quoted literal,
        // and there must be no raw `'g'` string-open left in the output.
        let script = graph_drive_script("a'b", &empty_graph_data(), "init");
        assert!(script.contains("window.beginGraph.init(\"a'b\""));
        assert!(!script.contains("['a'b']"));
    }

    #[test]
    fn graph_drive_script_escapes_a_double_quote_and_backslash_in_the_container_id() {
        // JSON-encoding escapes `"` and `\`, so the emitted literal stays a single well-formed
        // JS string rather than terminating early.
        let script = graph_drive_script("a\"b\\c", &empty_graph_data(), "init");
        assert!(script.contains(r#"window.beginGraph.init("a\"b\\c""#));
    }
}
