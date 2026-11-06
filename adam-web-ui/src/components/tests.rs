//! Exercises controlled numeric callbacks through real VirtualDom native events.
use super::*;
use crate::input::{InputError, NumberKind, NumericRange};
use dioxus::{
    core::{ElementId, Mutation},
    document::{Document, Eval, EvalError, Evaluator},
    prelude::*,
};
use std::{
    cell::RefCell,
    rc::Rc,
    task::{Context, Poll},
};

/// Retains native evaluator storage and executes the real locale scripts in Node.
#[derive(Default)]
struct NodeDocument {
    owner: generational_box::Owner,
    gate: Option<Rc<RefCell<Gate>>>,
}

/// Holds formatter transport replies until the test explicitly releases them.
#[derive(Default)]
struct Gate {
    held: bool,
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
    focus(&dom, id, "focus");
    input(&dom, id, "-");
    gate.borrow_mut().release();
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"-\""));
    assert!(observed.borrow().writes.is_empty());
    focus(&dom, id, "blur");
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"1,234.5\""));
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
            arrow_up(&dom, target);
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

/// Mounts a real editor and records its native input listener identity.
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
            Mutation::NewEventListener { name, id } if name == "input" => Some(id),
            _ => None,
        })
        .expect("numeric editor registers native input");
    (dom, observed, id)
}

/// Dispatches a native composition boundary without asynchronously reading the DOM.
fn composition(dom: &VirtualDom, id: ElementId, name: &str) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let data: dioxus_html::SerializedCompositionData =
        serde_json::from_value(serde_json::json!({"data": ""})).unwrap();
    dom.runtime().handle_event(
        name,
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
            true,
        )
        .into_any(),
        id,
    );
}

/// Dispatches the native Arrow Up key to the editor.
fn arrow_up(dom: &VirtualDom, id: ElementId) {
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
        focus(&dom, id, "focus");
        composition(&dom, id, "compositionstart");
        input(&dom, id, text);
        assert!(observed.borrow().writes.is_empty());
        composition(&dom, id, "compositionend");
        input(&dom, id, text);
        assert_eq!(observed.borrow().writes, expected);
    }
}

/// Compositionend cannot treat the last intermediate input as the final native value.
#[tokio::test]
async fn composition_end_waits_for_final_native_input_value() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&dom, id, "focus");
    composition(&dom, id, "compositionstart");
    input(&dom, id, "17");
    composition(&dom, id, "compositionend");
    assert!(observed.borrow().writes.is_empty());
    input(&dom, id, "19");
    assert_eq!(observed.borrow().writes, [Some("19".into())]);
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
    focus(&dom, id, "focus");
    input(&dom, id, "-");
    let Controls {
        mut value,
        mut locale,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("1234.5".into()));
    locale.set(Some("de-DE".into()));
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"-\""));
    input(&dom, id, "1.5");
    assert_eq!(observed.borrow().writes, [Some("1.5".into())]);
    focus(&dom, id, "blur");
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"1.234,5\""));
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
        focus(&dom, id, "focus");
        let aspiration = if kind == NumberKind::Unsigned(128) {
            "340282366920938463463374607431768211455"
        } else {
            "200"
        };
        input(&dom, id, aspiration);
        assert_eq!(observed.borrow().writes, [Some(aspiration.to_string())]);
        value.set(Some(aspiration.to_string()));
        range.set(Some(NumericRange {
            min: "0".into(),
            max: "10".into(),
        }));
        settle(&mut dom).await;
        assert!(dioxus_ssr::render(&dom).contains(&format!("value=\"{aspiration}\"")));
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
    arrow_up(&dom, id);
    assert_eq!(observed.borrow().writes, [Some("13".into())]);
    let Controls {
        mut value,
        mut readonly,
        ..
    } = observed.borrow().controls.unwrap();
    value.set(Some("NaN".into()));
    settle(&mut dom).await;
    arrow_up(&dom, id);
    assert_eq!(observed.borrow().writes.len(), 1);
    assert!(!observed.borrow().errors.is_empty());
    readonly.set(true);
    value.set(Some("10".into()));
    settle(&mut dom).await;
    arrow_up(&dom, id);
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
    input(&dom, id, "3");
    assert!(observed.borrow().writes.is_empty());
    assert!(dioxus_ssr::render(&dom).contains("Number locale error"));
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
    focus(&dom, id, "focus");
    input(&dom, id, "-");
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
    assert!(dioxus_ssr::render(&dom).contains("value=\"99.5\""));
    locale.set(Some("de-DE".into()));
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"99,5\""));
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
    focus(&dom, id, "focus");
    value.set(Some("99.5".into()));
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"99,5\""));
    locale.set(Some("en-US".into()));
    value.set(Some("88.5".into()));
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"88,5\""));
    input(&dom, id, "88,6");
    assert_eq!(observed.borrow().writes, [Some("88.6".into())]);
    focus(&dom, id, "blur");
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"88.5\""));
}

