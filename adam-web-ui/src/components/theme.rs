//! Shared control styling.
//!
//! Wrap each independent Dioxus root in [`Theme`].
use dioxus::prelude::*;

include!(concat!(env!("OUT_DIR"), "/theme-styles.rs"));

/// Installs reusable component tokens and wraps children in a theme scope.
///
/// The immutable stylesheet is embedded, not a consumer-relative URL.
/// The document-owned installer deduplicates styles across independent roots;
/// unmounting one wrapper does not remove another root's styles.
/// The renderer supplies Dioxus's `Document` context; missing context reports
/// a stylesheet-delivery failure on the platform diagnostic channel.
///
/// # Examples
/// ```
/// use adam_web_ui::components::Theme;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Theme { p { "Content" } } } }
/// ```
#[component]
pub fn Theme(children: Element) -> Element {
    super::styles::use_control_styles();
    rsx! {
        div { class: "{Styles::adam_theme} adam-theme", {children} }
    }
}
