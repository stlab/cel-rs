//! Exercises bound controls and host boundaries with actual Dioxus event dispatch.
use crate::{InspectorPresentation, SheetInspector, binding::*, components::Theme};
use dioxus::{
    core::{ElementId, Mutation},
    prelude::*,
};
use std::{cell::RefCell, rc::Rc};

/// Retains consumer-visible outcomes and the scope-owned handle.
#[derive(Default)]
struct Observed {
    sheet: Option<SheetHandle>,
    outcomes: Vec<HostOutcome>,
    restored: bool,
}

/// Builds a multi-way model with text, Boolean and exact numeric controls.
fn factory() -> ModelFactory {
    Rc::new(|| {
        let mut built = crate::build_sheet(
            r#"sheet controls {
                cell text: String = "001";
                cell flag: bool = true;
                cell a: i32 = 1;
                cell b: i32 = 1;
                cell ranged: i64 = 9007199254740993i64
                    filter 9007199254740992i64..=9007199254740994i64;
                cell unused: i32 = 2;
                out result := (text, flag, a, b, ranged) require { @keep flag; };
            }"#,
            "same.adm2",
        )?;
        built
            .parsed
            .add_relationship(vec![
                adam_rs::Method::from_fn_1_1(built.keys["a"], built.keys["b"], |a: &i32| Ok(*a)),
                adam_rs::Method::from_fn_1_1(built.keys["b"], built.keys["a"], |b: &i32| Ok(*b)),
            ])
            .unwrap();
        built.parsed.propagate().unwrap();
        Ok(built)
    })
}

/// Mounts real bound widgets plus named application actions.
fn host() -> Element {
    let observed: Rc<RefCell<Observed>> = consume_context();
    let callbacks = observed.clone();
    let sheet = use_sheet(
        factory(),
        "controls/v1".into(),
        EventHandler::new(move |event: SheetEvent| {
            if matches!(
                event.outcome,
                HostOutcome::Rejected(HostRejection::Validation(_))
            ) {
                event.sheet.restore_accepted().unwrap();
                callbacks.borrow_mut().restored = true;
            }
            callbacks.borrow_mut().outcomes.push(event.outcome);
        }),
    )
    .unwrap();
    observed.borrow_mut().sheet = Some(sheet);
    let a = sheet.cell("a");
    let b = sheet.cell("b");
    rsx! { Theme {
        EditText { bind: sheet.cell("text"), label: "first" }
        EditText { bind: sheet.cell("text"), label: "second" }
        CheckBox { bind: sheet.cell("flag"), label: "Flag" }
        EditNumber { bind: sheet.cell("a"), label: "A" }
        EditSlider { bind: sheet.cell("ranged"), label: "Ranged" }
        button { id: "edit-a".to_string(), onclick: move |_| a.submit("7".into()), "Edit A" }
        button { id: "edit-b".to_string(), onclick: move |_| b.submit("9".into()), "Edit B" }
        SheetInspector { sheet }
    } }
}

/// Mounts the fixture and returns its real mutation stream.
fn mounted() -> (VirtualDom, Rc<RefCell<Observed>>, dioxus::core::Mutations) {
    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(host);
    dom.provide_root_context(observed.clone());
    let edits = dom.rebuild_to_vec();
    (dom, observed, edits)
}

/// Dispatches a native serialized event to an actual mounted listener.
fn dispatch(dom: &mut VirtualDom, name: &str, id: ElementId, value: &str) {
    dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
    let payload: Box<dyn std::any::Any> = match name {
        "input" => Box::new(dioxus_html::SerializedFormData::new(value.into(), vec![])),
        _ => Box::<dioxus_html::SerializedMouseData>::default(),
    };
    dom.runtime().handle_event(
        name,
        Event::new(Rc::new(dioxus_html::PlatformEventData::new(payload)), true).into_any(),
        id,
    );
    dom.render_immediate_to_vec();
}

