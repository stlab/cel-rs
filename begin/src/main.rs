//! Entry point for the `begin` property model development environment.
mod app;
mod bridge;
mod demo_source;
mod graph_view;
mod inspector;
mod spectrum;

use dioxus::prelude::*;

#[allow(deprecated)]
fn main() {
    LaunchBuilder::new()
        .with_cfg(desktop! {
            dioxus::desktop::Config::new().with_icon(
                dioxus::desktop::icon_from_memory(include_bytes!("../assets/icon-512.png"))
                    .expect("bundled app icon is a valid image"),
            )
        })
        .launch(app::App);
}
