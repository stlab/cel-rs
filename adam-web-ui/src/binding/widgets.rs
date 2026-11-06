//! Adapts model-independent controls to exact, instance-owned cell bindings.
//!
//! Use [`EditNumber`], [`EditText`], [`CheckBox`] and [`EditSlider`] inside
//! [`crate::components::Theme`]. Controls report errors; only the host restores.
use super::*;
use crate::{
    components::{Checkbox, Input, Label, NumericEditor, NumericSlider},
    input::InputError,
};
use ::dioxus::prelude::*;
use dioxus_primitives::checkbox::CheckboxState;
use std::rc::Rc;

/// Selects an editor capability without consulting a cell's source direction.
#[derive(Clone, Copy, PartialEq)]
enum Editor {
    Number,
    Text,
    Bool,
}

/// Checks scalar capability and editor type, including read-only output displays.
fn check(view: &CellView, editor: Editor) -> Result<(), BindingError> {
    if !view.codec_supported {
        return Err(BindingError::UnsupportedCodec(view.key.clone()));
    }
    let compatible = match editor {
        Editor::Number => view.number_kind.is_some(),
        Editor::Bool => view.is_bool,
        Editor::Text => !view.is_bool && view.number_kind.is_none(),
    };
    if compatible {
        Ok(())
    } else {
        Err(BindingError::IncompatibleEditor(view.key.clone()))
    }
}

/// Renders and reports one explicit binding/configuration error per mounted error.
#[component]
fn ConfigurationError(bind: CellBinding, error: BindingError) -> Element {
    let message = error.to_string();
    use_effect(move || bind.report(HostRejection::Binding(error.clone())));
    rsx! { div { role: "alert", class: "adam-binding-error", "{message}" } }
}

/// Renders the checked failure without silently hiding an incorrectly configured control.
fn failure(bind: CellBinding, error: BindingError) -> Element {
    rsx! { ConfigurationError { key: "{bind.identity()}:{error}", bind, error } }
}

/// Returns a distinct stable ID for this mounted control, even for the same binding.
fn use_control_id(bind: &CellBinding) -> String {
    let allocation = use_hook(|| Rc::new(()));
    format!(
        "adam-cell-{}-{:p}",
        bind.identity(),
        Rc::as_ptr(&allocation)
    )
}

/// Binds an exact localized numeric editor to a named numeric cell.
///
/// Drafts and native composition remain owned by [`NumericEditor`]. Only
/// complete user aspirations submit; synchronization and blur never write.
///
/// - Complexity: Publication lookup plus numeric control transition costs.
///
/// # Examples
/// See [`use_sheet`] for `EditNumber { bind: sheet.cell("x") }`.
#[component]
pub fn EditNumber(
    bind: CellBinding,
    #[props(default)] label: String,
    #[props(default)] class: String,
    #[props(default)] style: String,
    #[props(default)] locale: Option<String>,
    #[props(default = "1".into())] step: String,
) -> Element {
    numeric(bind, label, class, style, locale, step, false)
}

/// Binds slider and exact numeric entry without clamping aspirations in the widget.
///
/// Range changes display the model value, not a locally clamped value.
///
/// - Complexity: Publication lookup plus numeric/slider transition costs.
///
/// # Examples
/// Inside a scope initialized by [`use_sheet`],
/// `rsx! { EditSlider { bind: sheet.cell("bounded") } }` uses its live range.
#[component]
pub fn EditSlider(
    bind: CellBinding,
    #[props(default)] label: String,
    #[props(default)] class: String,
    #[props(default)] style: String,
    #[props(default)] locale: Option<String>,
    #[props(default = "1".into())] step: String,
) -> Element {
    numeric(bind, label, class, style, locale, step, true)
}

/// Selects one checked numeric implementation and keys it by model installation.
///
/// - Complexity: O(named cells + label/style bytes) plus child rendering.
fn numeric(
    bind: CellBinding,
    label: String,
    class: String,
    style: String,
    locale: Option<String>,
    step: String,
    slider: bool,
) -> Element {
    let label = bind.label(label);
    let view = match bind
        .view()
        .and_then(|view| check(&view, Editor::Number).map(|_| view))
    {
        Ok(view) => view,
        Err(error) => return failure(bind, error),
    };
    // Dioxus reconciles keys in dynamic fragments, not a lone static child.
    rsx! { for (bind, label, class, style, locale, step) in
        std::iter::once((bind, label, class, style, locale, step)) {
        NumericBinding { key: "{bind.identity()}", bind, label, class, style, locale, step,
            slider, kind: view.number_kind.expect("checked numeric capability") }
    } }
}

