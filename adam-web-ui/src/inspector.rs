//! [`SheetInspector`] — a live, editable list of a sheet's cells with a write form.

use adam_lang::ParsedSheet;
use adam_rs::{CellId, FilterViolation, Sheet};
use dioxus::prelude::*;
#[cfg(test)]
#[path = "inspector_tests.rs"]
mod component_tests;

use crate::labels::{Labels, Renderer, format_adam_error, format_rounded};
use crate::spectrum::{
    SpCheckbox, SpDivider, SpFieldLabel, SpHeading, SpHelpText, SpNumberfield, SpSlider,
    SpTextfield,
};

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Aggregate out-cell status for the whole sheet, computed once per render and shared by
/// every `CellRow` so `Sheet::requirement_relevant_cells`/`requirement_violation_cells` run
/// once instead of once per row.
#[derive(Clone, PartialEq)]
struct OutputStatus {
    /// `true` if the sheet has at least one output.
    has_outputs: bool,
    /// Union of three sets: `Sheet::contributing_cells(id)` for every `id` in
    /// `Sheet::out_cells()`; `Sheet::requirement_relevant_cells()`, taken as-is (it already
    /// returns contributing cells internally, so it is not run through `contributing_cells`
    /// again here); and every conditional's match cell, added directly, not traced.
    ///
    /// The `contributing_cells` term restores the pre-generalization guarantee that an
    /// out cell's contributors are always relevant, regardless of whether that out cell
    /// has any requirements attached — `requirement_relevant_cells()` alone only covers
    /// cells feeding a requirement, so an out cell with zero requirements would otherwise
    /// silently drop out of "relevant" even though it's still an out cell whose inputs
    /// the user should see as meaningful.
    ///
    /// `Sheet::contributing_cells` never traces back through a conditional's match
    /// cell (it only follows relationship method inputs), so without the match-cell
    /// addition a conditional's own switch could be marked "don't care" and disabled
    /// once the sheet has any output — blocking the toggle that controls which branch is
    /// active. Match cells are therefore always treated as relevant, independent of
    /// which branch is currently active.
    relevant: HashSet<CellId>,
    /// Union of `Sheet::requirement_violation_cells()` and `Sheet::filter_violation_cells()`
    /// — the root cells that produced a violating value. Rendered with the same `invalid`
    /// state as the cell actually carrying the violation (see `invalid_outputs`/
    /// `filter_violated`): Spectrum has no notion of "implicated but not itself invalid",
    /// and inventing one (a past version of this code used a CSS class literally named
    /// `warning`) collided with `mdbook`'s own book-wide `.warning` admonition styles —
    /// an unrelated `ⓘ` callout icon rendered wherever this class landed, since class
    /// names aren't scoped to a component the way shadow-DOM parts are.
    invalid_contributors: HashSet<CellId>,
    /// Cells backing an output whose `Sheet::cell_requirements_valid` is currently `false`.
    invalid_outputs: HashSet<CellId>,
    /// `Sheet::filter_violated_cells()` — cells whose own filter didn't hold, shown the
    /// same way a parse error is: this is the cell's own value that's out of domain, not
    /// just a contributor to someone else's.
    filter_violated: HashSet<CellId>,
    /// Cells backing any live `out` declaration, via `Sheet::out_cells`, regardless of
    /// whether its requirements currently hold. An output's cell is always `forced` (see
    /// [`cell_flags`]), so without this it would render as `disabled` like any other
    /// non-writable cell — hiding a failed `require`'s `invalid` treatment, since a
    /// disabled `sp-number-field` suppresses `invalid` styling entirely. This set lets a
    /// numeric out cell render `readonly` instead, which keeps `invalid` visible.
    output_cells: HashSet<CellId>,
    /// Violated requirement names for each currently-invalid output, joined for display and
    /// keyed by the output's own cell — via `Sheet::violated_requirements`/
    /// `ParsedSheet::requirement_names`. A cell absent from this map has no currently-failing
    /// explicitly labeled requirement; unnamed failures still contribute to `invalid_outputs`.
    invalid_output_requirement_names: HashMap<CellId, String>,
}

/// Computes `parsed`'s current out-cell status and resolves caller-owned labels.
///
/// - Complexity: O(sum of `Sheet::contributing_cells` cost over every out cell +
///   `Sheet::requirement_relevant_cells` + `Sheet::requirement_violation_cells` +
///   `Sheet::filter_violation_cells` + the number of conditionals in the sheet +
///   violated requirements and their total label length), with expected O(1) label lookup.
fn compute_output_status(parsed: &ParsedSheet) -> OutputStatus {
    let sheet = &parsed.sheet;
    // `Sheet::out_cells()` now returns each out cell's own `CellId` directly — the
    // `OutputId` → `CellId` lookup this function used to do via `Sheet::output_cell` is
    // gone because there's no longer a separate handle to look up.
    let out_cells: Vec<CellId> = sheet.out_cells().collect();
    let relevant = out_cells
        .iter()
        .flat_map(|&id| sheet.contributing_cells(id))
        .chain(sheet.requirement_relevant_cells())
        .chain(
            sheet
                .conditionals()
                .filter_map(|id| sheet.conditional_match_cells(id))
                .flatten()
                .copied(),
        )
        .collect();
    let invalid_outputs = out_cells
        .iter()
        .copied()
        .filter(|&id| !sheet.cell_requirements_valid(id))
        .collect();
    let invalid_contributors = sheet
        .requirement_violation_cells()
        .into_iter()
        .chain(sheet.filter_violation_cells())
        .collect();
    let filter_violated = sheet.filter_violated_cells().collect();
    let output_cells = out_cells.iter().copied().collect();
    let invalid_output_requirement_names = out_cells
        .iter()
        .filter_map(|&cell| {
            let names: Vec<&str> = sheet
                .violated_requirements(cell)
                .filter_map(|rid| parsed.requirement_names.get(&rid).map(String::as_str))
                .collect();
            (!names.is_empty()).then(|| (cell, names.join(", ")))
        })
        .collect();
    OutputStatus {
        has_outputs: !out_cells.is_empty(),
        relevant,
        invalid_contributors,
        invalid_outputs,
        filter_violated,
        output_cells,
        invalid_output_requirement_names,
    }
}

/// Formats a diagnostic message for `label`'s filter `violation`, for
/// `crate::diagnostics::report_error`.
///
/// - Complexity: O(label/output bytes) formatting work, plus underlying error display.
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

/// Returns the message to show as a range-filtered cell's own invalid indicator.
///
/// `sp-slider` (unlike `SpNumberfield`/`SpCheckbox`/`SpTextfield`) has no `invalid` visual
/// state of its own (see `SpSlider`'s doc comment), so this sibling `SpHelpText` is the only
/// way a slider-rendered cell can surface a violation — every other control instead relies on
/// its own `invalid` prop and only adds this same text as a supplementary detail.
///
/// Prefers, in order: a currently-violated `require` name attached to `id` (only possible when
/// `id` is an out cell), then `id`'s own currently-violated filter, then a generic fallback
/// naming `label` when `id` is flagged invalid for some other reason (e.g. it merely
/// contributes to another cell's violated filter or requirement).
///
/// - Postcondition: `None` when `id` is in none of `status.invalid_outputs`,
///   `status.invalid_contributors`, and has no currently-violated filter (`Sheet::filter_violation`).
fn slider_invalid_message(
    id: CellId,
    label: &str,
    sheet: &Sheet,
    status: &OutputStatus,
) -> Option<String> {
    if let Some(names) = status.invalid_output_requirement_names.get(&id) {
        return Some(names.clone());
    }
    if let Some(violation) = sheet.filter_violation(id) {
        return Some(format_filter_violation(label, violation));
    }
    if status.invalid_outputs.contains(&id) || status.invalid_contributors.contains(&id) {
        return Some(format!("`{label}` is invalid"));
    }
    None
}

