//! Dioxus element bindings and component wrappers for Spectrum Web Components.
//!
//! Import with `use crate::spectrum::*;` to bring component wrappers into scope.
//! Callers only need the `SpXxx` component functions.

#![allow(non_snake_case)]

use dioxus::prelude::*;

// ─── Component wrappers ─────────────────────────────────────────────────────
// PascalCase functions are Dioxus components; RSX resolves them via function
// call, not as element bindings. Each wraps one SWC custom element.
//
// Hyphenated identifiers in RSX (e.g. `sp-theme`) are parsed as custom-element
// string literals by the RSX macro (ElementName::Custom), so no element module
// declaration is required — the tag name is emitted verbatim.

/// Provides Spectrum token context for all descendant SWC components.
///
/// Must be the root ancestor of any `SpXxx` component. Maps to `<sp-theme>`.
///
/// - Complexity: O(a) attribute construction for a forwarded string bytes, plus child rendering.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpTheme;
/// # fn example() -> Element {
/// rsx! { SpTheme { color: "light", scale: "medium", system: "spectrum-two", "Content" } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpTheme(color: String, scale: String, system: String, children: Element) -> Element {
    rsx! {
        sp-theme {
            "color": "{color}",
            "scale": "{scale}",
            "system": "{system}",
            {children}
        }
    }
}

/// Single-line text input.
///
/// Maps to `<sp-textfield>`. Fires standard DOM `input`, `focus`, and `blur`
/// events. Setting `invalid` to `true` renders the SWC error state (red ring
/// and `aria-invalid`). Setting `disabled` to `true` renders the SWC disabled
/// state and blocks focus/input at the DOM level.
///
/// - Complexity: O(id.len() + value.len()) attribute construction.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpTextfield;
/// # fn example() -> Element {
/// rsx! { SpTextfield { id: "title", value: "Hello", invalid: false,
///     disabled: false, oninput: |_| {}, onfocus: |_| {}, onblur: |_| {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpTextfield(
    id: String,
    value: String,
    invalid: bool,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
) -> Element {
    rsx! {
        sp-textfield {
            "id": "{id}",
            "value": "{value}",
            // Boolean attribute: omit entirely when false; presence = invalid.
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
        }
    }
}

