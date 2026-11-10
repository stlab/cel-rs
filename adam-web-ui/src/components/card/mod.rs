//! Groups related content into a bordered card surface.
//!
//! Compose [`Card`] with [`CardHeader`], [`CardTitle`], [`CardDescription`],
//! [`CardAction`], [`CardContent`], and [`CardFooter`] slots. Each slot renders
//! its upstream `data-slot` attribute and a scoped class so callers can target
//! layout from either Rust or CSS without reaching into the control's markup.
//!
//! # Examples
//! ```
//! use adam_web_ui::components::card::{Card, CardContent, CardHeader, CardTitle};
//! use dioxus::prelude::*;
//! fn app() -> Element {
//!     rsx! {
//!         Card {
//!             CardHeader { CardTitle { "Inspector" } }
//!             CardContent { "Body" }
//!         }
//!     }
//! }
//! ```
mod component;
pub use component::*;
