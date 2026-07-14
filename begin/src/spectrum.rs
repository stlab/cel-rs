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
/// and `aria-invalid`). Setting `disabled` to `true` renders the SWC disabled
/// state and blocks focus/input at the DOM level.
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
/// Maps to `<sp-action-button>`.
#[component]
pub fn SpActionButton(onclick: EventHandler<MouseEvent>, children: Element) -> Element {
    rsx! {
        sp-action-button {
            onclick: move |e| onclick.call(e),
            {children}
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
