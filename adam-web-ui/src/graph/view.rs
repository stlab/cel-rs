//! [`GraphView`] — renders the D3 force graph inside a `<div>`.
//!
//! Mount [`GraphView`] with a container id, graph-data signal and source id.
//! Ordinary data updates preserve layout. Source/container changes retire the
//! old evaluation channel and initialize a fresh graph; unmount cancels both
//! Rust reception and JS polling/subscriptions. The shared component owns this
//! lifecycle on desktop and web, independently of any Begin-only integration.
//! D3 submits geometry and absolute gesture proposals; only Adam-approved
//! frames reach the visible zoom layer.

use std::{
    cell::RefCell,
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    task::Poll,
};

use dioxus::core::Task;
use dioxus::prelude::*;

use super::{BridgeState, data::GraphData};

/// Owns the Rust receiver and its matching cancellable JavaScript evaluation.
struct ChannelSession {
    /// An identity retained by one-shot updates to reject delayed old failures.
    identity: Rc<()>,
    /// The live bidirectional evaluation.
    eval: document::Eval,
    /// The component-scoped Rust receiver.
    task: Task,
}

/// Returns whether a one-shot update still belongs to the current channel owner.
fn same_owner(current: Option<&Rc<()>>, expected: Option<&Rc<()>>) -> bool {
    match (current, expected) {
        (Some(current), Some(expected)) => Rc::ptr_eq(current, expected),
        (None, None) => true,
        _ => false,
    }
}

/// Selects a message before channel completion when both are ready.
///
/// A pending message never hides a completed/failed evaluation. This preserves
/// queued diagnostics while preventing a broken JS sender from hanging Rust.
async fn prefer_message<M: Future, E: Future>(
    message: M,
    mut ended: Pin<&mut E>,
) -> Result<M::Output, E::Output> {
    let mut message = std::pin::pin!(message);
    poll_fn(|context| {
        if let Poll::Ready(message) = message.as_mut().poll(context) {
            return Poll::Ready(Ok(message));
        }
        ended.as_mut().poll(context).map(Err)
    })
    .await
}

impl ChannelSession {
    /// Closes JS polling/subscriptions/resources before aborting the Rust receiver.
    fn stop(self) {
        let _ = self.eval.send(serde_json::json!({ "kind": "close" }));
        self.task.cancel();
    }
}

/// Embeds the tested shared channel runtime and safely encoded initial arguments.
///
/// - Complexity: O(n) time and space in serialized graph data.
fn graph_channel_script(container_id: &str, data: &GraphData, external_container: bool) -> String {
    let id = serde_json::to_string(container_id).expect("strings serialize");
    let data = serde_json::to_string(data).expect("graph data serializes");
    format!(
        "{}\nawait runGraphChannel(window, {id}, {data}, dioxus, {external_container});",
        include_str!("channel.js")
    )
}

/// Reports a terminal channel failure and requests JS resource disposal.
fn channel_failed(eval: document::Eval, mut failure: Signal<Option<String>>, message: String) {
    crate::diagnostics::report_error(&message);
    failure.set(Some(message.clone()));
    let _ = eval.send(serde_json::json!({ "kind": "fatal", "message": message }));
}

/// Processes typed messages sequentially through the generation's cached model.
async fn receive_views(mut eval: document::Eval, failure: Signal<Option<String>>) {
    let mut bridge = BridgeState::default();
    let mut ended = std::pin::pin!(eval.join::<serde_json::Value>());
    loop {
        let result = async {
            let payload =
                match prefer_message(eval.recv::<serde_json::Value>(), ended.as_mut()).await {
                    Ok(message) => message?,
                    Err(completion) => {
                        completion?;
                        anyhow::bail!("graph channel ended unexpectedly");
                    }
                };
            if let Some(reply) = bridge.receive(payload)? {
                eval.send(reply)?;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            channel_failed(eval, failure, format!("Graph view failed: {error}"));
            return;
        }
    }
}

/// Which `window.beginGraph` driver method a [`graph_drive_script`] invokes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GraphDrive {
    /// `window.beginGraph.init` — build a fresh D3 instance for the container, replacing any
    /// existing one (a new sheet, or the first mount).
    Init,
    /// `window.beginGraph.update` — push new data into the container's existing instance.
    Update,
}

impl GraphDrive {
    /// The `window.beginGraph` method name this variant invokes.
    fn method(self) -> &'static str {
        match self {
            GraphDrive::Init => "init",
            GraphDrive::Update => "update",
        }
    }
}

