//! Exercises panel operations through native form events and the actual numeric eval receiver.
//!
//! The renderer fixture runs the production Intl scripts in Node with German as the
//! host default, and delivers ordered snapshots through the component's source channel.

use super::*;
use crate::persistence::to_json;
use dioxus::document::{Document as DocumentProvider, Eval, EvalError, Evaluator};
use serde_json::{Value, json};
use std::{collections::VecDeque, task::Poll};

/// Exposes the replacement generation without changing the product component's API.
pub(super) type EpochObserver = Rc<Cell<Option<Signal<u64>>>>;

/// Models one renderer-owned, ordered native input transport.
#[derive(Default)]
struct Source {
    token: String,
    epoch: String,
    sequence: u64,
    frames: VecDeque<Value>,
    waker: Option<Waker>,
    value: String,
    closed: bool,
}

impl Source {
    /// Captures an immutable native snapshot and wakes its actual component receiver.
    fn push(&mut self, action: &str, value: &str) {
        self.sequence = self.sequence.checked_add(1).unwrap();
        self.value = value.into();
        self.frames.push_back(json!({
            "token": self.token, "epoch": self.epoch,
            "sequence": self.sequence.to_string(), "session": "1",
            "action": action, "value": value, "message": "native range unavailable",
        }));
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

/// Runs real locale scripts and owns persistent renderer channels for the mounted controls.
#[derive(Default)]
struct NativeDocument {
    owner: dioxus::core::internal::generational_box::Owner,
    sources: RefCell<Vec<Rc<RefCell<Source>>>>,
}

impl DocumentProvider for NativeDocument {
    /// Runs Intl scripts in Node or creates the production source adapter's transport.
    ///
    /// - Complexity: O(script bytes) plus the Node process and Intl execution.
    fn eval(&self, script: String) -> Eval {
        if let Some((_, tail)) = script.rsplit_once("return await runNumericEvents(window, \"") {
            let source = Rc::new(RefCell::new(Source {
                token: tail.split('"').next().unwrap().into(),
                epoch: "0".into(),
                ..Default::default()
            }));
            source.borrow_mut().push("ready", "");
            self.sources.borrow_mut().push(source.clone());
            return Eval::new(self.owner.insert(Box::new(SourceReply(source))));
        }
        let value = if script.contains("await ")
            || script.contains("setTimeout(() => resolve(null), 5000)")
        {
            Value::Null
        } else {
            let output = std::process::Command::new("node")
                .args(["-e", &format!(
                    "const Original=Intl.NumberFormat;Intl.NumberFormat=function(l,o){{return new Original(l??'de-DE',o)}};Object.setPrototypeOf(Intl.NumberFormat,Original);const messages=[];new Function('dioxus',{})({{send:x=>messages.push(x)}});console.log(JSON.stringify(messages[0]));",
                    serde_json::to_string(&script).unwrap()
                )])
                .output()
                .expect("Node is required for native locale adapter tests");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "{error}: stdout={}; script={script}",
                    String::from_utf8_lossy(&output.stdout)
                )
            })
        };
        Eval::new(self.owner.insert(Box::new(Reply(Some(value)))))
    }

    /// Leaves head links to the renderer rather than interpreting them in Node.
    fn create_link(&self, _: dioxus::document::LinkProps) {}

    /// Leaves head styles to the renderer rather than interpreting them in Node.
    fn create_style(&self, _: dioxus::document::StyleProps) {}
}

/// Supplies a one-shot Intl result through the real Eval decoder.
struct Reply(Option<Value>);

impl Evaluator for Reply {
    /// Rejects unused commands to a one-shot evaluation.
    fn send(&self, _: Value) -> Result<(), EvalError> {
        Err(EvalError::Unsupported)
    }
    /// Returns the script's JSON reply.
    fn poll_recv(&mut self, _: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        Poll::Ready(self.0.take().ok_or(EvalError::Unsupported))
    }
    /// Completes one-shot evaluations.
    fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        Poll::Ready(Ok(Value::Null))
    }
}

