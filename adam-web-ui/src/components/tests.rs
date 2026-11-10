//! Exercises numeric callbacks through mounted eval receivers and source-captured snapshots.
use super::*;
use crate::input::{InputError, NumberKind, NumericRange};
use dioxus::{
    core::{ElementId, Mutation},
    document::{Document, Eval, EvalError, Evaluator},
    prelude::*,
};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    task::{Context, Poll},
};

thread_local! {
    /// Retains renderer-owned channels independently of component implementation state.
    static SOURCES: RefCell<HashMap<usize, Rc<RefCell<SourceQueue>>>> = RefCell::default();
}

/// Models the renderer's ordered JSON eval transport, including display/reset delivery.
#[derive(Default)]
struct SourceQueue {
    token: usize,
    sequence: u64,
    epoch: String,
    frames: VecDeque<serde_json::Value>,
    waker: Option<std::task::Waker>,
    value: String,
    composing: bool,
    closed: bool,
    fail_send: bool,
    drop_ack: bool,
    fail_recv: bool,
}

impl SourceQueue {
    /// Enqueues an immutable source snapshot and wakes the component receiver.
    fn push(&mut self, action: &str, value: &str) {
        match action {
            "start" => self.composing = true,
            "end" | "cancel" | "blur" => self.composing = false,
            _ => {}
        }
        self.sequence += 1;
        self.value = value.to_string();
        self.frames.push_back(serde_json::json!({
            "token":self.token.to_string(),"epoch":self.epoch,
            "sequence":self.sequence.to_string(),"session":"1","action":action,"value":value,
        }));
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

/// Delivers renderer messages to the actual component-scoped source receiver.
struct SourceReply(Rc<RefCell<SourceQueue>>);

impl Evaluator for SourceReply {
    /// Applies display/reset commands as the native event source would.
    fn send(&self, message: serde_json::Value) -> Result<(), EvalError> {
        let mut source = self.0.borrow_mut();
        if source.fail_send {
            return Err(EvalError::Unsupported);
        }
        match message["kind"].as_str() {
            Some("close") => source.closed = true,
            Some("reset") => {
                source.epoch = message["epoch"].as_str().unwrap().into();
                source.composing = false;
                if !source.drop_ack {
                    source.push("ready", message["value"].as_str().unwrap());
                }
            }
            Some("display")
                if !source.composing
                    && message["sequence"].as_str() == Some(&source.sequence.to_string()) =>
            {
                source.value = message["value"].as_str().unwrap().into();
            }
            Some("display") => {}
            _ => return Err(EvalError::Unsupported),
        }
        Ok(())
    }

    /// Preserves asynchronous reception rather than completing a persistent eval.
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
        let mut source = self.0.borrow_mut();
        if source.fail_recv {
            return Poll::Ready(Err(EvalError::Unsupported));
        }
        if let Some(frame) = source.frames.pop_front() {
            Poll::Ready(Ok(frame))
        } else {
            source.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    /// Keeps the source evaluation alive until the component closes it.
    fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
        if self.0.borrow().closed {
            Poll::Ready(Ok(serde_json::Value::Null))
        } else {
            Poll::Pending
        }
    }
}

/// Runs real locale scripts in Node and models the renderer-owned persistent JSON channel.
#[derive(Default)]
pub(crate) struct NodeDocument {
    owner: generational_box::Owner,
    gate: Option<Rc<RefCell<Gate>>>,
}

/// Holds formatter transport replies until the test explicitly releases them.
#[derive(Default)]
struct Gate {
    held: bool,
    fail: bool,
    wakers: Vec<std::task::Waker>,
}

impl Gate {
    /// Releases pending replies and schedules their consumers.
    fn release(&mut self) {
        self.held = false;
        for waker in self.wakers.drain(..) {
            waker.wake();
        }
    }
}

impl Document for NodeDocument {
    /// Runs locale code with a single JSON message transport.
    fn eval(&self, script: String) -> Eval {
        if script.starts_with("/* adam-control-styles */") {
            return Eval::new(self.owner.insert(Box::new(Reply(
                None,
                None,
                Some(serde_json::Value::Null),
            ))));
        }
        if script.contains("setTimeout(() => resolve(null), 5000)") {
            return Eval::new(self.owner.insert(Box::new(Reply(
                None,
                None,
                Some(serde_json::Value::Null),
            ))));
        }
        if let Some((_, tail)) = script.rsplit_once("return await runNumericEvents(window, \"") {
            let token: usize = tail.split('"').next().unwrap().parse().unwrap();
            let source = Rc::new(RefCell::new(SourceQueue {
                token,
                epoch: "0".into(),
                ..Default::default()
            }));
            source.borrow_mut().push("ready", "12");
            SOURCES.with(|sources| sources.borrow_mut().insert(token, source.clone()));
            return Eval::new(self.owner.insert(Box::new(SourceReply(source))));
        }
        let output = std::process::Command::new("node")
            .arg("-e")
            .arg(format!(
                "const messages=[];const returned=new Function('dioxus',{})({{send:x=>messages.push(x)}});console.log(JSON.stringify({{messages,returned}}));",
                serde_json::to_string(&script).unwrap()
            ))
            .output()
            .unwrap();
        let result = if output.status.success() {
            serde_json::from_slice(&output.stdout).ok()
        } else {
            None
        };
        let gate = script
            .contains("Ok: formatNumber(")
            .then(|| self.gate.clone())
            .flatten();
        let message = result
            .as_ref()
            .and_then(|result: &serde_json::Value| result["messages"].get(0))
            .cloned();
        let returned = result.and_then(|result| result.get("returned").cloned());
        Eval::new(self.owner.insert(Box::new(Reply(message, gate, returned))))
    }

    /// Leaves head assets to the renderer rather than evaluating browser code in Node.
    fn create_link(&self, _: dioxus::document::LinkProps) {}

    /// Leaves head assets to the renderer rather than evaluating browser code in Node.
    fn create_style(&self, _: dioxus::document::StyleProps) {}
}

/// Supplies one script result, or a transport failure.
struct Reply(
    Option<serde_json::Value>,
    Option<Rc<RefCell<Gate>>>,
    Option<serde_json::Value>,
);

impl Evaluator for Reply {
    /// Rejects unused host messages.
    fn send(&self, _: serde_json::Value) -> Result<(), EvalError> {
        Err(EvalError::Unsupported)
    }

    /// Delivers the actual script result.
    fn poll_recv(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<serde_json::Value, EvalError>> {
        if let Some(gate) = &self.1 {
            let mut gate = gate.borrow_mut();
            if gate.fail {
                return Poll::Ready(Err(EvalError::Unsupported));
            }
            if gate.held {
                gate.wakers.push(context.waker().clone());
                return Poll::Pending;
            }
        }
        Poll::Ready(self.0.take().ok_or(EvalError::Unsupported))
    }

    /// Completes evaluation.
    fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
        Poll::Ready(self.2.take().ok_or(EvalError::Unsupported))
    }
}

/// A pending formatter cannot overwrite a newer incomplete native draft.
#[tokio::test]
async fn delayed_formatting_keeps_newer_incomplete_draft() {
    let gate = Rc::new(RefCell::new(Gate::default()));
    let document = Rc::new(NodeDocument {
        owner: Default::default(),
        gate: Some(gate.clone()),
    });
    let (mut dom, observed, id) = mount_config(false, NumberKind::F64, false, document);
    settle(&mut dom).await;
    gate.borrow_mut().held = true;
    let mut value = observed.borrow().controls.unwrap().value;
    value.set(Some("1234.5".into()));
    settle(&mut dom).await;
    assert!(
        !gate.borrow().wakers.is_empty(),
        "formatter actually suspended"
    );
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "-");
    gate.borrow_mut().release();
    settle(&mut dom).await;
    assert_display(id, "-");
    assert!(observed.borrow().writes.is_empty());
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert_display(id, "1,234.5");
}

/// Mounts one official wrapper with a callback that reports its actual interaction.
fn official_host(which: u8) -> Element {
    let observed: Rc<RefCell<Observed>> = consume_context();
    match which {
        0 => {
            rsx! { Theme { Button { onclick: move |_| observed.borrow_mut().writes.push(Some("clicked".into())), "Action" } } }
        }
        1 => {
            rsx! { Theme { Checkbox { aria_label: "Enabled", on_checked_change: move |v: dioxus_primitives::checkbox::CheckboxState| observed.borrow_mut().writes.push(Some(format!("{v:?}"))) } } }
        }
        2 => {
            rsx! { Theme { Switch { aria_label: "Enabled", on_checked_change: move |v: bool| observed.borrow_mut().writes.push(Some(v.to_string())) } } }
        }
        3 => {
            rsx! { Theme { Slider { label: Some("Zoom".to_string()), value: Some(5.0), on_value_change: move |v: f64| observed.borrow_mut().writes.push(Some(v.to_string())) } } }
        }
        4 => {
            rsx! { Theme { RangeSlider { label: Some("Interval".to_string()), on_value_change: move |_| observed.borrow_mut().writes.push(Some("range".into())) } } }
        }
        _ => {
            rsx! { Theme { Label { html_for: "named", "Named" } Input { id: "named" } Separator { decorative: false } } }
        }
    }
}

/// Every imported wrapper preserves native interaction or its accessible structural role.
#[tokio::test]
async fn official_controls_keep_callbacks_and_semantics() {
    for which in 0..6 {
        let observed = Rc::new(RefCell::new(Observed::default()));
        let mut dom = VirtualDom::new_with_props(official_host, which);
        dom.provide_root_context(observed.clone());
        dom.provide_root_context(Rc::new(NodeDocument::default()) as Rc<dyn Document>);
        let mutations = dom.rebuild_to_vec();
        settle(&mut dom).await;
        if which == 5 {
            let html = dioxus_ssr::render(&dom);
            assert!(html.contains("for=\"named\"") && html.contains("id=\"named\""));
            assert!(html.contains("role=\"separator\""));
            continue;
        }
        let target = mutations
            .edits
            .into_iter()
            .find_map(|mutation| match mutation {
                Mutation::NewEventListener { name, id }
                    if name == if which < 3 { "click" } else { "keydown" } =>
                {
                    Some(id)
                }
                _ => None,
            })
            .expect("wrapper registers its interactive primitive");
        if which < 3 {
            dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
            dom.runtime().handle_event(
                "click",
                Event::new(
                    Rc::new(dioxus_html::PlatformEventData::new(Box::new(
                        dioxus_html::SerializedMouseData::default(),
                    ))),
                    true,
                )
                .into_any(),
                target,
            );
        } else {
            arrow_up(&mut dom, target);
        }
        assert_eq!(observed.borrow().writes.len(), 1, "control {which}");
    }
}
/// Captures only consumer-visible state and writes from the mounted controls.
#[derive(Clone, Copy)]
struct Controls {
    value: Signal<Option<String>>,
    disabled: Signal<bool>,
    readonly: Signal<bool>,
    invalid: Signal<bool>,
    range: Signal<Option<NumericRange>>,
    locale: Signal<Option<String>>,
}

/// Shares fixture observations with the owning test.
#[derive(Default)]
struct Observed {
    controls: Option<Controls>,
    writes: Vec<Option<String>>,
    errors: Vec<InputError>,
}

/// Renders an editor with live controlled state and independently observable callbacks.
fn host((optional, kind, slider): (bool, NumberKind, bool)) -> Element {
    let value = use_signal(|| Some("12".to_string()));
    let disabled = use_signal(|| false);
    let readonly = use_signal(|| false);
    let invalid = use_signal(|| false);
    let range = use_signal(|| None::<NumericRange>);
    let locale = use_signal(|| Some("en-US".to_string()));
    let observed: Rc<RefCell<Observed>> = consume_context();
    observed.borrow_mut().controls = Some(Controls {
        value,
        disabled,
        readonly,
        invalid,
        range,
        locale,
    });
    let errors = observed.clone();
    rsx! {
        Theme {
            if slider {
                NumericSlider {
                    id: "amount", label: "Amount", value, kind, range, step: "1",
                    disabled, readonly, invalid, locale: locale(), optional,
                    on_value: move |v| observed.borrow_mut().writes.push(v),
                    on_error: move |e| errors.borrow_mut().errors.push(e),
                }
            } else {
              NumericEditor {
                id: "amount", label: "Amount", value,
                kind,
                range,
                step: "1", disabled, readonly, invalid,
                locale: locale(), optional,
                on_value: move |v| observed.borrow_mut().writes.push(v),
                on_error: move |e| errors.borrow_mut().errors.push(e),
            }
            }
        }
    }
}

/// Mounts a real editor and records its opaque native source identity.
fn mount(optional: bool) -> (VirtualDom, Rc<RefCell<Observed>>, ElementId) {
    mount_config(
        optional,
        NumberKind::Signed(32),
        false,
        Rc::new(NodeDocument::default()),
    )
}

/// Mounts a configured control over a renderer-owned evaluator.
fn mount_config(
    optional: bool,
    kind: NumberKind,
    slider: bool,
    document: Rc<dyn Document>,
) -> (VirtualDom, Rc<RefCell<Observed>>, ElementId) {
    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new_with_props(host, (optional, kind, slider));
    dom.provide_root_context(observed.clone());
    dom.provide_root_context(document);
    let mutations = dom.rebuild_to_vec();
    let id = mutations
        .edits
        .into_iter()
        .find_map(|mutation| match mutation {
            Mutation::SetAttribute {
                name: "data-adam-number-source",
                value: dioxus::core::AttributeValue::Text(token),
                ..
            } => Some(ElementId(token.parse().unwrap())),
            _ => None,
        })
        .expect("numeric editor registers native input");
    (dom, observed, id)
}

/// Delivers a source-validated composition boundary without asynchronously reading the DOM.
fn composition(dom: &mut VirtualDom, id: ElementId, name: &str) {
    let value = SOURCES.with(|sources| {
        let sources = sources.borrow();
        let mut source = sources[&id.0].borrow_mut();
        source.composing = name == "compositionstart";
        source.value.clone()
    });
    frame(
        dom,
        id,
        if name == "compositionstart" {
            "start"
        } else {
            "end"
        },
        &value,
    );
}

/// Dispatches the native Arrow Up key to the editor.
pub(crate) fn arrow_up(dom: &mut VirtualDom, id: ElementId) {
    if SOURCES.with(|sources| sources.borrow().contains_key(&id.0)) {
        frame(dom, id, "up", "");
        return;
    }
    use keyboard_types::{Code, Location, Modifiers};
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let data = dioxus_html::SerializedKeyboardData::new(
        Key::ArrowUp,
        Code::ArrowUp,
        Location::Standard,
        false,
        Modifiers::empty(),
        false,
    );
    dom.runtime().handle_event(
        "keydown",
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
            true,
        )
        .into_any(),
        id,
    );
}

