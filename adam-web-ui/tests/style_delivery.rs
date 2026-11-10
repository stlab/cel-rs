//! Verifies self-contained shared styles independently of asset bundlers.
use adam_web_ui::components::{CONTROL_CSS, style_delivery_script};

/// A renderer without a document provider reports missing stylesheet delivery.
#[test]
fn absent_document_reports_style_delivery_failure() {
    const CHILD: &str = "ADAM_STYLE_CONTEXT_TEST";
    if std::env::var_os(CHILD).is_some() {
        use dioxus::prelude::*;
        /// Renders the real shared Theme without adding a document provider.
        fn root() -> Element {
            rsx! { adam_web_ui::components::Theme { "unstyled context check" } }
        }
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "absent_document_reports_style_delivery_failure",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Control stylesheet delivery failed: Document context unavailable"),
        "missing context was silent: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Shared styles retain scoped tokens and contain no deployment-relative resources.
#[test]
fn shared_styles_are_self_contained_and_scoped() {
    assert!(CONTROL_CSS.contains(".adam_theme_adam-theme"));
    assert!(CONTROL_CSS.contains(".adam_checkbox_dx-checkbox"));
    assert!(CONTROL_CSS.contains(".adam_slider_dx-slider"));
    assert!(!CONTROL_CSS.contains("url("));
    assert!(!CONTROL_CSS.contains("This should be replaced"));
}

/// Repeated installation and independent root removal preserve one document stylesheet.
#[test]
fn delivery_is_document_owned_and_deduplicated() {
    let script = style_delivery_script();
    let program = format!(
        r#"
const styles = new Map();
let insertions = 0;
global.document = {{
  getElementById: id => styles.get(id),
  createElement: () => ({{}}),
  head: {{ appendChild: element => {{ styles.set(element.id, element); ++insertions; }} }}
}};
const install = new Function({});
if (install() !== null) throw Error("non-serializable delivery acknowledgment");
install();
// Root removal does not own or remove the document's stylesheet.
install();
if (insertions !== 1 || styles.size !== 1) throw Error("duplicate or lost styles");
if (![...styles.values()][0].textContent.includes(".adam_checkbox_dx-checkbox")) throw Error("missing actual styles");
"#,
        serde_json::to_string(&script).unwrap()
    );
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(program)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
