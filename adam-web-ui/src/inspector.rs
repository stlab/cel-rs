//! [`SheetInspector`] — a live, editable list of a sheet's cells with a write form.

use adam_rs::{CellId, FilterViolation, Sheet};
use dioxus::prelude::*;

use crate::labels::{Labels, Renderer, format_adam_error, format_rounded};
use crate::spectrum::{
    SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpNumberfield, SpSlider, SpTextfield,
};

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
    /// Union of `Sheet::output_violation_cells()` and `Sheet::filter_violation_cells()` —
    /// the root cells that produced a violating value, shown less severely than the cell
    /// actually carrying the violation (see `invalid_outputs`/`filter_violated`).
    warning: HashSet<CellId>,
    /// Cells backing an output whose `Sheet::output_valid` is currently `false`.
    invalid_outputs: HashSet<CellId>,
    /// `Sheet::filter_violated_cells()` — cells whose own filter didn't hold, shown the
    /// same way a parse error is: this is the cell's own value that's out of domain, not
    /// just a contributor to someone else's.
    filter_violated: HashSet<CellId>,
}

/// Computes `sheet`'s current out-cell status for the Inspector.
///
/// - Complexity: O(`Sheet::output_relevant_cells` + `Sheet::output_violation_cells` +
///   `Sheet::filter_violation_cells` + the number of conditionals in the sheet).
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
    let warning = sheet
        .output_violation_cells()
        .into_iter()
        .chain(sheet.filter_violation_cells())
        .collect();
    let filter_violated = sheet.filter_violated_cells().collect();
    OutputStatus {
        has_outputs: !outputs.is_empty(),
        relevant,
        warning,
        invalid_outputs,
        filter_violated,
    }
}