/// Locates a rendered application's explicit action by its emitted ID attribute.
fn named(edits: &dioxus::core::Mutations, wanted: &str) -> ElementId {
    edits
        .edits
        .iter()
        .find_map(|mutation| match mutation {
            Mutation::SetAttribute {
                name: "id",
                value: dioxus::core::AttributeValue::Text(value),
                id,
                ..
            } if value == wanted => Some(*id),
            _ => None,
        })
        .unwrap()
}

/// Editing one bound text control refreshes another without writing on synchronization.
#[test]
fn binding_edit_updates_other_widget() {
    let (mut dom, observed, edits) = mounted();
    let input = edits
        .edits
        .iter()
        .find_map(|mutation| match mutation {
            Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
            _ => None,
        })
        .unwrap();
    dispatch(&mut dom, "input", input, "changed");
    let sheet = observed.borrow().sheet.unwrap();
    assert_eq!(sheet.cell("text").view().unwrap().text, "changed");
    let html = dioxus_ssr::render(&dom);
    assert!(html.matches("value=\"changed\"").count() >= 2, "{html}");
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
}

/// Derived cells remain writable and reverse the relationship's source direction.
#[test]
fn derived_cell_write_changes_direction() {
    let (mut dom, observed, edits) = mounted();
    dispatch(&mut dom, "click", named(&edits, "edit-a"), "");
    let sheet = observed.borrow().sheet.unwrap();
    assert_eq!(sheet.cell("b").view().unwrap().text, "7");
    assert!(!sheet.cell("b").view().unwrap().is_source);
    assert!(!sheet.cell("b").view().unwrap().flags.disabled);
    assert!(!sheet.cell("b").view().unwrap().flags.readonly);
    assert!(sheet.cell("unused").view().unwrap().flags.disabled);
    assert!(!sheet.cell("unused").view().unwrap().flags.relevant);
    assert!(sheet.cell("result").view().unwrap().flags.readonly);
    dispatch(&mut dom, "click", named(&edits, "edit-b"), "");
    assert_eq!(sheet.cell("a").view().unwrap().text, "9");
    assert!(sheet.cell("b").view().unwrap().is_source);
}

/// Unknown and incompatible bindings stay visible as structured configuration errors.
#[test]
fn unknown_or_incompatible_binding_reports_error() {
    /// Renders invalid configuration without panicking or silently omitting controls.
    fn invalid() -> Element {
        let observed: Rc<RefCell<Observed>> = consume_context();
        let sheet = use_sheet(
            factory(),
            "controls/v1".into(),
            EventHandler::new(move |e: SheetEvent| observed.borrow_mut().outcomes.push(e.outcome)),
        )
        .unwrap();
        rsx! {
            EditText { bind: sheet.cell("missing") }
            CheckBox { bind: sheet.cell("text") }
            EditNumber { bind: sheet.cell("flag") }
            EditSlider { bind: sheet.cell("text") }
        }
    }
    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(invalid);
    dom.provide_root_context(observed.clone());
    dom.rebuild_in_place();
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert_eq!(html.matches("role=\"alert\"").count(), 4, "{html}");
    assert_eq!(observed.borrow().outcomes.len(), 4);
    assert!(matches!(
        observed.borrow().outcomes[0],
        HostOutcome::Rejected(HostRejection::Binding(_))
    ));
}

/// Old event handlers cannot write into a same-key replacement.
#[test]
fn retired_binding_cannot_write_replacement() {
    let (mut dom, observed, edits) = mounted();
    let sheet = observed.borrow().sheet.unwrap();
    sheet.replace(factory(), "controls/v1".into()).unwrap();
    dispatch(&mut dom, "click", named(&edits, "edit-a"), "");
    assert_eq!(sheet.cell("a").view().unwrap().text, "1");
    assert!(matches!(
        observed.borrow().outcomes.last().unwrap(),
        HostOutcome::Rejected(HostRejection::Binding(BindingError::RetiredInstance))
    ));
}

