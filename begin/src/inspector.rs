//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Pressing Enter in an input field writes the parsed value to the cell and propagates.
#[component]
pub fn Inspector(sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let cell_rows: Vec<(CellId, String, String)> = {
        let s = sheet.read();
        let l = labels.read();
        l.cells
            .iter()
            .map(|(id, meta)| (*id, meta.label.clone(), (meta.display)(&s)))
            .collect()
    };

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; border-left: 1px solid #ddd; padding: 12px; box-sizing: border-box; font-family: monospace; font-size: 13px;",
            h3 { style: "margin: 0 0 12px 0; font-size: 14px;", "Cells" }
            for (id, label, value) in cell_rows {
                CellRow { key: "{id:?}", id, label, value, sheet, labels }
            }
        }
    }
}

#[component]
fn CellRow(
    id: CellId,
    label: String,
    value: String,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
) -> Element {
    let mut input = use_signal(|| value.clone());

    // Keep input text in sync when the value changes externally (e.g. via propagation).
    use_effect(move || {
        let current = {
            let s = sheet.read();
            let l = labels.read();
            l.cells
                .get(&id)
                .map(|m| (m.display)(&s))
                .unwrap_or_default()
        };
        input.set(current);
    });

    rsx! {
        div {
            style: "margin-bottom: 10px;",
            div { style: "font-weight: bold; margin-bottom: 2px;", "{label}" }
            div { style: "color: #888; margin-bottom: 4px; font-size: 11px;", "{value}" }
            input {
                r#type: "text",
                value: "{input}",
                style: "width: 100%; box-sizing: border-box; font-family: monospace; font-size: 12px;",
                oninput: move |e| input.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        let s = input.peek().clone();
                        let mut sheet_w = sheet.write();
                        let labels_r = labels.read();
                        if let Some(meta) = labels_r.cells.get(&id) {
                            if (meta.write_str)(&mut sheet_w, &s).is_ok() {
                                let _ = sheet_w.propagate();
                            }
                        }
                    }
                }
            }
        }
    }
}
