//! Parses adam-lang source into a live [`adam_rs::Sheet`], formatting any failure as a
//! diagnostic instead of a bare error.

use crate::labels::{Labels, format_adam_error, labels_from_cell_names};
use adam_lang::{AdamParser, TypeRegistry};
use adam_rs::Sheet;
use annotate_snippets::Renderer;

/// The result of parsing and building a sheet from adam-lang source.
///
/// `sheet_labels` is `None` only on parse failure. A successful parse that
/// then fails to propagate still returns the built sheet and labels alongside
/// the formatted error, matching how [`crate::SheetInspector`] already tolerates
/// propagate failures during cell edits.
pub struct BuildOutcome {
    /// The built sheet and its UI labels, if parsing succeeded.
    pub sheet_labels: Option<(Sheet, Labels)>,
    /// A formatted rustc-style diagnostic, if parsing or propagation failed.
    pub error: Option<String>,
}

/// Builds a [`cel_parser::OpLookup`] with the CEL standard library installed, so every
/// adam-lang source [`build_sheet`] parses has the same function set (`min`, `max`, `clamp`,
/// etc.) available.
///
/// Every source parsed through this function therefore has `cel-std` installed — a source
/// that deliberately relies on the standard library being *absent* cannot use this function
/// or [`build_sheet`]; construct an `AdamParser` directly with an empty `OpLookup` instead.
pub fn op_lookup() -> cel_parser::OpLookup {
    let mut lookup = cel_parser::OpLookup::new();
    cel_std::install(&mut lookup);
    lookup
}

/// Parses `source` as adam-lang, builds a `Sheet` and `Labels`, and propagates
/// once so initial derived values are populated. `file_name` is used only to
/// build diagnostic headers (e.g. `--> begin/examples/toy_example.adm2:8:11`),
/// not to locate `source` itself.
///
/// - Complexity: O(n) in the length of `source` plus the cost of one `propagate()`.
pub fn build_sheet(source: &str, file_name: &str) -> BuildOutcome {
    let mut parser = AdamParser::new(TypeRegistry::new(), op_lookup());
    let mut parsed = match parser.parse_str(source) {
        Ok(p) => p,
        Err(e) => {
            let msg = e.format_rustc_style(source, file_name, 1, &Renderer::styled());
            return BuildOutcome {
                sheet_labels: None,
                error: Some(msg),
            };
        }
    };
    let labels = labels_from_cell_names(&parsed.sheet, &parsed.cell_names);
    match parsed.propagate() {
        Ok(()) => {
            parsed.clear_changed();
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: None,
            }
        }
        Err(e) => {
            let msg = format_adam_error(&e, source, file_name);
            BuildOutcome {
                sheet_labels: Some((parsed.sheet, labels)),
                error: Some(msg),
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
                c := a * b;
                a := c / b;
                b := c / a;
            }
        }
    "#;

    #[test]
    fn build_sheet_valid_source_succeeds_with_no_error() {
        let outcome = build_sheet(VALID_SOURCE, "test.adm2");
        assert!(outcome.sheet_labels.is_some());
        assert!(outcome.error.is_none());
    }

    #[test]
    fn build_sheet_parse_error_has_no_sheet_and_formatted_message() {
        let outcome = build_sheet("sheet s { cell x }", "test.adm2");
        assert!(outcome.sheet_labels.is_none());
        let msg = outcome.error.expect("expected a parse error message");
        assert!(msg.contains("error"), "{msg}");
    }

    #[test]
    fn build_sheet_runtime_error_still_returns_sheet_and_message() {
        let source = "sheet s { cell x: i32 = 0; cell y: i32; relationship { y := 10i32 / x; } }";
        let outcome = build_sheet(source, "test.adm2");
        assert!(
            outcome.sheet_labels.is_some(),
            "sheet should still be built after a propagate error"
        );
        assert!(outcome.error.is_some());
    }
}