/// Builds the JavaScript that stores `data` (serialized) as container `container_id`'s entry in
/// `window.__beginGraphData` and invokes the [`GraphDrive`] method `call` on `window.beginGraph`
/// when the driver script (`begin/assets/graph.js`) is loaded.
///
/// `container_id` is embedded as a JSON-encoded JS string literal, so any characters it contains
/// (quotes, backslashes, newlines) are escaped rather than breaking or injecting into the emitted
/// JS — `graph_id` is a public [`GraphView`] prop, so the id is not assumed to be a bare DOM id.
/// The invoked method name comes from the [`GraphDrive`] enum, never a free string, so it can
/// never be an arbitrary (injecting) value.
///
/// - Complexity: O(n) in the size of `data` (serialization plus string formatting).
pub fn graph_drive_script(container_id: &str, data: &GraphData, call: GraphDrive) -> String {
    let id = serde_json::to_string(container_id).unwrap_or_else(|_| "\"\"".to_string());
    // Fall back to a valid JS literal (`null`), never an empty string, so the emitted assignment
    // stays syntactically valid even in the (unreachable for `GraphData`) serialization-failure case.
    let json = serde_json::to_string(data).unwrap_or_else(|_| "null".to_string());
    let method = call.method();
    format!(
        "window.__beginGraphData = window.__beginGraphData || Object.create(null); \
         window.__beginGraphData[{id}] = {json}; \
         if (typeof window.beginGraph !== 'undefined') \
         window.beginGraph.{method}({id}, window.__beginGraphData[{id}]); \
         dioxus.close(); return null;"
    )
}

