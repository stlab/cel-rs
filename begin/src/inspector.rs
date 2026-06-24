//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Pressing Enter in an input field writes the parsed value to the cell and propagates.
#[component]
pub fn Inspector(sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; border-left: 1px solid #ddd; padding: 12px; box-sizing: border-box; font-family: monospace; font-size: 13px;",
            h3 { style: "margin: 0 0 12px 0; font-size: 14px;", "Cells" }
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels }
            }
        }
    }
}

#[component]
fn CellRow(id: CellId, sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let label = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.label.clone())
            .unwrap_or_default()
    });

    let value = use_memo(move || {
        let s = sheet.read();
        let l = labels.read();
        l.cells
            .get(&id)
            .map(|m| (m.display)(&s))
            .unwrap_or_default()
    });

    let mut input = use_signal(|| value.peek().clone());

    // Keep input text in sync when the cell value changes externally (e.g. via propagation).
    use_effect(move || {
        input.set(value.read().clone());
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
                        if let Some(meta) = labels_r.cells.get(&id)
                            && (meta.write_str)(&mut sheet_w, &s).is_ok()
                        {
                            let _ = sheet_w.propagate();
                        }
                    }
                }
            }
        }
    }
}