/// Rejected Boolean edits restore through a callback after the owner borrow ends.
#[test]
fn host_callback_runs_after_model_borrow_is_released() {
    let (mut dom, observed, edits) = mounted();
    let checkbox = edits
        .edits
        .iter()
        .find_map(|mutation| match mutation {
            Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
            _ => None,
        })
        .unwrap();
    dispatch(&mut dom, "click", checkbox, "");
    assert!(observed.borrow().restored);
    assert_eq!(
        observed
            .borrow()
            .sheet
            .unwrap()
            .cell("flag")
            .view()
            .unwrap()
            .text,
        "true"
    );
}

/// Independent roots use distinct control IDs and keep drafts and callbacks isolated.
#[test]
fn independent_roots_isolate_controls_and_outcomes() {
    let (mut left, left_seen, left_edits) = mounted();
    let (right, right_seen, _) = mounted();
    let left_html = dioxus_ssr::render(&left);
    let right_html = dioxus_ssr::render(&right);
    let left_ids: Vec<_> = left_html
        .split(" id=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
        .filter(|id| id.starts_with("adam-cell-"))
        .collect();
    let right_ids: Vec<_> = right_html
        .split(" id=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
        .filter(|id| id.starts_with("adam-cell-"))
        .collect();
    assert!(!left_ids.is_empty());
    assert!(left_ids.iter().all(|id| !right_ids.contains(id)));
    let input = left_edits
        .edits
        .iter()
        .find_map(|m| match m {
            Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
            _ => None,
        })
        .unwrap();
    dispatch(&mut left, "input", input, "left only");
    assert_eq!(left_seen.borrow().outcomes.len(), 1);
    assert!(right_seen.borrow().outcomes.is_empty());
    assert_eq!(
        right_seen
            .borrow()
            .sheet
            .unwrap()
            .cell("text")
            .view()
            .unwrap()
            .text,
        "001"
    );
}

/// Numeric adapters preserve exact values, typed validation, and bounded nudges.
#[tokio::test]
async fn bound_numeric_and_slider_keep_exact_types_and_host_filtering() {
    use crate::components::tests::{NodeDocument, arrow_up, input, settle};
    use dioxus::document::Document;
    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(host);
    dom.provide_root_context(observed.clone());
    dom.provide_root_context(Rc::new(NodeDocument::default()) as Rc<dyn Document>);
    let edits = dom.rebuild_to_vec();
    let sources: Vec<_> = edits
        .edits
        .iter()
        .filter_map(|m| match m {
            Mutation::SetAttribute {
                name: "data-adam-number-source",
                value: dioxus::core::AttributeValue::Text(token),
                ..
            } => Some(ElementId(token.parse().unwrap())),
            _ => None,
        })
        .collect();
    assert!(sources.len() >= 2);
    settle(&mut dom).await;
    observed.borrow_mut().outcomes.clear();
    input(&mut dom, sources[0], "17");
    settle(&mut dom).await;
    let sheet = observed.borrow().sheet.unwrap();
    assert_eq!(sheet.cell("b").view().unwrap().text, "17");
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
    input(&mut dom, sources[1], "9007199254740995");
    settle(&mut dom).await;
    assert_eq!(
        sheet.cell("ranged").view().unwrap().text,
        "9007199254740994"
    );
    sheet
        .with_accepted_state(|state| {
            assert_eq!(state.transactions[1][0].canonical, "9007199254740995",)
        })
        .unwrap();
    arrow_up(&mut dom, sources[1]);
    settle(&mut dom).await;
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        2
    );
    assert_eq!(observed.borrow().outcomes.len(), 2);
    assert_eq!(
        sheet.cell("ranged").view().unwrap().text,
        "9007199254740994"
    );
}

/// Concise bindings use their stable cell names as accessible labels.
#[test]
fn omitted_labels_use_binding_keys() {
    /// Mounts controls without repeated label plumbing.
    fn app() -> Element {
        let sheet = use_sheet(factory(), "controls/v1".into(), EventHandler::new(|_| {})).unwrap();
        rsx! {
            EditText { bind: sheet.cell("text") }
            CheckBox { bind: sheet.cell("flag") }
            EditNumber { bind: sheet.cell("a") }
            EditSlider { bind: sheet.cell("ranged") }
        }
    }
    let mut dom = VirtualDom::new(app);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains(">text</label>"), "{html}");
    assert!(html.contains("aria-label=\"flag\""), "{html}");
    assert!(html.contains(">a</label>"), "{html}");
    assert!(html.contains(">ranged</label>"), "{html}");
}

