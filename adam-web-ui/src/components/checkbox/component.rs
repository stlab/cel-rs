//! Styled upstream checkbox implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_icons::lucide::Check;
use dioxus_primitives::checkbox::{self, CheckboxProps};

include!(concat!(env!("OUT_DIR"), "/checkbox-styles.rs"));

/// Renders the official checkbox with controlled state and accessible interaction.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::checkbox::Checkbox;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Checkbox { aria_label: "Enabled" } } }
/// ```
#[component]
pub fn Checkbox(props: CheckboxProps) -> Element {
    crate::components::styles::use_control_styles();
    rsx! {
        checkbox::Checkbox {
            class: Styles::dx_checkbox,
            checked: props.checked,
            default_checked: props.default_checked,
            required: props.required,
            disabled: props.disabled,
            name: props.name,
            value: props.value,
            on_checked_change: props.on_checked_change,
            attributes: props.attributes,
            checkbox::CheckboxIndicator { class: Styles::dx_checkbox_indicator,
                Check { size: "1rem" }
            }
        }
    }
}