/// IME intermediates never submit, and the final native duplicate does not submit twice.
#[tokio::test]
async fn composition_submits_only_final_value_and_optional_clear_once() {
    for (optional, text, expected) in [
        (false, "17", vec![Some("17".to_string())]),
        (true, "", vec![None]),
    ] {
        let (mut dom, observed, id) = mount(optional);
        settle(&mut dom).await;
        focus(&mut dom, id, "focus");
        composition(&mut dom, id, "compositionstart");
        input(&mut dom, id, text);
        assert!(observed.borrow().writes.is_empty());
        composition(&mut dom, id, "compositionend");
        assert_eq!(observed.borrow().writes, expected);
    }
}

/// Compositionend cannot treat the last intermediate input as the final native value.
#[tokio::test]
async fn composition_end_waits_for_final_native_input_value() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    composition(&mut dom, id, "compositionstart");
    input(&mut dom, id, "17");
    settle(&mut dom).await;
    assert!(observed.borrow().writes.is_empty());
    frame(&mut dom, id, "end", "19");
    assert_eq!(observed.borrow().writes, [Some("19".into())]);
}

/// Delayed completion and ordinary equal-valued edits retain separate consumer aspirations.
#[tokio::test]
async fn delayed_source_completion_and_equal_ordinary_edit_have_exact_callback_counts() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    frame(&mut dom, id, "start", "12");
    frame(&mut dom, id, "draft", "17");
    settle(&mut dom).await;
    assert!(observed.borrow().writes.is_empty());
    frame(&mut dom, id, "end", "19");
    frame(&mut dom, id, "input", "19");
    assert_eq!(
        observed.borrow().writes,
        [Some("19".into()), Some("19".into())]
    );
}