/// Single-line numeric input.
///
/// Maps to `<sp-number-field>`. Fires standard DOM `input`, `focus`, and `blur` events, exactly
/// like [`SpTextfield`] — including the same custom-element caveat: Dioxus's event serializer
/// never populates `event.target.value` for a custom element, so reading the live value off the
/// DOM (not the synthetic event) is the caller's job. `value` is passed as its string
/// representation; the element renders and edits it as a number internally. The field fills its
/// container's inline size, leaving room for formatted values and validation icons. `min`/`max`, when
/// present, are passed through to the underlying SWC element, which natively disables its
/// increment/decrement stepper buttons once the value reaches the corresponding bound. Setting
/// `readonly` to `true` renders the SWC read-only state — the value stays visible and
/// selectable, but not editable — distinct from `disabled`, which additionally grays the field
/// out and suppresses the `invalid` treatment (the input's border) entirely; `readonly` leaves
/// `invalid`'s border alone but SWC's own `:host([readonly]) .input` rule pins the border color
/// to transparent regardless, so on a `readonly` field the border itself never visibly reflects
/// `invalid` either — confirmed against the rendered shadow-DOM stylesheet via the
/// verifying-begin-ui skill; there is no documented or supported `readonly`+`invalid` border
/// treatment in Spectrum. `invalid` still matters on a `readonly` field for two reasons that
/// don't depend on the border: it sets `aria-invalid` for assistive tech, and it's exactly what
/// gates whether SWC exposes the `negative-help-text` slot at all (see `children`) — so pass a
/// slotted [`SpHelpText`] there rather than inventing a non-standard visual override to
/// compensate for the missing border. `step`, when present, sets the amount the stepper
/// buttons/arrow keys/scroll wheel move the value by — pass `"1"` for an integer-typed cell so
/// the stepper can never land it on a fractional value. `format_options` supplies JSON-encoded
/// `Intl.NumberFormat` options for locale-aware display without changing the stored value.
/// `children` slots into the element's light DOM, e.g. an [`SpHelpText`] with
/// `slot: "negative-help-text"` or `slot: "help-text"`.
///
/// - Complexity: O(a) attribute construction for a forwarded string bytes, plus child rendering.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpNumberfield;
/// # fn example() -> Element {
/// rsx! { SpNumberfield { id: "count", value: "42", step: "1",
///     invalid: false, disabled: false, readonly: false,
///     oninput: |_| {}, onfocus: |_| {}, onblur: |_| {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpNumberfield(
    id: String,
    value: String,
    min: Option<String>,
    max: Option<String>,
    step: Option<String>,
    format_options: Option<&'static str>,
    invalid: bool,
    disabled: bool,
    readonly: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
    #[props(default)] children: Element,
) -> Element {
    rsx! {
        sp-number-field {
            style: "inline-size: 100%;",
            "id": "{id}",
            "value": "{value}",
            "min": min.as_deref(),
            "max": max.as_deref(),
            "step": step.as_deref(),
            "format-options": format_options.unwrap_or("{}"),
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            "readonly": if readonly { "true" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
            {children}
        }
    }
}

/// A draggable slider for a numeric value with live min/max bounds, its own built-in label,
/// and an optional inline editable number field.
///
/// Maps to `<sp-slider>`. `label` renders as part of the slider's own markup (its internal
/// `sp-field-label`, linked to the handle or, when `editable` is set, to the paired
/// `sp-number-field`) — a caller using this component does not also render a separate
/// [`SpFieldLabel`] alongside it. `min`/`max` are passed as strings, recomputed by the caller on
/// every render from the cell's current filter bounds (see `adam-web-ui/src/labels.rs`'s
/// `CellMeta::range`), so a range driven by other cells stays live. `step`, when present, sets
/// the amount the drag handle/stepper/arrow keys move the value by — pass `"1"` for an
/// integer-typed cell, mirroring [`SpNumberfield`]'s own `step`, or `"0.01"` for a floating-point
/// cell displayed to two decimal places. Omitting `step` leaves Spectrum's default of one,
/// which also snaps the inline number field to whole numbers. Setting `editable` to `true`
/// renders an internal `sp-number-field` alongside the track (SWC's own `Slider.render`),
/// letting the user type an exact value instead of only dragging; both the drag handle and the
/// editable field dispatch the same DOM `input` event on this element (SWC's
/// `SliderHandle.dispatchInputEvent`), so a single `oninput` handler here covers both — reading
/// the live numeric value off the DOM (not the synthetic event) is the caller's job, mirroring
/// [`SpTextfield`]/[`SpNumberfield`]. `sp-slider` has no `invalid`/`readonly` state; a caller
/// that needs to surface either (e.g. a failing `require` on a range-filtered out cell) has to
/// render that separately, alongside this component. `onfocus`/`onblur` fire for both the drag
/// handle and (when `editable` is set) the inline number field — neither SWC handle re-dispatches
/// a bubbling event, but DOM `focus`/`blur` are `composed`, so a listener on this host element
/// still observes them across the intervening shadow boundaries, mirroring
/// [`SpTextfield`]/[`SpNumberfield`]. `format_options` supplies JSON-encoded `Intl.NumberFormat`
/// options for both the value label and the inline number field without changing the stored value.
/// Empty inline edits remain uncommitted rather than restoring the slider's last numeric value.
/// A mounted slider captures unmanaged inline edits after Spectrum parses them, stores the raw
/// localized text (including blanks) in `adamNumericDraft`, and redispatches a host `input` event.
/// The Inspector's numeric input bridge consumes and deletes that draft once; handle drags instead
/// read the canonical host value. Empty inline `change` events are suppressed to preserve blanks.
///
/// - Complexity: O(a) attribute/bridge construction for a forwarded string bytes.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpSlider;
/// # fn example() -> Element {
/// rsx! { SpSlider { id: "volume", label: "Volume", value: "50",
///     min: "0", max: "100", editable: true, disabled: false,
///     oninput: |_| {}, onfocus: |_| {}, onblur: |_| {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpSlider(
    id: String,
    label: String,
    value: String,
    min: String,
    max: String,
    step: Option<String>,
    format_options: Option<&'static str>,
    editable: bool,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
) -> Element {
    rsx! {
        sp-slider {
            "id": "{id}",
            "label": "{label}",
            "value": "{value}",
            "min": "{min}",
            "max": "{max}",
            "step": step.as_deref(),
            "format-options": format_options.unwrap_or("{}"),
            "editable": if editable { "true" },
            "disabled": if disabled { "true" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
            onmounted: move |_| {
                let script = format!(
                    r#"
                    {bridge}
                    installNumericSliderBridge(document.getElementById({id:?}));
                    "#,
                    bridge = include_str!("numeric_input.js"),
                );
                document::eval(&script);
            },
        }
    }
}

/// Label associated with a form control.
///
/// Maps to `<sp-field-label>`. The `for_` prop sets the `for` HTML attribute
/// linking the label to an input by id.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpFieldLabel;
/// let _ = rsx! { SpFieldLabel { for_: "count", "Count" } };
/// ```
#[component]
///
/// - Complexity: O(for_.len()) attribute construction, plus child rendering.
pub fn SpFieldLabel(for_: String, children: Element) -> Element {
    rsx! {
        sp-field-label {
            "for": "{for_}",
            {children}
        }
    }
}

/// Descriptive or validation text associated with a form control.
///
/// Maps to `<sp-help-text>`. Meaningless on its own — a form control like
/// [`SpNumberfield`] renders it only when passed as that control's `children`, with
/// `slot` set to one of the two slot names the control's own shadow DOM exposes:
/// `"help-text"` for default/non-negative text, always rendered when present, or
/// `"negative-help-text"` for text shown only while the control's own `invalid` prop is
/// `true` (confirmed against the bundled SWC source's `HelpTextManager.render`, which
/// switches which slot it exposes based on exactly that prop — the control ignores an
/// `sp-help-text` slotted into `"negative-help-text"` whenever `invalid` is `false`).
/// `variant` mirrors the element's own `variant` property (`"negative"` renders the
/// error/red treatment); note that slotting into `"negative-help-text"` sets this to
/// `"negative"` automatically on the control's own side, so passing it here is
/// belt-and-suspenders, not load-bearing.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpHelpText;
/// let _ = rsx! { SpHelpText { slot: "negative-help-text", variant: "negative", "Invalid value" } };
/// ```
#[component]
///
/// - Complexity: O(slot.len() + variant.len()) attribute construction, plus child rendering.
pub fn SpHelpText(slot: String, variant: String, children: Element) -> Element {
    rsx! {
        sp-help-text {
            "slot": "{slot}",
            "variant": "{variant}",
            {children}
        }
    }
}

/// Horizontal visual separator.
///
/// Maps to `<sp-divider>` with `size="s"` (small).
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpDivider;
/// let _ = rsx! { SpDivider {} };
/// ```
#[component]
pub fn SpDivider() -> Element {
    rsx! {
        sp-divider {
            "size": "s",
        }
    }
}

/// Section heading.
///
/// Maps to `<sp-heading>`.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpHeading;
/// let _ = rsx! { SpHeading { "Cells" } };
/// ```
#[component]
pub fn SpHeading(children: Element) -> Element {
    rsx! {
        sp-heading {
            {children}
        }
    }
}

/// Groups a row of `SpActionButton`s into a single visual cluster.
///
/// Maps to `<sp-action-group>`. Setting `compact` to `true` removes the gaps between
/// buttons and rounds only the group's outermost corners — interior buttons (including
/// a lone middle button) render square on both sides.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::{SpActionButton, SpActionGroup};
/// # fn example() -> Element {
/// rsx! { SpActionGroup { compact: true, SpActionButton { onclick: |_| {}, "Fit" } } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpActionGroup(compact: bool, children: Element) -> Element {
    rsx! {
        sp-action-group {
            // Boolean attribute: omit entirely when false; presence = compact.
            "compact": if compact { "true" },
            {children}
        }
    }
}

