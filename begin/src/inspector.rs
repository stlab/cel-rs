//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use adam_rs::{CellId, Sheet};
use dioxus::prelude::*;

use crate::bridge::{Labels, format_adam_error};
use crate::spectrum::{SpDivider, SpFieldLabel, SpHeading, SpTextfield};

use std::collections::HashSet;

/// Aggregate out-cell status for the whole sheet, computed once per render and shared by
/// every `CellRow` so `Sheet::output_relevant_cells`/`output_violation_cells` run once
/// instead of once per row.
#[derive(Clone, PartialEq)]
struct OutputStatus {
    /// `true` if the sheet has at least one output.
    has_outputs: bool,
    /// `Sheet::output_relevant_cells()`, plus every conditional's match cell.
    ///
    /// `Sheet::contributing_cells` never traces back through a conditional's match
    /// cell (it only follows relationship method inputs), so without this addition a
    /// conditional's own switch could be marked "don't care" and disabled once the
    /// sheet has any output — blocking the toggle that controls which branch is
    /// active. Match cells are therefore always treated as relevant, independent of
    /// which branch is currently active.
    relevant: HashSet<CellId>,
    /// Union of `Sheet::output_violation_cells()`.
    warning: HashSet<CellId>,
    /// Cells backing an output whose `Sheet::output_valid` is currently `false`.
    invalid_outputs: HashSet<CellId>,
}

/// Computes `sheet`'s current out-cell status for the Inspector.
///
/// - Complexity: O(`Sheet::output_relevant_cells` + `Sheet::output_violation_cells` +
///   the number of conditionals in the sheet).
fn compute_output_status(sheet: &Sheet) -> OutputStatus {
    let outputs: Vec<_> = sheet.outputs().collect();
    let relevant = sheet
        .output_relevant_cells()
        .into_iter()
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cell(id)),
        )
        .collect();
    let invalid_outputs = outputs
        .iter()
        .filter(|&&id| !sheet.output_valid(id))
        .filter_map(|&id| sheet.output_cell(id))
        .collect();
    OutputStatus {
        has_outputs: !outputs.is_empty(),
        relevant,
        warning: sheet.output_violation_cells(),
        invalid_outputs,
    }
}

/// A cell's Inspector display flags, derived from its own forced/error state and the
/// sheet-wide out-cell status.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CellFlags {
    disabled: bool,
    invalid: bool,
    warning: bool,
}

/// Derives `id`'s Inspector display flags from its own `forced`/`has_error` state and the
/// sheet-wide `status`.
///
/// - Postcondition: `warning` is `false` whenever `invalid` is `true` — a field never
///   shows both states at once.
fn cell_flags(id: CellId, forced: bool, has_error: bool, status: &OutputStatus) -> CellFlags {
    let disabled = forced || (status.has_outputs && !status.relevant.contains(&id));
    let invalid = has_error || status.invalid_outputs.contains(&id);
    let warning = !invalid && status.warning.contains(&id);
    CellFlags {
        disabled,
        invalid,
        warning,
    }
}

/// Sidebar panel showing all cells with labels, current values, and text inputs for writing.
///
/// Editing an input field immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), `SpTextfield` renders in its invalid state until the user blurs, and the
/// formatted diagnostic is printed to stderr. The input is not reset while the field is
/// focused; it syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::demo_source::ActiveSource>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&sheet.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, active_source, output_status }
            }
        }
    }
}

#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::demo_source::ActiveSource>,
    output_status: Memo<OutputStatus>,
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

    let forced = use_memo(move || sheet.read().is_forced(id));

    let mut input = use_signal(|| value.peek().clone());
    let mut is_focused = use_signal(|| false);
    let mut has_error = use_signal(|| false);

    let flags =
        use_memo(move || cell_flags(id, *forced.read(), *has_error.read(), &output_status.read()));

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
                invalid: flags.read().invalid,
                warning: flags.read().warning,
                disabled: flags.read().disabled,
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
                            }
                            Err(e) => {
                                has_error.set(true);
                                let active = active_source.read();
                                eprintln!("{}", format_adam_error(&e, &active.text, &active.file_name()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status(
        has_outputs: bool,
        relevant: &[CellId],
        warning: &[CellId],
        invalid_outputs: &[CellId],
    ) -> OutputStatus {
        OutputStatus {
            has_outputs,
            relevant: relevant.iter().copied().collect(),
            warning: warning.iter().copied().collect(),
            invalid_outputs: invalid_outputs.iter().copied().collect(),
        }
    }

    fn dummy_cell() -> CellId {
        let mut sheet = Sheet::new();
        sheet.add_cell(0_i32)
    }

    #[test]
    fn cell_flags_enabled_when_no_outputs_even_if_not_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(false, &[], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    fn cell_flags_disabled_when_forced_regardless_of_outputs() {
        let id = dummy_cell();
        let flags = cell_flags(id, true, false, &status(false, &[], &[], &[]));
        assert!(flags.disabled);
    }

    #[test]
    fn cell_flags_disabled_when_has_outputs_and_cell_not_relevant() {
        // Both ids must come from the same Sheet: two fresh Sheets' first added cell
        // return equal CellId values (slotmap's key generation is deterministic per
        // map), which would make `id` and `other` indistinguishable below.
        let mut sheet = Sheet::new();
        let id = sheet.add_cell(0_i32);
        let other = sheet.add_cell(0_i32);
        let flags = cell_flags(id, false, false, &status(true, &[other], &[], &[]));
        assert!(flags.disabled);
    }

    #[test]
    fn cell_flags_enabled_when_has_outputs_and_cell_is_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    fn cell_flags_invalid_when_has_error() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, true, &status(false, &[], &[], &[]));
        assert!(flags.invalid);
    }

    #[test]
    fn cell_flags_invalid_when_cell_is_an_invalid_output() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[id]));
        assert!(flags.invalid);
    }

    #[test]
    fn cell_flags_warning_when_in_warning_set_and_not_invalid() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[id], &[]));
        assert!(flags.warning);
    }

    #[test]
    fn cell_flags_warning_suppressed_when_also_invalid() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, true, &status(true, &[id], &[id], &[]));
        assert!(!flags.warning);
        assert!(flags.invalid);
    }
}
