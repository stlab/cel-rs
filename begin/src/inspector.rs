//! [`Inspector`] — sidebar listing all cells with their current values and a write form.

use adam_rs::{CellId, Sheet};
use dioxus::prelude::*;

use crate::bridge::{Labels, format_adam_error};
use crate::spectrum::{SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpTextfield};

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
                .filter_map(|id| sheet.conditional_match_cells(id))
                .flatten()
                .copied(),
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

/// Returns `true` if writing `id` can invalidate more than just the cached plan's
/// execution order, so a full `Sheet::propagate()` is required instead of the cheaper
/// `Sheet::propagate_without_replan()`.
///
/// This holds for a conditional's match cell (writing it can switch the active branch,
/// which `propagate_without_replan` never re-evaluates) and for any cell that feeds an
/// output condition's inputs (`propagate_without_replan` does not re-evaluate output
/// conditions at all, per its own documented contract — so `output_valid`/
/// `output_violation_cells` would otherwise go stale after such a write).
///
/// - Complexity: O(number of conditionals + number of output conditions in the sheet).
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_condition = sheet.outputs().any(|oid| {
        sheet.output_conditions(oid).is_some_and(|conditions| {
            conditions.iter().any(|&cid| {
                sheet
                    .condition_inputs(cid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    is_match_cell || feeds_condition
}

/// Parses `val` for `id` via its `Labels` metadata, writes it to `sheet`, and propagates the
/// sheet's constraints, updating `has_error` and reporting any error to `crate::diagnostics`.
///
/// - Postcondition: `has_error` is `false` on success, `true` on parse or propagation failure.
fn write_and_propagate(
    mut sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    id: CellId,
    val: &str,
    mut has_error: Signal<bool>,
    active_source: Signal<crate::example_source::ActiveSource>,
) {
    let mut sheet_w = sheet.write();
    let labels_r = labels.read();
    let Some(meta) = labels_r.cells.get(&id) else {
        return;
    };
    let write_result = (meta.write_str)(&mut sheet_w, val);
    drop(labels_r);
    let propagate_result = match write_result {
        Ok(()) => {
            if sheet_w.is_source(id) && !cell_needs_full_propagate(&sheet_w, id) {
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
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &active.text,
                &active.file_name(),
            ));
        }
    }
}

/// Sidebar panel showing all cells with labels and inputs for writing — a checkbox for
/// `bool`-typed cells, a text field for everything else.
///
/// Editing an input immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), `SpTextfield` renders in its invalid state until the user blurs, and the
/// formatted diagnostic is printed to stderr. The text field's input is not reset while it is
/// focused; it syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn Inspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    active_source: Signal<crate::example_source::ActiveSource>,
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
    active_source: Signal<crate::example_source::ActiveSource>,
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

    let is_bool = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.is_bool)
            .unwrap_or(false)
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
            if *is_bool.read() {
                SpCheckbox {
                    id: field_id,
                    checked: *value.read() == "true",
                    invalid: flags.read().invalid,
                    warning: flags.read().warning,
                    disabled: flags.read().disabled,
                    onclick: move |_| {
                        let next = if *value.peek() == "true" { "false" } else { "true" };
                        write_and_propagate(sheet, labels, id, next, has_error, active_source);
                    },
                }
            } else {
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
                            write_and_propagate(sheet, labels, id, &val, has_error, active_source);
                        });
                    },
                    onfocus: move |_| is_focused.set(true),
                    onblur: move |_| {
                        is_focused.set(false);
                        has_error.set(false);
                    },
                }
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

    #[test]
    fn cell_needs_full_propagate_false_when_sheet_has_no_conditionals_or_outputs() {
        let id = dummy_cell();
        let sheet = Sheet::new();
        assert!(!cell_needs_full_propagate(&sheet, id));
    }

    #[test]
    fn cell_needs_full_propagate_true_for_conditional_match_cell() {
        use adam_rs::{MatchExpr, Method};

        let mut sheet = Sheet::new();
        let p = sheet.add_cell(0_i32);
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        let rel = sheet
            .add_relationship(vec![Method::from_fn_1_1(a, b, |v: &f64| Ok(*v))])
            .unwrap();
        sheet
            .add_conditional(MatchExpr::cell(p), vec![(vec![0_i32], vec![rel])], vec![])
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, p));
    }

    #[test]
    fn cell_needs_full_propagate_true_for_cell_feeding_an_output_condition() {
        use adam_rs::{Condition, Method};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_cell(0_i32);
        sheet
            .add_output(
                Method::from_fn_2_1([a, b], result, |x: &i32, y: &i32| Ok(x + y)),
                vec![(
                    "min_a",
                    Condition::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y)),
                )],
            )
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, a));
        assert!(cell_needs_full_propagate(&sheet, b));
    }

    #[test]
    fn cell_needs_full_propagate_false_for_cell_not_a_match_cell_or_condition_input() {
        use adam_rs::{Condition, Method};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_cell(0_i32);
        let unrelated = sheet.add_cell(0_i32);
        sheet
            .add_output(
                Method::from_fn_2_1([a, b], result, |x: &i32, y: &i32| Ok(x + y)),
                vec![(
                    "min_a",
                    Condition::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y)),
                )],
            )
            .unwrap();

        assert!(!cell_needs_full_propagate(&sheet, unrelated));
    }
}