/// Browsers with no post-composition input still submit the final captured native value.
#[tokio::test]
async fn composition_end_without_following_input_finishes_once() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&dom, id, "focus");
    composition(&dom, id, "compositionstart");
    input(&dom, id, "17");
    composition(&dom, id, "compositionend");
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
        focus(&dom, id, "focus");
        composition(&dom, id, "compositionstart");
        input(&dom, id, "17");
        composition(&dom, id, "compositionend");
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
    focus(&dom, id, "focus");
    input(&dom, id, "bad");
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
    assert!(html.contains("value=\"99\""));
    assert!(!html.contains("Invalid number"));
    invalid.set(true);
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("The model rejected this value."));
    assert!(observed.borrow().writes.is_empty());
}

/// Drains immediately-ready locale tasks and reactive renders without waiting on timers.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..8 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(5), dom.wait_for_work()).await;
        dom.render_immediate_to_vec();
    }
}

/// Dispatches a native value captured by FormEvent, not a later DOM read.
fn input(dom: &VirtualDom, id: ElementId, value: &str) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let data: dioxus_html::SerializedFormData = serde_json::from_value(serde_json::json!({
        "value": value, "values": [], "files": null
    }))
    .unwrap();
    dom.runtime().handle_event(
        "input",
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
            true,
        )
        .into_any(),
        id,
    );
}

/// Dispatches focus events to the same actual native input.
fn focus(dom: &VirtualDom, id: ElementId, name: &str) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    dom.runtime().handle_event(
        name,
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(
                dioxus_html::SerializedFocusData::default(),
            ))),
            true,
        )
        .into_any(),
        id,
    );
}

/// Required blanks and blur never submit; optional clearing submits absence exactly once.
#[tokio::test]
async fn clearing_respects_optional_contract() {
    for optional in [false, true] {
        let (mut dom, observed, id) = mount(optional);
        settle(&mut dom).await;
        focus(&dom, id, "focus");
        input(&dom, id, "");
        focus(&dom, id, "blur");
        settle(&mut dom).await;
        let expected = if optional { vec![None] } else { vec![] };
        assert_eq!(observed.borrow().writes, expected);
        assert!(dioxus_ssr::render(&dom).contains("value=\"12\""));
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
    assert!(dioxus_ssr::render(&dom).contains("value=\"99\""));
    assert!(observed.borrow().writes.is_empty());
    focus(&dom, id, "focus");
    input(&dom, id, "-");
    disabled.set(true);
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"99\""));
    input(&dom, id, "10");
    disabled.set(false);
    readonly.set(true);
    settle(&mut dom).await;
    input(&dom, id, "11");
    assert!(observed.borrow().writes.is_empty());
}

/// Complete native input submits exact canonical text and preserves its focused spelling.
#[tokio::test]
async fn input_submits_and_acknowledgment_preserves_spelling() {
    let (mut dom, observed, id) = mount(false);
    settle(&mut dom).await;
    focus(&dom, id, "focus");
    input(&dom, id, "0013");
    assert_eq!(observed.borrow().writes, [Some("0013".to_string())]);
    let mut value = observed.borrow().controls.unwrap().value;
    value.set(Some("13".into()));
    settle(&mut dom).await;
    assert!(dioxus_ssr::render(&dom).contains("value=\"0013\""));
    focus(&dom, id, "blur");
    settle(&mut dom).await;
    assert_eq!(observed.borrow().writes.len(), 1);
    assert!(dioxus_ssr::render(&dom).contains("value=\"13\""));
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
        html.contains("aria-invalid=\"true\"") && html.contains("readonly"),
        "{html}"
    );
    assert!(html.contains("Increase Amount") && html.contains("Decrease Amount"));
}