/// Inspector publication retains invalid contributors, forced values and output relevance.
#[tokio::test]
async fn inspector_preserves_output_and_invalid_contributor_contracts() {
    use crate::components::tests::{NodeDocument, settle};
    use dioxus::document::Document;

    /// Mounts a forced filter violation feeding an output without requirements.
    fn app() -> Element {
        let observed: Rc<RefCell<Observed>> = consume_context();
        let sheet = use_sheet(
            Rc::new(|| {
                crate::build_sheet(
                    "sheet flags {
                        cell driver: i32 = -3;
                        cell bounded: i32 = 0 filter 0..=10;
                        cell unused: i32 = 1;
                        relationship { bounded := driver; }
                        out result := bounded;
                    }",
                    "flags.adm2",
                )
            }),
            "flags/v1".into(),
            EventHandler::new(|_| {}),
        )
        .unwrap();
        observed.borrow_mut().sheet = Some(sheet);
        rsx! { SheetInspector { sheet } }
    }

    /// Returns an actual rendered input identified by its accessible label.
    ///
    /// - Complexity: O(rendered HTML + label bytes) time and O(label + ID bytes) space.
    fn input_for<'a>(html: &'a str, label: &str) -> &'a str {
        let marker = format!(">{label}</label>");
        let before = html.split_once(&marker).unwrap().0;
        let id = before
            .rsplit_once("for=\"")
            .unwrap()
            .1
            .split('"')
            .next()
            .unwrap();
        let attribute = format!("id=\"{id}\"");
        html.split("<input")
            .skip(1)
            .map(|part| part.split('>').next().unwrap())
            .find(|tag| tag.contains(&attribute))
            .unwrap()
    }

    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(observed.clone());
    dom.provide_root_context(Rc::new(NodeDocument::default()) as Rc<dyn Document>);
    dom.rebuild_in_place();
    settle(&mut dom).await;
    let sheet = observed.borrow().sheet.unwrap();
    let driver = sheet.cell("driver").view().unwrap();
    assert!(driver.flags.relevant && driver.flags.invalid);
    assert!(!driver.flags.disabled && !driver.flags.forced);
    let bounded = sheet.cell("bounded").view().unwrap();
    assert!(bounded.flags.forced && bounded.flags.disabled && bounded.flags.invalid);
    assert!(!bounded.flags.readonly);
    let result = sheet.cell("result").view().unwrap();
    assert!(result.flags.readonly && result.flags.forced);
    assert!(!sheet.cell("unused").view().unwrap().flags.relevant);
    let html = dioxus_ssr::render(&dom);
    assert!(input_for(&html, "driver").contains("aria-invalid=\"true\""));
    assert!(!input_for(&html, "driver").contains("disabled"));
    assert!(input_for(&html, "bounded").contains("disabled"));
    assert!(input_for(&html, "bounded").contains("aria-invalid=\"true\""));
    assert!(input_for(&html, "result").contains("readonly"));
    assert!(!input_for(&html, "result").contains("disabled"));
    assert!(input_for(&html, "unused").contains("disabled"));
    sheet.cell("driver").submit("5".into());
    settle(&mut dom).await;
    assert!(!sheet.cell("driver").view().unwrap().flags.invalid);
    assert!(!sheet.cell("bounded").view().unwrap().flags.invalid);
    let html = dioxus_ssr::render(&dom);
    assert!(input_for(&html, "driver").contains("aria-invalid=\"false\""));
    assert!(input_for(&html, "bounded").contains("aria-invalid=\"false\""));
}