/// Cancellation restores the model even when no new reactive display text is needed.
#[tokio::test]
async fn cancellation_restores_native_display_even_when_model_text_has_not_changed() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    frame(&mut dom, id, "start", "12");
    frame(&mut dom, id, "cancel", "999");
    settle(&mut dom).await;
    let source = SOURCES.with(|sources| sources.borrow()[&id.0].clone());
    assert_eq!(source.borrow().value, "12");
    assert!(observed.borrow().writes.is_empty());
}

/// A formatter transport failure retires pending native completion as a locale failure.
#[tokio::test]
async fn formatter_failure_retires_pending_composition() {
    let gate = Rc::new(RefCell::new(Gate::default()));
    let document = Rc::new(NodeDocument {
        owner: Default::default(),
        gate: Some(gate.clone()),
    });
    let (mut dom, observed, id) = mount_config(false, NumberKind::F64, false, document);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    composition(&mut dom, id, "compositionstart");
    input(&mut dom, id, "17");
    gate.borrow_mut().fail = true;
    let mut value = observed.borrow().controls.unwrap().value;
    value.set(Some("99".into()));
    settle(&mut dom).await;
    frame(&mut dom, id, "end", "19");
    assert!(
        observed
            .borrow()
            .errors
            .iter()
            .any(|e| matches!(e, InputError::Locale(_)))
    );
    assert!(observed.borrow().writes.is_empty());
}

