//! Reusable Dioxus controls and UI for browsing and editing a live `adam_rs::Sheet`.
//!
//! [`components`] supplies model-independent official Dioxus controls and numeric
//! editors. Wrap each root in [`components::Theme`]. [`binding::use_sheet`] owns
//! one reactive model and supplies bindings to controls and [`SheetInspector`].
//!
//! Components render as DOM in a browser or embedded webview. The library does
//! not select a Dioxus renderer: use it from a desktop app (`begin`), a `dioxus/web`
//! app, or a `wasm-bindgen` embed without a full application shell.

pub mod binding;
pub mod build;
pub mod components;
pub mod diagnostics;
pub mod graph;
pub mod input;
mod inspector;
pub mod labels;

pub use build::{
    BuildError, BuiltSheet, RuntimeFailure, ValidationIssue, build_sheet, initialize_sheet,
};
pub use graph::{
    GraphData, GraphDrive, GraphView, LinkData, LinkKind, NodeData, NodeKind, graph_drive_script,
    to_graph_data,
};
pub use inspector::SheetInspector;
pub use labels::{
    CaptureFn, CellMeta, ExactRangeFn, Labels, PrepareFn, PreparedWrite, Renderer, ScalarCodec,
    format_adam_error, format_module_diagnostic, format_parse_error, format_rounded,
    labels_from_cell_names,
};
