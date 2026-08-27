//! Reusable Dioxus UI for browsing and editing a live `adam_rs::Sheet`, built on Spectrum Web
//! Components. Named for the web rendering stack it targets — Dioxus + Spectrum Web
//! Components render as DOM whether hosted in a real browser or an embedded webview — not
//! tied to any one Dioxus renderer feature, so it's usable from a desktop app (`begin`), a
//! `dioxus/web` app, or a plain `wasm-bindgen` embed with no full app shell around it.

pub mod build;
pub mod diagnostics;
pub mod labels;
pub mod spectrum;

pub use build::{BuildOutcome, build_sheet};
pub use labels::{
    CellMeta, Labels, WriteStrFn, format_adam_error, format_rounded, labels_from_cell_names,
};
