//! Root [`App`] component and demo sheet factory.

use dioxus::prelude::*;
use property_model::{Method, Sheet};

use crate::bridge::{Labels, to_graph_data};
use crate::graph_view::GraphView;
use crate::inspector::Inspector;
use crate::spectrum::SpTheme;

/// Builds the demo sheet.
///
/// Two independent bidirectional constraint systems (`a × b = c` and `d × e = f`)
/// are linked by a conditional on `p`:
/// - `p = 0`: the relationship `c = f` (bidirectional) becomes active.
/// - `p = 1`: the relationship `c = f × 2` (bidirectional) becomes active.
/// - Any other `p`: the two systems are independent.
///
/// Cells `c` and `f` are added first (lowest strength) so they are natural outputs
/// within each system. `propagate()` is called once to compute initial values;
/// `clear_changed()` resets pulse state so no cells flash on startup.
pub fn make_demo_sheet() -> (Sheet, Labels) {
    let mut sheet = Sheet::new();
    let mut labels = Labels::new();

    // c and f added first → lowest strength (natural outputs).
    let c = sheet.add_cell(0.0_f64);
    let f = sheet.add_cell(0.0_f64);

    // a, b, d, e, p added later → higher strength (natural sources).
    let a = sheet.add_cell(2.0_f64);
    let b = sheet.add_cell(3.0_f64);
    let d = sheet.add_cell(4.0_f64);
    let e = sheet.add_cell(5.0_f64);
    let p = sheet.add_cell(0_i32);

    // a × b = c (three bidirectional methods)
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([a, b], c, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([b, c], a, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([a, c], b, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    // d × e = f (three bidirectional methods)
    sheet
        .add_relationship(vec![
            Method::from_fn_2_1([d, e], f, |x: &f64, y: &f64| Ok(x * y)),
            Method::from_fn_2_1([e, f], d, |x: &f64, y: &f64| Ok(y / x)),
            Method::from_fn_2_1([d, f], e, |x: &f64, y: &f64| Ok(y / x)),
        ])
        .unwrap();

    // Branch p=0: c = f (bidirectional)
    let rel_eq = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(f, c, |v: &f64| Ok(*v)),
            Method::from_fn_1_1(c, f, |v: &f64| Ok(*v)),
        ])
        .unwrap();

    // Branch p=1: c = f × 2  ↔  f = c / 2 (bidirectional)
    let rel_double = sheet
        .add_relationship(vec![
            Method::from_fn_1_1(f, c, |v: &f64| Ok(v * 2.0)),
            Method::from_fn_1_1(c, f, |v: &f64| Ok(v / 2.0)),
        ])
        .unwrap();

    sheet
        .add_conditional(
            p,
            vec![(vec![0_i32], vec![rel_eq]), (vec![1_i32], vec![rel_double])],
            vec![],
        )
        .unwrap();

    sheet.propagate().unwrap();
    sheet.clear_changed();

    labels.add_cell::<f64>(a, "a");
    labels.add_cell::<f64>(b, "b");
    labels.add_cell::<f64>(c, "c");
    labels.add_cell::<f64>(d, "d");
    labels.add_cell::<f64>(e, "e");
    labels.add_cell::<f64>(f, "f");
    labels.add_cell::<i32>(p, "p");

    (sheet, labels)
}

/// Root component: Spectrum theme wrapper, two-panel layout with the D3 graph on the
/// left and the Inspector on the right.
#[component]
pub fn App() -> Element {
    let (initial_sheet, initial_labels) = make_demo_sheet();
    let sheet = use_signal(|| initial_sheet);
    let labels = use_signal(|| initial_labels);

    let graph_data = use_memo(move || to_graph_data(&sheet.read(), &labels.read()));

    rsx! {
        document::Link { rel: "icon", r#type: "image/x-icon", href: asset!("/assets/favicon.ico") }
        document::Link { rel: "stylesheet", href: asset!("/assets/graph.css") }
        document::Script { src: asset!("/assets/d3.v7.min.js") }
        document::Script { src: asset!("/assets/graph.js") }
        document::Script { r#type: "module", src: asset!("/assets/swc.js") }

        SpTheme {
            color: "light".to_string(),
            scale: "medium".to_string(),
            div {
                style: "position: fixed; inset: 0; display: flex; overflow: hidden;",
                GraphView { data: graph_data }
                Inspector { sheet, labels }
            }
        }
    }
}
