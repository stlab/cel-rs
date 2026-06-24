//! Entry point for the `begin` property model development environment.
mod bridge;

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
