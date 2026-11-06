//! Exercises checked reactive ownership through public handle contracts.
use super::*;
use ::dioxus::prelude::*;
use std::{cell::RefCell, rc::Rc};

/// Exposes a scope-owned handle to deterministic event-loop tests.
type Slot = Rc<RefCell<Option<SheetHandle>>>;

/// Observes typed borrow failures at the host's public event boundary.
type Errors = Rc<RefCell<Vec<BindingError>>>;

/// Records each host outcome together with callback-time model availability.
type Outcomes = Rc<RefCell<Vec<(HostOutcome, Result<(), BindingError>)>>>;

/// Initial false output requirements reject ownership before any publication.
#[test]
fn owner_rejects_initial_false_output_requirement() {
    let result = SheetOwner::new(
        Rc::new(|| {
            crate::build_sheet(
                "sheet s { cell amount: i32 = 0;
                    out total := amount require { @positive total > 0; }; }",
                "initial-output.adm2",
            )
        }),
        "validation/v1".into(),
    );
    let Err(crate::BuildError::Validation { issues, .. }) = result else {
        panic!("initial requirement failure must return validation without an owner");
    };
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].key, "total");
    assert_eq!(issues[0].requirement.as_deref(), Some("positive"));
    let location = issues[0].labels[0].location.as_ref().unwrap();
    assert!(location.source.text().contains("@positive total > 0"));
}

/// Builds an editable value and a validation boundary.
fn factory() -> ModelFactory {
    Rc::new(|| {
        crate::build_sheet(
            "sheet s { cell x: i32 = 1; out result := x require { @positive x > 0; }; }",
            "same.adm2",
        )
    })
}

/// Mounts one cached owner and restores recoverable host rejections.
fn host() -> Element {
    let slot: Slot = consume_context();
    let errors = try_consume_context::<Errors>();
    let sheet = use_sheet(
        factory(),
        "s/v1".into(),
        EventHandler::new(move |event: SheetEvent| {
            if let HostOutcome::Rejected(HostRejection::Binding(error)) = &event.outcome
                && let Some(errors) = &errors
            {
                errors.borrow_mut().push(error.clone());
            }
            if matches!(
                event.outcome,
                HostOutcome::Rejected(HostRejection::Validation(_))
            ) {
                event.sheet.restore_accepted().unwrap();
            }
        }),
    )
    .unwrap();
    *slot.borrow_mut() = Some(sheet);
    rsx! {}
}

/// Returns a mounted owner whose signals remain alive with its virtual DOM.
fn mounted() -> (VirtualDom, SheetHandle) {
    let slot: Slot = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new(host);
    dom.provide_root_context(slot.clone());
    dom.rebuild_in_place();
    let sheet = slot.borrow().unwrap();
    (dom, sheet)
}

/// Accepted history remains borrowed, and callback-initiated writes reject without panicking.
#[test]
fn accepted_history_access_is_checked_and_not_copied() {
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let slot: Slot = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new(host);
    dom.provide_root_context(slot.clone());
    dom.provide_root_context(errors.clone());
    dom.rebuild_in_place();
    let sheet = slot.borrow().unwrap();
    sheet.apply(
        &[CellEdit {
            key: "x".into(),
            canonical: "4".into(),
        }],
        UpdateOrigin::User,
    );
    sheet
        .with_accepted_state(|state| {
            assert_eq!(state.transactions.len(), 1);
            assert_eq!(state.transactions[0][0].canonical, "4");
            sheet.apply(
                &[CellEdit {
                    key: "x".into(),
                    canonical: "5".into(),
                }],
                UpdateOrigin::User,
            );
            assert_eq!(state.transactions.len(), 1);
        })
        .unwrap();
    assert_eq!(sheet.cell("x").view().unwrap().text, "4");
    assert_eq!(*errors.borrow(), [BindingError::ReentrantMutation]);
}

/// A factory can retain a handle and observe structured contention during restoration.
#[test]
fn factory_reentrant_history_access_does_not_invoke_callback() {
    /// Builds a factory that attempts a checked read only after handle initialization.
    fn app() -> Element {
        let slot: Slot = consume_context();
        let retained = slot.clone();
        let factory: ModelFactory = Rc::new(move || {
            if let Some(sheet) = *retained.borrow() {
                let result = sheet.with_accepted_state(|_| panic!("contended callback ran"));
                assert_eq!(result, Err(BindingError::ReentrantMutation));
            }
            factory()()
        });
        let sheet = use_sheet(factory, "s/v1".into(), EventHandler::new(|_| {})).unwrap();
        *slot.borrow_mut() = Some(sheet);
        rsx! {}
    }
    let slot: Slot = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(slot.clone());
    dom.rebuild_in_place();
    let sheet = slot.borrow().unwrap();
    sheet.restore_accepted().unwrap();
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        0
    );
}

