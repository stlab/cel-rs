//! Context-sensitive side panel for property and formula editing. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §5.

use crate::model::cell::{CellId, CellType};
use crate::model::document::Document;
use crate::ops::cells::{set_output, set_restrict};
use dioxus::prelude::*;

/// Returns `(min_text, max_text)` for `ty`'s clamp bounds if it's numeric,
/// or `None` if `ty` is `Bool`/`Text` (no clamp fields to show). Each
/// bound renders as an empty string when unset, matching an editable
/// text-field's natural "nothing typed" state.
pub fn clamp_bounds_text(ty: &CellType) -> Option<(String, String)> {
    match ty {
        CellType::F64 { clamp } => Some((
            clamp.min.map(|v| v.to_string()).unwrap_or_default(),
            clamp.max.map(|v| v.to_string()).unwrap_or_default(),
        )),
        CellType::I64 { clamp } => Some((
            clamp.min.map(|v| v.to_string()).unwrap_or_default(),
            clamp.max.map(|v| v.to_string()).unwrap_or_default(),
        )),
        CellType::Bool | CellType::Text => None,
    }
}

/// Parses a restrict expression input field: empty string becomes `None`,
/// non-empty becomes `Some(text)`.
fn parse_restrict_input(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

/// Renders `cell`'s editable properties: name, type (read-only — changing
/// a cell's type is out of scope for this plan), output checkbox, clamp
/// min/max (numeric types only, via [`clamp_bounds_text`]), and restrict
/// expression text.
#[component]
pub fn CellPanel(mut document: Signal<Document>, cell: CellId) -> Element {
    let (name, ty, output, restrict) = {
        let doc = document.read();
        let c = &doc.cells[cell];
        (
            c.name.clone(),
            c.ty.clone(),
            c.output,
            c.restrict.clone().unwrap_or_default(),
        )
    };
    let bounds = clamp_bounds_text(&ty);

    rsx! {
        div {
            class: "cell-panel",
            div { "Name: {name}" }
            label {
                input {
                    r#type: "checkbox",
                    checked: output,
                    onchange: move |evt| set_output(&mut document.write(), cell, evt.checked()),
                }
                "Output"
            }
            if let Some((min, max)) = bounds {
                div {
                    "Clamp min: "
                    input { value: "{min}" }
                    " max: "
                    input { value: "{max}" }
                }
            }
            div {
                "Restrict: "
                input {
                    value: "{restrict}",
                    onchange: move |evt| {
                        let text = evt.value();
                        set_restrict(&mut document.write(), cell, parse_restrict_input(text));
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cell::ClampRange;

    #[test]
    fn clamp_bounds_text_is_none_for_bool_and_text() {
        assert_eq!(clamp_bounds_text(&CellType::Bool), None);
        assert_eq!(clamp_bounds_text(&CellType::Text), None);
    }

    #[test]
    fn clamp_bounds_text_shows_set_bounds() {
        let ty = CellType::I64 {
            clamp: ClampRange {
                min: Some(0),
                max: Some(100),
            },
        };
        assert_eq!(
            clamp_bounds_text(&ty),
            Some(("0".to_string(), "100".to_string()))
        );
    }

    #[test]
    fn clamp_bounds_text_shows_empty_string_for_unset_bounds() {
        let ty = CellType::F64 {
            clamp: ClampRange {
                min: None,
                max: None,
            },
        };
        assert_eq!(clamp_bounds_text(&ty), Some((String::new(), String::new())));
    }

    #[test]
    fn parse_restrict_input_converts_empty_to_none() {
        assert_eq!(parse_restrict_input(String::new()), None);
    }

    #[test]
    fn parse_restrict_input_converts_nonempty_to_some() {
        assert_eq!(
            parse_restrict_input("x > 0".to_string()),
            Some("x > 0".to_string())
        );
    }
}