/// Forced reset rejects queued old epochs without losing the next enabled input.
#[tokio::test]
async fn source_reset_rejects_old_completion_then_accepts_enabled_edit() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    frame(&mut dom, id, "start", "12");
    frame(&mut dom, id, "draft", "17");
    let source = SOURCES.with(|sources| sources.borrow()[&id.0].clone());
    let old_epoch = source.borrow().epoch.clone();
    let Controls {
        mut disabled,
        mut value,
        ..
    } = observed.borrow().controls.unwrap();
    disabled.set(true);
    value.set(Some("99".into()));
    settle(&mut dom).await;
    disabled.set(false);
    settle(&mut dom).await;
    {
        let mut source = source.borrow_mut();
        let current = source.epoch.clone();
        source.epoch = old_epoch;
        source.push("end", "19");
        source.epoch = current;
    }
    dom.render_immediate_to_vec();
    assert!(observed.borrow().writes.is_empty());
    input(&mut dom, id, "7");
    assert_eq!(observed.borrow().writes, [Some("7".into())]);
    drop(dom);
    assert!(
        source.borrow().closed,
        "unmount closes renderer-owned JS channel"
    );
}

/// Reset send failures and missing acknowledgments report explicit source errors.
#[tokio::test]
async fn source_reset_failure_is_visible_instead_of_indefinitely_inert() {
    for fail_send in [true, false] {
        let (mut dom, observed, id) = mount(false);
        settle(&mut dom).await;
        SOURCES.with(|sources| {
            let sources = sources.borrow();
            let mut source = sources[&id.0].borrow_mut();
            source.fail_send = fail_send;
            source.drop_ack = !fail_send;
        });
        let mut disabled = observed.borrow().controls.unwrap().disabled;
        disabled.set(true);
        settle(&mut dom).await;
        assert!(
            observed
                .borrow()
                .errors
                .iter()
                .any(|e| matches!(e, InputError::EventSource(_)))
        );
        assert!(dioxus_ssr::render(&dom).contains("Number event source error"));
    }
}

