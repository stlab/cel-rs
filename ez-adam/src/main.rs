//! Entry point for the `ez-adam` desktop editor.

use dioxus::prelude::*;

/// Launches the desktop UI.
fn main() {
    #[allow(deprecated)]
    LaunchBuilder::new()
        .with_cfg(desktop! {
            dioxus::desktop::Config::new()
        })
        .launch(ez_adam::ui::App);
}