/// A single button within an `SpActionGroup` (or standalone).
///
/// Maps to `<sp-action-button>`. `selected` renders it in its pressed/active
/// visual state (e.g. to highlight the current choice in a group of buttons
/// used as a picker).
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpActionButton;
/// # fn example() -> Element {
/// rsx! { SpActionButton { onclick: |_| {}, selected: true, "Select" } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpActionButton(
    onclick: EventHandler<MouseEvent>,
    #[props(default)] selected: bool,
    children: Element,
) -> Element {
    rsx! {
        sp-action-button {
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = selected.
            "selected": if selected { "true" },
            {children}
        }
    }
}

/// A labeled on/off toggle.
///
/// Maps to `<sp-switch>`; slotted `children` render as its label. `checked`
/// renders its current state; `onclick` fires on every toggle press,
/// mirroring `SpActionButton`'s `selected`/`onclick` pattern (the caller owns
/// the boolean state and re-renders `checked` from it) rather than reading
/// the new state back off a native `change` event.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpSwitch;
/// # fn example() -> Element {
/// rsx! { SpSwitch { checked: true, onclick: |_| {}, "Show inactive" } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpSwitch(checked: bool, onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    rsx! {
        sp-switch {
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = checked.
            "checked": if checked { "true" },
            {children}
        }
    }
}