/// Recovery in a host outcome callback runs after the mutable owner borrow ends.
#[test]
fn host_callback_runs_after_model_borrow_is_released() {
    let (_dom, sheet) = mounted();
    sheet.apply(
        &[CellEdit {
            key: "x".into(),
            canonical: "-1".into(),
        }],
        UpdateOrigin::User,
    );
    assert_eq!(
        sheet
            .inspect(|p, _| *p.read::<i32>(p.cell_names["x"].0).unwrap())
            .unwrap(),
        1
    );
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        0
    );
}

/// A replaced instance cannot accept events captured by its former symbolic bindings.
#[test]
fn retired_binding_cannot_write_replacement() {
    let (_dom, sheet) = mounted();
    let old = sheet.cell("x");
    sheet.replace(factory(), "s/v1".into()).unwrap();
    old.submit("9".into());
    assert_eq!(sheet.cell("x").view().unwrap().text, "1");
    assert!(matches!(old.view(), Err(BindingError::RetiredInstance)));
}

/// Rejected and fatal candidates leave accepted history available without failed-model reads.
#[test]
fn history_survives_rejection_and_fatal_retirement() {
    /// Mounts an arithmetic failure boundary and records host-visible classifications.
    fn app() -> Element {
        let slot: Slot = consume_context();
        let outcomes: Outcomes = consume_context();
        let factory: ModelFactory = Rc::new(|| {
            crate::build_sheet(
                "sheet s { cell x: i32 = 1; out r := 10 / x; }",
                "fatal.adm2",
            )
        });
        let sheet = use_sheet(
            factory,
            "fatal/v1".into(),
            EventHandler::new(move |event: SheetEvent| {
                let available = event.sheet.inspect(|_, _| ());
                outcomes.borrow_mut().push((event.outcome, available));
            }),
        )
        .unwrap();
        *slot.borrow_mut() = Some(sheet);
        rsx! {}
    }

    let slot: Slot = Rc::new(RefCell::new(None));
    let outcomes: Outcomes = Rc::new(RefCell::new(Vec::new()));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(slot.clone());
    dom.provide_root_context(outcomes.clone());
    dom.rebuild_in_place();
    let sheet = slot.borrow().unwrap();
    sheet.cell("x").submit("2".into());
    sheet.cell("x").submit("invalid".into());
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
    assert_eq!(sheet.inspect(|_, _| ()), Err(BindingError::Unavailable));
    sheet.restore_accepted().unwrap();
    sheet.cell("x").submit("0".into());
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
    assert_eq!(sheet.inspect(|_, _| ()), Err(BindingError::RetiredInstance));
    assert!(sheet.restore_accepted().is_err());
    let called = Rc::new(std::cell::Cell::new(false));
    let marker = called.clone();
    assert!(
        sheet
            .replace(
                Rc::new(move || {
                    marker.set(true);
                    factory()()
                }),
                "replacement".into()
            )
            .is_err()
    );
    assert!(!called.get(), "a fatal owner must not rebuild");
    let outcomes = outcomes.borrow();
    assert!(matches!(outcomes[0], (HostOutcome::Accepted, Ok(()))));
    assert!(matches!(
        outcomes[1],
        (
            HostOutcome::Rejected(HostRejection::Input(_)),
            Err(BindingError::Unavailable)
        )
    ));
    assert!(matches!(
        outcomes[2],
        (HostOutcome::Fatal(_), Err(BindingError::RetiredInstance))
    ));
}

/// Recording failure preserves accepted callback reads and reactive publication.
#[test]
fn recording_failure_callback_observes_accepted_publication() {
    /// Mounts an exact capture codec whose recording-only invocation fails.
    fn app() -> Element {
        let slot: Slot = consume_context();
        let outcomes: Outcomes = consume_context();
        let captures = Rc::new(std::cell::Cell::new(0));
        let factory: ModelFactory = Rc::new(move || {
            let mut built = factory()()?;
            let id = built.keys["x"];
            let captures = captures.clone();
            built.labels.cells.get_mut(&id).unwrap().capture = Box::new(move |sheet| {
                captures.set(captures.get() + 1);
                if captures.get() == 4 {
                    Err(crate::input::InputError::UnsupportedEditor(
                        "capture failed".into(),
                    ))
                } else {
                    Ok(sheet.read::<i32>(id).unwrap().to_string())
                }
            });
            Ok(built)
        });
        let sheet = use_sheet(
            factory,
            "s/v1".into(),
            EventHandler::new(move |event: SheetEvent| {
                let available = event.sheet.inspect(|_, _| ());
                outcomes.borrow_mut().push((event.outcome, available));
            }),
        )
        .unwrap();
        *slot.borrow_mut() = Some(sheet);
        let view = use_memo(move || sheet.cell("x").view().unwrap().text.clone());
        rsx! { p { "{view}" } }
    }

    let slot: Slot = Rc::new(RefCell::new(None));
    let outcomes: Outcomes = Rc::new(RefCell::new(Vec::new()));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(slot.clone());
    dom.provide_root_context(outcomes.clone());
    dom.rebuild_in_place();
    let sheet = slot.borrow().unwrap();
    sheet.start_recording(&["result"]).unwrap();
    sheet.cell("x").submit("5".into());
    dom.render_immediate_to_vec();
    assert!(matches!(
        outcomes.borrow()[0],
        (HostOutcome::RecordingFailed(_), Ok(()))
    ));
    assert_eq!(sheet.cell("x").view().unwrap().text, "5");
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
    assert!(dioxus_ssr::render(&dom).contains(">5</p>"));
    sheet.cell("x").submit("6".into());
    assert!(matches!(
        outcomes.borrow()[1],
        (HostOutcome::Accepted, Ok(()))
    ));
}

