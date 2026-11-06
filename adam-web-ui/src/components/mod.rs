//! Model-independent control behavior.
//!
//! Use [`number`] to normalize localized input before submitting an edit.
pub mod button;
pub mod checkbox;
pub mod input;
pub mod label;
pub mod number;
mod numeric_editor;
pub mod separator;
pub mod slider;
pub mod switch;
mod theme;

pub use button::{Button, ButtonProps, ButtonSize, ButtonVariant};
pub use checkbox::Checkbox;
pub use input::{Input, InputProps};
pub use label::Label;
pub use numeric_editor::{NumericEditor, NumericEditorProps, NumericSlider};
pub use separator::Separator;
pub use slider::{RangeSlider, Slider};
pub use switch::Switch;
pub use theme::Theme;

#[cfg(test)]
mod tests;
