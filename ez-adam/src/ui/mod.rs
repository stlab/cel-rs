//! The `ez-adam` UI: a canvas for placing and connecting cells,
//! relationship groups, and conditional groups, a toolbar selecting the
//! active interaction tool, and a context-sensitive side panel. Native menus
//! and file dialogs are available only with the `desktop` feature.

mod app;
mod canvas;
#[cfg(feature = "desktop")]
mod file_io;
mod history;
#[cfg(feature = "desktop")]
mod menu;
mod side_panel;
mod toolbar;

pub use app::App;
#[cfg(feature = "desktop")]
pub use menu::build_menu;
