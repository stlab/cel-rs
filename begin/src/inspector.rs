//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;
use crate::spectrum::{SpDivider, SpFieldLabel, SpHeading, SpTextfield};

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If propagation fails (for example, division by zero), `SpTextfield` renders
/// in its invalid state until the user blurs. The input is not reset while the field is
/// focused; it syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn Inspector(sheet: Signal<Sheet>, labels: Signal<Labels>) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
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

    let field_id = format!("cell-{id:?}");

    rsx! {
        div {
            style: "margin-bottom: 8px;",
            SpFieldLabel { for_: field_id.clone(), "{label}" }
            SpTextfield {
                value: input.read().clone(),
                invalid: *has_error.read(),
                oninput: move |e: FormEvent| {
                    let s = e.value();
                    input.set(s.clone());
                    let mut sheet_w = sheet.write();
                    let labels_r = labels.read();
                    if let Some(meta) = labels_r.cells.get(&id)
                        && (meta.write_str)(&mut sheet_w, &s).is_ok()
                    {
                        let result = if sheet_w.is_source(id) {
                            sheet_w.propagate_without_replan()
                        } else {
                            sheet_w.propagate()
                        };
                        has_error.set(result.is_err());
                    }
                },
                onfocus: move |_| is_focused.set(true),
                onblur: move |_| {
                    is_focused.set(false);
                    has_error.set(false);
                },
            }
        }
        SpDivider {}
    }
}
