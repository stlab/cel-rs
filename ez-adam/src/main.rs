//! Entry point for the `ez-adam` desktop editor.

use dioxus::prelude::*;

/// Launches the desktop UI.
fn main() {
    #[allow(deprecated)]
    LaunchBuilder::new()
        .with_cfg(desktop! {
            dioxus::desktop::Config::new().with_menu(ez_adam::ui::build_menu())
        })
        .launch(ez_adam::ui::App);
}
