//! Entry point for the `begin` property model development environment.
mod app;
mod bridge;
mod graph_view;
mod inspector;

fn main() {
    dioxus::launch(app::App);
}
