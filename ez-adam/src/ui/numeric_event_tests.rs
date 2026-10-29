//! Exercises production clamp input events against controlled asynchronous document evaluations.

use super::*;
use dioxus::document::{Document as DocumentProvider, Eval, EvalError, Evaluator};
use serde_json::{Value, json};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

/// Owns one evaluator request, its deferred response, and its waiting task.
#[derive(Default)]
struct Pending {
    script: String,
    response: Option<Result<Value, EvalError>>,
    waker: Option<Waker>,
}

/// Supplies scope-owned evaluators while retaining requests for controlled completion.
#[derive(Default)]
struct ControlledDocument {
    owner: dioxus::core::internal::generational_box::Owner,
    requests: RefCell<Vec<Rc<RefCell<Pending>>>>,
}

/// Receives the test-controlled result through the real Eval deserializer.
struct ControlledEvaluator(Rc<RefCell<Pending>>);

impl Evaluator for ControlledEvaluator {
    /// Rejects unused outbound traffic.
    fn send(&self, _: Value) -> Result<(), EvalError> {
        Err(EvalError::Unsupported)
    }

    /// Delivers a deferred result or registers the current receiver's waker.
    fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        let mut pending = self.0.borrow_mut();
        if let Some(response) = pending.response.take() {
            Poll::Ready(response)
        } else {
            pending.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    /// Leaves script completion pending because these event handlers only receive.
    fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<Value, EvalError>> {
        Poll::Pending
    }
}

impl DocumentProvider for ControlledDocument {
    /// Records a real bridge script and returns its independently controlled evaluator.
    ///
    /// - Complexity: amortized O(1) recording, O(request count) worst-case vector
    ///   growth; retains O(script.len()) script storage.
    fn eval(&self, script: String) -> Eval {
        let pending = Rc::new(RefCell::new(Pending {
            script,
            ..Pending::default()
        }));
        self.requests.borrow_mut().push(pending.clone());
        Eval::new(
            self.owner
                .insert(Box::new(ControlledEvaluator(pending)) as Box<dyn Evaluator>),
        )
    }
}

/// Pumps event tasks and renders their resulting state without a browser or external executor.
///
/// - Complexity: queued task polling plus component rendering.
fn settle(dom: &mut VirtualDom) {
    dom.render_immediate_to_vec();
    {
        let mut work = std::pin::pin!(dom.wait_for_work());
        let _ = work.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    }
    dom.render_immediate_to_vec();
}

/// Dispatches an actual input event; the synthetic empty value cannot supply the live draft.
///
/// - Complexity: event dispatch plus queued task polling and component rendering.
fn input(dom: &mut VirtualDom, id: ElementId) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let data = dioxus_html::SerializedFormData::new(String::new(), Vec::new());
    let event = Event::new(
        Rc::new(dioxus_html::PlatformEventData::new(Box::new(data))),
        true,
    );
    dom.runtime().handle_event("input", event.into_any(), id);
    settle(dom);
}

/// Completes the specified actual request and wakes its awaiting production handler.
///
/// - Complexity: delivery plus queued task polling and component rendering.
fn respond(
    provider: &ControlledDocument,
    index: usize,
    response: Result<Value, EvalError>,
    dom: &mut VirtualDom,
) {
    let request = provider.requests.borrow()[index].clone();
    let waker = {
        let mut request = request.borrow_mut();
        request.response = Some(response);
        request.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
    settle(dom);
}

/// Real minimum/maximum events apply exact drafts and surface transport/decoding failures.
#[test]
fn numeric_events_route_bounds_errors_and_stale_replies() {
    for mode in [0, 6] {
        let (mut dom, events, observed, _) = mount(mode);
        let provider = Rc::new(ControlledDocument::default());
        dom.provide_root_context(provider.clone() as Rc<dyn DocumentProvider>);
        let (document, _, _) = observed.borrow().unwrap();
        let cell = document.peek().cell_order[0];
        let inputs: Vec<_> = events
            .iter()
            .filter(|(name, _)| name == "input")
            .map(|(_, id)| *id)
            .collect();
        let min = inputs[1];
        let max = inputs[2];
        let (minimum, maximum) = if mode == 0 {
            ("9007199254740993", "9007199254740995")
        } else {
            ("-1.25", "3.5")
        };
        input(&mut dom, min);
        assert!(
            provider.requests.borrow()[0]
                .borrow()
                .script
                .contains("clamp-min-")
        );
        respond(&provider, 0, Ok(json!(format!("1|{minimum}"))), &mut dom);
        input(&mut dom, max);
        assert!(
            provider.requests.borrow()[1]
                .borrow()
                .script
                .contains("clamp-max-")
        );
        respond(&provider, 1, Ok(json!(format!("1|{maximum}"))), &mut dom);
        let expected = if mode == 0 {
            CellType::I64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(9007199254740993),
                    max: Some(9007199254740995),
                },
            }
        } else {
            CellType::F64 {
                clamp: crate::model::cell::ClampRange {
                    min: Some(-1.25),
                    max: Some(3.5),
                },
            }
        };
        assert_eq!(document.peek().cells[cell].ty, expected);
        let html = dioxus_ssr::render(&dom);
        assert!(
            html.contains(&format!("value=\"{minimum}\""))
                && html.contains(&format!("value=\"{maximum}\""))
        );
        for (index, response, visible) in [
            (
                2,
                Err(EvalError::Communication("offline".into())),
                "numeric field read failed",
            ),
            (3, Ok(json!(42)), "numeric field read failed"),
            (4, Ok(json!("malformed")), "invalid numeric field response"),
        ] {
            input(&mut dom, min);
            respond(&provider, index, response, &mut dom);
            let html = dioxus_ssr::render(&dom);
            assert!(
                html.contains("role=\"alert\"") && html.contains(visible),
                "{html}"
            );
            assert_eq!(document.peek().cells[cell].ty, expected);
        }
        input(&mut dom, min); // old success must not clear the newer transport error.
        input(&mut dom, min);
        respond(
            &provider,
            6,
            Err(EvalError::Communication("latest failure".into())),
            &mut dom,
        );
        let newest_error = dioxus_ssr::render(&dom);
        assert!(newest_error.contains("numeric field read failed"));
        respond(&provider, 5, Ok(json!("1|0")), &mut dom);
        assert_eq!(document.peek().cells[cell].ty, expected);
        assert_eq!(dioxus_ssr::render(&dom), newest_error);
        input(&mut dom, max); // old error must not replace the newer successful field state.
        input(&mut dom, max);
        respond(&provider, 8, Ok(json!(format!("1|{maximum}"))), &mut dom);
        respond(
            &provider,
            7,
            Err(EvalError::Communication("obsolete".into())),
            &mut dom,
        );
        assert_eq!(document.peek().cells[cell].ty, expected);
        assert!(!dioxus_ssr::render(&dom).contains("role=\"alert\""));
        assert_eq!(
            html.matches("<sp-number-field").count(),
            if mode == 0 { 0 } else { 2 }
        );
    }
}
