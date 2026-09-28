//! Entry point for the `ez-adam` editor.

#[cfg(any(feature = "desktop", feature = "web"))]
use dioxus::prelude::*;

/// Launches the desktop UI when the `desktop` feature is enabled.
#[cfg(feature = "desktop")]
fn main() {
    #[allow(deprecated)]
    LaunchBuilder::new()
        .with_cfg(desktop! {
            dioxus::desktop::Config::new().with_menu(ez_adam::ui::build_menu())
        })
        .launch(ez_adam::ui::App);
}

/// Launches the browser UI when only the `web` feature is enabled.
#[cfg(all(not(feature = "desktop"), feature = "web"))]
fn main() {
    LaunchBuilder::new().launch(ez_adam::ui::App);
}

/// Provides no application entry point without a renderer feature.
#[cfg(not(any(feature = "desktop", feature = "web")))]
fn main() {}
