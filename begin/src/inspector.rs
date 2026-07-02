//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use dioxus::prelude::*;
use property_model::{CellId, Sheet};

use crate::bridge::Labels;
use crate::spectrum::{SpDivider, SpFieldLabel, SpHeading, SpTextfield};

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), `SpTextfield` renders in its invalid state until the user blurs. The input is
/// not reset while the field is focused; it syncs back to the computed value on blur,
/// keeping non-edited cells up to date.
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    applied_source: Signal<String>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, error, applied_source }
            }
        }
    }
}

#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    applied_source: Signal<String>,
) -> Element {
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
                id: field_id,
                value: input.read().clone(),
                invalid: *has_error.read(),
                // Dioxus's event serializer only reads event.target.value for
                // HTMLInputElement — custom elements (sp-textfield) always give "".
                // Use dioxus.send() in JS and eval.recv() to read the live value.
                oninput: move |_: FormEvent| {
                    spawn(async move {
                        let mut eval = document::eval(&format!(
                            r#"dioxus.send(document.getElementById("cell-{id:?}").value)"#
                        ));
                        let Ok(val) = eval.recv::<String>().await else { return; };
                        // Discard the result if the user blurred while the round-trip was
                        // in flight; blur already cleared the error and use_effect will
                        // restore the last valid computed value.
                        if !*is_focused.read() {
                            return;
                        }
                        input.set(val.clone());
                        let mut sheet_w = sheet.write();
                        let labels_r = labels.read();
                        let Some(meta) = labels_r.cells.get(&id) else { return; };
                        let write_result = (meta.write_str)(&mut sheet_w, &val);
                        drop(labels_r);
                        let propagate_result = match write_result {
                            Ok(()) => {
                                // A conditional match cell changes the active constraint set
                                // when written, which invalidates the plan even if the cell
                                // is a source — so we must always replan for match cells.
                                let is_match_cell = sheet_w
                                    .conditionals()
                                    .any(|cid| sheet_w.conditional_match_cell(cid) == Some(id));
                                if sheet_w.is_source(id) && !is_match_cell {
                                    sheet_w.propagate_without_replan()
                                } else {
                                    sheet_w.propagate()
                                }
                            }
                            Err(e) => Err(e),
                        };
                        match propagate_result {
                            Ok(()) => {
                                has_error.set(false);
                                error.set(None);
                            }
                            Err(e) => {
                                has_error.set(true);
                                let source = applied_source.read().clone();
                                error.set(Some(crate::bridge::format_property_model_error(&e, &source)));
                            }
                        }
                    });
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