/// Supplies native snapshots and applies display/reset commands from the real adapter.
struct SourceReply(Rc<RefCell<Source>>);

impl Evaluator for SourceReply {
    /// Applies source control messages without reading a later DOM value.
    fn send(&self, message: Value) -> Result<(), EvalError> {
        let mut source = self.0.borrow_mut();
        match message["kind"].as_str() {
            Some("close") => source.closed = true,
            Some("reset") => {
                source.epoch = message["epoch"].as_str().unwrap().into();
                source.push("ready", message["value"].as_str().unwrap());
            }
            Some("display")
                if message["sequence"].as_str() == Some(&source.sequence.to_string()) =>
            {
                source.value = message["value"].as_str().unwrap().into();
            }
            Some("display") => {}
            _ => return Err(EvalError::Unsupported),
        }
        Ok(())
    }
    /// Delivers the next ordered snapshot or suspends the receiver.
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        let mut source = self.0.borrow_mut();
        if let Some(frame) = source.frames.pop_front() {
            Poll::Ready(Ok(frame))
        } else {
            source.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
    /// Keeps the persistent evaluation open until the component retires it.
    fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        if self.0.borrow().closed {
            Poll::Ready(Ok(Value::Null))
        } else {
            Poll::Pending
        }
    }
}

/// Drains immediately runnable source, locale, and render work.
fn settle(dom: &mut VirtualDom) {
    for _ in 0..16 {
        settle_effects(dom);
    }
}

/// Mounts a production side panel with an observable document and renderer transport.
fn native_mount(mode: u8) -> (Mounted, Rc<NativeDocument>, EpochObserver) {
    let observed: Observer = Rc::new(RefCell::new(None));
    let renders = Rc::new(Cell::new(0));
    let epoch = Rc::new(Cell::new(None));
    let renderer = Rc::new(NativeDocument::default());
    let mut dom = VirtualDom::new_with_props(host, mode);
    dom.provide_root_context((observed.clone(), renders.clone()));
    dom.provide_root_context(epoch.clone());
    dom.provide_root_context(renderer.clone() as Rc<dyn DocumentProvider>);
    let edits = dom.rebuild_to_vec().edits;
    let events = edits
        .into_iter()
        .filter_map(|edit| match edit {
            Mutation::NewEventListener { name, id } => Some((name, id)),
            Mutation::SetAttribute {
                name: "aria-label",
                value: dioxus::core::AttributeValue::Text(label),
                id,
                ..
            } => Some((format!("label:{label}"), id)),
            _ => None,
        })
        .collect();
    settle(&mut dom);
    ((dom, events, observed, renders), renderer, epoch)
}

/// Dispatches a native text event with its synchronously captured value.
fn text(dom: &mut VirtualDom, id: ElementId, value: &str) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let data = dioxus_html::SerializedFormData::new(value.into(), Vec::new());
    dom.runtime().handle_event(
        "input",
        Event::new(
            Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
            true,
        )
        .into_any(),
        id,
    );
    settle(dom);
}

/// Delivers one native source event, including its matching bubbling focusout on blur.
fn frame(
    dom: &mut VirtualDom,
    source: &Rc<RefCell<Source>>,
    action: &str,
    value: &str,
    events: &[(String, ElementId)],
) {
    source.borrow_mut().push(action, value);
    if action == "blur" {
        for (_, id) in events.iter().filter(|(name, _)| name == "focusout") {
            dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
            dom.runtime().handle_event(
                "focusout",
                Event::new(
                    Rc::new(dioxus_html::PlatformEventData::new(Box::new(
                        dioxus_html::SerializedFocusData::default(),
                    ))),
                    true,
                )
                .into_any(),
                *id,
            );
        }
    }
    settle(dom);
}