/// A persistent receiver failure reports once and disposes its source owner.
#[tokio::test]
async fn source_receive_failure_closes_the_owner() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    let source = SOURCES.with(|sources| sources.borrow()[&id.0].clone());
    {
        let mut source = source.borrow_mut();
        source.fail_recv = true;
        source.push("input", "19");
    }
    dom.render_immediate_to_vec();
    assert!(source.borrow().closed);
    assert!(observed.borrow().writes.is_empty());
    assert_eq!(
        observed
            .borrow()
            .errors
            .iter()
            .filter(|e| matches!(e, InputError::EventSource(_)))
            .count(),
        1
    );
}

/// Focused drafts retain their parsing locale while new formatting and blur use the new locale.
#[tokio::test]
async fn locale_changes_do_not_reinterpret_focused_drafts() {
    let (mut dom, observed, id) = mount_config(
        false,
        NumberKind::F64,
        false,
        Rc::new(NodeDocument::default()),
    );
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "-");
    let Controls {
        mut value,
        mut locale,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("1234.5".into()));
    locale.set(Some("de-DE".into()));
    settle(&mut dom).await;
    assert_display(id, "-");
    input(&mut dom, id, "1.5");
    assert_eq!(observed.borrow().writes, [Some("1.5".into())]);
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert_display(id, "1.234,5");
}

/// Typed aspirations are not range-clamped, and changing bounds cannot rewrite the model display.
#[tokio::test]
async fn slider_ranges_and_wide_integers_preserve_exact_model_values() {
    for kind in [NumberKind::Signed(32), NumberKind::Unsigned(128)] {
        let (mut dom, observed, id) =
            mount_config(false, kind, true, Rc::new(NodeDocument::default()));
        settle(&mut dom).await;
        let Controls {
            mut range,
            mut value,
            ..
        } = observed.borrow().controls.unwrap();
        range.set(Some(NumericRange {
            min: "0".into(),
            max: "100".into(),
        }));
        settle(&mut dom).await;
        let html = dioxus_ssr::render(&dom);
        assert_eq!(
            html.contains("role=\"slider\""),
            kind == NumberKind::Signed(32)
        );
        focus(&mut dom, id, "focus");
        let aspiration = if kind == NumberKind::Unsigned(128) {
            "340282366920938463463374607431768211455"
        } else {
            "200"
        };
        input(&mut dom, id, aspiration);
        assert_eq!(observed.borrow().writes, [Some(aspiration.to_string())]);
        value.set(Some(aspiration.to_string()));
        range.set(Some(NumericRange {
            min: "0".into(),
            max: "10".into(),
        }));
        settle(&mut dom).await;
        assert_display(id, aspiration);
        assert_eq!(observed.borrow().writes.len(), 1);
    }
}