/// A cell's Inspector display flags, derived from its own forced/error state and the
/// sheet-wide out-cell status.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CellFlags {
    disabled: bool,
    invalid: bool,
    /// `true` when `id` backs a live `out` declaration. A numeric field uses this to
    /// render `readonly` instead of `disabled` — see [`OutputStatus::output_cells`] for
    /// why `disabled` alone would hide a failed `require`'s `invalid` treatment.
    readonly: bool,
}

/// Derives `id`'s Inspector display flags from its own `forced`/`has_error` state and the
/// sheet-wide `status`.
fn cell_flags(id: CellId, forced: bool, has_error: bool, status: &OutputStatus) -> CellFlags {
    let disabled = forced || (status.has_outputs && !status.relevant.contains(&id));
    let invalid = has_error
        || status.invalid_outputs.contains(&id)
        || status.filter_violated.contains(&id)
        || status.invalid_contributors.contains(&id);
    let readonly = status.output_cells.contains(&id);
    CellFlags {
        disabled,
        invalid,
        readonly,
    }
}

/// Returns the toggled value ("true"/"false") for a bool cell currently displaying `current`.
fn toggled_bool_value(current: &str) -> &'static str {
    if current == "true" { "false" } else { "true" }
}

/// Returns a stable DOM namespace unique among live inspector owners.
///
/// The hook retains the allocation until its controls unmount. Diagnostic names
/// and document-local cell keys do not establish instance identity.
fn dom_id_namespace(owner: &Rc<()>) -> usize {
    Rc::as_ptr(owner) as usize
}

/// Identifies one source installation without comparing document-local keys.
#[derive(Clone)]
struct InputOwner(Rc<()>);

impl PartialEq for InputOwner {
    /// Compares installation identity rather than the allocation's unit value.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Authorizes only the latest request belonging to the current source installation.
fn input_reply_is_current(
    requested_source: &InputOwner,
    current_source: &InputOwner,
    requested_revision: u64,
    current_revision: u64,
) -> bool {
    requested_source == current_source && requested_revision == current_revision
}

/// Returns the `min`/`max` bounds to pass to a range-filtered numeric cell's [`SpSlider`]:
/// `range`'s bounds, widened if necessary so `current` (the field's own displayed text) always
/// falls within them.
///
/// `sp-slider` (and, when `editable`, its inline `sp-number-field`) clamps its displayed value
/// to fit whatever `min`/`max` it's given — not just in response to user input, but on *any*
/// update to `min`, `max`, or `value` where the three momentarily disagree — mirroring
/// `sp-number-field`'s own documented clamp-on-update behavior (see [`SpNumberfield`]). A range
/// filter's live bounds (`range`, recomputed from the filter's *current* argument values) can
/// transiently exclude a cell's actual stored value: the filter only re-clamps a cell at the
/// moment that cell itself is written, so changing another cell its bounds depend on (e.g. a
/// shared `max` cell) does not retroactively pull this cell back in range. Widening `range` to
/// always include `current` guarantees the two never disagree, so the widget never mis-clamps —
/// at the cost of its handle/stepper not disabling exactly at the filter's true limit while a
/// cell sits outside it, until the cell's own next write brings it back in range and the true
/// bounds resume being enforced.
///
/// - Complexity: O(current.len()) parsing work; endpoint formatting is bounded by f64 size.
fn slider_bounds(current: &str, range: (f64, f64)) -> (String, String) {
    let (lo, hi) = range;
    let current = current.parse::<f64>().unwrap_or(lo);
    (lo.min(current).to_string(), hi.max(current).to_string())
}

/// Returns Spectrum display options for a numeric cell, preserving integer defaults and
/// displaying floating-point values with exactly two fractional digits.
fn numeric_format_options(is_integer: bool) -> Option<&'static str> {
    if is_integer {
        None
    } else {
        Some(r#"{"minimumFractionDigits":2,"maximumFractionDigits":2}"#)
    }
}

/// Returns slider increments of one for integer cells and one hundredth for floating-point cells.
///
/// The fractional increment matches the Inspector's two-decimal display precision.
fn slider_step(is_integer: bool) -> Option<&'static str> {
    Some(if is_integer { "1" } else { "0.01" })
}

/// Represents an empty number-field draft as `NaN`, avoiding JavaScript's empty-string-to-zero conversion.
fn number_field_value(input: &str) -> &str {
    if input.is_empty() { "NaN" } else { input }
}

/// Keeps the slider's numeric handle at the computed value while its inline draft is empty.
fn slider_field_value<'a>(input: &'a str, computed: &'a str) -> &'a str {
    if input.is_empty() { computed } else { input }
}

/// Preserves a blank numeric draft until its computed value changes, including across blur.
///
/// Nonempty drafts retain the existing focus-based synchronization policy.
/// - Complexity: O(n) in the computed value text lengths.
fn sync_input(is_numeric: bool, input: &str, focused: bool, previous: &str, current: &str) -> bool {
    if is_numeric && input.is_empty() {
        previous != current
    } else {
        !focused
    }
}

