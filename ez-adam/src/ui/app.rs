//! The top-level component: owns all UI state, composes the toolbar,
//! canvas, and side panel.

use dioxus::prelude::*;

/// The application's root component.
#[component]
pub fn App() -> Element {
    rsx! {
        div { "ez-adam" }
    }
}