/// Keyboard stepping submits a checked aspiration but cannot step nonfinite or blocked values.
#[tokio::test]
async fn keyboard_stepping_rejects_nonfinite_and_blocked_values() {
    let (mut dom, observed, id) = mount_config(
        false,
        NumberKind::F64,
        false,
        Rc::new(NodeDocument::default()),
    );
    settle(&mut dom).await;
    arrow_up(&mut dom, id);
    assert_eq!(observed.borrow().writes, [Some("13".into())]);
    let Controls {
        mut value,
        mut readonly,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("NaN".into()));
    settle(&mut dom).await;
    arrow_up(&mut dom, id);
    assert_eq!(observed.borrow().writes.len(), 1);
    assert!(!observed.borrow().errors.is_empty());
    readonly.set(true);
    value.set(Some("10".into()));
    settle(&mut dom).await;
    arrow_up(&mut dom, id);
    assert_eq!(observed.borrow().writes.len(), 1);
}

/// Missing renderer eval support produces a locale diagnostic instead of a successful fallback.
#[tokio::test]
async fn eval_transport_failure_reaches_error_callback() {
    let (mut dom, observed, id) = mount_config(
        false,
        NumberKind::F64,
        false,
        Rc::new(dioxus::document::NoOpDocument),
    );
    settle(&mut dom).await;
    assert!(
        observed
            .borrow()
            .errors
            .iter()
            .any(|e| matches!(e, InputError::Locale(_)))
    );
    input(&mut dom, id, "3");
    assert!(observed.borrow().writes.is_empty());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Number locale error") || html.contains("Number event source error"));
}

/// Locale failure remains explicit while effective model values stay visible and recoverable.
#[tokio::test]
async fn locale_failure_keeps_effective_model_display_and_recovers() {
    let (mut dom, observed, id) = mount_config(
        false,
        NumberKind::F64,
        false,
        Rc::new(NodeDocument::default()),
    );
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "-");
    let Controls {
        mut value,
        mut locale,
        ..
    } = observed.borrow().controls.unwrap();
    locale.set(Some("not_a_locale".into()));
    settle(&mut dom).await;
    assert!(
        observed
            .borrow()
            .errors
            .iter()
            .any(|e| matches!(e, InputError::Locale(_)))
    );
    value.set(Some("99.5".into()));
    settle(&mut dom).await;
    assert_display(id, "99.5");
    locale.set(Some("de-DE".into()));
    settle(&mut dom).await;
    assert_display(id, "99,5");
    assert!(observed.borrow().writes.is_empty());
}

/// Authoritative focused updates remain valid text in the focus session's parsing locale.
#[tokio::test]
async fn focused_authoritative_updates_use_the_originating_locale() {
    let (mut dom, observed, id) = mount_config(
        false,
        NumberKind::F64,
        false,
        Rc::new(NodeDocument::default()),
    );
    settle(&mut dom).await;
    let Controls {
        mut value,
        mut locale,
        ..
    } = observed.borrow().controls.unwrap();
    locale.set(Some("de-DE".into()));
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    value.set(Some("99.5".into()));
    settle(&mut dom).await;
    assert_display(id, "99,5");
    locale.set(Some("en-US".into()));
    value.set(Some("88.5".into()));
    settle(&mut dom).await;
    assert_display(id, "88,5");
    input(&mut dom, id, "88,6");
    assert_eq!(observed.borrow().writes, [Some("88.6".into())]);
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert_display(id, "88.5");
}

/// Browsers with no post-composition input still submit the final captured native value.
#[tokio::test]
async fn composition_end_without_following_input_finishes_once() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    composition(&mut dom, id, "compositionstart");
    input(&mut dom, id, "17");
    composition(&mut dom, id, "compositionend");
    settle(&mut dom).await;
    assert_eq!(observed.borrow().writes, [Some("17".into())]);
    assert!(observed.borrow().errors.is_empty());
}