/// Formats a diagnostic message for `label`'s filter `violation`, for
/// `crate::diagnostics::report_error`.
fn format_filter_violation(label: &str, violation: &FilterViolation) -> String {
    match violation {
        FilterViolation::NotConformed => {
            format!("filter violation: `{label}`'s derived value does not conform to its filter")
        }
        FilterViolation::Failed(e) => {
            format!("filter violation: `{label}`'s filter failed on its derived value: {e}")
        }
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
    let invalid =
        has_error || status.invalid_outputs.contains(&id) || status.filter_violated.contains(&id);
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
/// output requirement's inputs (`propagate_without_replan` does not re-evaluate output
/// requirements at all, per its own documented contract — so `output_valid`/
/// `output_violation_cells` would otherwise go stale after such a write).
///
/// This also holds for a cell referenced as another cell's filter argument
/// ([`adam_rs::Sheet::filter_dependents`]): a source-cell filter reclamp is folded into
/// the planner's own dependency graph (see the adam-rs planner) and is only revalidated
/// by a full `Sheet::propagate()`'s own diagnostic phase, not by
/// `propagate_without_replan`.
///
/// - Complexity: O(number of conditionals + number of output requirements + number of
///   filter dependents of `id`).
fn cell_needs_full_propagate(sheet: &Sheet, id: CellId) -> bool {
    let is_match_cell = sheet.conditionals().any(|cid| {
        sheet
            .conditional_match_cells(cid)
            .is_some_and(|c| c.contains(&id))
    });
    let feeds_requirement = sheet.outputs().any(|oid| {
        sheet.output_requirements(oid).is_some_and(|requirements| {
            requirements.iter().any(|&rid| {
                sheet
                    .requirement_inputs(rid)
                    .is_some_and(|inputs| inputs.contains(&id))
            })
        })
    });
    let feeds_a_filter = !sheet.filter_dependents(id).is_empty();
    is_match_cell || feeds_requirement || feeds_a_filter
}

/// Returns the toggled value ("true"/"false") for a bool cell currently displaying `current`.
fn toggled_bool_value(current: &str) -> &'static str {
    if current == "true" { "false" } else { "true" }
}

/// Returns the `min`/`max` bounds to pass to a cell's [`SpNumberfield`]: `range`'s bounds,
/// widened if necessary so `current` (the field's own displayed text) always falls within them.
///
/// `sp-number-field` clamps its displayed value to fit whatever `min`/`max` it's given — not
/// just in response to user input, but on *any* update to `min`, `max`, or `value` where the
/// three momentarily disagree — and, worse, resets its displayed value to `0` rather than
/// restoring the true value if `min`/`max` are later removed entirely rather than merely
/// changed. A range filter's live bounds (`range`, recomputed from the filter's *current*
/// argument values) can transiently exclude a cell's actual stored value: the filter only
/// re-clamps a cell at the moment that cell itself is written, so changing another cell its
/// bounds depend on (e.g. a shared `max` cell) does not retroactively pull this cell back in
/// range. Widening `range` to always include `current` guarantees the three never disagree, so
/// the widget never mis-clamps or resets — at the cost of its stepper arrows not disabling
/// exactly at the filter's true limit while a cell sits outside it, until the cell's own next
/// write brings it back in range and the true bounds resume being enforced.
///
/// - Postcondition: returns `(None, None)` whenever `range` is `None`.
fn number_field_bounds(
    current: &str,
    range: Option<(f64, f64)>,
) -> (Option<String>, Option<String>) {
    let Some((lo, hi)) = range else {
        return (None, None);
    };
    let current = current.parse::<f64>().unwrap_or(lo);
    (
        Some(lo.min(current).to_string()),
        Some(hi.max(current).to_string()),
    )
}

/// Returns `true` if `typed`, read as a number, differs from `actual` (a cell's post-write
/// display string) after applying [`format_rounded`]'s own rounding to both — i.e. a range
/// filter silently clamped the written value to something other than what was typed. A range
/// filter's `write_str`/`Sheet::propagate` both return `Ok` in that case (clamping always
/// produces some valid value), so this is the only place that distinguishes it from an
/// unmodified write.
///
/// - Postcondition: returns `false` whenever `typed` doesn't parse as `f64` — a non-numeric
///   cell's write is never treated as clamped.
fn clamped_away(typed: &str, actual: &str) -> bool {
    match typed.parse::<f64>() {
        Ok(v) => format_rounded(v) != actual,
        Err(_) => false,
    }
}

/// Parses `val` for `id` via its `Labels` metadata, writes it to `sheet`, and propagates the
/// sheet's constraints, updating `has_error` and reporting any error — or, on success, any
/// currently-violated filter — to `crate::diagnostics`.
///
/// - Postcondition: `has_error` is `true` on parse or propagation failure, or when a range
///   filter clamped `val` away from what was typed (see [`clamped_away`]); `false` otherwise.
fn write_and_propagate(
    mut sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    id: CellId,
    val: &str,
    mut has_error: Signal<bool>,
    source_text: Memo<String>,
    source_name: Memo<String>,
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
            let labels_r = labels.read();
            let clamped = labels_r
                .cells
                .get(&id)
                .is_some_and(|m| clamped_away(val, &(m.display)(&sheet_w)));
            has_error.set(clamped);
            for violated_id in sheet_w.filter_violated_cells().collect::<Vec<_>>() {
                let Some(violation) = sheet_w.filter_violation(violated_id) else {
                    continue;
                };
                let label = labels_r
                    .cells
                    .get(&violated_id)
                    .map(|m| m.label.as_str())
                    .unwrap_or("<unknown cell>");
                crate::diagnostics::report_error(&format_filter_violation(label, violation));
            }
        }
        Err(e) => {
            has_error.set(true);
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &source_text.read(),
                &source_name.read(),
                &Renderer::styled(),
            ));
        }
    }
}

/// Sidebar panel showing all cells with labels and inputs for writing — a checkbox for
/// `bool`-typed cells, a number field (plus a live-range slider when the cell has a range
/// filter) for numeric cells, and a text field for everything else.
///
/// Editing an input immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), the field renders in its invalid state until the user blurs, and the formatted
/// diagnostic is printed to stderr. A field's input is not reset while it is focused; it
/// syncs back to the computed value on blur, keeping non-edited cells up to date.
#[component]
pub fn SheetInspector(
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    source_text: Memo<String>,
    source_name: Memo<String>,
) -> Element {
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&sheet.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{id:?}", id, sheet, labels, source_text, source_name, output_status }
            }
        }
    }
}

