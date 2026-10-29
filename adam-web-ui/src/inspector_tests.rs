//! Exercises Inspector controls, flags, and source ownership through real Dioxus components.

use super::*;
use dioxus::core::Mutation;
use std::cell::RefCell;

/// Exposes the current component-owned sheet and metadata for real event observations.
type Observer = Rc<RefCell<Option<(Signal<ParsedSheet>, Signal<Labels>)>>>;

/// Builds controls of every supported kind with a named failing output requirement.
fn host() -> Element {
    let source = r#"sheet controls {
        cell text: String = "001";
        cell flag: bool = true;
        cell count: i64 = 10i64;
        cell ranged: i64 = 20i64 filter 0i64..=100i64;
        out result := (text, flag, count, ranged) require { @limit count < 5i64; };
    }"#;
    let (parsed, labels) = use_hook(|| {
        let outcome =
            crate::build_sheet(source, "same.adm2", &annotate_snippets::Renderer::plain());
        let (parsed, labels) = outcome.sheet_labels.expect("valid control fixture");
        (Signal::new(parsed), Signal::new(labels))
    });
    let observed: Observer = consume_context();
    *observed.borrow_mut() = Some((parsed, labels));
    let source_text = use_memo(move || source.to_string());
    let source_name = use_memo(|| "same.adm2".to_string());
    rsx! { SheetInspector { parsed, labels, source_text, source_name } }
}

/// Extracts actual rendered control IDs without assuming how the namespace is encoded.
///
/// - Complexity: O(rendered HTML bytes) time and space.
fn ids(html: &str) -> HashSet<String> {
    html.split(" id=\"")
        .skip(1)
        .map(|part| part.split('"').next().unwrap().to_string())
        .collect()
}

/// Rows choose Boolean, text, integer, and ranged controls, preserve flags, and retire replaced owners.
#[test]
fn rendered_controls_flags_writes_and_source_replacement() {
    let observed: Observer = Rc::new(RefCell::new(None));
    let mut dom = VirtualDom::new(host);
    dom.provide_root_context(observed.clone());
    let mutations = dom.rebuild_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert_eq!(html.matches("<sp-checkbox").count(), 1);
    assert_eq!(html.matches("<sp-number-field").count(), 1);
    assert_eq!(html.matches("<sp-slider").count(), 1);
    assert_eq!(html.matches("<sp-textfield").count(), 2);
    assert!(html.contains("value=\"001\"") && html.contains("value=\"10\""));
    assert!(html.contains("invalid=\"true\"") && html.contains("disabled=\"true\""));
    let click = mutations
        .edits
        .into_iter()
        .find_map(|mutation| match mutation {
            Mutation::NewEventListener { name, id } if name == "click" => Some(id),
            _ => None,
        })
        .unwrap();
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
        click,
    );
    let (mut parsed, mut labels) = observed.borrow().unwrap();
    assert!(
        !*parsed
            .peek()
            .read::<bool>(parsed.peek().cell_names["flag"].0)
            .unwrap()
    );
    let replacement = crate::build_sheet(
        "sheet new { cell fresh: i64 = 42i64; }",
        "same.adm2",
        &annotate_snippets::Renderer::plain(),
    );
    let (new_parsed, new_labels) = replacement.sheet_labels.unwrap();
    parsed.set(new_parsed);
    labels.set(new_labels);
    dom.render_immediate_to_vec();
    let after = dioxus_ssr::render(&dom);
    assert!(after.contains("fresh") && after.contains("value=\"42\""));
    assert!(
        ids(&html).is_disjoint(&ids(&after)),
        "replaced source must retire DOM control identity"
    );
}