/// Reads a normalized numeric draft from a Spectrum field or a slider's editable field.
///
/// Consumes the localized `adamNumericDraft` installed by [`SpSlider`] once, preserving blanks
/// and partial edits. Slider dragging reads the canonical host value without locale normalization.
/// The script sends `1|value` while the host is focused and `0|value` otherwise.
/// - Complexity: O(n) in the field ID and edited text lengths.
///
/// # Examples
///
/// ```
/// let script = adam_web_ui::numeric_input_script("width", false);
/// // Pass script to Dioxus document::eval and receive one String payload.
/// ```
pub fn numeric_input_script(field_id: &str, slider: bool) -> String {
    format!(
        r#"
        {bridge}
        const host = document.getElementById({field_id:?});
        dioxus.send(readNumericInput(host, {slider}, document.activeElement === host));
        "#,
        bridge = include_str!("numeric_input.js"),
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
///
/// - Complexity: O(typed.len() + actual.len()) numeric parsing work.
fn clamped_away(typed: &str, actual: &str) -> bool {
    match typed.parse::<f64>() {
        Ok(v) => format_rounded(v) != actual,
        Err(_) => false,
    }
}

/// The source context needed to format an evaluation error into a rustc-style diagnostic:
/// the current source text and name, plus the parsed sheet whose span tables resolve an
/// `adam_rs::Error`'s sites back to their declaration locations.
#[derive(Clone, Copy)]
struct ErrorContext {
    source_text: Memo<String>,
    source_name: Memo<String>,
    parsed: Signal<ParsedSheet>,
}

/// Parses `val` for `id` via its `Labels` metadata, writes it to `parsed`, and propagates the
/// sheet's constraints, updating `has_error` and reporting any error — or, on success, any
/// currently-violated filter — to `crate::diagnostics`.
///
/// An empty numeric draft clears its parse error without writing or propagating; empty
/// nonnumeric values still pass through the cell's parser.
///
/// - Postcondition: `has_error` is `true` on parse or propagation failure, or when a range
///   filter clamped `val` away from what was typed (see [`clamped_away`]); `false` otherwise.
fn write_and_propagate(
    mut parsed: Signal<ParsedSheet>,
    labels: Signal<Labels>,
    id: CellId,
    val: &str,
    mut has_error: Signal<bool>,
    errors: ErrorContext,
) {
    let labels_r = labels.read();
    let Some(meta) = labels_r.cells.get(&id) else {
        return;
    };
    if meta.is_numeric && val.is_empty() {
        has_error.set(false);
        return;
    }
    let mut parsed_w = parsed.write();
    let write_result = (meta.write_str)(&mut parsed_w, val);
    drop(labels_r);
    let propagate_result = match write_result {
        Ok(()) => parsed_w.propagate(),
        Err(e) => Err(e),
    };
    match propagate_result {
        Ok(()) => {
            let labels_r = labels.read();
            let clamped = labels_r.cells.get(&id).is_some_and(|m| {
                if !m.is_numeric || m.range.is_none() {
                    return false;
                }
                let actual = (m.display)(&parsed_w);
                if m.is_integer {
                    match (val.parse::<i128>(), actual.parse::<i128>()) {
                        (Ok(typed), Ok(actual)) => typed != actual,
                        _ => match (val.parse::<u128>(), actual.parse::<u128>()) {
                            (Ok(typed), Ok(actual)) => typed != actual,
                            _ => false,
                        },
                    }
                } else {
                    clamped_away(val, &actual)
                }
            });
            has_error.set(clamped);
            for violated_id in parsed_w.filter_violated_cells().collect::<Vec<_>>() {
                let Some(violation) = parsed_w.filter_violation(violated_id) else {
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
            // Drop the write guard before reading `errors.parsed` (the same signal) again
            // below — `format_adam_error` needs `&ParsedSheet` to locate `e`'s sites, and
            // the write guard and a fresh read both borrowing the same signal at once would
            // panic.
            drop(parsed_w);
            has_error.set(true);
            crate::diagnostics::report_error(&format_adam_error(
                &e,
                &errors.parsed.read(),
                &errors.source_text.read(),
                &errors.source_name.read(),
                &Renderer::styled(),
            ));
        }
    }
}

/// Sidebar panel showing all cells with labels and inputs for writing — a checkbox for
/// `bool`-typed cells, a single editable slider (its label built in) for a numeric cell with a
/// range filter, a plain number field for any other numeric cell, and a text field for
/// everything else.
///
/// Editing an input immediately writes the parsed value to the sheet and propagates
/// constraints. If parsing or propagation fails (for example, non-numeric input or division
/// by zero), the field renders in its invalid state until the user blurs, and the formatted
/// diagnostic is reported through the renderer's diagnostic sink. A field's input is not reset while it is focused; it
/// syncs back to the computed value on blur, keeping non-edited cells up to date.
///
/// Each mounted source owns distinct DOM IDs and retires pending responses on replacement.
///
/// - Complexity: O(c) row construction for c labeled cells, plus output-status evaluation
///   and each row's formatting/rendering costs.
///
/// # Examples
///
/// Inside a Dioxus scope, `rsx! { SheetInspector { parsed, labels, source_text, source_name } }`
/// binds a parsed-sheet signal and its labels to source-diagnostic memos.
#[component]
pub fn SheetInspector(
    parsed: Signal<ParsedSheet>,
    labels: Signal<Labels>,
    source_text: Memo<String>,
    source_name: Memo<String>,
) -> Element {
    let source_owner = use_memo(move || {
        let _labels = labels.read();
        InputOwner(Rc::new(()))
    });
    let namespace = dom_id_namespace(&source_owner.read().0);
    let ids: Vec<CellId> = labels.read().cells.keys().copied().collect();
    let output_status = use_memo(move || compute_output_status(&parsed.read()));

    rsx! {
        div {
            style: "width: 260px; min-width: 260px; height: 100%; overflow-y: auto; padding: 12px; box-sizing: border-box;",
            SpHeading { "Cells" }
            SpDivider {}
            for id in ids {
                CellRow { key: "{namespace}-{id:?}", id, namespace, source_owner, parsed, labels, source_text, source_name, output_status }
            }
        }
    }
}

/// Renders one source-owned control and discards obsolete asynchronous input replies.
///
/// - Complexity: O(l + v + d) per render in label, displayed value, and diagnostic bytes,
///   plus the registered formatter's cost; edits additionally parse and propagate.
#[component]
fn CellRow(
    id: CellId,
    namespace: usize,
    source_owner: Memo<InputOwner>,
    parsed: Signal<ParsedSheet>,
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
        let s = parsed.read();
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
            .map(|f| f(&parsed.read()))
    });

    let is_integer = use_memo(move || {
        labels
            .read()
            .cells
            .get(&id)
            .map(|m| m.is_integer)
            .unwrap_or(false)
    });

    let forced = use_memo(move || parsed.read().is_forced(id));

    let mut input = use_signal(|| value.peek().clone());
    let mut previous_value = use_signal(|| value.peek().clone());
    let mut is_focused = use_signal(|| false);
    let mut has_error = use_signal(|| false);
    let mut revision = use_signal(|| 0u64);

    let errors = ErrorContext {
        source_text,
        source_name,
        parsed,
    };

    let flags =
        use_memo(move || cell_flags(id, *forced.read(), *has_error.read(), &output_status.read()));

    // `None` for a cell with no currently-failing labeled `require`, including every non-output
    // cell — set from `OutputStatus::invalid_output_requirement_names`, itself keyed by
    // `Sheet::violated_requirements`/`ParsedSheet::requirement_names`.
    let violated_requirement_names = use_memo(move || {
        output_status
            .read()
            .invalid_output_requirement_names
            .get(&id)
            .cloned()
    });

    // Only consumed by the slider branch below — `sp-slider` has no native `invalid` state,
    // so it needs the full message (filter violation or generic fallback), not just a
    // requirement name.
    let slider_message = use_memo(move || {
        slider_invalid_message(id, &label.read(), &parsed.read(), &output_status.read())
    });

    // Sync input to the computed value whenever it changes, but not while the user
    // is actively editing — that would interrupt mid-value typing (e.g. "1." → "1").
    use_effect(move || {
        let v = value.read().clone();
        if sync_input(
            *is_numeric.read(),
            &input.peek(),
            *is_focused.read(),
            &previous_value.peek(),
            &v,
        ) {
            input.set(v);
        }
        previous_value.set(value.peek().clone());
    });

    let field_id = format!("cell-{namespace}-{id:?}");

    rsx! {
        div {
            style: "margin-bottom: 8px;",
            if let Some((lo, hi)) = *range.read() {
                {
                    let slider_value = slider_field_value(&input.read(), &value.read()).to_string();
                    let (min, max) = slider_bounds(&slider_value, (lo, hi));
                    let step = slider_step(*is_integer.read()).map(str::to_string);
                    rsx! {
                        SpSlider {
                            id: field_id,
                            label: label.read().clone(),
                            value: slider_value,
                            min,
                            max,
                            step,
                            format_options: numeric_format_options(*is_integer.read()),
                            editable: true,
                            disabled: flags.read().disabled,
                            oninput: move |_: FormEvent| {
                                let ns = namespace;
                                let request_source = source_owner.peek().clone();
                                let request_revision = revision.peek().checked_add(1).expect("input revision overflow");
                                revision.set(request_revision);
                                spawn(async move {
                                    // Dragging needs no focus; inline typing reads its raw draft
                                    // instead of the slider's last committed numeric value.
                                    let mut eval = document::eval(&numeric_input_script(
                                        &format!("cell-{ns}-{id:?}"),
                                        true,
                                    ));
                                    let Ok(payload) = eval.recv::<String>().await else { return; };
                                    if !input_reply_is_current(&request_source, &source_owner.peek(), request_revision, *revision.peek()) { return; }
                                    let Some((focused, val)) = payload.split_once('|') else { return; };
                                    is_focused.set(focused == "1");
                                    input.set(val.to_string());
                                    write_and_propagate(parsed, labels, id, val, has_error, errors);
                                });
                            },
                            onfocus: move |_| is_focused.set(true),
                            onblur: move |_| {
                                let next = revision.peek().checked_add(1).expect("input revision overflow");
                                revision.set(next);
                                is_focused.set(false);
                                has_error.set(false);
                            },
                        }
                        // `sp-slider` has no `invalid` state or `negative-help-text` slot the
                        // way `sp-number-field` does (see `SpSlider`'s doc comment), so any
                        // violation on a range-filtered cell — a failing `require` on an out
                        // cell, or the cell's own filter rejecting its forced value (see
                        // `slider_invalid_message`) — is surfaced as a plain sibling
                        // `SpHelpText` instead of slotted content — its parent here is a plain
                        // `div`, not a shadow host, so `slot` has no effect; "negative-help-text"
                        // is passed anyway (rather than an empty string) to match the name used
                        // for the same purpose in the non-range branch below. `slider_message`
                        // is `None` exactly when `flags.invalid` would be `false` for a non-slider
                        // control (see `slider_invalid_message`'s postcondition), so this is
                        // already gated on invalidity without needing to check `flags` directly.
                        if let Some(msg) = slider_message.read().clone() {
                            SpHelpText { slot: "negative-help-text".to_string(), variant: "negative".to_string(), "{msg}" }
                        }
                    }
                }
            } else if *is_bool.read() {
                // `sp-checkbox`'s label is its own light-DOM child text, not a sibling
                // `SpFieldLabel` (which renders as a separate line above the checkbox,
                // disconnected from it) -- unlike the numeric/text fields below, which
                // have no such built-in label slot.
                SpCheckbox {
                    id: field_id,
                    checked: *value.read() == "true",
                    invalid: flags.read().invalid,
                    disabled: flags.read().disabled,
                    onclick: move |_| {
                        let next = toggled_bool_value(&value.peek());
                        write_and_propagate(parsed, labels, id, next, has_error, errors);
                        // `sp-checkbox` toggles its own shadow-DOM `checked` state
                        // natively in response to the click, before this handler runs
                        // and independent of the `checked` prop below. If the write above
                        // was rejected, `value` recomputes to the same string as before,
                        // so Dioxus's diff sees no change and never re-touches the DOM —
                        // leaving the visual checkbox desynced from the sheet. Force the
                        // element back to the actual committed value here.
                        let checked = *value.read() == "true";
                        let ns = namespace;
                        spawn(async move {
                            let _ = document::eval(&format!(
                                r#"document.getElementById("cell-{ns}-{id:?}").checked = {checked};"#
                            ))
                            .await;
                        });
                    },
                    "{label}"
                }
            } else {
                SpFieldLabel { for_: field_id.clone(), "{label}" }
                if *is_numeric.read() {
                    SpNumberfield {
                        id: field_id,
                        value: number_field_value(&input.read()).to_string(),
                        min: None,
                        max: None,
                        step: is_integer.read().then(|| "1".to_string()),
                        format_options: numeric_format_options(*is_integer.read()),
                        invalid: flags.read().invalid,
                        // An out cell's field is always `disabled` too (its cell is
                        // always `forced`, never a candidate write target), but
                        // `readonly` is what actually renders here — a disabled
                        // `sp-number-field` suppresses `invalid` styling, which would
                        // hide a failed `require`.
                        disabled: flags.read().disabled && !flags.read().readonly,
                        readonly: flags.read().readonly,
                        oninput: move |_: FormEvent| {
                            let ns = namespace;
                            let request_source = source_owner.peek().clone();
                            let request_revision = revision.peek().checked_add(1).expect("input revision overflow");
                            revision.set(request_revision);
                            spawn(async move {
                                // Reads the shadow-DOM `<input>`'s raw text, not the host's
                                // `value` property: once `min`/`max` are set, `sp-number-field`
                                // clamps its own `value` to that range on every keystroke,
                                // which would hide an out-of-range-for-type entry from
                                // `write_and_propagate` below before it ever sees the digits
                                // the user actually typed.
                                //
                                // `sp-number-field` displays (and lets the user type) numbers
                                // grouped and decimal-marked per the resolved locale — e.g.
                                // `"1,920"` in `en`, `"1.920,5"` in `de`. `el.numberFormatter`
                                // is the exact `Intl.NumberFormat` the element itself renders
                                // with, so reading its group/decimal separators here (rather
                                // than assuming `,`/`.`) and normalizing the raw text to a
                                // plain `.`-decimal, ungrouped string keeps this locale-aware
                                // for any resolved locale. Rust only ever sees that normalized
                                // form, both for `write_and_propagate` below and for the
                                // `value` this component's `value` prop round-trips back —
                                // `sp-number-field`'s host `value` is a plain JS `Number`
                                // property, so echoing back ungrouped text is required to
                                // avoid the element itself parsing it as `NaN`.
                                //
                                // The stepper buttons (and scroll-wheel/arrow-key stepping)
                                // change `value` and fire this same `input` event without ever
                                // focusing the field the way Dioxus's `onfocus` can observe:
                                // `sp-number-field::stepBy()` calls the DOM `.focus()` method
                                // directly on itself, which moves `document.activeElement` but
                                // — unlike focus arriving from an actual pointer/keyboard
                                // interaction with the field — never dispatches a `focus` event
                                // Dioxus's delegated listener sees, so `is_focused` stays stuck
                                // at `false` and every stepper click was silently dropped below.
                                // Reading `document.activeElement` fresh here, instead of
                                // trusting the `is_focused` signal, sidesteps that gap while
                                // still discarding a response that arrives after a genuine
                                // blur-while-in-flight (`activeElement` has moved on by then).
                                let mut eval = document::eval(&numeric_input_script(
                                    &format!("cell-{ns}-{id:?}"),
                                    false,
                                ));
                                let Ok(payload) = eval.recv::<String>().await else { return; };
                                if !input_reply_is_current(&request_source, &source_owner.peek(), request_revision, *revision.peek()) { return; }
                                let Some((still_focused, val)) = payload.split_once('|') else { return; };
                                if still_focused != "1" {
                                    return;
                                }
                                is_focused.set(true);
                                input.set(val.to_string());
                                write_and_propagate(parsed, labels, id, val, has_error, errors);
                            });
                        },
                        onfocus: move |_| is_focused.set(true),
                        onblur: move |_| {
                            let next = revision.peek().checked_add(1).expect("input revision overflow");
                            revision.set(next);
                            is_focused.set(false);
                            has_error.set(false);
                        },
                        // `sp-number-field` only exposes its `negative-help-text` slot
                        // while its own `invalid` prop is `true` (SWC's `HelpTextManager`
                        // renders `<slot name="negative-help-text">` vs. a discarded
                        // pass-through slot based on exactly that), so this always mounts
                        // when there's a name to show — never independently gated on
                        // `invalid` here, since `violated_requirement_names` is already
                        // empty whenever `invalid` is false for an output cell (see
                        // `OutputStatus::invalid_output_requirement_names`). Names the
                        // `require` currently failing on this out cell — a stopgap: a real
                        // message (see `Requirement::from_fn_*`'s own
                        // `require { @name expression; }` source) would need the sheet to
                        // carry more than just a name, so this just surfaces the name a
                        // sheet author already chose.
                        if let Some(names) = violated_requirement_names.read().clone() {
                            SpHelpText { slot: "negative-help-text".to_string(), variant: "negative".to_string(), "{names}" }
                        }
                    }
                } else {
                    SpTextfield {
                        id: field_id,
                        value: input.read().clone(),
                        invalid: flags.read().invalid,
                        disabled: flags.read().disabled,
                        // Dioxus's event serializer only reads event.target.value for
                        // HTMLInputElement — custom elements (sp-textfield) always give "".
                        // Use dioxus.send() in JS and eval.recv() to read the live value.
                        oninput: move |_: FormEvent| {
                            let ns = namespace;
                            let request_source = source_owner.peek().clone();
                            let request_revision = revision.peek().checked_add(1).expect("input revision overflow");
                            revision.set(request_revision);
                            spawn(async move {
                                let mut eval = document::eval(&format!(
                                    r#"dioxus.send(document.getElementById("cell-{ns}-{id:?}").value)"#
                                ));
                                let Ok(val) = eval.recv::<String>().await else { return; };
                                if !input_reply_is_current(&request_source, &source_owner.peek(), request_revision, *revision.peek()) { return; }
                                // Discard the result if the user blurred while the round-trip was
                                // in flight; blur already cleared the error and use_effect will
                                // restore the last valid computed value.
                                if !*is_focused.read() {
                                    return;
                                }
                                input.set(val.clone());
                                write_and_propagate(parsed, labels, id, &val, has_error, errors);
                            });
                        },
                        onfocus: move |_| is_focused.set(true),
                        onblur: move |_| {
                            let next = revision.peek().checked_add(1).expect("input revision overflow");
                            revision.set(next);
                            is_focused.set(false);
                            has_error.set(false);
                        },
                    }
                }
            }
        }
        SpDivider {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies blank numeric drafts stay blank through blur and unrelated recomputation.
    #[test]
    fn sync_input_preserves_blank_numeric_drafts() {
        for focused in [false, true] {
            assert!(!sync_input(true, "", focused, "51", "51"));
            assert!(sync_input(true, "", focused, "51", "10"));
        }
    }

    /// Verifies nonempty and nonnumeric drafts retain focus-based synchronization.
    #[test]
    fn sync_input_preserves_existing_editing_behavior() {
        for numeric in [false, true] {
            assert!(!sync_input(numeric, "1.", true, "1", "2"));
            assert!(sync_input(numeric, "1.", false, "1", "2"));
        }
        assert!(sync_input(false, "", false, "old", "old"));
        assert!(!sync_input(false, "", true, "old", "new"));
    }

    /// Verifies empty numeric drafts do not become JavaScript zero values.
    #[test]
    fn number_field_value_preserves_empty_and_nonempty_drafts() {
        assert_eq!(number_field_value(""), "NaN");
        for value in ["0", "5", "-1.5", "1."] {
            assert_eq!(number_field_value(value), value);
        }
    }

    /// Verifies clearing a slider draft preserves its handle and the bounds that contain it.
    #[test]
    fn slider_field_value_preserves_computed_value_for_empty_drafts() {
        assert_eq!(slider_field_value("", "51"), "51");
        assert_eq!(
            slider_bounds(slider_field_value("", "51"), (0.0, 10.0)),
            ("0".to_string(), "51".to_string())
        );
        assert_eq!(slider_field_value("5", "51"), "5");
    }

    thread_local! {
        /// Records whether the live-write fixture completed its assertions outside Dioxus's error boundary.
        static EMPTY_WRITES_CHECKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Exercises empty writes for numeric cells and a string cell through the live write path.
    ///
    /// - Complexity: One sheet parse and three writes.
    fn empty_numeric_write_app() -> Element {
        let source = r#"sheet s { cell float: f64 = 51.0; cell integer: i32 = 12; cell text: String = "old"; }"#;
        let sheet = parse(source);
        let metadata = crate::labels::labels_from_cell_names(&sheet, &sheet.cell_names);
        let parsed = use_signal(move || sheet);
        let labels = use_signal(move || metadata);
        let mut has_error = use_signal(|| true);
        let source_text = use_memo(move || source.to_string());
        let source_name = use_memo(|| "empty-edit.adm2".to_string());
        use_hook(move || {
            for name in ["float", "integer", "text"] {
                has_error.set(true);
                let id = parsed.read().cell_names[name].0;
                write_and_propagate(
                    parsed,
                    labels,
                    id,
                    "",
                    has_error,
                    ErrorContext {
                        source_text,
                        source_name,
                        parsed,
                    },
                );
                assert!(
                    !*has_error.read(),
                    "empty {name} edit must not report a parse error"
                );
            }
            let sheet = parsed.read();
            assert_eq!(
                *sheet.read::<f64>(sheet.cell_names["float"].0).unwrap(),
                51.0
            );
            assert_eq!(
                *sheet.read::<i32>(sheet.cell_names["integer"].0).unwrap(),
                12
            );
            assert_eq!(
                sheet.read::<String>(sheet.cell_names["text"].0).unwrap(),
                ""
            );
            EMPTY_WRITES_CHECKED.set(true);
        });
        rsx! { div {} }
    }

    /// Verifies blank numeric edits leave stored values unchanged while empty strings remain writable.
    #[test]
    fn empty_numeric_writes_preserve_sheet_values() {
        EMPTY_WRITES_CHECKED.set(false);
        VirtualDom::new(empty_numeric_write_app).rebuild_in_place();
        assert!(
            EMPTY_WRITES_CHECKED.get(),
            "live-write fixture must complete every assertion"
        );
    }

    /// Verifies that both floating-point types configure fractional slider increments.
    #[test]
    fn slider_step_configures_hundredths_for_floats() {
        let parsed = parse("sheet s { cell a: f32 = 3.0_f32; cell b: f64 = 25.5; }");
        let labels = crate::labels::labels_from_cell_names(&parsed, &parsed.cell_names);
        assert_eq!(labels.cells.len(), 2);
        for meta in labels.cells.values() {
            assert_eq!(slider_step(meta.is_integer), Some("0.01"));
        }
    }

    /// Verifies that integer cells retain whole-number slider increments.
    #[test]
    fn slider_step_preserves_whole_numbers_for_integers() {
        let parsed = parse("sheet s { cell a: i32 = 51; cell b: u32 = 26_u32; }");
        let labels = crate::labels::labels_from_cell_names(&parsed, &parsed.cell_names);
        assert_eq!(labels.cells.len(), 2);
        for meta in labels.cells.values() {
            assert_eq!(slider_step(meta.is_integer), Some("1"));
        }
    }

    /// Verifies that both floating-point types request exactly two fractional digits.
    #[test]
    fn numeric_format_options_sets_two_fractional_digits_for_floats() {
        let parsed = parse("sheet s { cell a: f32 = 3.0_f32; cell b: f64 = 1.234; }");
        let labels = crate::labels::labels_from_cell_names(&parsed, &parsed.cell_names);
        assert_eq!(labels.cells.len(), 2);
        for meta in labels.cells.values() {
            let options: serde_json::Value =
                serde_json::from_str(numeric_format_options(meta.is_integer).unwrap()).unwrap();
            assert_eq!(options["minimumFractionDigits"], 2);
            assert_eq!(options["maximumFractionDigits"], 2);
        }
    }

    /// Verifies that integer widgets keep Spectrum's existing formatting.
    #[test]
    fn numeric_format_options_preserves_integer_defaults() {
        assert_eq!(numeric_format_options(true), None);
    }

    /// Verifies the composed example preserves its starting aspect and both edit directions when locked.
    #[test]
    fn composed_example_lock_preserves_aspect_ratio() {
        let mut parsed = parse(include_str!("../../adam-slides/examples/three_way.adm2"));
        let lock = parsed.cell_names["lock"].0;
        let width = parsed.cell_names["width_inches"].0;
        let height = parsed.cell_names["height_inches"].0;
        parsed.propagate().unwrap();
        parsed.write(lock, true).unwrap();
        parsed.propagate().unwrap();
        assert!((*parsed.read::<f64>(width).unwrap() - 3.0).abs() < 1e-10);
        assert!((*parsed.read::<f64>(height).unwrap() - 5.0).abs() < 1e-10);
        parsed.write(width, 6.0_f64).unwrap();
        parsed.propagate().unwrap();
        assert!((*parsed.read::<f64>(height).unwrap() - 10.0).abs() < 1e-10);
        parsed.write(height, 15.0_f64).unwrap();
        parsed.propagate().unwrap();
        assert!((*parsed.read::<f64>(width).unwrap() - 9.0).abs() < 1e-10);
    }

    /// Parses a valid test sheet with the standard type registry.
    ///
    /// - Complexity: The parser's source compilation cost.
    fn parse(source: &str) -> ParsedSheet {
        adam_lang::AdamParser::new(adam_lang::TypeRegistry::new(), cel_parser::OpLookup::new())
            .parse_str(source)
            .unwrap()
    }

    /// Wraps a runtime-only fixture without attaching display metadata.
    fn parsed_with_sheet(sheet: Sheet) -> ParsedSheet {
        let mut parsed = parse("sheet s {}");
        parsed.sheet = sheet;
        parsed
    }

    /// Builds a real parsed binding with a native failing method at the same registered IDs.
    ///
    /// The native method has no CEL `SpanContext`, so diagnostics must use the parser's
    /// actual binding span rather than expression error context.
    ///
    /// - Precondition: `source` declares exactly two i32 cells and one identity binding.
    /// - Complexity: The parser's source compilation cost.
    fn live_write_fixture(source: &str, mismatch: bool) -> (ParsedSheet, Labels, CellId) {
        use adam_rs::Method;
        use std::any::TypeId;

        let mut parsed = parse(source);
        debug_assert_eq!(parsed.cell_names.len(), 2);
        debug_assert_eq!(parsed.method_spans.len(), 1);
        let mut sheet = Sheet::new();
        let input = sheet.add_cell(0_i32);
        let output = sheet.add_cell(0_i32);
        assert_eq!(input, parsed.cell_names[0].0);
        assert_eq!(output, parsed.cell_names[1].0);
        let relationship = sheet
            .add_relationship(vec![Method::new(
                vec![input],
                vec![output],
                vec![TypeId::of::<i32>()],
                vec![TypeId::of::<i32>()],
                move |_| {
                    if mismatch {
                        Ok(vec![Box::new(true)])
                    } else {
                        Err(anyhow::anyhow!("live method failure"))
                    }
                },
            )])
            .unwrap();
        assert!(parsed.method_spans.contains_key(&(relationship, 0)));
        parsed.sheet = sheet;
        let mut labels = Labels::new();
        labels.add_cell::<i32>(input, parsed.cell_names.get_index(0).unwrap().0);
        (parsed, labels, input)
    }

    /// Exercises live writes before and after replacing the parsed sheet in one signal.
    ///
    /// - Complexity: Two parses and two propagations.
    fn live_write_app() -> Element {
        let mismatch = std::env::var("ADAM_WEB_UI_LIVE_WRITE_CHILD").unwrap() == "type";
        let original = "sheet s {\ncell trigger: i32;\ncell failed: i32;\nrelationship {\nfailed := trigger;\n}\n}";
        let replacement = "sheet replacement {\n\ncell edited: i32;\ncell fresh: i32;\n\nrelationship {\nfresh := edited;\n}\n}";
        let (initial_parsed, initial_labels, _) = live_write_fixture(original, mismatch);
        let mut parsed = use_signal(move || initial_parsed);
        let mut labels = use_signal(move || initial_labels);
        let has_error = use_signal(|| false);
        let source_text = use_memo(move || original.to_string());
        let source_name = use_memo(|| "original.adm2".to_string());
        let fresh_text = use_memo(move || replacement.to_string());
        let fresh_name = use_memo(|| "replacement.adm2".to_string());
        use_hook(move || {
            let id = parsed.read().cell_names["trigger"].0;
            write_and_propagate(
                parsed,
                labels,
                id,
                "7",
                has_error,
                ErrorContext {
                    source_text,
                    source_name,
                    parsed,
                },
            );
            assert!(*has_error.read());
            assert_eq!(*parsed.read().read::<i32>(id).unwrap(), 7);
            let (fresh, fresh_labels, id) = live_write_fixture(replacement, mismatch);
            parsed.set(fresh);
            labels.set(fresh_labels);
            write_and_propagate(
                parsed,
                labels,
                id,
                "9",
                has_error,
                ErrorContext {
                    source_text: fresh_text,
                    source_name: fresh_name,
                    parsed,
                },
            );
            assert!(*has_error.read());
            assert_eq!(*parsed.read().read::<i32>(id).unwrap(), 9);
        });
        rsx! { div {} }
    }

    #[test]
    /// Verifies the real live-write error channel retains filenames and fresh binding spans.
    fn live_write_errors_use_current_parsed_sheet_spans() {
        if std::env::var_os("ADAM_WEB_UI_LIVE_WRITE_CHILD").is_some() {
            let mut dom = VirtualDom::new(live_write_app);
            dom.rebuild_in_place();
            return;
        }
        for (kind, error) in [("method", "live method failure"), ("type", "type mismatch")] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "inspector::tests::live_write_errors_use_current_parsed_sheet_spans",
                    "--nocapture",
                ])
                .env("ADAM_WEB_UI_LIVE_WRITE_CHILD", kind)
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(output.status.success(), "{kind}: {stderr}");
            for expected in [
                error,
                "original.adm2:5",
                "failed := trigger;",
                "replacement.adm2:7",
                "fresh := edited;",
            ] {
                assert!(stderr.contains(expected), "missing {expected}: {stderr}");
            }
        }
    }

    /// Builds status sets for pure control-policy tests.
    ///
    /// - Complexity: expected O(total supplied IDs) time and space.
    fn status(
        has_outputs: bool,
        relevant: &[CellId],
        invalid_contributors: &[CellId],
        invalid_outputs: &[CellId],
    ) -> OutputStatus {
        status_with_filter_violated(
            has_outputs,
            relevant,
            invalid_contributors,
            invalid_outputs,
            &[],
        )
    }

    /// Builds status sets including explicit filter violations.
    ///
    /// - Complexity: expected O(total supplied IDs) time and space.
    fn status_with_filter_violated(
        has_outputs: bool,
        relevant: &[CellId],
        invalid_contributors: &[CellId],
        invalid_outputs: &[CellId],
        filter_violated: &[CellId],
    ) -> OutputStatus {
        status_full(
            has_outputs,
            relevant,
            invalid_contributors,
            invalid_outputs,
            filter_violated,
            &[],
            &[],
        )
    }

    /// Builds all supplied status sets without evaluating another policy implementation.
    ///
    /// - Complexity: expected O(total supplied IDs) time and space.
    fn status_full(
        has_outputs: bool,
        relevant: &[CellId],
        invalid_contributors: &[CellId],
        invalid_outputs: &[CellId],
        filter_violated: &[CellId],
        output_cells: &[CellId],
        invalid_output_requirement_names: &[(CellId, &str)],
    ) -> OutputStatus {
        OutputStatus {
            has_outputs,
            relevant: relevant.iter().copied().collect(),
            invalid_contributors: invalid_contributors.iter().copied().collect(),
            invalid_outputs: invalid_outputs.iter().copied().collect(),
            filter_violated: filter_violated.iter().copied().collect(),
            output_cells: output_cells.iter().copied().collect(),
            invalid_output_requirement_names: invalid_output_requirement_names
                .iter()
                .map(|&(id, name)| (id, name.to_string()))
                .collect(),
        }
    }

    #[test]
    /// Status identifies the cell whose own filter rejected its computed value.
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

        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.filter_violated.contains(&a));
        assert!(status.invalid_contributors.contains(&b));
    }

    #[test]
    /// Satisfied filters leave the violation set empty.
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

        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.filter_violated.is_empty());
        assert!(status.invalid_contributors.is_empty());
    }

    #[test]
    /// Output classification includes the output cell itself.
    fn compute_output_status_output_cells_contains_the_outputs_own_cell() {
        use adam_rs::Method;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(0_i32);
        let result = sheet.add_cell(0_i32);
        sheet
            .add_out(Method::from_fn_1_1(a, result, |x: &i32| Ok(*x)), vec![])
            .unwrap();

        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.output_cells.contains(&result));
        assert!(!status.output_cells.contains(&a));
    }

    #[test]
    /// Sheets without outputs produce no output-cell classifications.
    fn compute_output_status_output_cells_empty_when_no_outputs() {
        let sheet = Sheet::new();
        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.output_cells.is_empty());
    }

    #[test]
    /// Named failing requirements appear in output diagnostics.
    fn compute_output_status_invalid_output_requirement_names_names_the_failing_requirement() {
        let mut parsed = parse(
            "sheet s { cell input: i32 = 2000; out area := input require { @not_too_big area <= 300i32; }; }",
        );
        parsed.propagate().unwrap();
        let area = parsed.output_names["area"];
        let status = compute_output_status(&parsed);
        assert_eq!(
            status.invalid_output_requirement_names.get(&area),
            Some(&"not_too_big".to_string())
        );
    }

    #[test]
    /// Satisfied requirements produce no failure names.
    fn compute_output_status_invalid_output_requirement_names_empty_when_requirement_holds() {
        let mut parsed = parse(
            "sheet s { cell input: i32 = 200; out area := input require { @not_too_big area <= 300i32; }; }",
        );
        parsed.propagate().unwrap();
        let status = compute_output_status(&parsed);
        assert!(status.invalid_output_requirement_names.is_empty());
    }

    #[test]
    /// Diagnostics join failing named requirements without including passing ones.
    fn compute_output_status_invalid_output_requirement_names_joins_multiple_violated_names() {
        let mut parsed = parse(
            "sheet s { cell input: i32 = 101; out result := input require {
                @too_big result <= 10i32;
                result < 0i32;
                @passing result > 0i32;
                @not_even result % 2i32 == 0i32;
            }; }",
        );
        parsed.propagate().unwrap();
        let result = parsed.output_names["result"];
        let status = compute_output_status(&parsed);
        let names = status
            .invalid_output_requirement_names
            .get(&result)
            .expect("result should have violated requirements");
        assert_eq!(names, "too_big, not_even");
    }

    #[test]
    /// Marks unnamed failures invalid without manufacturing a display label.
    fn compute_output_status_unnamed_failure_retains_generic_invalid_message() {
        let mut parsed = parse(
            "sheet s { cell input: i32 = 1; out result := input require { result < 0i32; result > 9i32; }; }",
        );
        parsed.propagate().unwrap();
        let result = parsed.output_names["result"];
        let status = compute_output_status(&parsed);
        assert!(status.invalid_outputs.contains(&result));
        assert!(
            !status
                .invalid_output_requirement_names
                .contains_key(&result)
        );
        assert_eq!(
            slider_invalid_message(result, "result", &parsed, &status),
            Some("`result` is invalid".to_string())
        );
    }

    #[test]
    /// Requirement classification also covers ordinary cells with requirements.
    fn compute_output_status_covers_a_plain_cells_requirement_violation() {
        use adam_rs::Requirement;

        // `add_requirement` hard-fails immediately if a plain/source cell's *current*
        // value already violates it, so the requirement is attached while it still
        // holds (200 > 100), then a later `write` — not the initial value — is what
        // drives it into violation.
        let mut sheet = Sheet::new();
        let a = sheet.add_cell(200_i32);
        sheet
            .add_requirement(a, Requirement::from_fn_1(a, |x: &i32| Ok(*x > 100)))
            .unwrap();
        sheet.write(a, 5_i32).unwrap();
        sheet.propagate().unwrap();

        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.invalid_contributors.contains(&a));
    }

    #[test]
    /// Output contributors remain relevant even when the output has no requirements.
    fn compute_output_status_relevant_includes_an_out_cells_contributor_even_without_requirements()
    {
        // Regression test for the `relevant`-set narrowing: `requirement_relevant_cells()`
        // alone only covers cells feeding a requirement, so an out cell with zero
        // requirements would otherwise silently drop out of "relevant" even though it's
        // still an out cell whose inputs the user should see as meaningful.
        use adam_rs::Method;

        let mut sheet = Sheet::new();
        let a = sheet.add_cell(5_i32);
        let result = sheet.add_cell(0_i32);
        sheet
            .add_out(Method::from_fn_1_1(a, result, |x: &i32| Ok(*x)), vec![])
            .unwrap();
        sheet.propagate().unwrap();

        let status = compute_output_status(&parsed_with_sheet(sheet));
        assert!(status.relevant.contains(&a));
    }

    #[test]
    /// Nonconformance diagnostics identify the affected cell.
    fn format_filter_violation_not_conformed_names_the_cell() {
        let msg = format_filter_violation("a", &FilterViolation::NotConformed);
        assert!(msg.contains('a'));
        assert!(msg.contains("does not conform"));
    }

    #[test]
    /// Filter-failure diagnostics retain the underlying error message.
    fn format_filter_violation_failed_includes_the_underlying_error() {
        let msg = format_filter_violation(
            "a",
            &FilterViolation::Failed(anyhow::anyhow!("out of range")),
        );
        assert!(msg.contains('a'));
        assert!(msg.contains("out of range"));
    }

    /// Returns an isolated cell key for pure identity-based policy tests.
    fn dummy_cell() -> CellId {
        let mut sheet = Sheet::new();
        sheet.add_cell(0_i32)
    }

    #[test]
    /// Slider diagnostics prefer a named failing requirement.
    fn slider_invalid_message_prefers_a_requirement_name_when_present() {
        let sheet = Sheet::new();
        let id = dummy_cell();
        let status = status_full(true, &[], &[], &[], &[], &[], &[(id, "not_too_big")]);
        assert_eq!(
            slider_invalid_message(id, "a", &sheet, &status),
            Some("not_too_big".to_string())
        );
    }

    #[test]
    /// Slider diagnostics report filter violations when no named requirement takes precedence.
    fn slider_invalid_message_reports_a_filter_violation_when_no_requirement_name_applies() {
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

        let status = status(false, &[], &[], &[]);
        let msg = slider_invalid_message(a, "a", &sheet, &status).unwrap();
        assert!(msg.contains("does not conform"));
    }

    #[test]
    /// Invalid contributors receive a generic slider diagnostic.
    fn slider_invalid_message_falls_back_to_a_generic_message_for_a_mere_contributor() {
        let sheet = Sheet::new();
        let id = dummy_cell();
        let status = status(true, &[], &[id], &[]);
        assert_eq!(
            slider_invalid_message(id, "b", &sheet, &status),
            Some("`b` is invalid".to_string())
        );
    }

    #[test]
    /// Unnamed output failures receive a generic slider diagnostic.
    fn slider_invalid_message_falls_back_to_a_generic_message_for_an_unnamed_invalid_output() {
        let sheet = Sheet::new();
        let id = dummy_cell();
        let status = status(true, &[], &[], &[id]);
        assert_eq!(
            slider_invalid_message(id, "c", &sheet, &status),
            Some("`c` is invalid".to_string())
        );
    }

    #[test]
    /// Valid slider cells have no diagnostic message.
    fn slider_invalid_message_none_when_nothing_flags_the_cell() {
        let sheet = Sheet::new();
        let id = dummy_cell();
        let status = status(false, &[], &[], &[]);
        assert_eq!(slider_invalid_message(id, "d", &sheet, &status), None);
    }

    #[test]
    /// Sheets without outputs do not disable otherwise editable irrelevant cells.
    fn cell_flags_enabled_when_no_outputs_even_if_not_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(false, &[], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    /// Forced cells remain disabled independently of output presence.
    fn cell_flags_disabled_when_forced_regardless_of_outputs() {
        let id = dummy_cell();
        let flags = cell_flags(id, true, false, &status(false, &[], &[], &[]));
        assert!(flags.disabled);
    }

    #[test]
    /// Cells irrelevant to existing outputs are disabled.
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
    /// Relevant nonforced cells remain editable.
    fn cell_flags_enabled_when_has_outputs_and_cell_is_relevant() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[]));
        assert!(!flags.disabled);
    }

    #[test]
    /// Local input errors mark the control invalid.
    fn cell_flags_invalid_when_has_error() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, true, &status(false, &[], &[], &[]));
        assert!(flags.invalid);
    }

    #[test]
    /// Invalid outputs retain their invalid control state.
    fn cell_flags_invalid_when_cell_is_an_invalid_output() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[], &[id]));
        assert!(flags.invalid);
    }

    #[test]
    /// Filter violations mark the affected control invalid.
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
    /// Output cells render read-only controls.
    fn cell_flags_readonly_when_cell_is_an_output_cell() {
        let id = dummy_cell();
        let flags = cell_flags(
            id,
            true,
            false,
            &status_full(true, &[], &[], &[], &[], &[id], &[]),
        );
        assert!(flags.readonly);
    }

    #[test]
    /// Nonoutput cells do not inherit output-only read-only state.
    fn cell_flags_not_readonly_when_cell_is_not_an_output_cell() {
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(false, &[], &[], &[]));
        assert!(!flags.readonly);
    }

    #[test]
    /// Read-only outputs retain invalid state when their requirements fail.
    fn cell_flags_readonly_and_invalid_together_when_an_output_cell_fails_its_requirement() {
        let id = dummy_cell();
        let flags = cell_flags(
            id,
            true,
            false,
            &status_full(true, &[], &[], &[id], &[], &[id], &[]),
        );
        assert!(flags.readonly);
        assert!(flags.invalid);
    }

    #[test]
    /// Contributors implicated in invalid outputs are marked invalid.
    fn cell_flags_invalid_when_cell_is_an_invalid_contributor() {
        // A cell that merely contributes to a failing out-cell condition renders
        // `invalid` too — Spectrum has no softer "implicated but not itself invalid"
        // state to reach for instead.
        let id = dummy_cell();
        let flags = cell_flags(id, false, false, &status(true, &[id], &[id], &[]));
        assert!(flags.invalid);
    }

    #[test]
    /// Matching typed and displayed values do not imply clamping.
    fn clamped_away_false_when_typed_matches_actual() {
        assert!(!clamped_away("42", "42"));
    }

    #[test]
    /// Different numeric values reveal a range-clamped edit.
    fn clamped_away_true_when_a_range_filter_clamped_the_value() {
        assert!(clamped_away("150", "100"));
    }

    #[test]
    /// Display rounding alone does not imply clamping.
    fn clamped_away_ignores_display_rounding_differences() {
        assert!(!clamped_away("1.005", &format_rounded(1.005)));
    }

    #[test]
    /// Nonnumeric text does not produce a numeric clamp diagnostic.
    fn clamped_away_false_for_non_numeric_input() {
        assert!(!clamped_away("not a number", "0"));
    }

    /// String and integer writes retain their exact semantics.
    #[test]
    fn successful_nonfloat_writes_do_not_look_clamped() {
        /// Edits a real sheet and reports its field error flag.
        ///
        /// - Complexity: source build/propagation plus draft parsing and edited-sheet propagation.
        fn edit(source: &str, value: &str) -> bool {
            let source = source.to_string();
            let value = value.to_string();
            /// Runs one write through the framework-owned signals.
            fn host(
                (source, value, result): (String, String, std::rc::Rc<std::cell::Cell<bool>>),
            ) -> Element {
                let outcome = crate::build_sheet(&source, "test.adm2", &Renderer::plain());
                let (sheet, labels) = outcome.sheet_labels.unwrap();
                let id = sheet.cell_names["x"].0;
                let parsed = use_signal(|| sheet);
                let labels = use_signal(|| labels);
                let flag = use_signal(|| false);
                let source_text = use_memo(move || source.clone());
                let source_name = use_memo(|| "test.adm2".to_string());
                write_and_propagate(
                    parsed,
                    labels,
                    id,
                    &value,
                    flag,
                    ErrorContext {
                        source_text,
                        source_name,
                        parsed,
                    },
                );
                result.set(*flag.read());
                rsx! {}
            }
            let result = std::rc::Rc::new(std::cell::Cell::new(false));
            let mut dom = VirtualDom::new_with_props(host, (source, value, result.clone()));
            dom.rebuild_in_place();
            result.get()
        }
        assert!(!edit("sheet s { cell x: String; }", "001"));
        assert!(!edit("sheet s { cell x: i64; }", "9007199254740993"));
    }

    /// Old source owners and out-of-order replies cannot authorize writes.
    #[test]
    fn input_reply_requires_current_source_and_revision() {
        let source = InputOwner(Rc::new(()));
        let replacement = InputOwner(Rc::new(()));
        assert!(input_reply_is_current(&source, &source, 2, 2));
        assert!(!input_reply_is_current(&source, &replacement, 2, 2));
        assert!(!input_reply_is_current(&source, &source, 1, 2));
        assert!(!input_reply_is_current(&source, &source, 2, 3));
    }

    /// Fixture relevance follows real Inspector policy, including all conditional inputs.
    #[test]
    fn image_resize_relevance_survives_strength_changes() {
        let source = include_str!("../../begin/examples/image_resize.adm2");
        let outcome = crate::build_sheet(source, "image_resize.adm2", &Renderer::plain());
        assert!(outcome.error.is_none(), "{:?}", outcome.error);
        let (mut parsed, _) = outcome.sheet_labels.unwrap();
        for width in [1600i32, 1920i32] {
            let id = parsed.cell_names["width_pixels"].0;
            parsed.write(id, width).unwrap();
            parsed.propagate().unwrap();
            let relevant = compute_output_status(&parsed).relevant;
            for name in [
                "width_pixels",
                "width_pixels_percent",
                "width_inches",
                "resolution",
                "original_width_pixels",
                "resample",
                "constrain",
                "auto_quality",
                "screen_lpi",
            ] {
                assert!(relevant.contains(&parsed.cell_names[name].0), "{name}");
            }
        }
    }

    /// Two independently mounted inspectors never share control identities.
    #[test]
    fn same_source_inspectors_have_distinct_control_ids() {
        /// Renders the public inspector with a constant diagnostic name.
        fn inspector_host() -> Element {
            let mut pair = use_signal(|| {
                crate::build_sheet("sheet s { cell x: i64; }", "same.adm2", &Renderer::plain())
                    .sheet_labels
            });
            let parsed = use_signal(|| pair.write().take().unwrap().0);
            let labels = use_signal(|| {
                crate::build_sheet("sheet s { cell x: i64; }", "same.adm2", &Renderer::plain())
                    .sheet_labels
                    .unwrap()
                    .1
            });
            let source_text = use_memo(|| "sheet s { cell x: i64; }".to_string());
            let source_name = use_memo(|| "same.adm2".to_string());
            rsx! { SheetInspector { parsed, labels, source_text, source_name } }
        }
        let mut a = VirtualDom::new(inspector_host);
        let mut b = VirtualDom::new(inspector_host);
        a.rebuild_in_place();
        b.rebuild_in_place();
        let a = dioxus_ssr::render(&a);
        let b = dioxus_ssr::render(&b);
        let id_a = a
            .split("id=\"cell-")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let id_b = b
            .split("id=\"cell-")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_ne!(id_a, id_b);
        assert!(a.contains("sp-number-field"));
    }

    #[test]
    /// An in-range displayed value preserves slider bounds.
    fn slider_bounds_returns_range_unchanged_when_current_is_within_it() {
        assert_eq!(
            slider_bounds("50", (0.0, 100.0)),
            ("0".to_string(), "100".to_string())
        );
    }

    #[test]
    /// Slider display bounds include a value above the filter maximum.
    fn slider_bounds_widens_max_to_include_a_current_value_above_it() {
        assert_eq!(
            slider_bounds("150", (0.0, 100.0)),
            ("0".to_string(), "150".to_string())
        );
    }

    #[test]
    /// Slider display bounds include a value below the filter minimum.
    fn slider_bounds_widens_min_to_include_a_current_value_below_it() {
        assert_eq!(
            slider_bounds("-50", (0.0, 100.0)),
            ("-50".to_string(), "100".to_string())
        );
    }

    #[test]
    /// Unparseable displayed values preserve the filter's original bounds.
    fn slider_bounds_falls_back_to_the_unwidened_range_when_current_does_not_parse() {
        assert_eq!(
            slider_bounds("not a number", (0.0, 100.0)),
            ("0".to_string(), "100".to_string())
        );
    }

    #[test]
    /// Toggling true produces false.
    fn toggled_bool_value_true_becomes_false() {
        assert_eq!(toggled_bool_value("true"), "false");
    }

    #[test]
    /// Toggling false produces true.
    fn toggled_bool_value_false_becomes_true() {
        assert_eq!(toggled_bool_value("false"), "true");
    }

    #[test]
    /// A retained live owner has a stable DOM namespace.
    fn dom_id_namespace_is_stable_for_the_same_owner() {
        let owner = Rc::new(());
        assert_eq!(dom_id_namespace(&owner), dom_id_namespace(&owner.clone()));
    }

    #[test]
    /// Distinct live owners have distinct DOM namespaces.
    fn dom_id_namespace_differs_for_different_owners() {
        assert_ne!(
            dom_id_namespace(&Rc::new(())),
            dom_id_namespace(&Rc::new(()))
        );
    }

    #[test]
    /// Namespace formatting remains safe in generated DOM IDs.
    fn dom_id_namespace_contains_only_ascii_digits() {
        let ns = dom_id_namespace(&Rc::new(())).to_string();
        assert!(ns.chars().all(|c| c.is_ascii_digit()));
    }
}