#[component]
fn CellRow(
    id: CellId,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    source_text: Memo<String>,
    source_name: Memo<String>,
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

    let is_numeric = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.is_numeric)
            .unwrap_or(false)
    });

    let range = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .and_then(|m| m.range.as_ref())
            .map(|f| f(&sheet.read()))
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
                        let next = toggled_bool_value(&value.peek());
                        write_and_propagate(sheet, labels, id, next, has_error, source_text, source_name);
                        // `sp-checkbox` toggles its own shadow-DOM `checked` state
                        // natively in response to the click, before this handler runs
                        // and independent of the `checked` prop below. If the write above
                        // was rejected, `value` recomputes to the same string as before,
                        // so Dioxus's diff sees no change and never re-touches the DOM —
                        // leaving the visual checkbox desynced from the sheet. Force the
                        // element back to the actual committed value here.
                        let checked = *value.read() == "true";
                        spawn(async move {
                            let _ = document::eval(&format!(
                                r#"document.getElementById("cell-{id:?}").checked = {checked};"#
                            ))
                            .await;
                        });
                    },
                }
            } else if *is_numeric.read() {
                {
                    let (min, max) = number_field_bounds(&input.read(), *range.read());
                    rsx! {
                        SpNumberfield {
                            id: field_id.clone(),
                            value: input.read().clone(),
                            min,
                            max,
                            invalid: flags.read().invalid,
                            warning: flags.read().warning,
                            disabled: flags.read().disabled,
                            oninput: move |_: FormEvent| {
                                spawn(async move {
                                    // Reads the shadow-DOM `<input>`'s raw text, not the host's
                                    // `value` property: once `min`/`max` are set, `sp-number-field`
                                    // clamps its own `value` to that range on every keystroke,
                                    // which would hide an out-of-range-for-type entry from
                                    // `write_and_propagate` below before it ever sees the digits
                                    // the user actually typed.
                                    let mut eval = document::eval(&format!(
                                        r#"dioxus.send(document.getElementById("cell-{id:?}").shadowRoot.querySelector("input").value)"#
                                    ));
                                    let Ok(val) = eval.recv::<String>().await else { return; };
                                    if !*is_focused.read() {
                                        return;
                                    }
                                    input.set(val.clone());
                                    write_and_propagate(sheet, labels, id, &val, has_error, source_text, source_name);
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
                if let Some((lo, hi)) = *range.read() {
                    SpSlider {
                        id: format!("cell-{id:?}-slider"),
                        value: input.read().clone(),
                        min: format!("{lo}"),
                        max: format!("{hi}"),
                        disabled: flags.read().disabled,
                        oninput: move |_: FormEvent| {
                            spawn(async move {
                                let mut eval = document::eval(&format!(
                                    r#"dioxus.send(document.getElementById("cell-{id:?}-slider").value.toString())"#
                                ));
                                let Ok(val) = eval.recv::<String>().await else { return; };
                                input.set(val.clone());
                                write_and_propagate(sheet, labels, id, &val, has_error, source_text, source_name);
                            });
                        },
                    }
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
                            write_and_propagate(sheet, labels, id, &val, has_error, source_text, source_name);
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
        status_with_filter_violated(has_outputs, relevant, warning, invalid_outputs, &[])
    }

    fn status_with_filter_violated(
        has_outputs: bool,
        relevant: &[CellId],
        warning: &[CellId],
        invalid_outputs: &[CellId],
        filter_violated: &[CellId],
    ) -> OutputStatus {
        OutputStatus {
            has_outputs,
            relevant: relevant.iter().copied().collect(),
            warning: warning.iter().copied().collect(),
            invalid_outputs: invalid_outputs.iter().copied().collect(),
            filter_violated: filter_violated.iter().copied().collect(),
        }
    }

    #[test]
    fn compute_output_status_filter_violated_includes_the_cell_whose_own_filter_failed() {
        use adam_rs::{Filter, Method};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &f64| Ok(x.clamp(0.0, 100.0))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(b, a, |v: &f64| Ok(*v))])
            .unwrap();
        sheet.write(b, -30.0_f64).unwrap();
        sheet.propagate().unwrap();

        let status = compute_output_status(&sheet);
        assert!(status.filter_violated.contains(&a));
        assert!(status.warning.contains(&b));
    }

    #[test]
    fn compute_output_status_filter_violated_empty_when_no_filter_is_violated() {
        use adam_rs::{Filter, Method};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0.0_f64);
        let b = sheet.add_cell(0.0_f64);
        sheet
            .add_filter(a, Filter::from_fn_0(|x: &f64| Ok(x.clamp(0.0, 100.0))))
            .unwrap();
        sheet
            .add_relationship(vec![Method::from_fn_1_1(b, a, |v: &f64| Ok(*v))])
            .unwrap();
        sheet.write(b, 30.0_f64).unwrap();
        sheet.propagate().unwrap();

        let status = compute_output_status(&sheet);
        assert!(status.filter_violated.is_empty());
        assert!(status.warning.is_empty());
    }

    #[test]
    fn format_filter_violation_not_conformed_names_the_cell() {
        let msg = format_filter_violation("a", &FilterViolation::NotConformed);
        assert!(msg.contains('a'));
        assert!(msg.contains("does not conform"));
    }

    #[test]
    fn format_filter_violation_failed_includes_the_underlying_error() {
        let msg = format_filter_violation(
            "a",
            &FilterViolation::Failed(anyhow::anyhow!("out of range")),
        );
        assert!(msg.contains('a'));
        assert!(msg.contains("out of range"));
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
    fn cell_flags_invalid_when_cell_is_filter_violated() {
        let id = dummy_cell();
        let flags = cell_flags(
            id,
            false,
            false,
            &status_with_filter_violated(true, &[id], &[], &[], &[id]),
        );
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
    fn clamped_away_false_when_typed_matches_actual() {
        assert!(!clamped_away("42", "42"));
    }

    #[test]
    fn clamped_away_true_when_a_range_filter_clamped_the_value() {
        assert!(clamped_away("150", "100"));
    }

    #[test]
    fn clamped_away_ignores_display_rounding_differences() {
        assert!(!clamped_away("1.005", &format_rounded(1.005)));
    }

    #[test]
    fn clamped_away_false_for_non_numeric_input() {
        assert!(!clamped_away("not a number", "0"));
    }

    #[test]
    fn number_field_bounds_none_when_no_range() {
        assert_eq!(number_field_bounds("50", None), (None, None));
    }

    #[test]
    fn number_field_bounds_returns_range_unchanged_when_current_is_within_it() {
        assert_eq!(
            number_field_bounds("50", Some((0.0, 100.0))),
            (Some("0".to_string()), Some("100".to_string()))
        );
    }

    #[test]
    fn number_field_bounds_widens_max_to_include_a_current_value_above_it() {
        assert_eq!(
            number_field_bounds("150", Some((0.0, 100.0))),
            (Some("0".to_string()), Some("150".to_string()))
        );
    }

    #[test]
    fn number_field_bounds_widens_min_to_include_a_current_value_below_it() {
        assert_eq!(
            number_field_bounds("-50", Some((0.0, 100.0))),
            (Some("-50".to_string()), Some("100".to_string()))
        );
    }

    #[test]
    fn number_field_bounds_falls_back_to_the_unwidened_range_when_current_does_not_parse() {
        assert_eq!(
            number_field_bounds("not a number", Some((0.0, 100.0))),
            (Some("0".to_string()), Some("100".to_string()))
        );
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
    fn cell_needs_full_propagate_true_for_cell_feeding_an_output_requirement() {
        use adam_rs::{Method, Requirement};

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let b = sheet.add_cell(0_i32);
        let result = sheet.add_cell(0_i32);
        sheet
            .add_output(
                Method::from_fn_2_1([a, b], result, |x: &i32, y: &i32| Ok(x + y)),
                vec![(
                    "min_a",
                    Requirement::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y)),
                )],
            )
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, a));
        assert!(cell_needs_full_propagate(&sheet, b));
    }

    #[test]
    fn cell_needs_full_propagate_true_for_a_cell_referenced_as_a_filter_argument() {
        use adam_rs::Filter;

        let mut sheet = Sheet::new();
        let bound = sheet.add_cell(10_i32);
        let a = sheet.add_cell(5_i32);
        sheet
            .add_filter(
                a,
                Filter::from_fn_1(bound, |v: &i32, b: &i32| Ok((*v).min(*b))),
            )
            .unwrap();

        assert!(cell_needs_full_propagate(&sheet, bound));
    }

    #[test]
    fn cell_needs_full_propagate_false_for_cell_not_a_match_cell_or_requirement_input() {
        use adam_rs::{Method, Requirement};

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
                    Requirement::from_fn_2([a, b], |x: &i32, y: &i32| Ok(x <= y)),
                )],
            )
            .unwrap();

        assert!(!cell_needs_full_propagate(&sheet, unrelated));
    }

    #[test]
    fn toggled_bool_value_true_becomes_false() {
        assert_eq!(toggled_bool_value("true"), "false");
    }

    #[test]
    fn toggled_bool_value_false_becomes_true() {
        assert_eq!(toggled_bool_value("false"), "true");
    }
}
