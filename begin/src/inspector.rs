//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If propagation fails (for example, division by zero), the input field shows
/// a red border until the user blurs. The input is not reset while the field is focused;
/// it syncs back to the computed value on blur, keeping non-edited cells up to date.
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
    let mut is_focused = use_signal(|| false);
    let mut has_error = use_signal(|| false);

    // Sync input to the computed value whenever it changes, but not while the user
    // is actively editing — that would interrupt mid-value typing (e.g. "1." → "1").
    use_effect(move || {
        let v = value.read().clone();
        if !*is_focused.read() {
            input.set(v);
        }
    });

    rsx! {
        div {
            style: "margin-bottom: 10px;",
            div { style: "font-weight: bold; margin-bottom: 2px;", "{label}" }
            div { style: "color: #888; margin-bottom: 4px; font-size: 11px;", "{value}" }
            input {
                r#type: "text",
                value: "{input}",
                style: if *has_error.read() {
                    "width: 100%; box-sizing: border-box; font-family: monospace; font-size: 12px; border-color: #c00;"
                } else {
                    "width: 100%; box-sizing: border-box; font-family: monospace; font-size: 12px;"
                },
                onfocus: move |_| is_focused.set(true),
                onblur: move |_| {
                    is_focused.set(false);
                    has_error.set(false);
                },
                oninput: move |e| {
                    let s = e.value();
                    input.set(s.clone());
                    let mut sheet_w = sheet.write();
                    let labels_r = labels.read();
                    if let Some(meta) = labels_r.cells.get(&id) {
                        if (meta.write_str)(&mut sheet_w, &s).is_ok() {
                            has_error.set(sheet_w.propagate().is_err());
                        }
                    }
                },
            }
        }
    }
}