/// Renders the property model bipartite graph using D3.
///
/// Installs a container-scoped bidirectional evaluation channel before D3
/// initialization on desktop and web (including book hosts). Each generation
/// owns one cached Adam model. Source/container replacement and unmount close
/// JavaScript subscriptions/polling and abort the Rust receiver. Errors are
/// reported through the platform diagnostics channel and a visible alert;
/// no JavaScript view policy substitutes for a failed model.
///
/// With `external_container`, the host supplies an existing container outside
/// this Dioxus root (as mdBook does). This component renders only its lifecycle
/// anchor and fallback diagnostic, never a second element with the same id.
/// The shared channel also displays failures inside that external container.
///
/// The zoom controls and the "Show inactive" toggle live in `App`'s top bar
/// (not here) — they only ever call `window.beginGraph.*`/set a signal `App`
/// owns, so they don't need to be inside this component to work.
///
/// # Examples
///
/// ```
/// use adam_web_ui::{GraphData, GraphView};
/// use dioxus::prelude::*;
///
/// fn graph(id: ReadSignal<String>, data: ReadSignal<GraphData>,
///          source: ReadSignal<String>) -> Element {
///     rsx! { GraphView { graph_id: id, data, source_id: source } }
/// }
/// ```
#[component]
pub fn GraphView(
    graph_id: ReadSignal<String>,
    data: ReadSignal<GraphData>,
    source_id: ReadSignal<String>,
    #[props(default = false)] external_container: bool,
) -> Element {
    let container_id = graph_id.read().clone();
    let mut mounted = use_signal(|| false);
    let mut failure = use_signal(|| None::<String>);
    let session = use_hook(|| Rc::new(RefCell::new(None::<ChannelSession>)));
    let cleanup = session.clone();
    use_drop(move || {
        if let Some(session) = cleanup.borrow_mut().take() {
            session.stop();
        }
    });

    let update_session = session.clone();
    use_effect(move || {
        let id = graph_id.read().clone();
        let _source = source_id.read();
        if !mounted() {
            return;
        }
        if let Some(previous) = session.borrow_mut().take() {
            previous.stop();
        }
        failure.set(None);
        let eval = document::eval(&graph_channel_script(&id, &data.peek(), external_container));
        let task = spawn(receive_views(eval, failure));
        *session.borrow_mut() = Some(ChannelSession {
            identity: Rc::new(()),
            eval,
            task,
        });
    });

    use_effect(move || {
        let id = graph_id.read().clone();
        let script = graph_drive_script(&id, &data.read(), GraphDrive::Update);
        let update_session = update_session.clone();
        let identity = update_session
            .borrow()
            .as_ref()
            .map(|session| session.identity.clone());
        spawn(async move {
            if let Err(error) = document::eval(&script).await {
                let current = update_session.borrow();
                if !same_owner(
                    current.as_ref().map(|session| &session.identity),
                    identity.as_ref(),
                ) {
                    return;
                }
                let message = format!("Graph update failed: {error}");
                if let Some(session) = current.as_ref() {
                    channel_failed(session.eval, failure, message);
                } else {
                    crate::diagnostics::report_error(&message);
                    failure.set(Some(message));
                }
            }
        });
    });

    rsx! {
        div {
            id: (!external_container).then_some(container_id),
            class: if external_container { "graph-channel-owner" } else { "graph-view" },
            onmounted: move |_| mounted.set(true),
            if let Some(message) = failure() {
                p { class: "graph-failure", role: "alert", "{message}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// External book containers retain their DOM identity while sharing the channel owner.
    #[test]
    fn external_graph_host_does_not_steal_existing_container() {
        /// Renders a lifecycle owner for a container outside this Dioxus root.
        fn external_host() -> Element {
            let id = use_signal(|| "book-graph".to_string());
            let data = use_signal(empty_graph_data);
            let source = use_signal(|| "book-source".to_string());
            rsx! { GraphView { graph_id: id, data, source_id: source, external_container: true } }
        }
        let mut dom = VirtualDom::new(external_host);
        let mutations = dom.rebuild_to_vec();
        assert!(!mutations.edits.iter().any(|edit| matches!(
            edit,
            dioxus::core::Mutation::SetAttribute {
                name: "id",
                value: dioxus::core::AttributeValue::Text(_),
                ..
            }
        )));
    }

    /// A replaced channel's one-shot evaluation failure cannot affect its successor.
    #[test]
    fn delayed_update_error_requires_the_same_channel_owner() {
        let old = Rc::new(());
        let new = Rc::new(());
        assert!(same_owner(Some(&old), Some(&old)));
        assert!(!same_owner(Some(&new), Some(&old)));
        assert!(!same_owner(None, Some(&old)));
        assert!(!same_owner(Some(&new), None));
        assert!(same_owner(None, None));
    }

    /// Queued diagnostics win a simultaneous channel completion; closed channels cannot hang.
    #[test]
    fn receiver_prioritizes_messages_and_detects_completion() {
        use std::{
            future::{pending, ready},
            task::{Context, Poll, Waker},
        };
        let mut context = Context::from_waker(Waker::noop());
        let mut completed = std::pin::pin!(ready("closed"));
        let mut message = std::pin::pin!(prefer_message(ready("diagnostic"), completed.as_mut()));
        assert_eq!(
            message.as_mut().poll(&mut context),
            Poll::Ready(Ok("diagnostic"))
        );
        let mut completed = std::pin::pin!(ready("closed"));
        let mut message = std::pin::pin!(prefer_message(pending::<()>(), completed.as_mut()));
        assert_eq!(
            message.as_mut().poll(&mut context),
            Poll::Ready(Err("closed"))
        );
        let mut completed = std::pin::pin!(pending::<()>());
        let mut message = std::pin::pin!(prefer_message(pending::<()>(), completed.as_mut()));
        assert_eq!(message.as_mut().poll(&mut context), Poll::Pending);
    }
    /// The mounted channel script starts its receiver before graph initialization.
    #[test]
    fn channel_script_embeds_shared_runtime_and_escapes_ids() {
        let script = graph_channel_script("a\"b", &empty_graph_data(), false);
        assert!(script.contains("async function runGraphChannel"));
        assert!(script.contains(r#"await runGraphChannel(window, "a\"b""#));
    }
    fn empty_graph_data() -> GraphData {
        GraphData {
            nodes: vec![],
            links: vec![],
            changed: vec![],
            forced: vec![],
            forced_relationships: vec![],
        }
    }

    #[test]
    fn graph_drive_script_init_calls_begin_graph_init() {
        let script = graph_drive_script("g1", &empty_graph_data(), GraphDrive::Init);
        // The id is embedded as a JSON string literal (double-quoted).
        assert!(script.contains("window.beginGraph.init(\"g1\""));
        assert!(script.contains("window.__beginGraphData[\"g1\"]"));
    }

    #[test]
    fn graph_drive_script_update_calls_begin_graph_update() {
        let script = graph_drive_script("g1", &empty_graph_data(), GraphDrive::Update);
        assert!(script.contains("window.beginGraph.update(\"g1\""));
    }

    #[test]
    fn graph_drive_script_guards_on_begin_graph_being_defined() {
        let script = graph_drive_script("g1", &empty_graph_data(), GraphDrive::Init);
        assert!(script.contains("typeof window.beginGraph !== 'undefined'"));
    }

    /// The one-shot evaluation closes its transport and returns a JSON value on web.
    #[test]
    fn graph_drive_script_has_explicit_serializable_completion() {
        let script = graph_drive_script("g1", &empty_graph_data(), GraphDrive::Update);
        assert!(script.ends_with("dioxus.close(); return null;"));
    }

    #[test]
    fn graph_drive_script_embeds_the_serialized_data() {
        let script = graph_drive_script("g1", &empty_graph_data(), GraphDrive::Init);
        // GraphData serializes its fields; `nodes` is always present.
        assert!(script.contains("\"nodes\""));
    }

    #[test]
    fn graph_drive_script_escapes_a_quote_in_the_container_id() {
        // A container id containing a single quote must not break out of the JS string or
        // inject: JSON-encoding turns `'` into a plain character inside a double-quoted literal,
        // and there must be no raw `'g'` string-open left in the output.
        let script = graph_drive_script("a'b", &empty_graph_data(), GraphDrive::Init);
        assert!(script.contains("window.beginGraph.init(\"a'b\""));
        assert!(!script.contains("['a'b']"));
    }

    #[test]
    fn graph_drive_script_escapes_a_double_quote_and_backslash_in_the_container_id() {
        // JSON-encoding escapes `"` and `\`, so the emitted literal stays a single well-formed
        // JS string rather than terminating early.
        let script = graph_drive_script("a\"b\\c", &empty_graph_data(), GraphDrive::Init);
        assert!(script.contains(r#"window.beginGraph.init("a\"b\\c""#));
    }
}