/// A togglable checkbox for a `bool`-typed value.
///
/// Maps to `<sp-checkbox>`. `children` is the checkbox's own label, rendered as its
/// light-DOM content (`<sp-checkbox>label</sp-checkbox>`) — SWC gives `sp-checkbox` no
/// separate label slot, so a sibling field label would render disconnected from the
/// control. Setting `invalid` to `true` renders the SWC error state. Setting `disabled`
/// to `true` renders the SWC disabled state. `onclick` fires on every toggle press,
/// mirroring `SpSwitch`'s `checked`/`onclick` pattern — the caller owns the boolean
/// state and re-renders `checked` from it rather than reading the new state off a
/// native `change` event.
///
/// - Complexity: O(id.len()) attribute construction, plus child rendering.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpCheckbox;
/// # fn example() -> Element {
/// rsx! { SpCheckbox { id: "enabled", checked: true, invalid: false,
///     disabled: false, onclick: |_| {}, "Enabled" } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpCheckbox(
    id: String,
    checked: bool,
    invalid: bool,
    disabled: bool,
    onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    rsx! {
        sp-checkbox {
            "id": "{id}",
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = checked/invalid/disabled.
            "checked": if checked { "true" },
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            {children}
        }
    }
}

/// A scrollable, selectable vertical list of items — used as the examples
/// picker sidebar.
///
/// Maps to `<sp-sidenav>`. `value` must equal the `value` of whichever child
/// `SpSideNavItem` should render selected. This is load-bearing, not
/// cosmetic: `<sp-sidenav-item>` derives its own selected state by comparing
/// its `value` against its parent's once per connect (SWC's
/// `SidenavItem.startTrackingSelection`), silently clearing any `selected`
/// attribute set directly on an item whose `value` doesn't match — confirmed
/// by inspecting the bundled `assets/swc.js` source and the rendered DOM.
///
/// - Complexity: O(value.len()) attribute construction, plus child rendering.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::{SpSideNav, SpSideNavItem};
/// # fn example() -> Element {
/// rsx! { SpSideNav { value: "first",
///     SpSideNavItem { label: "First", value: "first", selected: true, onclick: |_| {} }
/// } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpSideNav(value: String, children: Element) -> Element {
    rsx! {
        sp-sidenav {
            "value": "{value}",
            {children}
        }
    }
}

/// A single item within an `SpSideNav`.
///
/// Maps to `<sp-sidenav-item>`. `label` sets the item's visible text (via the
/// element's `label` attribute, not slotted content). `value` must be unique
/// among sibling items; pass the parent `SpSideNav`'s `value` here exactly
/// when this item should render selected — see `SpSideNav`'s doc comment for
/// why a matching `value` (not just `selected`) is required. `selected`
/// renders it in its highlighted/active state (e.g. to mark the current
/// choice in a list used as a picker); pass the same condition used to
/// decide `SpSideNav`'s `value`, so the attribute is already correct on the
/// very first render rather than only after a later selection change.
///
/// - Complexity: O(label.len() + value.len()) attribute construction.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::SpSideNavItem;
/// # fn example() -> Element {
/// rsx! { SpSideNavItem { label: "First", value: "first", onclick: |_| {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpSideNavItem(
    label: String,
    value: String,
    onclick: EventHandler<MouseEvent>,
    #[props(default)] selected: bool,
) -> Element {
    rsx! {
        sp-sidenav-item {
            "label": "{label}",
            "value": "{value}",
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = selected.
            "selected": if selected { "true" },
        }
    }
}