/// Native property and formula events invoke existing Document operations.
#[test]
fn native_property_and_formula_events_update_document() {
    for mode in [0, 1, 2, 7] {
        let ((mut dom, events, observed, _), _, _) = native_mount(mode);
        let (document, _, _) = observed.borrow().unwrap();
        let name_id = events
            .iter()
            .find(|(name, _)| name == "label:Name")
            .unwrap()
            .1;
        text(&mut dom, name_id, "renamed");
        let doc = document.peek();
        match mode {
            0 => assert_eq!(doc.cells[doc.cell_order[0]].name, "renamed"),
            1 => assert_eq!(
                doc.relationship_groups[doc.relationship_group_order[0]].display_name,
                "renamed"
            ),
            _ => assert_eq!(
                doc.conditional_groups[doc.conditional_group_order[0]].display_name,
                "renamed"
            ),
        }
        drop(doc);
        if mode < 2 {
            let input_id = events
                .iter()
                .find(|(name, _)| {
                    if mode == 0 {
                        name == "label:Restrict"
                    } else {
                        name.starts_with("label:Formula for ")
                    }
                })
                .unwrap()
                .1;
            text(
                &mut dom,
                input_id,
                if mode == 0 { "positive" } else { "amount + 1" },
            );
            let doc = document.peek();
            if mode == 0 {
                assert_eq!(
                    doc.cells[doc.cell_order[0]].restrict.as_deref(),
                    Some("positive")
                );
            } else {
                assert!(
                    doc.relationship_groups[doc.relationship_group_order[0]]
                        .members
                        .iter()
                        .any(|(_, formula)| formula == "amount + 1")
                );
            }
        }
        if mode == 7 {
            let input_id = events
                .iter()
                .find(|(name, _)| name == "label:Condition")
                .unwrap()
                .1;
            text(&mut dom, input_id, "enabled && true");
            let doc = document.peek();
            let crate::model::conditional_group::ConditionExpr::Formula { expr, .. } =
                &doc.conditional_groups[doc.conditional_group_order[0]].condition
            else {
                panic!("formula")
            };
            assert_eq!(expr, "enabled && true");
        }
    }
}

/// Localized bounds commit exact values, clear to absence, and recover lexical drafts on blur.
#[test]
fn native_localized_bounds_clear_errors_and_blur_preserve_serialization() {
    let ((mut dom, events, observed, _), renderer, _) = native_mount(6);
    let (mut document, _, _) = observed.borrow().unwrap();
    let cell = document.peek().cell_order[0];
    let source = renderer.sources.borrow()[0].clone();
    frame(&mut dom, &source, "focus", "", &events);
    frame(&mut dom, &source, "input", "1.234,5", &events);
    {
        let doc = document.peek();
        let CellType::F64 { clamp } = &doc.cells[cell].ty else {
            panic!("F64")
        };
        assert_eq!(clamp.min, Some(1234.5));
    }
    frame(&mut dom, &source, "input", "wrong", &events);
    let invalid_html = dioxus_ssr::render(&dom);
    assert!(invalid_html.contains("Invalid number"), "{invalid_html}");
    let before = to_json(&document.peek()).unwrap();
    frame(&mut dom, &source, "blur", "wrong", &events);
    assert_eq!(to_json(&document.peek()).unwrap(), before);
    assert_eq!(source.borrow().value, "1.234,5");
    assert!(!dioxus_ssr::render(&dom).contains("Invalid number"));
    frame(&mut dom, &source, "focus", "1.234,5", &events);
    frame(&mut dom, &source, "input", "", &events);
    {
        let doc = document.peek();
        let CellType::F64 { clamp } = &doc.cells[cell].ty else {
            panic!("F64")
        };
        assert_eq!(clamp.min, None);
    }
    set_clamp_min(&mut document.write(), cell, "3.5");
    let after_external = to_json(&document.peek()).unwrap();
    frame(&mut dom, &source, "blur", "", &events);
    assert_eq!(to_json(&document.peek()).unwrap(), after_external);
    assert_eq!(source.borrow().value, "3,5");
}

