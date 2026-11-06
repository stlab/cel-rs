//! Verifies self-contained shared styles independently of asset bundlers.
use adam_web_ui::components::{CONTROL_CSS, style_delivery_script};

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
