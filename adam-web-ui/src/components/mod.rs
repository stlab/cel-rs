//! Model-independent control behavior.
//!
//! Use [`number`] to normalize localized input before submitting an edit.
pub mod button;
pub mod checkbox;
pub mod input;
pub mod label;
pub mod number;
mod numeric_editor;
mod numeric_events;
pub mod separator;
pub mod slider;
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
pub use switch::Switch;
pub use theme::Theme;

#[cfg(test)]
pub(crate) mod tests;