/// An unresolved source failure survives blur without writing the document or becoming model rejection.
#[test]
fn native_source_failure_remains_visible_after_blur() {
    let ((mut dom, events, observed, _), renderer, _) = native_mount(6);
    let (document, _, _) = observed.borrow().unwrap();
    let before = to_json(&document.peek()).unwrap();
    let source = renderer.sources.borrow()[0].clone();
    frame(&mut dom, &source, "focus", "", &events);
    frame(&mut dom, &source, "error", "", &events);
    assert!(dioxus_ssr::render(&dom).contains("native range unavailable"));
    frame(&mut dom, &source, "blur", "", &events);
    let html = dioxus_ssr::render(&dom);
    assert!(
        html.contains("Number event source error: native range unavailable"),
        "{html}"
    );
    assert!(!html.contains("The model rejected this value."), "{html}");
    assert_eq!(to_json(&document.peek()).unwrap(), before);
}

/// Retired selection and document source callbacks cannot write a successor document.
#[test]
fn native_old_selection_and_document_events_cannot_write_successors() {
    let ((mut dom, events, observed, _), renderer, epoch) = native_mount(6);
    let (mut document, mut selection, _) = observed.borrow().unwrap();
    let old_source = renderer.sources.borrow()[0].clone();
    let cell = document.peek().cell_order[0];
    let alias = add_cell_node(&mut document.write(), cell, Point::new(500.0, 60.0));
    selection.set(HashSet::from([NodeId::CellNode(alias)]));
    settle(&mut dom);
    assert!(old_source.borrow().closed);
    let before = to_json(&document.peek()).unwrap();
    frame(&mut dom, &old_source, "input", "99", &events);
    assert_eq!(to_json(&document.peek()).unwrap(), before);
    let active_source = {
        let sources = renderer.sources.borrow();
        sources[sources.len() - 2].clone()
    };
    active_source.borrow_mut().push("input", "77");
    document.set(fixture());
    selection.set(HashSet::from([NodeId::CellNode(
        document.peek().cell_nodes.keys().next().unwrap(),
    )]));
    let replacement = to_json(&document.peek()).unwrap();
    let mut generation = epoch.get().unwrap();
    generation.set(1);
    settle(&mut dom);
    assert!(active_source.borrow().closed);
    assert_eq!(to_json(&document.peek()).unwrap(), replacement);
    frame(&mut dom, &active_source, "input", "88", &events);
    assert_eq!(to_json(&document.peek()).unwrap(), replacement);
}

/// Exact integer aspirations and range failures traverse both mounted numeric bound controls.
#[test]
fn native_integer_bounds_preserve_digits_and_reject_out_of_range_edits() {
    let ((mut dom, events, observed, _), renderer, _) = native_mount(0);
    let (document, _, _) = observed.borrow().unwrap();
    let sources = renderer.sources.borrow();
    let min = sources[0].clone();
    let max = sources[1].clone();
    drop(sources);
    frame(&mut dom, &min, "input", "9007199254740993", &events);
    frame(&mut dom, &max, "input", "9007199254740995", &events);
    {
        let doc = document.peek();
        let CellType::I64 { clamp } = &doc.cells[doc.cell_order[0]].ty else {
            panic!("I64")
        };
        assert_eq!(clamp.min, Some(9_007_199_254_740_993));
        assert_eq!(clamp.max, Some(9_007_199_254_740_995));
    }
    let before = to_json(&document.peek()).unwrap();
    frame(&mut dom, &min, "input", "9223372036854775808", &events);
    assert!(dioxus_ssr::render(&dom).contains("outside Signed(64)"));
    assert_eq!(to_json(&document.peek()).unwrap(), before);
    frame(&mut dom, &max, "input", "", &events);
    let doc = document.peek();
    let CellType::I64 { clamp } = &doc.cells[doc.cell_order[0]].ty else {
        panic!("I64")
    };
    assert_eq!(clamp.max, None);
}
