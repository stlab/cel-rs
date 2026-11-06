//! Owns event-loop-confined models and publishes accepted cell views.
//!
//! Construct a [`SheetOwner`] with a repeatable [`ModelFactory`], submit canonical
//! [`CellEdit`] groups, and explicitly restore accepted history after rejection.
//! Enable action capture with [`SheetOwner::start_recording`], persist shared
//! [`Recording`] values with serde, and replay them on fresh compatible models.
//! [`HostOutcome::RecordingFailed`] accepts the model transaction but stops the
//! recorder; do not roll accepted state back in response to that outcome.

mod dioxus;
mod owner;
mod recording;
mod types;
mod widgets;

pub use dioxus::{CellBinding, SheetEvent, SheetHandle, use_sheet};
pub use owner::SheetOwner;
pub use recording::{CaptureError, PlaybackError, Recording};
pub use types::*;
pub use widgets::{CheckBox, EditNumber, EditSlider, EditText};

#[cfg(test)]
mod dioxus_tests;
#[cfg(test)]
mod tests;
