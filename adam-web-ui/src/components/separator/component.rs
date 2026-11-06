//! Styled upstream separator implementation; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::separator::{self, SeparatorProps};
use dioxus_primitives::{dioxus_attributes::attributes, merge_attributes};

include!(concat!(env!("OUT_DIR"), "/separator-styles.rs"));

/// Renders the official semantic or decorative separator.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::separator::Separator;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Separator {} } }
/// ```
#[component]
pub fn Separator(props: SeparatorProps) -> Element {
    crate::components::styles::use_control_styles();
    let base = attributes!(div {
        class: Styles::dx_separator,
    });
    let merged = merge_attributes(vec![base, props.attributes]);

    rsx! {
        separator::Separator {
            horizontal: props.horizontal,
            decorative: props.decorative,
            attributes: merged,
            {props.children}
        }
    }
}
