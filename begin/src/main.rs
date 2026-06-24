//! Entry point for the `begin` property model development environment.
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
