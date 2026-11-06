//! Delivers immutable official styles without a bundler or deployment prefix.
use dioxus::prelude::*;

/// Contains the official styles transformed with public CSS-module exports.
///
/// Publish this exact stylesheet when a host stages shared control assets.
///
/// # Examples
/// ```
/// assert!(!adam_web_ui::components::CONTROL_CSS.is_empty());
/// ```
pub const CONTROL_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/adam-controls.css"));

/// Returns an idempotent document-owned stylesheet installation script.
///
/// The stylesheet survives independent root removal and requires no asset fetch.
/// - Complexity: O(stylesheet bytes) time and space.
///
/// # Examples
/// ```
/// let script = adam_web_ui::components::style_delivery_script();
/// assert!(script.contains("createElement"));
/// ```
pub fn style_delivery_script() -> String {
    format!(
        "/* adam-control-styles */ if (!document.getElementById('stlab-adam-controls-v1')) {{ const style = document.createElement('style'); style.id = 'stlab-adam-controls-v1'; style.textContent = {}; document.head.appendChild(style); }} return null;",
        serde_json::to_string(CONTROL_CSS).expect("static CSS encodes as JSON")
    )
}

/// Installs document-owned styles once per component scope and reports delivery failure.
///
/// - Complexity: O(stylesheet bytes) delivery on initialization; O(1) thereafter.
pub(crate) fn use_control_styles() {
    use_hook(|| {
        if let Some(document) = try_consume_context::<std::rc::Rc<dyn dioxus::document::Document>>()
        {
            let evaluation = document.eval(style_delivery_script());
            spawn(async move {
                if let Err(error) = evaluation.await {
                    crate::diagnostics::report_error(&format!(
                        "Control stylesheet delivery failed: {error}"
                    ));
                }
            });
        }
    });
}
