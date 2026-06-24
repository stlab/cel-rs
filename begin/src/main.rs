//! Entry point for the `begin` property model development environment.
mod bridge;
mod graph_view;

use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        div { "begin" }
    }
}
