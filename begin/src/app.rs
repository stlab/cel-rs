//! Root [`App`] component and demo sheet factory.

use dioxus::prelude::*;
use property_model::{Method, Sheet};

use crate::bridge::{Labels, to_graph_data};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;

/// Builds the `a × b = c` demo sheet with three bidirectional methods.
pub fn make_demo_sheet() -> (Sheet, Labels) {
    let mut sheet = Sheet::new();
    let mut labels = Labels::new();

    let a = sheet.add_cell(2.0_f64);
    labels.add_cell::<f64>(a, "a");

    let b = sheet.add_cell(3.0_f64);
    labels.add_cell::<f64>(b, "b");

    let c = sheet.add_cell(0.0_f64);
    labels.add_cell::<f64>(c, "c");

    let rel = sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();
    labels.add_relationship(rel, "×");

    (sheet, labels)
}

/// Root component: two-panel layout with the D3 graph on the left and the Inspector on the right.
#[component]
pub fn App() -> Element {
    let (initial_sheet, initial_labels) = make_demo_sheet();
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    rsx! {
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }

        div {
            style: "display: flex; width: 100vw; height: 100vh; overflow: hidden;",
            GraphView { data: graph_data }
            Inspector { sheet, labels }
        }
    }
}
