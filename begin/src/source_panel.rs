//! [`SourcePanel`] — collapsible bottom panel for editing and applying pm-lang source.

use annotate_snippets::Renderer;
use dioxus::prelude::*;
use pm_lang::{PmParser, TypeRegistry};
use property_model::Sheet;

use crate::bridge::{Labels, format_property_model_error, labels_from_cell_names};

/// The result of parsing and building a sheet from pm-lang source.
///
/// `sheet_labels` is `None` only on parse failure. A successful parse that
/// then fails to propagate still returns the built sheet and labels alongside
/// the formatted error, matching how the Inspector already tolerates
/// propagate failures during cell edits.
pub struct BuildOutcome {
    /// The built sheet and its UI labels, if parsing succeeded.
    pub sheet_labels: Option<(Sheet, Labels)>,
    /// A formatted rustc-style diagnostic, if parsing or propagation failed.
    pub error: Option<String>,
}

/// Parses `source` as pm-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str) -> BuildOutcome {
    let mut parser = PmParser::new(TypeRegistry::new(), cel_parser::OpLookup::new());
    let mut parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.format_rustc_style(source, "<pm-lang source>", 1, &Renderer::plain());
            return BuildOutcome {
                sheet_labels: None,
                error: Some(msg),
            };
        }
    };
    let labels = labels_from_cell_names(&parsed.cell_names);
    match parsed.propagate() {
        Ok(()) => {
            parsed.clear_changed();
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: None,
            }
        }
        Err(e) => {
            let msg = format_property_model_error(&e, source);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
            }
        }
    }
}

/// Collapsible bottom panel: a pm-lang source textarea, an Apply button, and
/// a rustc-style diagnostic for the most recent parse or runtime failure.
///
/// Clicking Apply parses `editor_source`, builds a new sheet and labels via
/// [`build_sheet`], and — on success or on a runtime (propagate) failure —
/// replaces `sheet`/`labels` and updates `applied_source` to match. On a
/// parse failure, `sheet`/`labels` are left unchanged.
#[component]
pub fn SourcePanel(
    editor_source: Signal<String>,
    applied_source: Signal<String>,
    sheet: Signal<Sheet>,
    labels: Signal<Labels>,
    error: Signal<Option<String>>,
    open: Signal<bool>,
) -> Element {
    let mut editor_source = editor_source;
    let mut applied_source = applied_source;
    let mut sheet = sheet;
    let mut labels = labels;
    let mut error = error;
    let mut open = open;

    rsx! {
        div {
            style: "border-top: 1px solid #ccc; display: flex; flex-direction: column; flex-shrink: 0;",
            div {
                style: "display: flex; align-items: center; gap: 8px; padding: 4px 8px;",
                button {
                    onclick: move |_| open.toggle(),
                    if *open.read() { "▼ Source" } else { "▶ Source" }
                }
                if *open.read() {
                    button {
                        onclick: move |_| {
                            let source = editor_source.read().clone();
                            applied_source.set(source.clone());
                            let outcome = build_sheet(&source);
                            if let Some((new_sheet, new_labels)) = outcome.sheet_labels {
                                sheet.set(new_sheet);
                                labels.set(new_labels);
                            }
                            error.set(outcome.error);
                        },
                        "Apply"
                    }
                }
            }
            if *open.read() {
                textarea {
                    style: "width: 100%; height: 160px; font-family: monospace; box-sizing: border-box; margin: 0; border: none; border-top: 1px solid #ccc;",
                    value: "{editor_source}",
                    oninput: move |evt: FormEvent| editor_source.set(evt.value()),
                }
                if let Some(msg) = error.read().as_ref() {
                    pre {
                        style: "margin: 0; padding: 8px; background: #fee; color: #900; overflow: auto; max-height: 200px; white-space: pre-wrap; font-family: monospace;",
                        "{msg}"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SOURCE: &str = r#"
        sheet s {
            cell a: f64 = 2.0;
            cell b: f64 = 3.0;
            cell c: f64;
            relationship {
                method [a, b] -> [c] { a * b }
                method [b, c] -> [a] { c / b }
                method [a, c] -> [b] { c / a }
            }
        }
    "#;

    #[test]
    fn build_sheet_valid_source_succeeds_with_no_error() {
        let outcome = build_sheet(VALID_SOURCE);
        assert!(outcome.sheet_labels.is_some());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn build_sheet_parse_error_has_no_sheet_and_formatted_message() {
        let outcome = build_sheet("sheet s { cell x }");
        assert!(outcome.sheet_labels.is_none());
        let msg = outcome.error.expect("expected a parse error message");
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    fn build_sheet_runtime_error_still_returns_sheet_and_message() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { method [x] -> [y] { 10i32 / x } } }";
        let outcome = build_sheet(source);
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }
}