/// Real conditional inputs remain relevant after strength-changing accepted edits.
#[test]
fn conditional_relevance_survives_binding_edits() {
    /// Mounts the complete image-resize fixture through the reactive owner.
    fn app() -> Element {
        let observed: Rc<RefCell<Observed>> = consume_context();
        let sheet = use_sheet(
            Rc::new(|| {
                crate::build_sheet(
                    include_str!("../../../begin/examples/image_resize.adm2"),
                    "image_resize.adm2",
                )
            }),
            "image/v1".into(),
            EventHandler::new(|_| {}),
        )
        .unwrap();
        observed.borrow_mut().sheet = Some(sheet);
        rsx! { SheetInspector { sheet } }
    }
    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(observed.clone());
    dom.rebuild_in_place();
    let sheet = observed.borrow().sheet.unwrap();
    for width in ["1600", "1920"] {
        sheet.cell("width_pixels").submit(width.into());
        dom.render_immediate_to_vec();
        for key in [
            "width_pixels",
            "width_pixels_percent",
            "width_inches",
            "resolution",
            "original_width_pixels",
            "resample",
            "constrain",
            "auto_quality",
            "screen_lpi",
        ] {
            assert!(sheet.cell(key).view().unwrap().flags.relevant, "{key}");
        }
    }
}

/// False output requirements reach the host without publishing rejected candidate views.
#[test]
fn output_requirement_rejection_preserves_accepted_views_and_authored_diagnostics() {
    /// Mounts an editable contributor and an output with named and unnamed requirements.
    fn app() -> Element {
        let observed: Rc<RefCell<Observed>> = consume_context();
        let callbacks = observed.clone();
        let sheet = use_sheet(
            Rc::new(|| {
                crate::build_sheet(
                    "sheet validation {
                        cell amount: i32 = 1;
                        out total := amount require {
                            @positive total > 0;
                            total != 0;
                        };
                    }",
                    "authored-requirements.adm2",
                )
            }),
            "validation/v1".into(),
            EventHandler::new(move |event: SheetEvent| {
                if matches!(event.outcome, HostOutcome::Rejected(_)) {
                    assert_eq!(
                        event.sheet.inspect(|_, _| panic!("rejected model exposed")),
                        Err(BindingError::Unavailable)
                    );
                }
                callbacks.borrow_mut().outcomes.push(event.outcome);
            }),
        )
        .unwrap();
        observed.borrow_mut().sheet = Some(sheet);
        let amount = sheet.cell("amount");
        rsx! {
            button { id: "reject-output".to_string(),
                onclick: move |_| amount.submit("0".into()), "Reject"
            }
            SheetInspector { sheet }
        }
    }

    let observed = Rc::new(RefCell::new(Observed::default()));
    let mut dom = VirtualDom::new(app);
    dom.provide_root_context(observed.clone());
    let edits = dom.rebuild_to_vec();
    let sheet = observed.borrow().sheet.unwrap();
    sheet.cell("amount").submit("2".into());
    dom.render_immediate_to_vec();
    let accepted = sheet.views().unwrap();
    let old_amount = sheet.cell("amount");
    let old_total = sheet.cell("total");
    assert_eq!(old_amount.view().unwrap().text, "2");
    assert_eq!(old_total.view().unwrap().text, "2");
    assert!(old_total.view().unwrap().flags.readonly);
    assert!(
        accepted
            .iter()
            .all(|v| !v.flags.invalid && v.issues.is_empty())
    );
    dispatch(&mut dom, "click", named(&edits, "reject-output"), "");
    let state = observed.borrow();
    let HostOutcome::Rejected(HostRejection::Validation(issues)) = &state.outcomes[1] else {
        panic!("false requirements must report typed validation rejection");
    };
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].key, "total");
    assert_eq!(issues[0].requirement.as_deref(), Some("positive"));
    assert_eq!(issues[1].key, "total");
    assert_eq!(issues[1].requirement, None);
    assert!(issues[0].message.contains("positive"));
    for issue in issues {
        assert_eq!(issue.labels.len(), 1);
        assert!(!issue.labels[0].label.is_empty());
        let location = issue.labels[0].location.as_ref().unwrap();
        assert!(location.source.text().contains("@positive total > 0"));
        assert!(location.source.text().contains("total != 0"));
    }
    drop(state);
    assert_eq!(sheet.views(), Err(BindingError::Unavailable));
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
    assert!(matches!(
        old_amount.view(),
        Err(BindingError::RetiredInstance)
    ));
    assert!(dioxus_ssr::render(&dom).contains("role=\"alert\""));
    assert!(accepted.iter().all(|v| v.text == "2" && !v.flags.invalid));
    sheet.restore_accepted().unwrap();
    dom.render_immediate_to_vec();
    let restored = sheet.views().unwrap();
    assert!(
        accepted
            .iter()
            .zip(&restored)
            .all(|(a, b)| Rc::ptr_eq(a, b))
    );
    assert_eq!(sheet.cell("amount").view().unwrap().text, "2");
    assert_eq!(sheet.cell("total").view().unwrap().text, "2");
    assert!(
        restored
            .iter()
            .all(|v| !v.flags.invalid && v.issues.is_empty())
    );
    assert_eq!(
        sheet.with_accepted_state(|s| s.transactions.len()).unwrap(),
        1
    );
}

