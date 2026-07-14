//! Entry point for the `begin` property model development environment.
mod app;
mod bridge;
mod demo_source;
mod graph_view;
mod inspector;
mod spectrum;

fn main() {
    dioxus::launch(app::App);
}
