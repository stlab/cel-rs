//! The `ez-adam` desktop UI: a canvas for placing and connecting cells,
//! relationship groups, and conditional groups, a toolbar selecting the
//! active interaction tool, and a context-sensitive side panel.

mod app;
mod canvas;
mod file_io;
mod side_panel;
mod toolbar;

pub use app::App;
