//! Shared control styling.
//!
//! Wrap each independent Dioxus root in [`Theme`].
use dioxus::prelude::*;

#[css_module("/assets/dx-components-theme.css")]
/// Contains document-deduplicated theme assets and the scoped root class.
struct Styles;

/// Installs reusable component tokens and wraps children in a theme scope.
///
/// The immutable stylesheet is a library asset, not a consumer-relative URL.
/// Dioxus's CSS module loader deduplicates the asset across independent roots;
/// unmounting one wrapper does not remove another root's styles.
///
/// # Examples
/// ```
/// use adam_web_ui::components::Theme;
/// use dioxus::prelude::*;
/// fn app() -> Element { rsx! { Theme { p { "Content" } } } }
/// ```
#[component]
pub fn Theme(children: Element) -> Element {
    rsx! {
        div { class: "{Styles::adam_theme} adam-theme", {children} }
    }
}