/// Replacing a model remounts standalone numeric controls, not just Inspector rows.
#[test]
fn replacement_retires_standalone_numeric_instances() {
    let (mut dom, observed, _) = mounted();
    let before = dioxus_ssr::render(&dom);
    let old_source = before
        .split("data-adam-number-source=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    observed
        .borrow()
        .sheet
        .unwrap()
        .replace(factory(), "controls/v1".into())
        .unwrap();
    dom.render_immediate_to_vec();
    let after = dioxus_ssr::render(&dom);
    let new_source = after
        .split("data-adam-number-source=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_ne!(
        old_source, new_source,
        "standalone numeric source must retire on replacement"
    );
}

/// Renders the inspector in card presentation for server-side assertions.
fn card_host() -> Element {
    let sheet = use_sheet(factory(), "controls/v1".into(), EventHandler::new(|_| {})).unwrap();
    rsx! { SheetInspector { sheet, presentation: InspectorPresentation::Card } }
}

/// Renders the inspector in the default panel presentation for server-side assertions.
fn panel_host() -> Element {
    let sheet = use_sheet(factory(), "controls/v1".into(), EventHandler::new(|_| {})).unwrap();
    rsx! { SheetInspector { sheet } }
}

/// The card presentation wraps the rows in a card and drops the "Cells" header.
#[test]
fn card_presentation_omits_header() {
    let mut dom = VirtualDom::new(card_host);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("data-slot=\"card\""), "{html}");
    assert!(!html.contains("<h2"), "{html}");
    assert!(html.contains("aria-label=\"flag\""), "{html}");
    for key in ["text", "a", "b", "ranged", "unused"] {
        assert!(html.contains(&format!(">{key}</label>")), "{key}: {html}");
    }
    assert!(html.contains("result:"), "result: {html}");
}

/// The default panel presentation keeps the "Cells" header and renders no card.
#[test]
fn panel_presentation_keeps_header() {
    let mut dom = VirtualDom::new(panel_host);
    dom.rebuild_in_place();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Cells"), "{html}");
    assert!(!html.contains("data-slot=\"card\""), "{html}");
}