/// Forced state and unmounting cancel delayed native composition work.
#[tokio::test]
async fn delayed_composition_is_cancelled_by_forced_state_and_unmount() {
    for unmount in [false, true] {
        let (mut dom, observed, id) = mount(false);
        settle(&mut dom).await;
        focus(&mut dom, id, "focus");
        composition(&mut dom, id, "compositionstart");
        input(&mut dom, id, "17");
        if unmount {
            drop(dom);
            tokio::task::yield_now().await;
        } else {
            observed.borrow().controls.unwrap().disabled.set(true);
            settle(&mut dom).await;
        }
        assert!(observed.borrow().writes.is_empty());
    }
}

/// Discarding a forced draft also discards its lexical error, not model validation.
#[tokio::test]
async fn forced_state_discards_local_error_but_preserves_model_validation() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "bad");
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("Invalid number"));
    let Controls {
        mut value,
        mut disabled,
        mut invalid,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("99".into()));
    disabled.set(true);
    settle(&mut dom).await;
    let html = dioxus_ssr::render(&dom);
    assert_display(id, "99");
    assert!(!html.contains("Invalid number"));
    invalid.set(true);
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("The model rejected this value."));
    assert!(observed.borrow().writes.is_empty());
}

/// Drains immediately-ready locale tasks and reactive renders without waiting on timers.
pub(crate) async fn settle(dom: &mut VirtualDom) {
    for _ in 0..8 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(5), dom.wait_for_work()).await;
        dom.render_immediate_to_vec();
    }
}

/// Delivers a synchronously captured native value rather than a later DOM read.
pub(crate) fn input(dom: &mut VirtualDom, id: ElementId, value: &str) {
    if !SOURCES.with(|sources| sources.borrow().contains_key(&id.0)) {
        return;
    }
    let composing = SOURCES.with(|sources| sources.borrow()[&id.0].borrow().composing);
    frame(dom, id, if composing { "draft" } else { "input" }, value);
}

/// Dispatches focus events to the same actual native input.
fn focus(dom: &mut VirtualDom, id: ElementId, name: &str) {
    let value = SOURCES.with(|sources| sources.borrow()[&id.0].borrow().value.clone());
    frame(dom, id, name, &value);
}

/// Asserts on renderer-applied display commands rather than an uncontrolled initial HTML attribute.
fn assert_display(id: ElementId, expected: &str) {
    SOURCES.with(|sources| assert_eq!(sources.borrow()[&id.0].borrow().value, expected));
}

/// Delivers a source-captured full-value frame through the component's real eval receiver.
fn frame(dom: &mut VirtualDom, id: ElementId, action: &str, value: &str) {
    SOURCES.with(|sources| sources.borrow()[&id.0].borrow_mut().push(action, value));
    dom.render_immediate_to_vec();
}

/// Blur restores valid text and clears draft errors without hiding source failures or resubmitting.
#[tokio::test]
async fn blur_recovers_draft_errors_without_hiding_source_failures() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "wrong");
    assert!(dioxus_ssr::render(&dom).contains("Invalid number"));
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert_display(id, "12");
    assert!(!dioxus_ssr::render(&dom).contains("Invalid number"));
    assert!(observed.borrow().writes.is_empty());
    frame(&mut dom, id, "error", "");
    assert!(dioxus_ssr::render(&dom).contains("Number event source error"));
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("Number event source error"));
    assert!(observed.borrow().writes.is_empty());
}

/// Rendering a lexical diagnostic never overwrites the focused native draft.
#[tokio::test]
async fn lexical_error_render_preserves_native_draft() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    SOURCES.with(|sources| sources.borrow()[&id.0].borrow_mut().push("input", "wrong"));
    for _ in 0..8 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(5), dom.wait_for_work()).await;
        for edit in dom.render_immediate_to_vec().edits {
            if let dioxus::core::Mutation::SetAttribute {
                name: "value",
                value: dioxus::core::AttributeValue::Text(value),
                ..
            } = edit
            {
                assert_eq!(value, "wrong", "render must preserve the live native draft");
            }
        }
    }
    assert_display(id, "wrong");
    assert!(dioxus_ssr::render(&dom).contains("Invalid number"));
    assert!(observed.borrow().writes.is_empty());
}

/// Required blanks and blur never submit; optional clearing submits absence exactly once.
#[tokio::test]
async fn clearing_respects_optional_contract() {
    for optional in [false, true] {
        let (mut dom, observed, id) = mount(optional);
        settle(&mut dom).await;
        focus(&mut dom, id, "focus");
        input(&mut dom, id, "");
        focus(&mut dom, id, "blur");
        settle(&mut dom).await;
        let expected = if optional { vec![None] } else { vec![] };
        assert_eq!(observed.borrow().writes, expected);
        assert_display(id, "12");
    }
}

