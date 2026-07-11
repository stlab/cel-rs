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
