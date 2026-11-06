//! Model-independent control behavior.
//!
//! Use [`number`] to normalize localized input before submitting an edit.
//! Styled controls install their embedded CSS through the renderer's Dioxus
//! `Document` context. Missing context reports a delivery failure on the platform
//! diagnostic channel; the library does not assume an alternate stylesheet.
pub mod button;
pub mod checkbox;
pub mod input;
pub mod label;
pub mod number;
mod numeric_editor;
mod numeric_events;
pub mod separator;
pub mod slider;
mod styles;
pub mod switch;
mod theme;

pub use button::{Button, ButtonProps, ButtonSize, ButtonVariant};
pub use checkbox::Checkbox;
/// Provides the checked state accepted by [`Checkbox`].
///
/// # Examples
///
/// ```
/// use adam_web_ui::components::CheckboxState;
/// let checked = CheckboxState::Checked;
/// ```
pub use dioxus_primitives::checkbox::CheckboxState;
pub use input::{Input, InputProps};
pub use label::Label;
pub use numeric_editor::{NumericEditor, NumericEditorProps, NumericSlider};
pub use separator::Separator;
pub use slider::{RangeSlider, Slider};
pub use styles::{CONTROL_CSS, style_delivery_script};
pub use switch::Switch;
pub use theme::Theme;

#[cfg(test)]
pub(crate) mod tests;