/// Model updates never submit, and forced flags discard drafts and reject synthetic input.
#[tokio::test]
async fn controlled_updates_and_blocked_events_do_not_submit() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    let Controls {
        mut value,
        mut disabled,
        mut readonly,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("99".into()));
    settle(&mut dom).await;
    assert_display(id, "99");
    assert!(observed.borrow().writes.is_empty());
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "-");
    disabled.set(true);
    settle(&mut dom).await;
    assert_display(id, "99");
    input(&mut dom, id, "10");
    disabled.set(false);
    readonly.set(true);
    settle(&mut dom).await;
    input(&mut dom, id, "11");
    assert!(observed.borrow().writes.is_empty());
}

/// Complete native input submits exact canonical text and preserves its focused spelling.
#[tokio::test]
async fn input_submits_and_acknowledgment_preserves_spelling() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&mut dom, id, "focus");
    input(&mut dom, id, "0013");
    assert_eq!(observed.borrow().writes, [Some("0013".to_string())]);
    let mut value = observed.borrow().controls.unwrap().value;
    value.set(Some("13".into()));
    settle(&mut dom).await;
    assert_display(id, "0013");
    focus(&mut dom, id, "blur");
    settle(&mut dom).await;
    assert_eq!(observed.borrow().writes.len(), 1);
    assert_display(id, "13");
}

/// Label, instructions, validation and blocked flags remain associated with the input.
#[tokio::test]
async fn accessibility_and_validation_follow_controlled_state() {
    let (mut dom, observed, _) = mount(false);
    settle(&mut dom).await;
    let Controls {
        mut invalid,
        mut readonly,
        ..
    } = observed.borrow().controls.unwrap();
    invalid.set(true);
    readonly.set(true);
    settle(&mut dom).await;
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("for=\"amount\""));
    assert!(html.contains("aria-describedby=\"amount-help amount-error\""));
    assert!(html.contains("id=\"amount-help\"") && html.contains("id=\"amount-error\""));
    assert!(
        html.contains("id=\"amount-help\" class=\"adam_numeric_editor_visually_hidden\""),
        "help element should carry the visually hidden module class: {html}"
    );
    assert!(
        html.contains("aria-invalid=\"true\"") && html.contains("readonly"),
        "{html}"
    );
    assert!(html.contains("Increase Amount") && html.contains("Decrease Amount"));
}

/// Every embedded `rem` length scales with the host-settable `--adam-rem` custom property.
///
/// Outside the documented `var(--adam-rem, 1rem)` fallback, no ASCII digit or `.` in
/// `CONTROL_CSS` is immediately followed by the literal `rem`, confirming `build.rs` rewrote
/// every bare `<n>rem` literal into `calc(<n> * var(--adam-rem, 1rem))`.
#[test]
fn embedded_lengths_scale_with_adam_rem() {
    const FALLBACK: &str = "var(--adam-rem, 1rem)";
    assert!(
        CONTROL_CSS.contains(FALLBACK),
        "expected CONTROL_CSS to contain the documented --adam-rem fallback"
    );
    let scrubbed = CONTROL_CSS.replace(FALLBACK, "");
    let bytes = scrubbed.as_bytes();
    for (index, window) in bytes.windows(3).enumerate() {
        if window != b"rem" || index == 0 {
            continue;
        }
        let preceding = bytes[index - 1];
        assert!(
            !preceding.is_ascii_digit() && preceding != b'.',
            "found a bare rem length in CONTROL_CSS near: {:?}",
            String::from_utf8_lossy(&bytes[index.saturating_sub(10)..(index + 3).min(bytes.len())])
        );
    }
}

/// `Card` and `CardContent` render the upstream `data-slot` attributes, the scoped class
/// exported for the official `.dx-card` selector, and caller-supplied inline style untouched.
#[test]
fn card_renders_upstream_slots() {
    fn app() -> Element {
        rsx! {
            Card { style: "max-width: 24rem",
                CardContent { "x" }
            }
        }
    }
    let mut dom = VirtualDom::new(app);
    dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("data-slot=\"card\""), "{html}");
    assert!(html.contains("data-slot=\"card-content\""), "{html}");
    assert!(html.contains("adam_card_dx-card"), "{html}");
    assert!(html.contains("style=\"max-width: 24rem\""), "{html}");
}
