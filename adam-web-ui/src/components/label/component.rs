//! Styled upstream label implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::label::{self, LabelProps};
#[css_module("/src/components/label/style.css")]
/// Contains the generated stylesheet and class identifiers.
struct Styles;

/// Renders the official label associated with the caller's control identifier.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::label::Label;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Label { html_for: "name", "Name" } } }
/// ```
#[component]
pub fn Label(props: LabelProps) -> Element {
    rsx! {
        label::Label {
            class: Styles::dx_label,
            html_for: props.html_for,
            attributes: props.attributes,
            {props.children}
        }
    }
}