/// Recorder controls share immutable results, preserve aspirations, and replay fresh owners.
#[test]
fn reactive_recording_and_prevalidated_installation() {
    let (_dom, sheet) = mounted();
    sheet.start_recording(&["result"]).unwrap();
    sheet.cell("x").submit("5".into());
    let recording = sheet.last_recording().unwrap();
    assert!(Rc::ptr_eq(&recording, &sheet.last_recording().unwrap()));
    assert_eq!(recording.values[0].canonical, "5");
    let captured = sheet.capture(&["result"]).unwrap();
    sheet.stop_recording();
    sheet.cell("x").submit("6".into());
    assert!(Rc::ptr_eq(&recording, &sheet.last_recording().unwrap()));
    let old = sheet.cell("x");
    sheet.playback(factory(), "s/v1".into(), &captured).unwrap();
    assert!(matches!(old.view(), Err(BindingError::RetiredInstance)));
    assert_eq!(sheet.cell("x").view().unwrap().text, "5");
    assert!(sheet.last_recording().is_none());
    let old = sheet.cell("x");
    sheet
        .install(SheetOwner::new(factory(), "s/v1".into()).unwrap())
        .unwrap();
    assert!(matches!(old.view(), Err(BindingError::RetiredInstance)));
    assert_eq!(sheet.cell("x").view().unwrap().text, "1");
}

/// Retains hook results and a render trigger without reaching into the model owner.
#[derive(Default)]
struct Cached {
    tick: Option<Signal<bool>>,
    results: Vec<Result<SheetHandle, Rc<crate::BuildError>>>,
}

/// Re-renders reuse successful owners and the exact independently owned build error.
#[test]
fn use_sheet_caches_success_and_failure_across_renders() {
    /// Calls the initialization hook with a newly constructed factory each render.
    fn app(fail: bool) -> Element {
        let (calls, observed): (Rc<std::cell::Cell<usize>>, Rc<RefCell<Cached>>) =
            consume_context();
        let tick = use_signal(|| false);
        let _ = tick();
        let factory: ModelFactory = Rc::new(move || {
            calls.set(calls.get() + 1);
            crate::build_sheet(
                if fail {
                    "sheet s { cell }"
                } else {
                    "sheet s {}"
                },
                "s.adm2",
            )
        });
        let result = use_sheet(factory, "s/v1".into(), EventHandler::new(|_| {}));
        let mut observed = observed.borrow_mut();
        observed.tick = Some(tick);
        observed.results.push(result);
        rsx! {}
    }
    for fail in [false, true] {
        let observed = Rc::new(RefCell::new(Cached::default()));
        let calls = Rc::new(std::cell::Cell::new(0_usize));
        let mut dom = VirtualDom::new_with_props(app, fail);
        dom.provide_root_context((calls.clone(), observed.clone()));
        dom.rebuild_in_place();
        observed.borrow().tick.unwrap().set(true);
        dom.render_immediate_to_vec();
        assert_eq!(calls.get(), 1);
        let observed = observed.borrow();
        assert_eq!(observed.results.len(), 2);
        match (&observed.results[0], &observed.results[1]) {
            (Ok(first), Ok(second)) => assert!(first == second),
            (Err(first), Err(second)) => assert!(Rc::ptr_eq(first, second)),
            _ => panic!("cached initialization changed category"),
        }
    }
}

/// A prebuilt failed candidate cannot replace the accepted owner.
#[test]
fn install_rejects_an_unavailable_candidate() {
    let (_dom, sheet) = mounted();
    let mut candidate = SheetOwner::new(factory(), "s/v1".into()).unwrap();
    candidate.apply(
        &[CellEdit {
            key: "x".into(),
            canonical: "bad".into(),
        }],
        UpdateOrigin::User,
    );
    assert_eq!(sheet.install(candidate), Err(BindingError::Unavailable));
    assert_eq!(sheet.cell("x").view().unwrap().text, "1");
}
