//! Styled upstream slider implementations; see `../UPSTREAM.md`.
use dioxus::prelude::*;
use dioxus_primitives::slider::{self, RangeSliderProps, SliderProps};

include!(concat!(env!("OUT_DIR"), "/slider-styles.rs"));

/// Renders the official finite-domain slider with controlled state.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::slider::Slider;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Slider { label: Some("Zoom".to_string()) } } }
/// ```
#[component]
pub fn Slider(props: SliderProps) -> Element {
    crate::components::styles::use_control_styles();
    rsx! {
        slider::Slider {
            class: Styles::dx_slider,
            value: props.value,
            default_value: props.default_value,
            min: props.min,
            max: props.max,
            step: props.step,
            disabled: props.disabled,
            horizontal: props.horizontal,
            inverted: props.inverted,
            on_value_change: props.on_value_change,
            label: props.label,
            attributes: props.attributes,
            slider::SliderTrack { class: Styles::dx_slider_track,
                slider::SliderRange { class: Styles::dx_slider_range }
                slider::SliderThumb { class: Styles::dx_slider_thumb }
            }
        }
    }
}

/// Renders the official two-thumb range slider with controlled state.
///
/// - Complexity: O(attributes + children) render work.
///
/// # Examples
/// ```
/// use adam_web_ui::components::slider::RangeSlider;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { RangeSlider { label: Some("Range".to_string()) } } }
/// ```
#[component]
pub fn RangeSlider(props: RangeSliderProps) -> Element {
    crate::components::styles::use_control_styles();
    rsx! {
        slider::RangeSlider {
            class: Styles::dx_slider,
            value: props.value,
            default_value: props.default_value,
            min: props.min,
            max: props.max,
            step: props.step,
            disabled: props.disabled,
            horizontal: props.horizontal,
            inverted: props.inverted,
            on_value_change: props.on_value_change,
            label: props.label,
            attributes: props.attributes,
            slider::SliderTrack { class: Styles::dx_slider_track,
                slider::SliderRange { class: Styles::dx_slider_range }
                slider::SliderThumb { class: Styles::dx_slider_thumb, index: 0usize }
                slider::SliderThumb { class: Styles::dx_slider_thumb, index: 1usize }
            }
        }
    }
}
