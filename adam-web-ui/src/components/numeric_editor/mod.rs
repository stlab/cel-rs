//! Controlled numeric controls styled through a component-local CSS module.
//!
//! [`NumericEditor`] renders a labeled native input with a stepper frame, a
//! visually hidden help description, and a live validation status. [`NumericSlider`]
//! wraps that editor beside a range slider, placing the slider left of a
//! fixed-width number field. Pass canonical strings and handle aspirations in
//! `on_value`; the host's validation remains authoritative. Wrap controls in
//! [`crate::components::Theme`].
//!
//! The layout classes (`editor`, `controls`, `stepper`, `slider`,
//! `visually_hidden`) live in `style.css` and are compiled into scoped
//! `adam_numeric_editor_<local>` names by [`crate::components`]' build script.
//!
//! # Examples
//! ```
//! use adam_web_ui::{components::NumericEditor, input::NumberKind};
//! use dioxus::prelude::*;
//! fn app() -> Element {
//!     let value = use_signal(|| Some("12".to_string()));
//!     let range = use_signal(|| None);
//!     let flags = use_signal(|| false);
//!     rsx! { NumericEditor {
//!         id: "amount", label: "Amount", value, range, kind: NumberKind::Signed(32),
//!         step: "1", disabled: flags, readonly: flags, invalid: flags,
//!         locale: None, optional: false,
//!         on_value: |_| {}, on_error: |_| {},
//!     } }
//! }
//! ```
mod component;
pub use component::*;
