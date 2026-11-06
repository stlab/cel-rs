//! Styled upstream switch implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::switch::{self, SwitchProps};

#[css_module("/src/components/switch/style.css")]
/// Contains the generated stylesheet and class identifiers.
struct Styles;

/// Renders the official switch with controlled state and accessible interaction.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::switch::Switch;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Switch { aria_label: "Enabled" } } }
/// ```
#[component]
pub fn Switch(props: SwitchProps) -> Element {
    rsx! {
        switch::Switch {
            class: Styles::dx_switch,
            checked: props.checked,
            default_checked: props.default_checked,
            disabled: props.disabled,
            required: props.required,
            name: props.name,
            value: props.value,
            on_checked_change: props.on_checked_change,
            attributes: props.attributes,
            switch::SwitchThumb { class: Styles::dx_switch_thumb }
        }
    }
}
