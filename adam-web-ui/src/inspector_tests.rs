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
        cell count: i32 = 10;
        cell ranged: i32 = 20 filter 0..=100;
        cell inactive: i32 = 1;
        out result := (text, flag, count, ranged) require { @limit count < 5; };
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

/// Wide signed/unsigned cells use exact text controls even when ranged or readonly.
#[test]
fn wide_integer_controls_preserve_the_complete_type_domain() {
    /// Builds one real Inspector over the requested primitive domain.
    ///
    /// - Complexity: source construction/build/propagation plus Inspector rendering.
    fn wide_host((ty, initial): (String, String)) -> Element {
        let source = format!(
            "sheet wide {{
            source value: {ty} = {initial};
            cell bounded: {ty} = 9007199254740993{ty}
                filter 9007199254740992{ty}..=9007199254740994{ty};
            out echoed := value + (bounded - bounded);
        }}"
        );
        let (parsed, labels) = use_hook(|| {
            let outcome = crate::build_sheet(&source, "wide.adm2", &Renderer::plain());
            assert!(outcome.error.is_none(), "{:?}", outcome.error);
            let (sheet, labels) = outcome.sheet_labels.unwrap();
            (Signal::new(sheet), Signal::new(labels))
        });
        let source_text = use_memo(move || source.clone());
        let source_name = use_memo(|| "wide.adm2".to_string());
        rsx! { SheetInspector { parsed, labels, source_text, source_name } }
    }
    for (ty, initial, expected) in [
        ("i64", "9007199254740993i64", "9007199254740993"),
        ("u64", "9007199254740993u64", "9007199254740993"),
        ("i128", "9007199254740993i128", "9007199254740993"),
        ("u128", "9007199254740993u128", "9007199254740993"),
        ("i64", "-9007199254740993i64", "-9007199254740993"),
        (
            "i64",
            "-9223372036854775807i64 - 1i64",
            "-9223372036854775808",
        ),
        ("u64", "18446744073709551615u64", "18446744073709551615"),
        ("u64", "0u64", "0"),
    ] {
        let mut dom = VirtualDom::new_with_props(wide_host, (ty.to_string(), initial.to_string()));
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_eq!(html.matches("<sp-number-field").count(), 0, "{ty}: {html}");
        assert_eq!(html.matches("<sp-slider").count(), 0, "{ty}: {html}");
        assert_eq!(html.matches("<sp-textfield").count(), 3, "{ty}: {html}");
        assert_eq!(
            html.matches(&format!(r#"value="{expected}""#)).count(),
            if expected == "9007199254740993" { 3 } else { 2 }
        );
        assert_eq!(html.matches(r#"readonly="true""#).count(), 1);
        assert!(!html.contains(r#"disabled="true""#));
    }
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
    assert_eq!(html.matches("<sp-number-field").count(), 2);
    assert_eq!(html.matches("<sp-slider").count(), 1);
    assert_eq!(html.matches("<sp-textfield").count(), 2);
    assert!(html.contains("value=\"001\"") && html.contains("value=\"10\""));
    assert!(html.contains("invalid=\"true\"") && html.contains("disabled=\"true\""));
    assert!(html.contains("readonly=\"true\""));
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