/// Owns reactive controlled projections for exactly one numeric installation.
#[component]
fn NumericBinding(
    bind: CellBinding,
    label: String,
    class: String,
    style: String,
    locale: Option<String>,
    step: String,
    slider: bool,
    kind: crate::input::NumberKind,
) -> Element {
    let id = use_control_id(&bind);
    let label = bind.label(label);
    let tracked = bind.clone();
    let view = use_memo(move || tracked.view());
    let value = use_memo(move || view.read().as_ref().ok().map(|view| view.text.clone()));
    let range = use_memo(move || {
        view.read()
            .as_ref()
            .ok()
            .and_then(|view| view.range.clone())
    });
    let disabled = use_memo(move || {
        view.read()
            .as_ref()
            .map_or(true, |view| view.flags.disabled && !view.flags.readonly)
    });
    let readonly = use_memo(move || view.read().as_ref().is_ok_and(|view| view.flags.readonly));
    let invalid = use_memo(move || view.read().as_ref().map_or(true, |view| view.flags.invalid));
    let errors = bind.clone();
    let on_value = EventHandler::new(move |value: Option<String>| {
        if let Some(value) = value {
            bind.submit(value);
        }
    });
    let on_error =
        EventHandler::new(move |error: InputError| errors.report(HostRejection::Input(error)));
    rsx! { div { class, style,
        if slider {
            NumericSlider { id, label, value, range, kind, step, disabled, readonly, invalid,
                locale, optional: false, on_value, on_error }
        } else {
            NumericEditor { id, label, value, range, kind, step, disabled, readonly, invalid,
                locale, optional: false, on_value, on_error }
        }
    } }
}

/// Binds exact scalar text without numeric coercion or synchronization writes.
///
/// Conversion and type failures render/report explicitly. Programmatic updates
/// replace displayed text without submitting another transaction.
///
/// - Complexity: O(named cells + text/label/style bytes) per render or edit.
///
/// # Examples
/// Inside a scope initialized by [`use_sheet`],
/// `rsx! { EditText { bind: sheet.cell("title"), label: "Title" } }`.
#[component]
pub fn EditText(
    bind: CellBinding,
    #[props(default)] label: String,
    #[props(default)] class: String,
    #[props(default)] style: String,
) -> Element {
    let id = use_control_id(&bind);
    let label = bind.label(label);
    let view = match bind
        .view()
        .and_then(|view| check(&view, Editor::Text).map(|_| view))
    {
        Ok(view) => view,
        Err(error) => return failure(bind, error),
    };
    rsx! { div { class, style,
        Label { html_for: id.clone(), "{label}" }
        for bind in std::iter::once(bind) {
            Input { key: "{bind.identity()}", id: id.clone(), value: view.text.clone(),
                disabled: view.flags.disabled && !view.flags.readonly,
                readonly: view.flags.readonly, "aria-invalid": view.flags.invalid.to_string(),
                oninput: move |event: FormEvent| bind.submit(event.value()),
            }
        }
    } }
}

/// Binds a controlled Boolean without guessing its value from a click or stale display.
///
/// Rejected edits retire their model installation; explicit host restoration
/// remounts controls so native and primitive checked state return to acceptance.
///
/// - Complexity: O(named cells + label/style bytes) per render or edit.
///
/// # Examples
/// Inside a scope initialized by [`use_sheet`],
/// `rsx! { CheckBox { bind: sheet.cell("enabled"), label: "Enabled" } }`.
#[component]
pub fn CheckBox(
    bind: CellBinding,
    #[props(default)] label: String,
    #[props(default)] class: String,
    #[props(default)] style: String,
) -> Element {
    let id = use_control_id(&bind);
    let label = bind.label(label);
    let view = match bind
        .view()
        .and_then(|view| check(&view, Editor::Bool).map(|_| view))
    {
        Ok(view) => view,
        Err(error) => return failure(bind, error),
    };
    let checked = if view.text == "true" {
        CheckboxState::Checked
    } else {
        CheckboxState::Unchecked
    };
    rsx! { div { class, style,
        for bind in std::iter::once(bind) {
            Checkbox { key: "{bind.identity()}", id: id.clone(), checked,
                aria_label: label.clone(), "aria-invalid": view.flags.invalid.to_string(),
                disabled: view.flags.disabled || view.flags.readonly,
                on_checked_change: move |state: CheckboxState| bind.submit((state == CheckboxState::Checked).to_string()),
            }
        }
        Label { html_for: id, "{label}" }
    } }
}
