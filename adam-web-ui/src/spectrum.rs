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
/// and `aria-invalid`). Setting `warning` to `true` (and `invalid` to `false`)
/// renders a softer amber treatment via the `warning` CSS class, styled in
/// `begin/assets/inspector.css` — not a native SWC state. Setting `disabled`
/// to `true` renders the SWC disabled state and blocks focus/input at the DOM
/// level.
#[component]
pub fn SpTextfield(
    id: String,
    value: String,
    invalid: bool,
    warning: bool,
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
            class: if warning { "warning" },
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
/// representation; the element renders and edits it as a number internally. `min`/`max`, when
/// present, are passed through to the underlying SWC element, which natively disables its
/// increment/decrement stepper buttons once the value reaches the corresponding bound.
#[component]
pub fn SpNumberfield(
    id: String,
    value: String,
    min: Option<String>,
    max: Option<String>,
    invalid: bool,
    warning: bool,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
    onfocus: EventHandler<FocusEvent>,
    onblur: EventHandler<FocusEvent>,
) -> Element {
    rsx! {
        sp-number-field {
            "id": "{id}",
            "value": "{value}",
            "min": min.as_deref(),
            "max": max.as_deref(),
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            class: if warning { "warning" },
            oninput: move |e| oninput.call(e),
            onfocus: move |e| onfocus.call(e),
            onblur: move |e| onblur.call(e),
        }
    }
}

/// A draggable range slider for a numeric value with live min/max bounds.
///
/// Maps to `<sp-slider>`. `min`/`max` are passed as strings, recomputed by the caller on every
/// render from the cell's current filter bounds (see `begin/src/bridge.rs`'s `CellMeta::range`),
/// so a range driven by other cells stays live. Fires a standard DOM `input` event; reading the
/// live numeric value off the DOM (not the synthetic event) is the caller's job, mirroring
/// [`SpTextfield`]/[`SpNumberfield`].
#[component]
pub fn SpSlider(
    id: String,
    value: String,
    min: String,
    max: String,
    disabled: bool,
    oninput: EventHandler<FormEvent>,
) -> Element {
    rsx! {
        sp-slider {
            "id": "{id}",
            "value": "{value}",
            "min": "{min}",
            "max": "{max}",
            "disabled": if disabled { "true" },
            oninput: move |e| oninput.call(e),
        }
    }
}

/// Label associated with a form control.
///
/// Maps to `<sp-field-label>`. The `for_` prop sets the `for` HTML attribute
/// linking the label to an input by id.
#[component]
pub fn SpFieldLabel(for_: String, children: Element) -> Element {
    rsx! {
        sp-field-label {
            "for": "{for_}",
            {children}
        }
    }
}

/// Horizontal visual separator.
///
/// Maps to `<sp-divider>` with `size="s"` (small).
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
/// Maps to `<sp-checkbox>`. Setting `invalid` to `true` renders the SWC error state.
/// Setting `warning` to `true` (and `invalid` to `false`) renders a softer amber
/// treatment via the `warning` CSS class, styled in `begin/assets/inspector.css` — not a
/// native SWC state, mirroring `SpTextfield`'s `warning` prop. Setting `disabled` to
/// `true` renders the SWC disabled state. `onclick` fires on every toggle press,
/// mirroring `SpSwitch`'s `checked`/`onclick` pattern — the caller owns the boolean
/// state and re-renders `checked` from it rather than reading the new state off a
/// native `change` event.
#[component]
pub fn SpCheckbox(
    id: String,
    checked: bool,
    invalid: bool,
    warning: bool,
    disabled: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        sp-checkbox {
            "id": "{id}",
            onclick: move |e| onclick.call(e),
            // Boolean attribute: omit entirely when false; presence = checked/invalid/disabled.
            "checked": if checked { "true" },
            "invalid": if invalid { "true" },
            "disabled": if disabled { "true" },
            class: if warning { "warning" },
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
#[component]
pub fn SpIconZoomOut() -> Element {
    rsx! {
        sp-icon-zoom-out {
            "slot": "icon",
        }
    }
}