/// Zoom-in glyph, used as `SpActionButton` icon content.
///
/// Maps to `<sp-icon-zoom-in>`, assigned to the button's `icon` slot so
/// `ActionButton` centers and sizes it like any other action-button icon
/// instead of treating it as label text. Rendered via `dangerous_inner_html`
/// on a wrapping `span` rather than the usual `sp-icon-zoom-in {}` RSX element
/// syntax: dioxus-rsx 0.7.9 reconstructs hyphenated custom-element tag names
/// by joining each `-`-separated segment's `Ident::to_string()`, and a segment
/// matching a Rust keyword (`in`) parses as a raw identifier whose
/// `to_string()` includes the `r#` prefix, corrupting the tag to
/// `sp-icon-zoom-r#in`.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::{SpActionButton, SpIconZoomIn};
/// # fn example() -> Element {
/// rsx! { SpActionButton { onclick: |_| {}, SpIconZoomIn {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpIconZoomIn() -> Element {
    rsx! {
        span {
            "slot": "icon",
            dangerous_inner_html: "<sp-icon-zoom-in></sp-icon-zoom-in>",
        }
    }
}

/// Zoom-out glyph, used as `SpActionButton` icon content.
///
/// Maps to `<sp-icon-zoom-out>`, assigned to the button's `icon` slot so
/// `ActionButton` centers and sizes it like any other action-button icon
/// instead of treating it as label text.
///
/// # Examples
///
/// ```
/// use dioxus::prelude::*;
/// use adam_web_ui::spectrum::{SpActionButton, SpIconZoomOut};
/// # fn example() -> Element {
/// rsx! { SpActionButton { onclick: |_| {}, SpIconZoomOut {} } }
/// # }
/// # VirtualDom::new(example).rebuild_in_place();
/// ```
#[component]
pub fn SpIconZoomOut() -> Element {
    rsx! {
        sp-icon-zoom-out {
            "slot": "icon",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    /// Renders every wrapper with shared event observers and explicit optional props.
    fn host((enabled, events): (bool, Rc<RefCell<Vec<&'static str>>>)) -> Element {
        let input_events = events.clone();
        let input = EventHandler::new(move |event: FormEvent| {
            assert_eq!(event.value(), "draft");
            input_events.borrow_mut().push("input");
        });
        let focus_events = events.clone();
        let focus = EventHandler::new(move |_: FocusEvent| focus_events.borrow_mut().push("focus"));
        let blur_events = events.clone();
        let blur = EventHandler::new(move |_: FocusEvent| blur_events.borrow_mut().push("blur"));
        let click = EventHandler::new(move |_: MouseEvent| events.borrow_mut().push("click"));
        rsx! {
            SpTheme { color: "dark", scale: "large", system: "spectrum-two",
                SpHeading { "Contract heading" }
                SpDivider {}
                SpFieldLabel { for_: "text", "Text label" }
                SpTextfield { id: "text", value: "draft", invalid: enabled, disabled: enabled, oninput: input, onfocus: focus, onblur: blur }
                SpNumberfield { id: "number", value: "12", min: enabled.then(|| "-2".to_string()), max: enabled.then(|| "20".to_string()), step: enabled.then(|| "2".to_string()), format_options: Some(r#"{"maximumFractionDigits":0}"#), invalid: enabled, disabled: enabled, readonly: enabled, oninput: input, onfocus: focus, onblur: blur,
                    SpHelpText { slot: "negative-help-text", variant: "negative", "Bound violated" }
                }
                SpSlider { id: "slider", label: "Amount", value: "3", min: "0", max: "10", step: None, format_options: None, editable: enabled, disabled: enabled, oninput: input, onfocus: focus, onblur: blur }
                SpActionGroup { compact: enabled,
                    SpActionButton { selected: enabled, onclick: click, SpIconZoomIn {} SpIconZoomOut {} "Zoom" }
                }
                SpSwitch { checked: enabled, onclick: click, "Switch label" }
                SpCheckbox { id: "check", checked: enabled, invalid: enabled, disabled: enabled, onclick: click, "Check label" }
                SpSideNav { value: "chosen",
                    SpSideNavItem { label: "Chosen label", value: "chosen", selected: enabled, onclick: click }
                }
            }
        }
    }

    /// Returns one rendered opening tag for semantic attribute assertions.
    ///
    /// - Complexity: O(html.len() + name.len()) search work and O(name.len()) scratch space.
    fn tag<'a>(html: &'a str, name: &str) -> &'a str {
        html.split_once(&format!("<{name}"))
            .unwrap()
            .1
            .split_once('>')
            .unwrap()
            .0
    }

    /// Every wrapper preserves its element, content, and optional/boolean attributes.
    #[test]
    fn wrappers_forward_content_and_attributes() {
        for enabled in [false, true] {
            let mut dom =
                VirtualDom::new_with_props(host, (enabled, Rc::new(RefCell::new(Vec::new()))));
            dom.rebuild_in_place();
            let html = dioxus_ssr::render(&dom);
            assert!(tag(&html, "sp-theme").contains(r#"color="dark""#));
            assert!(tag(&html, "sp-theme").contains(r#"scale="large""#));
            assert!(tag(&html, "sp-theme").contains(r#"system="spectrum-two""#));
            assert!(html.contains("<sp-heading>Contract heading</sp-heading>"));
            assert!(tag(&html, "sp-divider").contains(r#"size="s""#));
            assert!(tag(&html, "sp-field-label").contains(r#"for="text""#));
            assert!(tag(&html, "sp-textfield").contains(r#"value="draft""#));
            assert!(tag(&html, "sp-number-field").contains(r#"value="12""#));
            assert!(tag(&html, "sp-number-field").contains("format-options="));
            assert_eq!(tag(&html, "sp-number-field").contains("min="), enabled);
            assert_eq!(tag(&html, "sp-number-field").contains("max="), enabled);
            assert_eq!(tag(&html, "sp-number-field").contains("step="), enabled);
            assert!(tag(&html, "sp-help-text").contains(r#"slot="negative-help-text""#));
            assert!(html.contains("Bound violated</sp-help-text>"));
            assert!(tag(&html, "sp-slider").contains(r#"label="Amount""#));
            assert!(!tag(&html, "sp-slider").contains("step="));
            assert!(tag(&html, "sp-slider").contains(r#"min="0""#));
            assert!(tag(&html, "sp-slider").contains(r#"max="10""#));
            assert!(html.contains("<sp-icon-zoom-in></sp-icon-zoom-in>"));
            assert!(tag(&html, "span").contains(r#"slot="icon""#));
            assert!(tag(&html, "sp-icon-zoom-out").contains(r#"slot="icon""#));
            assert!(html.contains("Switch label</sp-switch>"));
            assert!(html.contains("Check label</sp-checkbox>"));
            assert!(tag(&html, "sp-sidenav").contains(r#"value="chosen""#));
            assert!(tag(&html, "sp-sidenav-item").contains(r#"label="Chosen label""#));
            for (element, attribute) in [
                ("sp-textfield", "invalid"),
                ("sp-textfield", "disabled"),
                ("sp-number-field", "invalid"),
                ("sp-number-field", "disabled"),
                ("sp-number-field", "readonly"),
                ("sp-slider", "editable"),
                ("sp-slider", "disabled"),
                ("sp-action-group", "compact"),
                ("sp-action-button", "selected"),
                ("sp-switch", "checked"),
                ("sp-checkbox", "checked"),
                ("sp-checkbox", "invalid"),
                ("sp-checkbox", "disabled"),
                ("sp-sidenav-item", "selected"),
            ] {
                assert_eq!(
                    tag(&html, element).contains(&format!("{attribute}=")),
                    enabled,
                    "{element}.{attribute}"
                );
            }
        }
    }

    /// Real Dioxus event dispatch reaches the caller through each interactive wrapper.
    #[test]
    fn wrappers_forward_events() {
        dioxus_html::set_event_converter(Box::new(dioxus_html::SerializedHtmlEventConverter));
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(host, (false, events.clone()));
        let mutations = dom.rebuild_to_vec();
        for edit in mutations.edits {
            let dioxus::core::Mutation::NewEventListener { name, id } = edit else {
                continue;
            };
            let payload: Box<dyn std::any::Any> = match name.as_str() {
                "click" => Box::<dioxus_html::SerializedMouseData>::default(),
                "input" => Box::new(dioxus_html::SerializedFormData::new(
                    "draft".to_string(),
                    vec![],
                )),
                "focus" | "blur" => Box::<dioxus_html::SerializedFocusData>::default(),
                _ => continue,
            };
            dom.runtime().handle_event(
                &name,
                Event::new(Rc::new(dioxus_html::PlatformEventData::new(payload)), true).into_any(),
                id,
            );
        }
        let events = events.borrow();
        for (name, count) in [("click", 4), ("input", 3), ("focus", 3), ("blur", 3)] {
            assert_eq!(
                events.iter().filter(|event| **event == name).count(),
                count,
                "{name}"
            );
        }
    }
}
